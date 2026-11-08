//! Bounded BEP 52 hash messages, independent of the v1 wire parser.
use crate::v2::{BLOCK, FileTree, Hash, parent, root, zero};
use anyhow::{Context, Result, bail, ensure};
use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const MAX_FRAME: usize = 53 + (512 + 32) * 32;

// A peer must not create an unbounded blocking disk worker for each hash request.
// The permit also covers cancellation/join, across all torrents in this process.
pub async fn hash_worker() -> Result<tokio::sync::OwnedSemaphorePermit> {
    static WORKERS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    Ok(WORKERS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .acquire_owned()
        .await?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub root: Hash,
    pub base: u32,
    pub index: u32,
    pub length: u32,
    pub proof: u32,
}
impl Request {
    pub fn parse(frame: &[u8]) -> Result<Self> {
        ensure!(frame.len() >= 53, "Truncated hash message");
        let num = |start: usize| u32::from_be_bytes(frame[start..start + 4].try_into().unwrap());
        let request = Self {
            root: frame[5..37].try_into().unwrap(),
            base: num(37),
            index: num(41),
            length: num(45),
            proof: num(49),
        };
        ensure!(
            (2..=512).contains(&request.length)
                && request.length.is_power_of_two()
                && request.index % request.length == 0
                && request.base <= 32
                && request.proof <= 32,
            "Invalid or excessive hash request"
        );
        ensure!(
            request.index.checked_add(request.length).is_some(),
            "Hash request range overflow"
        );
        Ok(request)
    }
    pub fn frame(self, id: u8, hashes: &[Hash]) -> Bytes {
        let mut b = Vec::with_capacity(53 + hashes.len() * 32);
        b.extend_from_slice(&(49 + hashes.len() as u32 * 32).to_be_bytes());
        b.push(id);
        b.extend_from_slice(&self.root);
        for n in [self.base, self.index, self.length, self.proof] {
            b.extend_from_slice(&n.to_be_bytes());
        }
        for hash in hashes {
            b.extend_from_slice(hash);
        }
        b.into()
    }
    fn proof_count(self) -> usize {
        self.proof.saturating_sub(self.length.ilog2() - 1) as usize
    }
    pub fn verify(self, file: &FileTree, frame: &[u8]) -> Result<Vec<Hash>> {
        ensure!(
            frame.get(4) == Some(&22) && Self::parse(frame)? == self,
            "Unsolicited or mismatched hash response"
        );
        ensure!(
            self.base == file.layer && self.proof + 1 == file.height() - file.layer,
            "Incorrect v2 proof depth"
        );
        let count = self.length as usize + self.proof_count();
        ensure!(
            frame.len() == 53 + count * 32,
            "Incorrect hash proof length"
        );
        let hashes: Vec<Hash> = frame[53..]
            .chunks_exact(32)
            .map(|b| b.try_into().unwrap())
            .collect();
        let mut computed = root(hashes[..self.length as usize].to_vec(), zero(self.base));
        let mut index = self.index / self.length;
        for sibling in &hashes[self.length as usize..] {
            computed = if index & 1 == 0 {
                parent(computed, *sibling)
            } else {
                parent(*sibling, computed)
            };
            index /= 2;
        }
        ensure!(
            computed == file.root,
            "V2 hash proof does not authenticate to the file root"
        );
        for (i, hash) in hashes[..self.length as usize].iter().enumerate() {
            if self.index as usize + i >= file.piece_count as usize {
                ensure!(*hash == zero(self.base), "Invalid v2 padding hash");
            }
        }
        Ok(hashes[..self.length as usize].to_vec())
    }
}

pub async fn fetch_layers(
    stream: &mut crate::peer::BoxedStream,
    buffer: &mut BytesMut,
    meta: &mut crate::meta::TorrentMeta,
) -> Result<()> {
    for file in &mut meta.v2 {
        if !file.hashes.is_empty() {
            continue;
        }
        let length = file.piece_count.next_power_of_two().min(512);
        let mut hashes = Vec::with_capacity(file.piece_count as usize);
        for index in (0..file.piece_count).step_by(length as usize) {
            let request = Request {
                root: file.root,
                base: file.layer,
                index,
                length,
                proof: file.height() - file.layer - 1,
            };
            stream.write_all(&request.frame(21, &[])).await?;
            let accepted = loop {
                if let Some(len) = crate::peer::frame_len(buffer, 1024 * 1024)? {
                    let frame = buffer.split_to(len).freeze();
                    if frame.get(4) == Some(&23) {
                        ensure!(
                            Request::parse(&frame)? != request,
                            "Peer rejected v2 piece hashes"
                        );
                    }
                    if frame.get(4) == Some(&22) {
                        break request.verify(file, &frame)?;
                    }
                    if frame.get(4) == Some(&21) {
                        let other = Request::parse(&frame)?;
                        stream.write_all(&other.frame(23, &[])).await?;
                    }
                    continue;
                }
                buffer.reserve(16 * 1024);
                if stream.read_buf(buffer).await? == 0 {
                    bail!("Peer closed while sending v2 hashes");
                }
            };
            hashes.extend(
                accepted
                    .into_iter()
                    .take((file.piece_count - index) as usize),
            );
        }
        file.install(hashes)?;
    }
    Ok(())
}

/// Read only completed pieces for leaf proofs. Higher layers come from authenticated
/// metadata, so constructing a small proof never reads or hashes the whole file.
#[cfg(test)]
pub fn answer(
    request: Request,
    storage: &crate::storage::Storage,
    meta: &crate::meta::TorrentMeta,
    have: &crate::bitfield::Bitfield,
) -> Result<Bytes> {
    answer_cancellable(
        request,
        storage,
        meta,
        have,
        &tokio_util::sync::CancellationToken::new(),
    )
}

pub fn answer_cancellable(
    request: Request,
    storage: &crate::storage::Storage,
    meta: &crate::meta::TorrentMeta,
    have: &crate::bitfield::Bitfield,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Bytes> {
    let file = meta
        .v2
        .iter()
        .find(|f| f.root == request.root)
        .context("Unknown file root")?;
    let height = file.height();
    ensure!(
        (request.base == 0 || request.base == file.layer) && request.base < height,
        "Unsupported hash layer"
    );
    let width = 1u64 << (height - request.base);
    ensure!(
        u64::from(request.index) + u64::from(request.length) <= width
            && request.proof < height - request.base,
        "Hash request exceeds tree"
    );
    fn node(
        file: &FileTree,
        level: u32,
        index: u32,
        storage: &crate::storage::Storage,
        meta: &crate::meta::TorrentMeta,
        have: &crate::bitfield::Bitfield,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Hash> {
        ensure!(!cancel.is_cancelled(), "Hash proof cancelled");
        if level >= file.layer {
            return file
                .levels
                .get((level - file.layer) as usize)
                .and_then(|l| l.get(index as usize))
                .copied()
                .context("Missing v2 hash layer");
        }
        let offset = u64::from(index) * u64::from(BLOCK) * (1u64 << level);
        if offset >= file.len {
            return Ok(zero(level));
        }
        let piece =
            file.first_piece + (offset / u64::from(meta.lengths.default_piece_length())) as u32;
        ensure!(
            have.get(piece),
            "Requested block hashes are not verified yet"
        );
        if let Some(hash) = storage.cached_hash(file.first_piece, level, index) {
            return Ok(hash);
        }
        if level > 0 {
            let hash = parent(
                node(file, level - 1, index * 2, storage, meta, have, cancel)?,
                node(file, level - 1, index * 2 + 1, storage, meta, have, cancel)?,
            );
            storage.cache_hash(file.first_piece, level, index, hash);
            return Ok(hash);
        }
        let mut data = vec![0; (file.len - offset).min(u64::from(BLOCK)) as usize];
        storage.read(file.offset + offset, &mut data)?;
        let hash = crate::v2::digest(&data);
        storage.cache_hash(file.first_piece, level, index, hash);
        Ok(hash)
    }
    let mut hashes = Vec::with_capacity(request.length as usize + request.proof_count());
    for index in request.index..request.index + request.length {
        hashes.push(node(
            file,
            request.base,
            index,
            storage,
            meta,
            have,
            cancel,
        )?);
    }
    let mut level = request.base + request.length.ilog2();
    let mut index = request.index / request.length;
    for _ in 0..request.proof_count() {
        hashes.push(node(file, level, index ^ 1, storage, meta, have, cancel)?);
        level += 1;
        index /= 2;
    }
    Ok(request.frame(22, &hashes))
}

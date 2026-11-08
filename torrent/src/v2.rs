//! BEP 52 metadata and authenticated SHA-256 trees. No network or filesystem side effects.
use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use librqbit_core::hash_id::{Id20, Id32};
use librqbit_core::torrent_limits::*;
use librqbit_core::torrent_metainfo::{TorrentMetaV1File, TorrentMetaV1Info};
use sha1w::{ISha1, ISha256, Sha1, Sha256};
use url::Url;

use crate::meta::TorrentMeta;

pub const BLOCK: u32 = 16 * 1024;
pub const MAX_LAYER_BYTES: usize = 32 * 1024 * 1024;
pub type Hash = [u8; 32];

fn reserve_tree_memory(pieces: u64) -> Result<std::sync::Arc<tokio::sync::OwnedSemaphorePermit>> {
    use std::sync::{Arc, OnceLock};
    use tokio::sync::Semaphore;
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    // Cover padded cached levels, validation copies, synthetic v1 layout and encoded
    // layers. Keep the reservation with the live metadata, not just its parser.
    let bytes = u32::try_from(pieces.checked_mul(320).context("V2 memory size overflow")?)?;
    let permit = MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(512 * 1024 * 1024)))
        .clone()
        .try_acquire_many_owned(bytes)
        .context("V2 metadata memory limit reached")?;
    Ok(Arc::new(permit))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InfoHashes {
    pub v1: Option<Id20>,
    pub v2: Option<Id32>,
}

impl From<Id20> for InfoHashes {
    fn from(value: Id20) -> Self {
        Self {
            v1: Some(value),
            v2: None,
        }
    }
}

impl InfoHashes {
    pub fn parse_magnet(link: &str) -> Result<Self> {
        let parsed = librqbit_core::magnet::Magnet::parse(link.trim())?;
        Ok(Self {
            v1: parsed.as_id20(),
            v2: parsed.as_id32(),
        })
    }
    pub fn wire(self) -> Id20 {
        self.v1
            .unwrap_or_else(|| self.v2.expect("validated identity").truncate_for_dht())
    }
    pub fn key(self) -> String {
        self.v1
            .map(|h| h.as_string())
            .unwrap_or_else(|| self.v2.expect("validated identity").as_string())
    }
    pub fn wires(self) -> Vec<Id20> {
        self.v1
            .into_iter()
            .chain(self.v2.map(|h| h.truncate_for_dht()))
            .collect()
    }
    pub fn contains(self, hash: Id20) -> bool {
        self.v1 == Some(hash) || self.v2.is_some_and(|h| h.truncate_for_dht() == hash)
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.v1.is_some() && self.v1 == other.v1 || self.v2.is_some() && self.v2 == other.v2
    }
    pub fn wire_overlaps(self, other: Self) -> bool {
        self.wires().into_iter().any(|h| other.contains(h))
    }
    pub fn verify(self, bytes: &[u8]) -> Result<()> {
        ensure!(
            self.v1.is_some() || self.v2.is_some(),
            "Missing torrent identity"
        );
        if let Some(expected) = self.v1 {
            let mut h = Sha1::new();
            h.update(bytes);
            ensure!(h.finish() == expected.0, "Torrent SHA-1 identity mismatch");
        }
        if let Some(expected) = self.v2 {
            ensure!(
                digest(bytes) == expected.0,
                "Torrent SHA-256 identity mismatch"
            );
        }
        Ok(())
    }
    pub fn magnet(self) -> String {
        let mut parts = Vec::new();
        if let Some(h) = self.v1 {
            parts.push(format!("xt=urn:btih:{}", h.as_string()));
        }
        if let Some(h) = self.v2 {
            parts.push(format!("xt=urn:btmh:1220{}", h.as_string()));
        }
        format!("magnet:?{}", parts.join("&"))
    }
}

pub fn digest(bytes: &[u8]) -> Hash {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finish()
}
pub fn parent(a: Hash, b: Hash) -> Hash {
    let mut h = Sha256::new();
    h.update(&a);
    h.update(&b);
    h.finish()
}
pub fn zero(level: u32) -> Hash {
    let mut h = [0; 32];
    for _ in 0..level {
        h = parent(h, h);
    }
    h
}
pub fn root(mut hashes: Vec<Hash>, pad: Hash) -> Hash {
    assert!(!hashes.is_empty());
    hashes.resize(hashes.len().next_power_of_two(), pad);
    while hashes.len() > 1 {
        hashes = hashes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| parent(p[0], p[1]))
            .collect();
    }
    hashes[0]
}

/// One entry per nonempty real file; layers are authenticated before a torrent becomes live.
#[derive(Clone, Debug)]
pub struct FileTree {
    pub root: Hash,
    pub len: u64,
    pub offset: u64,
    pub first_piece: u32,
    pub piece_count: u32,
    pub layer: u32,
    pub hashes: Vec<Hash>,
    pub levels: Vec<Vec<Hash>>,
}

impl FileTree {
    pub fn piece_size(&self, index: u32, piece_length: u32) -> u32 {
        (self.len - u64::from(index) * u64::from(piece_length)).min(u64::from(piece_length)) as u32
    }
    pub fn install(&mut self, hashes: Vec<Hash>) -> Result<()> {
        ensure!(
            hashes.len() == self.piece_count as usize,
            "Incorrect v2 piece-layer length"
        );
        ensure!(
            root(hashes.clone(), zero(self.layer)) == self.root,
            "V2 piece layer does not match its file root"
        );
        self.hashes = hashes;
        self.levels = vec![self.hashes.clone()];
        self.levels[0].resize(self.hashes.len().next_power_of_two(), zero(self.layer));
        while self.levels.last().unwrap().len() > 1 {
            let next = self
                .levels
                .last()
                .unwrap()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| parent(p[0], p[1]))
                .collect();
            self.levels.push(next);
        }
        Ok(())
    }
    pub fn height(&self) -> u32 {
        self.len
            .div_ceil(u64::from(BLOCK))
            .next_power_of_two()
            .ilog2()
    }
}

/// Canonical, bounded bencode with borrowed payloads and original byte spans. V2 identities
/// never depend on decoding and re-encoding untrusted dictionaries.
struct Node<'a> {
    raw: &'a [u8],
    value: Value<'a>,
}
enum Value<'a> {
    Bytes(&'a [u8]),
    Int(i64),
    List(Vec<Node<'a>>),
    Dict(BTreeMap<&'a [u8], Node<'a>>),
}
impl<'a> Node<'a> {
    fn dict(&self) -> Result<&BTreeMap<&'a [u8], Node<'a>>> {
        match &self.value {
            Value::Dict(d) => Ok(d),
            _ => bail!("Expected bencode dictionary"),
        }
    }
    fn get(&self, key: &[u8]) -> Option<&Node<'a>> {
        self.dict().ok()?.get(key)
    }
    fn required(&self, key: &[u8]) -> Result<&Node<'a>> {
        self.get(key).context("Missing required v2 metadata field")
    }
    fn bytes(&self) -> Result<&'a [u8]> {
        match self.value {
            Value::Bytes(b) => Ok(b),
            _ => bail!("Expected byte string"),
        }
    }
    fn number(&self) -> Result<u64> {
        match self.value {
            Value::Int(n) if n >= 0 => Ok(n as u64),
            _ => bail!("Expected nonnegative integer"),
        }
    }
}
fn parse(bytes: &[u8]) -> Result<Node<'_>> {
    fn string<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8]> {
        let start = *pos;
        while bytes.get(*pos).is_some_and(u8::is_ascii_digit) {
            *pos += 1;
        }
        ensure!(
            *pos > start && bytes.get(*pos) == Some(&b':'),
            "Invalid bencode string"
        );
        ensure!(
            *pos - start == 1 || bytes[start] != b'0',
            "Noncanonical bencode length"
        );
        let len: usize = std::str::from_utf8(&bytes[start..*pos])?.parse()?;
        *pos += 1;
        let end = pos.checked_add(len).context("Bencode length overflow")?;
        let value = bytes.get(*pos..end).context("Truncated bencode string")?;
        *pos = end;
        Ok(value)
    }
    fn node<'a>(
        bytes: &'a [u8],
        pos: &mut usize,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<Node<'a>> {
        *nodes += 1;
        ensure!(
            depth <= 70 && *nodes <= 200_000,
            "V2 metadata complexity exceeds safety limit"
        );
        let start = *pos;
        let value = match bytes.get(*pos).context("Truncated bencode")? {
            b'0'..=b'9' => Value::Bytes(string(bytes, pos)?),
            b'i' => {
                *pos += 1;
                let from = *pos;
                while bytes
                    .get(*pos)
                    .is_some_and(|b| b.is_ascii_digit() || *b == b'-')
                {
                    *pos += 1;
                }
                ensure!(
                    *pos > from && bytes.get(*pos) == Some(&b'e'),
                    "Invalid bencode integer"
                );
                let s = std::str::from_utf8(&bytes[from..*pos])?;
                ensure!(
                    !s.starts_with("-0") && (s.len() == 1 || !s.starts_with('0')),
                    "Noncanonical bencode integer"
                );
                // Negative optional fields are not used; still allow canonical negatives.
                let value = s.parse::<i64>()?;
                *pos += 1;
                Value::Int(value)
            }
            b'l' => {
                *pos += 1;
                let mut list = Vec::new();
                while bytes.get(*pos) != Some(&b'e') {
                    list.push(node(bytes, pos, depth + 1, nodes)?);
                }
                *pos += 1;
                Value::List(list)
            }
            b'd' => {
                *pos += 1;
                let mut dict = BTreeMap::new();
                let mut last = None;
                while bytes.get(*pos) != Some(&b'e') {
                    let key = string(bytes, pos)?;
                    ensure!(
                        last.is_none_or(|old| old < key),
                        "Unsorted or duplicate bencode keys"
                    );
                    last = Some(key);
                    dict.insert(key, node(bytes, pos, depth + 1, nodes)?);
                }
                *pos += 1;
                Value::Dict(dict)
            }
            _ => bail!("Invalid bencode token"),
        };
        Ok(Node {
            raw: &bytes[start..*pos],
            value,
        })
    }
    let mut pos = 0;
    let result = node(bytes, &mut pos, 0, &mut 0)?;
    ensure!(pos == bytes.len(), "Trailing bencode data");
    Ok(result)
}

#[derive(serde::Deserialize)]
struct Version {
    #[serde(rename = "meta version")]
    version: Option<u64>,
    #[serde(rename = "file tree")]
    tree: Option<serde::de::IgnoredAny>,
}
#[derive(serde::Deserialize)]
struct Probe<B> {
    info: bencode::WithRawBytes<Version, B>,
}
pub fn file_info(bytes: &[u8]) -> Result<Option<Bytes>> {
    let probe: Probe<buffers::ByteBuf<'_>> =
        bencode::from_bytes(bytes).map_err(|e| e.into_kind())?;
    if probe.info.data.version.is_some() || probe.info.data.tree.is_some() {
        ensure!(
            probe.info.data.version == Some(2),
            "Unsupported torrent metadata version"
        );
        Ok(Some(Bytes::copy_from_slice(probe.info.raw_bytes.as_ref())))
    } else {
        Ok(None)
    }
}
pub fn is_v2(bytes: &[u8]) -> Result<bool> {
    let probe: Version = bencode::from_bytes(bytes).map_err(|e| e.into_kind())?;
    if probe.version.is_some() || probe.tree.is_some() {
        ensure!(
            probe.version == Some(2),
            "Unsupported torrent metadata version"
        );
        Ok(true)
    } else {
        Ok(false)
    }
}

struct FlatFile {
    path: Vec<Vec<u8>>,
    len: u64,
    root: Option<Hash>,
}
fn flatten(
    node: &Node<'_>,
    path: &mut Vec<Vec<u8>>,
    out: &mut Vec<FlatFile>,
    path_bytes: &mut usize,
) -> Result<()> {
    let dict = node.dict()?;
    if let Some(file) = dict.get(&b""[..]) {
        ensure!(
            !path.is_empty() && dict.len() == 1,
            "V2 file has children or is the tree root"
        );
        ensure!(out.len() < MAX_FILES, "Too many torrent files");
        let len = file.required(b"length")?.number()?;
        let root = file
            .get(b"pieces root")
            .map(|n| Hash::try_from(n.bytes()?).context("Invalid v2 root length"))
            .transpose()?;
        ensure!(
            (len == 0) == root.is_none(),
            "Nonempty v2 files require a root; empty files must not have one"
        );
        if let Some(attr) = file.get(b"attr") {
            ensure!(
                !attr.bytes()?.iter().any(|b| matches!(b, b'l' | b'p')),
                "V2 file tree cannot contain symlinks or padding files"
            );
        }
        *path_bytes += path.iter().map(|p| p.len() + 1).sum::<usize>();
        ensure!(
            *path_bytes <= MAX_TOTAL_PATH_BYTES,
            "Torrent paths exceed safety limit"
        );
        out.push(FlatFile {
            path: path.clone(),
            len,
            root,
        });
    } else {
        ensure!(
            path.len() < MAX_PATH_COMPONENTS && !dict.is_empty(),
            "Invalid v2 directory"
        );
        for (name, child) in dict {
            ensure!(
                !name.is_empty()
                    && name.len() <= MAX_COMPONENT_BYTES
                    && *name != b"."
                    && *name != b".."
                    && !name.iter().any(|b| matches!(b, b'/' | b'\\' | 0)),
                "Unsafe v2 path component"
            );
            path.push(name.to_vec());
            flatten(child, path, out, path_bytes)?;
            path.pop();
        }
    }
    Ok(())
}

pub fn metadata(
    info_bytes: Bytes,
    trackers: Vec<Url>,
    expected: Option<InfoHashes>,
    layers: Option<&[u8]>,
) -> Result<TorrentMeta> {
    ensure!(
        info_bytes.len() <= crate::metadata::MAX_METADATA_SIZE as usize,
        "Torrent metadata exceeds size limit"
    );
    if let Some(expected) = expected {
        expected.verify(&info_bytes)?;
    }
    let info = parse(&info_bytes)?;
    ensure!(
        info.required(b"meta version")?.number()? == 2,
        "Unsupported torrent metadata version"
    );
    let piece_length = u32::try_from(info.required(b"piece length")?.number()?)?;
    ensure!(
        piece_length >= BLOCK && piece_length.is_power_of_two(),
        "V2 piece length must be a power of two of at least 16 KiB"
    );
    let mut flat = Vec::new();
    flatten(
        info.required(b"file tree")?,
        &mut Vec::new(),
        &mut flat,
        &mut 0,
    )?;
    let hybrid = info.get(b"pieces").is_some();
    ensure!(
        hybrid || info.get(b"files").is_none() && info.get(b"length").is_none(),
        "Incomplete hybrid metadata"
    );
    let mut v1 = Sha1::new();
    v1.update(&info_bytes);
    let hashes = InfoHashes {
        v1: hybrid.then(|| Id20::new(v1.finish())),
        v2: Some(Id32::new(digest(&info_bytes))),
    };
    if let Some(expected) = expected {
        ensure!(
            expected.v1.is_none() || hybrid,
            "A v1 hash cannot identify v2-only metadata"
        );
    }
    let mut files = Vec::new();
    let mut trees = Vec::new();
    let mut offset = 0u64;
    for f in &flat {
        if f.len > 0 && !offset.is_multiple_of(u64::from(piece_length)) {
            let pad = u64::from(piece_length) - offset % u64::from(piece_length);
            files.push(TorrentMetaV1File {
                length: pad,
                path: vec![b".pad".to_vec(), pad.to_string().into_bytes()],
                attr: Some(b"p".to_vec()),
                sha1: None,
                symlink_path: None,
            });
            offset += pad;
        }
        let count = f.len.div_ceil(u64::from(piece_length));
        if let Some(root) = f.root {
            ensure!(
                count <= MAX_LAYER_BYTES as u64 / 32,
                "V2 piece layer exceeds safety limit"
            );
            trees.push(FileTree {
                root,
                len: f.len,
                offset,
                first_piece: u32::try_from(offset / u64::from(piece_length))?,
                piece_count: u32::try_from(count)?,
                layer: (piece_length / BLOCK).ilog2(),
                hashes: if count == 1 { vec![root] } else { Vec::new() },
                levels: if count == 1 {
                    vec![vec![root]]
                } else {
                    Vec::new()
                },
            });
        }
        files.push(TorrentMetaV1File {
            length: f.len,
            path: f.path.clone(),
            attr: None,
            sha1: None,
            symlink_path: None,
        });
        offset = offset
            .checked_add(f.len)
            .context("Torrent length overflow")?;
    }
    let count = offset.div_ceil(u64::from(piece_length));
    ensure!(
        count > 0 && count <= MAX_LAYER_BYTES as u64 / 32,
        "Empty or excessively large v2 piece map"
    );
    let memory = reserve_tree_memory(count)?;
    let mut meta = if hybrid {
        let v1: TorrentMetaV1Info<buffers::ByteBuf<'_>> =
            bencode::from_bytes(&info_bytes).map_err(|e| e.into_kind())?;
        let checked = v1.clone().validate()?;
        let original: Vec<_> = checked
            .iter_file_details()
            .filter(|f| !f.attrs().padding)
            .collect();
        ensure!(original.len() == flat.len(), "Hybrid file lists disagree");
        for (a, b) in original.iter().zip(&flat) {
            ensure!(
                !a.attrs().symlink
                    && a.filename
                        .iter_components_bytes()
                        .eq(b.path.iter().map(Vec::as_slice)),
                "Hybrid original file paths disagree"
            );
        }
        let meta = TorrentMeta::build(hashes.wire(), v1, info_bytes.clone(), trackers)?;
        let actual: Vec<_> = meta.files.iter().filter(|f| !f.virtual_file).collect();
        ensure!(actual.len() == flat.len(), "Hybrid file lists disagree");
        let synthetic = TorrentMetaV1Info {
            name: None,
            pieces: vec![0u8; count as usize * 20],
            piece_length,
            files: Some(files),
            ..Default::default()
        }
        .validate()?;
        let wanted: Vec<_> = synthetic
            .iter_file_details_ext()
            .filter(|f| !f.details.attrs().padding)
            .collect();
        for (a, b) in actual.iter().zip(wanted) {
            ensure!(
                a.path == b.details.filename.to_pathbuf()
                    && a.len == b.details.len
                    && (a.len == 0 || a.offset == b.offset),
                "Hybrid file paths, lengths or alignment disagree"
            );
        }
        ensure!(
            meta.lengths.total_pieces() == count as u32,
            "Hybrid piece counts disagree"
        );
        meta
    } else {
        let name = info
            .get(b"name")
            .map(|n| n.bytes().map(Vec::from))
            .transpose()?;
        let private = info
            .get(b"private")
            .map(|n| n.number())
            .transpose()?
            .unwrap_or(0)
            != 0;
        let mut m = TorrentMeta::build(
            hashes.wire(),
            TorrentMetaV1Info {
                name,
                pieces: vec![0; count as usize * 20],
                piece_length,
                files: Some(files),
                private,
                ..Default::default()
            },
            info_bytes.clone(),
            trackers,
        )?;
        m.clear_v1_hashes();
        m.multi_file = flat.len() != 1 || flat[0].path.len() != 1;
        m
    };
    meta.hashes = hashes;
    meta.v2_memory = Some(memory);
    meta.v2 = trees;
    if let Some(bytes) = layers {
        install_layers(&mut meta, bytes)?;
    }
    Ok(meta)
}

pub fn from_file(bytes: &[u8], info: Bytes) -> Result<TorrentMeta> {
    let top = parse(bytes)?;
    let mut trackers = Vec::new();
    if let Some(tiers) = top.get(b"announce-list") {
        let Value::List(tiers) = &tiers.value else {
            bail!("Invalid tracker list")
        };
        for tier in tiers {
            let Value::List(urls) = &tier.value else {
                bail!("Invalid tracker tier")
            };
            for url in urls {
                trackers.push(url.bytes()?);
            }
        }
    } else if let Some(url) = top.get(b"announce") {
        trackers.push(url.bytes()?);
    }
    ensure!(
        trackers.len() <= MAX_TRACKERS && trackers.iter().all(|t| t.len() <= MAX_TRACKER_URL_BYTES),
        "Tracker list exceeds safety limit"
    );
    let trackers = trackers
        .into_iter()
        .filter_map(|t| std::str::from_utf8(t).ok())
        .filter_map(crate::meta::usable_tracker)
        .collect();
    let mut meta = metadata(
        info,
        trackers,
        None,
        Some(top.required(b"piece layers")?.raw),
    )?;
    meta.web_seeds = crate::webseed::from_metainfo(bytes)?;
    Ok(meta)
}

pub fn install_layers(meta: &mut TorrentMeta, bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= MAX_LAYER_BYTES + 1024 * 1024,
        "V2 layers exceed safety limit"
    );
    let layers = parse(bytes)?;
    let layers = layers.dict()?;
    for (key, value) in layers {
        let root = Hash::try_from(*key).context("Invalid piece layer root")?;
        ensure!(
            meta.v2.iter().any(|f| f.root == root && f.piece_count > 1),
            "Unexpected v2 piece layer"
        );
        ensure!(
            value.bytes()?.len() <= MAX_LAYER_BYTES,
            "Piece layer exceeds safety limit"
        );
    }
    for file in &mut meta.v2 {
        if file.piece_count > 1 {
            let bytes = layers
                .get(file.root.as_slice())
                .context("Missing v2 piece layer")?
                .bytes()?;
            ensure!(
                bytes.len() == file.piece_count as usize * 32,
                "Incorrect v2 piece-layer size"
            );
            file.install(bytes.as_chunks::<32>().0.to_vec())?;
        }
    }
    Ok(())
}

pub fn encode_layers(meta: &TorrentMeta) -> Result<Vec<u8>> {
    let map: BTreeMap<buffers::ByteBuf<'_>, buffers::ByteBufOwned> = meta
        .v2
        .iter()
        .filter(|f| f.piece_count > 1)
        .map(|f| {
            (
                buffers::ByteBuf(&f.root),
                buffers::ByteBufOwned(
                    f.hashes
                        .iter()
                        .flatten()
                        .copied()
                        .collect::<Vec<_>>()
                        .into(),
                ),
            )
        })
        .collect();
    let mut bytes = Vec::new();
    bencode::bencode_serialize_to_writer(&map, &mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn v2_tree_reservations_bound_parallel_layers_and_survive_shared_ownership() {
        // No large allocation: reserve the cost of the audit's 16 GiB / 16 KiB
        // torrent. Eight peers cannot independently build these trees at once.
        let pieces = 16u64 * 1024 * 1024 * 1024 / u64::from(BLOCK);
        let first = reserve_tree_memory(pieces).unwrap();
        assert_eq!(first.num_permits(), 320 * 1024 * 1024);
        for _ in 0..7 {
            assert!(reserve_tree_memory(pieces).is_err());
        }
        let shared = first.clone();
        drop(first);
        assert!(reserve_tree_memory(pieces).is_err());
        drop(shared);
        assert!(reserve_tree_memory(pieces).is_ok());
    }

    #[test]
    fn canonical_parser_rejects_ambiguous_and_excessive_input() {
        for input in [
            b"d1:ai1e1:ai2ee".as_slice(),
            b"d1:bi1e1:ai2ee",
            b"i01e",
            b"i-0e",
            b"01:a",
            b"i1ee",
            b"9999999999999999999999999:a",
            b"d1:ale",
        ] {
            assert!(parse(input).is_err(), "{input:?}");
        }
        let mut deep = vec![b'l'; 72];
        deep.extend(vec![b'e'; 72]);
        assert!(parse(&deep).is_err());
        let negative = parse(b"i-1e").unwrap();
        assert!(negative.number().is_err());
        assert!(negative.bytes().is_err());
    }

    #[test]
    fn hybrid_and_v2_wire_aliases_are_reserved_without_truncating_storage_identity() {
        let first = InfoHashes {
            v1: Some(Id20::new([1; 20])),
            v2: Some(Id32::new([2; 32])),
        };
        let mut second = InfoHashes {
            v1: None,
            v2: first.v2,
        };
        second.v2.as_mut().unwrap().0[31] = 3;
        assert!(!first.overlaps(second));
        assert!(first.wire_overlaps(second));
        assert_ne!(first.key(), second.key());
    }
}

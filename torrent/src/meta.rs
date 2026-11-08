//! Everything the engine needs to know about one torrent, checked and in owned form.

use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use buffers::ByteBuf;
use bytes::Bytes;
use librqbit_core::hash_id::Id20;
use librqbit_core::lengths::Lengths;
use librqbit_core::torrent_limits::{MAX_TRACKER_URL_BYTES, MAX_TRACKERS};
use librqbit_core::torrent_metainfo::{
    TorrentMetaV1Info, ValidatedTorrentMetaV1Info, torrent_from_bytes,
};
use url::Url;

use crate::bitfield::Bitfield;

const HASH_LEN: usize = 20;

#[derive(Clone, Debug)]
pub struct TorrentFile {
    /// Relative to the download folder, with every part made safe for Windows.
    pub path: PathBuf,
    /// The names as the torrent has them, joined with `/`. For showing, never for a path.
    pub display: String,
    pub len: u64,
    /// Where this file starts in the torrent's bytes.
    pub offset: u64,
    /// Padding and symlinks are part of the piece data but are never saved.
    pub virtual_file: bool,
    pub pieces: std::ops::Range<u32>,
    /// Percent-encoded original path components for web seeds, never a filesystem path.
    pub web_path: String,
}

pub struct TorrentMeta {
    pub info_hash: Id20,
    pub hashes: crate::v2::InfoHashes,
    pub v2: Vec<crate::v2::FileTree>,
    pub(crate) v2_memory: Option<std::sync::Arc<tokio::sync::OwnedSemaphorePermit>>,
    pub name: String,
    pub lengths: Lengths,
    pub files: Vec<TorrentFile>,
    /// Private torrents may only find peers through their own trackers.
    pub private: bool,
    pub trackers: Vec<Url>,
    pub web_seeds: Vec<crate::webseed::Seed>,
    pub(crate) web_name: String,
    pub(crate) multi_file: bool,
    /// The raw info dictionary, which peers can ask for when they start from a magnet link.
    pub info_bytes: Bytes,
    piece_hashes: Bytes,
}

impl TorrentMeta {
    /// Reads a .torrent file. Its contents come from a stranger and are checked here.
    pub fn from_torrent_file(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "Torrent file exceeds the size limit"
        );
        if let Some(info) = crate::v2::file_info(bytes)? {
            return crate::v2::from_file(bytes, info);
        }
        let torrent = torrent_from_bytes(bytes).context("This is not a valid torrent file")?;
        for tracker in torrent.iter_announce() {
            ensure!(
                tracker.as_ref().len() <= MAX_TRACKER_URL_BYTES,
                "Tracker URL exceeds the size limit"
            );
        }
        let trackers = torrent
            .iter_announce()
            .filter_map(|url| std::str::from_utf8(url.as_ref()).ok())
            .filter_map(usable_tracker)
            .collect();
        let info_bytes = Bytes::copy_from_slice(torrent.info.raw_bytes.as_ref());
        let mut meta = Self::build(
            torrent.info_hash,
            torrent.info.data,
            info_bytes.clone(),
            trackers,
        )?;
        meta.web_seeds = crate::webseed::from_metainfo(bytes)?;
        Ok(meta)
    }

    /// Reads an info dictionary fetched from peers. The hash is what makes it trustworthy,
    /// so nothing else is looked at until the hash matches.
    pub fn from_info_bytes(info_hash: Id20, info_bytes: Bytes, trackers: Vec<Url>) -> Result<Self> {
        Self::from_hashes(info_hash.into(), info_bytes, trackers)
    }

    pub fn from_hashes(
        hashes: crate::v2::InfoHashes,
        info_bytes: Bytes,
        trackers: Vec<Url>,
    ) -> Result<Self> {
        ensure!(
            info_bytes.len() <= crate::metadata::MAX_METADATA_SIZE as usize,
            "Torrent metadata exceeds the size limit"
        );
        ensure!(
            trackers.len() <= MAX_TRACKERS
                && trackers
                    .iter()
                    .all(|t| t.as_str().len() <= MAX_TRACKER_URL_BYTES),
            "Tracker list exceeds the safety limits"
        );
        hashes.verify(&info_bytes)?;
        if crate::v2::is_v2(&info_bytes)? && hashes.v2.is_some() {
            return crate::v2::metadata(info_bytes, trackers, Some(hashes), None);
        }
        // A btih-only magnet deliberately uses the v1 representation of a hybrid.
        // Older peers can provide its metadata without supporting BEP 52 hash messages.
        // A magnet carrying btmh must never silently downgrade this way.
        ensure!(hashes.v2.is_none(), "A v2 magnet requires v2 metadata");
        let info: TorrentMetaV1Info<ByteBuf> =
            bencode::from_bytes(&info_bytes).map_err(|error| error.into_kind())?;
        Self::build(hashes.wire(), info, info_bytes.clone(), trackers)
    }

    pub(crate) fn build<Buf: AsRef<[u8]>>(
        info_hash: Id20,
        info: TorrentMetaV1Info<Buf>,
        info_bytes: Bytes,
        trackers: Vec<Url>,
    ) -> Result<Self> {
        let private = info.private;
        ensure!(
            trackers.len() <= MAX_TRACKERS
                && trackers
                    .iter()
                    .all(|t| t.as_str().len() <= MAX_TRACKER_URL_BYTES),
            "Tracker list exceeds the safety limits"
        );
        let validated: ValidatedTorrentMetaV1Info<Buf> =
            info.validate().context("This torrent is not valid")?;
        let lengths = *validated.lengths();
        let multi_file = validated.info().files.is_some();
        let web_name = validated.info().name.as_ref().map_or_else(
            || "torrent-content".to_owned(),
            |name| crate::webseed::encoded_component(name.as_ref()),
        );
        let piece_hashes = Bytes::copy_from_slice(validated.info().pieces.as_ref());
        ensure!(
            piece_hashes.len() as u64 == u64::from(lengths.total_pieces()) * HASH_LEN as u64,
            "This torrent does not have one hash for each piece"
        );
        let files: Vec<TorrentFile> = validated
            .iter_file_details_ext()
            .map(|file| {
                let attrs = file.details.attrs();
                TorrentFile {
                    path: file.details.filename.to_pathbuf(),
                    display: file.details.filename.to_vec().join("/"),
                    len: file.details.len,
                    offset: file.offset,
                    virtual_file: attrs.padding || (attrs.symlink && file.details.len == 0),
                    pieces: file.pieces,
                    web_path: file
                        .details
                        .filename
                        .iter_components_bytes()
                        .map(crate::webseed::encoded_component)
                        .collect::<Vec<_>>()
                        .join("/"),
                }
            })
            .collect();
        #[cfg(target_os = "macos")]
        {
            use objc2_foundation::{NSString, NSStringCompareOptions};
            let mut names = std::collections::HashSet::new();
            for file in files.iter().filter(|file| !file.virtual_file) {
                // APFS/HFS+ can treat composed/decomposed Unicode names as the
                // same file. The portable Windows-name check alone misses this.
                let key = NSString::from_str(&file.path.to_string_lossy())
                    .stringByFoldingWithOptions_locale(
                        NSStringCompareOptions::CaseInsensitiveSearch,
                        None,
                    )
                    .decomposedStringWithCanonicalMapping()
                    .to_string();
                ensure!(
                    names.insert(key),
                    librqbit_core::Error::BadTorrentDuplicateFilenames
                );
            }
        }
        Ok(Self {
            info_hash,
            hashes: info_hash.into(),
            v2: Vec::new(),
            v2_memory: None,
            name: validated
                .safe_name()
                .unwrap_or_else(|| info_hash.as_string()),
            lengths,
            files,
            private,
            trackers,
            web_seeds: Vec::new(),
            web_name,
            multi_file,
            info_bytes,
            piece_hashes,
        })
    }

    pub fn piece_hash(&self, piece: u32) -> Option<&[u8]> {
        let start = piece as usize * HASH_LEN;
        self.piece_hashes.get(start..start + HASH_LEN)
    }

    pub(crate) fn clear_v1_hashes(&mut self) {
        self.piece_hashes = Bytes::new();
    }

    pub(crate) fn require_complete_layers(&self) -> Result<()> {
        ensure!(
            self.hashes.v2.is_none() || !self.v2.is_empty(),
            "Missing v2 file trees"
        );
        ensure!(
            self.v2
                .iter()
                .all(|f| f.hashes.len() == f.piece_count as usize),
            "V2 piece hashes must be authenticated before starting"
        );
        Ok(())
    }

    /// Data bytes in this piece. BEP 52 alignment gaps are never requested from peers.
    pub fn piece_data_length(&self, piece: u32) -> Option<u32> {
        let valid = self.lengths.validate_piece_index(piece)?;
        if self.hashes.v2.is_none() {
            return Some(self.lengths.piece_length(valid));
        }
        self.v2
            .iter()
            .find(|f| piece >= f.first_piece && piece - f.first_piece < f.piece_count)
            .map(|f| f.piece_size(piece - f.first_piece, self.lengths.default_piece_length()))
    }

    pub fn chunk_request(&self, piece: u32, begin: u32) -> Option<librqbit_peer_protocol::Request> {
        let len = self.piece_data_length(piece)?;
        if begin >= len || !begin.is_multiple_of(crate::v2::BLOCK) {
            return None;
        }
        Some(librqbit_peer_protocol::Request::new(
            piece,
            begin,
            (len - begin).min(crate::v2::BLOCK),
        ))
    }

    /// The pieces needed for the chosen files, or for all of them. The numbers come from
    /// the user or a magnet link, so one that names no file is an error.
    pub fn wanted_pieces(&self, only: Option<&[usize]>) -> Result<Bitfield> {
        let total = self.lengths.total_pieces();
        let Some(only) = only else {
            return Ok(Bitfield::full(total));
        };
        let mut wanted = Bitfield::new(total);
        for index in only {
            let Some(file) = self.files.get(*index) else {
                bail!("This torrent has no file number {index}");
            };
            for piece in file.pieces.clone() {
                wanted.set(piece);
            }
        }
        Ok(wanted)
    }

    /// How many bytes of the torrent these pieces hold.
    pub fn bytes_in(&self, pieces: impl Iterator<Item = u32>) -> u64 {
        pieces
            .filter_map(|piece| self.piece_data_length(piece))
            .map(u64::from)
            .sum()
    }

    /// How many bytes of each file these pieces cover. A piece can cross into a file that
    /// was not chosen, so a skipped file can show a little progress.
    pub fn file_progress(&self, have: &Bitfield) -> Vec<u64> {
        let last = self.lengths.last_piece_id().get();
        let piece_length = u64::from(self.lengths.default_piece_length());
        self.files
            .iter()
            .map(|file| {
                file.pieces
                    .clone()
                    .filter(|piece| have.get(*piece))
                    .map(|piece| {
                        let start = u64::from(piece) * piece_length;
                        let end = if piece == last {
                            self.lengths.total_length()
                        } else {
                            start + piece_length
                        };
                        end.min(file.offset + file.len)
                            .saturating_sub(start.max(file.offset))
                    })
                    .sum()
            })
            .collect()
    }
}

/// Tracker addresses come from strangers, so only the schemes the tracker code speaks count.
pub fn usable_tracker(url: &str) -> Option<Url> {
    let url = Url::parse(url).ok()?;
    matches!(url.scheme(), "http" | "https" | "udp").then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{multi_file_torrent, piece_hashes, single_file_torrent};
    use librqbit_core::constants::CHUNK_SIZE;
    use std::path::Path;

    #[test]
    fn test_reads_a_single_file_torrent() {
        let data = vec![7u8; 40_000];
        let meta = TorrentMeta::from_torrent_file(&single_file_torrent("movie.bin", &data, 16384))
            .unwrap();
        assert_eq!(meta.name, "movie.bin");
        assert_eq!(meta.lengths.total_length(), 40_000);
        assert_eq!(meta.files.len(), 1);
        assert_eq!(meta.files[0].path, Path::new("movie.bin"));
        assert_eq!(meta.files[0].pieces, 0..3);
        assert!(!meta.files[0].virtual_file);
    }

    #[test]
    fn test_reads_a_multi_file_torrent_with_offsets() {
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["a.bin"], &[1u8; 20_000], false),
                (&["sub", "b.bin"], &[2u8; 30_000], false),
            ],
            16384,
        );
        let meta = TorrentMeta::from_torrent_file(&torrent).unwrap();
        assert_eq!(meta.name, "pack");
        assert_eq!(meta.files[1].display, "sub/b.bin");
        assert_eq!(meta.files[1].path, Path::new("sub").join("b.bin"));
        assert_eq!(meta.files[1].offset, 20_000);
        assert_eq!(meta.files[0].pieces, 0..2);
        assert_eq!(meta.files[1].pieces, 1..4);
    }

    #[test]
    fn test_unsafe_names_are_safe_on_disk_and_kept_for_display() {
        let torrent = multi_file_torrent("pack", &[(&["C:evil.dll"], &[1u8; 100], false)], 16384);
        let meta = TorrentMeta::from_torrent_file(&torrent).unwrap();
        assert_eq!(meta.files[0].display, "C:evil.dll");
        assert_eq!(meta.files[0].path, Path::new("C_evil.dll"));
    }

    #[test]
    fn test_saved_filename_collisions_are_rejected_for_files_and_magnet_metadata() {
        for (first, second) in [
            (vec!["Guide.pdf"], vec!["GUIDE.PDF"]),
            (vec!["guide.pdf"], vec!["guide.pdf."]),
            (vec!["guide.pdf"], vec!["guide.pdf "]),
            (vec!["a:b"], vec!["a_b"]),
            (vec!["CON"], vec!["_con"]),
            (vec!["Images", "sample.png"], vec!["images", "SAMPLE.PNG"]),
            #[cfg(target_os = "macos")]
            (vec!["café.bin"], vec!["cafe\u{301}.bin"]),
            #[cfg(target_os = "macos")]
            (vec!["café", "A.bin"], vec!["cafe\u{301}", "a.bin"]),
        ] {
            let bytes = multi_file_torrent(
                "collision fixture",
                &[(&first, &[1u8; 10], false), (&second, &[2u8; 10], false)],
                16384,
            );
            // Use the actual metadata paths for a local file and for peer-supplied
            // magnet metadata, including its hash verification. Neither creates storage.
            let raw = torrent_from_bytes(&bytes).unwrap();
            let from_file = TorrentMeta::from_torrent_file(&bytes);
            let from_magnet = TorrentMeta::from_info_bytes(
                raw.info_hash,
                Bytes::copy_from_slice(raw.info.raw_bytes.as_ref()),
                Vec::new(),
            );
            for (source, result) in [("file", from_file), ("magnet metadata", from_magnet)] {
                let error = result
                    .err()
                    .expect("Colliding saved names must be rejected");
                assert!(
                    matches!(
                        error.downcast_ref::<librqbit_core::Error>(),
                        Some(librqbit_core::Error::BadTorrentDuplicateFilenames)
                    ),
                    "{source}: {first:?} / {second:?}: {error:#}"
                );
            }
        }
    }

    #[test]
    fn test_padding_is_marked_virtual() {
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["a.bin"], &[1u8; 100], false),
                (&[".pad", "16284"], &[0u8; 16284], true),
                (&["b.bin"], &[2u8; 100], false),
            ],
            16384,
        );
        let meta = TorrentMeta::from_torrent_file(&torrent).unwrap();
        let flags: Vec<bool> = meta.files.iter().map(|f| f.virtual_file).collect();
        assert_eq!(flags, [false, true, false]);
    }

    #[test]
    fn test_a_torrent_without_a_hash_for_each_piece_is_refused() {
        let info = format!(
            "d6:lengthi40000e4:name1:a12:piece lengthi16384e6:pieces20:{}e",
            "x".repeat(20)
        );
        let torrent = format!("d4:info{info}e");
        let error = TorrentMeta::from_torrent_file(torrent.as_bytes())
            .err()
            .unwrap();
        assert!(error.to_string().contains("one hash for each piece"));
    }

    #[test]
    fn test_garbage_is_refused() {
        assert!(TorrentMeta::from_torrent_file(b"not a torrent").is_err());
        assert!(TorrentMeta::from_torrent_file(b"").is_err());
    }

    #[test]
    fn test_info_bytes_must_match_their_hash() {
        let data = vec![7u8; 100];
        let torrent = single_file_torrent("a.bin", &data, 16384);
        let meta = TorrentMeta::from_torrent_file(&torrent).unwrap();
        let good =
            TorrentMeta::from_info_bytes(meta.info_hash, meta.info_bytes.clone(), Vec::new());
        assert_eq!(good.unwrap().name, "a.bin");
        let mut tampered = meta.info_bytes.to_vec();
        tampered[0] ^= 1;
        let bad = TorrentMeta::from_info_bytes(meta.info_hash, tampered.into(), Vec::new());
        assert!(bad.is_err());
    }

    #[test]
    fn test_piece_hashes_come_out_in_order() {
        let data = vec![9u8; 40_000];
        let meta =
            TorrentMeta::from_torrent_file(&single_file_torrent("a.bin", &data, 16384)).unwrap();
        let expected = piece_hashes(&data, 16384);
        assert_eq!(meta.piece_hash(1).unwrap(), &expected[20..40]);
        assert!(meta.piece_hash(3).is_none());
    }

    #[test]
    fn test_choosing_files_picks_their_pieces() {
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["a"], &[1u8; 20_000], false),
                (&["b"], &[2u8; 30_000], false),
            ],
            16384,
        );
        let meta = TorrentMeta::from_torrent_file(&torrent).unwrap();
        let first = meta.wanted_pieces(Some(&[0])).unwrap();
        assert_eq!(first.iter_set().collect::<Vec<_>>(), [0, 1]);
        assert!(meta.wanted_pieces(None).unwrap().is_full());
        assert!(meta.wanted_pieces(Some(&[2])).is_err());
    }

    #[test]
    fn test_file_progress_shares_a_boundary_piece_correctly() {
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["a"], &[1u8; 20_000], false),
                (&["b"], &[2u8; 30_000], false),
            ],
            16384,
        );
        let meta = TorrentMeta::from_torrent_file(&torrent).unwrap();
        let mut have = Bitfield::new(meta.lengths.total_pieces());
        have.set(1);
        // Piece 1 holds the last 3,616 bytes of a, then the first 12,768 of b.
        let chunk = u64::from(CHUNK_SIZE);
        assert_eq!(
            meta.file_progress(&have),
            [20_000 - chunk, 2 * chunk - 20_000]
        );
        // The last piece is short, and is counted at its real size.
        have = Bitfield::new(meta.lengths.total_pieces());
        have.set(3);
        assert_eq!(meta.file_progress(&have), [0, 50_000 - 3 * chunk]);
    }

    #[test]
    fn test_only_usable_tracker_addresses_are_kept() {
        assert!(usable_tracker("http://tracker.example/announce").is_some());
        assert!(usable_tracker("udp://tracker.example:80").is_some());
        assert!(usable_tracker("file:///etc/passwd").is_none());
        assert!(usable_tracker("not a url").is_none());
    }
}

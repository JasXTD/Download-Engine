//! Remembers which pieces are done, so a restart doesn't have to hash every file again.
//! A record only counts if the files look exactly as they did when it was saved. After a
//! crash, or if anything touched the files, they no longer match and are checked in full.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use librqbit_core::hash_id::Id20;
use serde::{Deserialize, Serialize};

use crate::bitfield::Bitfield;
use crate::meta::TorrentMeta;
use crate::storage::{FileStamp, Storage};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    info_hash: String,
    have: String,
    files: Vec<Option<FileStamp>>,
}

fn path_for(state_dir: &Path, info_hash: Id20) -> PathBuf {
    state_dir.join(format!("{}.resume.json", info_hash.as_string()))
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|[high, low]| {
            let high = char::from(*high).to_digit(16)?;
            let low = char::from(*low).to_digit(16)?;
            Some((high << 4 | low) as u8)
        })
        .collect()
}

/// Saves the record. The caller must have written everything to disk and closed the files,
/// because the record says those exact file times go with these pieces.
pub fn save(
    state_dir: &Path,
    meta: &TorrentMeta,
    storage: &Storage,
    have: &Bitfield,
) -> io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let saved = Saved {
        version: VERSION,
        info_hash: meta.info_hash.as_string(),
        have: to_hex(have.as_bytes()),
        files: storage.stamps(),
    };
    let path = path_for(state_dir, meta.info_hash);
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec(&saved)?)?;
    fs::rename(&temporary, &path)
}

/// Returns the saved pieces, if the record is good and the files haven't changed.
pub fn load(state_dir: &Path, meta: &TorrentMeta, storage: &Storage) -> Option<Bitfield> {
    let bytes = fs::read(path_for(state_dir, meta.info_hash)).ok()?;
    let saved: Saved = serde_json::from_slice(&bytes).ok()?;
    if saved.version != VERSION
        || saved.info_hash != meta.info_hash.as_string()
        || saved.files != storage.stamps()
    {
        return None;
    }
    Bitfield::from_wire(&from_hex(&saved.have)?, meta.lengths.total_pieces())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{TempFolder, meta_of, sample_bytes, single_file_torrent, touch_later};
    use std::sync::Arc;

    fn setup(folder: &TempFolder) -> (Arc<TorrentMeta>, Storage, PathBuf) {
        let data = sample_bytes(40_000, 1);
        let meta = meta_of(&single_file_torrent("a.bin", &data, 16384));
        let storage = Storage::new(meta.clone(), folder.path().join("output"));
        storage.write(0, &data).unwrap();
        storage.sync().unwrap();
        storage.close();
        (meta, storage, folder.path().join("state"))
    }

    fn two_pieces(meta: &TorrentMeta) -> Bitfield {
        let mut have = Bitfield::new(meta.lengths.total_pieces());
        have.set(0);
        have.set(2);
        have
    }

    #[test]
    fn test_a_saved_record_comes_back() {
        let folder = TempFolder::new();
        let (meta, storage, state) = setup(&folder);
        let have = two_pieces(&meta);
        save(&state, &meta, &storage, &have).unwrap();
        assert_eq!(load(&state, &meta, &storage), Some(have));
    }

    #[test]
    fn test_no_record_means_nothing_is_trusted() {
        let folder = TempFolder::new();
        let (meta, storage, state) = setup(&folder);
        assert_eq!(load(&state, &meta, &storage), None);
    }

    #[test]
    fn test_a_changed_file_voids_the_record() {
        let folder = TempFolder::new();
        let (meta, storage, state) = setup(&folder);
        save(&state, &meta, &storage, &two_pieces(&meta)).unwrap();
        let path = folder.path().join("output").join("a.bin");
        fs::write(&path, [1u8; 40_000]).unwrap();
        touch_later(&path);
        assert_eq!(load(&state, &meta, &storage), None);
    }

    #[test]
    fn test_a_missing_file_voids_the_record() {
        let folder = TempFolder::new();
        let (meta, storage, state) = setup(&folder);
        save(&state, &meta, &storage, &two_pieces(&meta)).unwrap();
        fs::remove_file(folder.path().join("output").join("a.bin")).unwrap();
        assert_eq!(load(&state, &meta, &storage), None);
    }

    #[test]
    fn test_a_damaged_record_is_ignored() {
        let folder = TempFolder::new();
        let (meta, storage, state) = setup(&folder);
        save(&state, &meta, &storage, &two_pieces(&meta)).unwrap();
        let path = path_for(&state, meta.info_hash);
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(load(&state, &meta, &storage), None);
    }

    #[test]
    fn test_a_record_for_another_torrent_is_ignored() {
        let folder = TempFolder::new();
        let (meta, storage, state) = setup(&folder);
        save(&state, &meta, &storage, &two_pieces(&meta)).unwrap();
        let path = path_for(&state, meta.info_hash);
        let text = fs::read_to_string(&path).unwrap();
        let other = Id20::new([9; 20]).as_string();
        fs::write(&path, text.replace(&meta.info_hash.as_string(), &other)).unwrap();
        assert_eq!(load(&state, &meta, &storage), None);
    }

    #[test]
    fn test_hex_round_trip_and_bad_input() {
        assert_eq!(from_hex(&to_hex(&[0, 15, 255])), Some(vec![0, 15, 255]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }
}

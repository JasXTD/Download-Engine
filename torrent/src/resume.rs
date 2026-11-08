//! Remembers which pieces are done, so a restart doesn't have to hash every file again.
//! A record only counts if the files look exactly as they did when it was saved. After a
//! crash, or if anything touched the files, they no longer match and are checked in full.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(test)]
use librqbit_core::hash_id::Id20;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::bitfield::Bitfield;
use crate::meta::TorrentMeta;
use crate::metadata::MAX_METADATA_SIZE;
use crate::storage::{FileStamp, Storage};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    v2_hash: Option<String>,
    info_hash: String,
    have: String,
    files: Vec<Option<FileStamp>>,
}

fn path_for(state_dir: &Path, info_hash: impl Into<crate::v2::InfoHashes>) -> PathBuf {
    state_dir.join(format!("{}.resume.json", info_hash.into().key()))
}

/// The torrent's details, saved so that coming back to it needs nothing from strangers.
fn info_path_for(state_dir: &Path, info_hash: impl Into<crate::v2::InfoHashes>) -> PathBuf {
    state_dir.join(format!("{}.info", info_hash.into().key()))
}

/// Full identities from authenticated local metadata, for removing all cache aliases.
pub fn cached_hashes(
    state_dir: &Path,
    hashes: crate::v2::InfoHashes,
) -> Option<crate::v2::InfoHashes> {
    let meta = load_info(state_dir, hashes, vec![])?;
    let mut known = meta.hashes;
    // A btih-only hybrid is intentionally transferred in v1 mode. Its authenticated
    // raw dictionary still identifies the same hybrid when opened through btmh.
    if known.v2.is_none() && crate::v2::is_v2(&meta.info_bytes).ok()? {
        known.v2 = Some(librqbit_core::hash_id::Id32::new(crate::v2::digest(
            &meta.info_bytes,
        )));
    }
    Some(known)
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
        version: if meta.hashes.v2.is_some() { 2 } else { VERSION },
        v2_hash: meta.hashes.v2.map(|h| h.as_string()),
        info_hash: meta.hashes.key(),
        have: to_hex(have.as_bytes()),
        files: storage.stamps(),
    };
    let path = path_for(state_dir, meta.hashes);
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec(&saved)?)?;
    fs::rename(&temporary, &path)
}

/// Removes the record, so that the next start reads every file. A missing record is fine.
pub fn forget(state_dir: &Path, info_hash: impl Into<crate::v2::InfoHashes>) -> io::Result<()> {
    match fs::remove_file(path_for(state_dir, info_hash)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Returns the saved pieces, if the record is good and the files haven't changed.
pub fn load(state_dir: &Path, meta: &TorrentMeta, storage: &Storage) -> Option<Bitfield> {
    let bytes = fs::read(path_for(state_dir, meta.hashes)).ok()?;
    let saved: Saved = serde_json::from_slice(&bytes).ok()?;
    if saved.version != if meta.hashes.v2.is_some() { 2 } else { VERSION }
        || saved.v2_hash != meta.hashes.v2.map(|h| h.as_string())
        || saved.info_hash != meta.hashes.key()
        || saved.files != storage.stamps()
    {
        return None;
    }
    Bitfield::from_wire(&from_hex(&saved.have)?, meta.lengths.total_pieces())
}

/// Saves a torrent's details.
pub fn save_info(state_dir: &Path, meta: &TorrentMeta) -> io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let path = info_path_for(state_dir, meta.hashes);
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, &meta.info_bytes)?;
    fs::rename(&temporary, &path)?;
    if meta.hashes.v2.is_some() {
        let layers = crate::v2::encode_layers(meta).map_err(io::Error::other)?;
        let target = path.with_extension("layers");
        let temporary = target.with_extension("layers.tmp");
        fs::write(&temporary, &layers)?;
        fs::rename(temporary, target)?;
        if meta.hashes.v1.is_some() {
            let alias = info_path_for(
                state_dir,
                crate::v2::InfoHashes {
                    v1: None,
                    v2: meta.hashes.v2,
                },
            );
            let temporary = alias.with_extension("tmp");
            fs::write(&temporary, &meta.info_bytes)?;
            fs::rename(temporary, &alias)?;
            let target = alias.with_extension("layers");
            let temporary = target.with_extension("layers.tmp");
            fs::write(&temporary, layers)?;
            fs::rename(temporary, target)?;
        }
    }
    let seeds = path.with_extension("webseeds.json");
    let temporary = seeds.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec(&meta.web_seeds)?)?;
    fs::rename(temporary, seeds)
}

/// Returns a torrent's saved details. They only count if they still match the hash.
pub fn load_info(
    state_dir: &Path,
    info_hash: impl Into<crate::v2::InfoHashes>,
    trackers: Vec<Url>,
) -> Option<TorrentMeta> {
    let info_hash = info_hash.into();
    let path = info_path_for(state_dir, info_hash);
    if fs::metadata(&path).ok()?.len() > u64::from(MAX_METADATA_SIZE) {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    let mut meta = TorrentMeta::from_hashes(info_hash, bytes.into(), trackers).ok()?;
    if meta.hashes.v2.is_some() {
        use std::io::Read;
        let file = fs::File::open(path.with_extension("layers")).ok()?;
        let mut layers = Vec::new();
        file.take((crate::v2::MAX_LAYER_BYTES + 1024 * 1024 + 1) as u64)
            .read_to_end(&mut layers)
            .ok()?;
        crate::v2::install_layers(&mut meta, &layers).ok()?;
    }
    // Seeds live outside the hashed info dictionary. Bound and revalidate the optional
    // sidecar; a missing/corrupt sidecar must not prevent recovering verified pieces.
    use std::io::Read;
    if let Ok(file) =
        fs::File::open(info_path_for(state_dir, info_hash).with_extension("webseeds.json"))
    {
        let mut bytes = Vec::new();
        if file.take(128 * 1024 + 1).read_to_end(&mut bytes).is_ok()
            && bytes.len() <= 128 * 1024
            && let Ok(seeds) = serde_json::from_slice::<Vec<crate::webseed::Seed>>(&bytes)
            && seeds.len() <= crate::webseed::MAX_SEEDS
        {
            meta.web_seeds = seeds
                .into_iter()
                .filter(|s| crate::webseed::checked_url(&s.url, true).is_some())
                .collect();
        }
    }
    Some(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{TempFolder, meta_of, sample_bytes, single_file_torrent, touch_later};
    use std::sync::Arc;

    #[test]
    fn web_seed_sidecar_round_trips_and_is_bounded() {
        let folder = TempFolder::new();
        let mut meta =
            TorrentMeta::from_torrent_file(&single_file_torrent("a.bin", b"abcd", 16384)).unwrap();
        meta.web_seeds = vec![crate::webseed::Seed {
            kind: crate::webseed::Kind::UrlList,
            url: "https://example.org/a.bin?token=value".into(),
        }];
        save_info(&folder.path(), &meta).unwrap();
        assert_eq!(
            load_info(&folder.path(), meta.info_hash, vec![])
                .unwrap()
                .web_seeds,
            meta.web_seeds
        );
        fs::write(
            info_path_for(&folder.path(), meta.info_hash).with_extension("webseeds.json"),
            vec![b' '; 128 * 1024 + 1],
        )
        .unwrap();
        assert!(
            load_info(&folder.path(), meta.info_hash, vec![])
                .unwrap()
                .web_seeds
                .is_empty()
        );
        meta.web_seeds.clear();
        save_info(&folder.path(), &meta).unwrap();
        assert!(
            load_info(&folder.path(), meta.info_hash, vec![])
                .unwrap()
                .web_seeds
                .is_empty()
        );
    }

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

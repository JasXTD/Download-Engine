//! Older versions did not record the staging directory's identity. Import valid pieces
//! by reading it as an untrusted source, never modifying or deleting the old directory.
use super::{Directory, HASH_BUFFER, Storage};
use crate::{resume, v2::InfoHashes};
use anyhow::Result;
use std::{io, path::Path, sync::Arc};
use tokio_util::sync::CancellationToken;

pub fn import_verified_pieces(
    state: &Path,
    hashes: InfoHashes,
    chosen: &[usize],
    source: &Path,
    destination: Arc<Directory>,
    cancel: &CancellationToken,
) -> Result<bool> {
    let source_dir = match Directory::open(source) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e.into()),
    };
    let Some(meta) = resume::load_info(state, hashes, vec![]) else {
        // Without authenticated metadata no legacy bytes can be trusted for reuse.
        return Ok(true);
    };
    let meta = Arc::new(meta);
    let mut old = Storage::for_seeding(meta.clone(), source.to_owned(), None, None)?;
    old.bind_directory(Arc::new(source_dir));
    let mut new = Storage::new(meta.clone(), Default::default());
    new.bind_directory(destination);
    let mut buffer = vec![0; HASH_BUFFER];
    for piece in meta.wanted_pieces(Some(chosen))?.iter_set() {
        if cancel.is_cancelled() {
            return Ok(false);
        }
        if new.piece_matches(piece)? || !old.piece_matches(piece)? {
            continue;
        }
        let index = meta
            .lengths
            .validate_piece_index(piece)
            .expect("wanted piece belongs to this torrent");
        let mut offset = meta.lengths.piece_offset(index);
        let mut left = meta.lengths.piece_length(index) as usize;
        while left > 0 {
            if cancel.is_cancelled() {
                return Ok(false);
            }
            let take = left.min(buffer.len());
            old.read(offset, &mut buffer[..take])?;
            new.write(offset, &buffer[..take])?;
            offset += take as u64;
            left -= take;
        }
    }
    new.sync()?;
    // The helper's normal piece check validates the destination again. The old resume
    // bitmap cannot bypass that check because it does not match the new directory ID.
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{TempFolder, meta_of, single_file_torrent};
    use std::fs;

    #[test]
    fn upgrade_reuses_only_matching_pieces_and_preserves_the_legacy_source() {
        let base = TempFolder::new();
        let data = vec![0x42; 32_768];
        let meta = meta_of(&single_file_torrent("sample.bin", &data, 16_384));
        let state = base.path().join("state");
        resume::save_info(&state, &meta).unwrap();
        let legacy = base.path().join("old");
        fs::create_dir(&legacy).unwrap();
        let mut partial = data.clone();
        partial[16_384..].fill(0x17);
        fs::write(legacy.join("sample.bin"), &partial).unwrap();
        let path = base.path().join("new");
        let dir = Arc::new(Directory::create_new(&path).unwrap());
        let cancel = CancellationToken::new();
        assert!(import_verified_pieces(&state, meta.hashes, &[0], &legacy, dir, &cancel).unwrap());
        let bytes = fs::read(path.join("sample.bin")).unwrap();
        assert_eq!(&bytes[..16_384], &data[..16_384]);
        assert!(bytes[16_384..].iter().all(|&byte| byte == 0));
        assert_eq!(fs::read(legacy.join("sample.bin")).unwrap(), partial);
    }
}

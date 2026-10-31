//! Builds small torrents and throwaway folders for tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use sha1w::{ISha1, Sha1};

use crate::meta::TorrentMeta;

/// A folder that is removed when the test ends.
pub struct TempFolder(PathBuf);

impl TempFolder {
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "de-torrent-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> PathBuf {
        self.0.clone()
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bytes that differ everywhere, so a misplaced block can't pass by accident.
pub fn sample_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn text(value: &str) -> Vec<u8> {
    format!("{}:{value}", value.len()).into_bytes()
}

fn raw(value: &[u8]) -> Vec<u8> {
    [format!("{}:", value.len()).as_bytes(), value].concat()
}

pub fn piece_hashes(data: &[u8], piece_length: u32) -> Vec<u8> {
    data.chunks(piece_length as usize)
        .flat_map(|piece| {
            let mut hash = Sha1::new();
            hash.update(piece);
            hash.finish()
        })
        .collect()
}

fn wrap(info: Vec<u8>, announce: Option<&str>) -> Vec<u8> {
    let mut out = b"d".to_vec();
    if let Some(url) = announce {
        out.extend(b"8:announce");
        out.extend(text(url));
    }
    out.extend(b"4:info");
    out.extend(info);
    out.push(b'e');
    out
}

pub fn single_file_torrent(name: &str, data: &[u8], piece_length: u32) -> Vec<u8> {
    let mut info = format!("d6:lengthi{}e4:name", data.len()).into_bytes();
    info.extend(text(name));
    info.extend(format!("12:piece lengthi{piece_length}e6:pieces").as_bytes());
    info.extend(raw(&piece_hashes(data, piece_length)));
    info.push(b'e');
    wrap(info, None)
}

pub fn multi_file_torrent(
    name: &str,
    files: &[(&[&str], &[u8], bool)],
    piece_length: u32,
) -> Vec<u8> {
    multi_file_torrent_with(name, files, piece_length, None, false)
}

pub fn multi_file_torrent_with(
    name: &str,
    files: &[(&[&str], &[u8], bool)],
    piece_length: u32,
    announce: Option<&str>,
    private: bool,
) -> Vec<u8> {
    let joined: Vec<u8> = files.iter().flat_map(|file| file.1.to_vec()).collect();
    let mut info = b"d5:filesl".to_vec();
    for (path, data, padding) in files {
        info.push(b'd');
        if *padding {
            info.extend(b"4:attr1:p");
        }
        info.extend(format!("6:lengthi{}e4:pathl", data.len()).as_bytes());
        for part in *path {
            info.extend(text(part));
        }
        info.extend(b"ee");
    }
    info.extend(b"e4:name");
    info.extend(text(name));
    info.extend(format!("12:piece lengthi{piece_length}e6:pieces").as_bytes());
    info.extend(raw(&piece_hashes(&joined, piece_length)));
    if private {
        info.extend(b"7:privatei1e");
    }
    info.push(b'e');
    wrap(info, announce)
}

pub fn meta_of(torrent: &[u8]) -> Arc<TorrentMeta> {
    Arc::new(TorrentMeta::from_torrent_file(torrent).unwrap())
}

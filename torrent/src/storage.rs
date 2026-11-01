//! Reads and writes torrent data as if it were one long file, across the real files on
//! disk. Everything here blocks, so callers run it on blocking threads.

use std::collections::HashMap;
#[cfg(not(unix))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use sha1w::{ISha1, Sha1};

use crate::meta::TorrentMeta;

#[cfg(unix)]
mod unix;

/// A torrent can have many thousands of files, and the system allows only so many open.
const MAX_OPEN_FILES: usize = 128;

const HASH_BUFFER: usize = 256 * 1024;

#[derive(Default)]
struct OpenFiles {
    files: HashMap<usize, OpenFile>,
    tick: u64,
}

struct OpenFile {
    file: Arc<File>,
    writable: bool,
    last_used: u64,
}

/// Part of a read or write that falls inside one file.
#[derive(Debug, PartialEq, Eq)]
struct Span {
    file: usize,
    offset: u64,
    len: usize,
}

/// What a file looks like on disk, to tell later whether anything changed it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileStamp {
    pub len: u64,
    pub modified: String,
}

pub struct Storage {
    meta: Arc<TorrentMeta>,
    root: PathBuf,
    #[cfg(unix)]
    native_root: unix::Root,
    open: Mutex<OpenFiles>,
    read_only: bool,
    file_name: Option<String>,
    allowed: Option<Vec<bool>>,
}

impl Storage {
    pub fn new(meta: Arc<TorrentMeta>, root: PathBuf) -> Self {
        Self {
            meta,
            #[cfg(unix)]
            native_root: unix::Root::new(root.clone()),
            root,
            open: Mutex::default(),
            read_only: false,
            file_name: None,
            allowed: None,
        }
    }

    fn path_of(&self, file: usize) -> PathBuf {
        self.root.join(
            self.file_name
                .as_deref()
                .map(Path::new)
                .unwrap_or(&self.meta.files[file].path),
        )
    }

    #[cfg(unix)]
    fn native_file(&self, index: usize, write: bool) -> io::Result<File> {
        let relative = self
            .file_name
            .as_deref()
            .map(Path::new)
            .unwrap_or(&self.meta.files[index].path);
        self.native_root
            .open(relative, write, self.meta.files[index].len)
    }

    pub fn for_seeding(
        meta: Arc<TorrentMeta>,
        root: PathBuf,
        file_name: Option<String>,
        only: Option<&[usize]>,
    ) -> anyhow::Result<Self> {
        if let Some(name) = &file_name {
            let stem = name
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
                .contains(&stem.as_str())
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'));
            anyhow::ensure!(
                meta.files.iter().filter(|file| !file.virtual_file).count() == 1
                    && !name.is_empty()
                    && !reserved
                    && !name.ends_with(['.', ' '])
                    && !name
                        .chars()
                        .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
                    && Path::new(name).components().count() == 1,
                "Invalid seed filename"
            );
        }
        let allowed = only.map(|indices| {
            let indices: std::collections::HashSet<_> = indices.iter().copied().collect();
            (0..meta.files.len())
                .map(|index| indices.contains(&index))
                .collect()
        });
        let mut storage = Self::new(meta, root);
        storage.read_only = true;
        storage.file_name = file_name;
        storage.allowed = allowed;
        Ok(storage)
    }

    fn spans(&self, offset: u64, len: usize) -> Vec<Span> {
        let files = &self.meta.files;
        let mut spans = Vec::new();
        let mut position = offset;
        let mut left = len as u64;
        let first = files.partition_point(|file| file.offset + file.len <= offset);
        for (index, file) in files.iter().enumerate().skip(first) {
            if left == 0 {
                break;
            }
            if file.len == 0 {
                continue;
            }
            let inside = position - file.offset;
            let take = left.min(file.len - inside);
            spans.push(Span {
                file: index,
                offset: inside,
                len: take as usize,
            });
            position += take;
            left -= take;
        }
        spans
    }

    fn cached(&self, file: usize, need_write: bool) -> Option<Arc<File>> {
        let mut open = self.open.lock();
        open.tick += 1;
        let tick = open.tick;
        let entry = open.files.get_mut(&file)?;
        if need_write && !entry.writable {
            return None;
        }
        entry.last_used = tick;
        Some(entry.file.clone())
    }

    fn remember(&self, index: usize, file: Arc<File>, writable: bool) {
        let mut open = self.open.lock();
        open.tick += 1;
        let tick = open.tick;
        if open.files.len() >= MAX_OPEN_FILES
            && !open.files.contains_key(&index)
            && let Some(oldest) = open
                .files
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| *index)
        {
            open.files.remove(&oldest);
        }
        open.files.insert(
            index,
            OpenFile {
                file,
                writable,
                last_used: tick,
            },
        );
    }

    fn for_reading(&self, index: usize) -> io::Result<Arc<File>> {
        if self.allowed.as_ref().is_some_and(|allowed| !allowed[index]) {
            return Err(io::ErrorKind::NotFound.into());
        }
        if let Some(file) = self.cached(index, false) {
            return Ok(file);
        }
        #[cfg(not(unix))]
        let mut options = OpenOptions::new();
        #[cfg(not(unix))]
        options.read(true);
        #[cfg(windows)]
        if self.read_only {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            };
            options
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        #[cfg(not(unix))]
        let file = Arc::new(options.open(self.path_of(index))?);
        #[cfg(unix)]
        let file = Arc::new(self.native_file(index, false)?);
        if self.read_only && !file.metadata()?.is_file() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        #[cfg(windows)]
        if self.read_only {
            use std::os::windows::fs::MetadataExt;
            if file.metadata()?.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        }
        self.remember(index, file.clone(), false);
        Ok(file)
    }

    fn for_writing(&self, index: usize) -> io::Result<Arc<File>> {
        if let Some(file) = self.cached(index, true) {
            return Ok(file);
        }
        #[cfg(not(unix))]
        let file = Arc::new(open_for_writing(
            &self.path_of(index),
            self.meta.files[index].len,
        )?);
        #[cfg(unix)]
        let file = Arc::new(self.native_file(index, true)?);
        self.remember(index, file.clone(), true);
        Ok(file)
    }

    /// Writes bytes at a position in the torrent. Padding is dropped.
    pub fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Seeding cannot modify completed files",
            ));
        }
        let mut rest = data;
        for span in self.spans(offset, data.len()) {
            let (part, tail) = rest.split_at(span.len);
            rest = tail;
            if !self.meta.files[span.file].virtual_file {
                write_all_at(&*self.for_writing(span.file)?, part, span.offset)?;
            }
        }
        Ok(())
    }

    /// Reads bytes at a position in the torrent. Padding reads as zeros.
    pub fn read(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let mut rest = buffer;
        for span in self.spans(offset, rest.len()) {
            let (part, tail) = rest.split_at_mut(span.len);
            rest = tail;
            if self.meta.files[span.file].virtual_file {
                part.fill(0);
            } else {
                read_exact_at(&*self.for_reading(span.file)?, part, span.offset)?;
            }
        }
        Ok(())
    }

    /// Whether a piece on disk matches its hash. A missing or short file just means the
    /// piece isn't there yet. Any other failure is a real problem and is returned.
    pub fn piece_matches(&self, piece: u32) -> io::Result<bool> {
        let lengths = &self.meta.lengths;
        let Some(valid) = lengths.validate_piece_index(piece) else {
            return Ok(false);
        };
        let Some(expected) = self.meta.piece_hash(piece) else {
            return Ok(false);
        };
        let mut left = lengths.piece_length(valid) as usize;
        let mut offset = lengths.piece_offset(valid);
        let mut hash = Sha1::new();
        let mut buffer = vec![0u8; HASH_BUFFER.min(left)];
        while left > 0 {
            let take = left.min(buffer.len());
            match self.read(offset, &mut buffer[..take]) {
                Ok(()) => {}
                Err(error) if is_absent(&error) => return Ok(false),
                Err(error) => return Err(error),
            }
            hash.update(&buffer[..take]);
            offset += take as u64;
            left -= take;
        }
        Ok(hash.finish() == expected)
    }

    /// Whether any real file of this torrent exists yet, so a new download can skip checking.
    pub fn any_file_exists(&self) -> bool {
        (0..self.meta.files.len())
            .filter(|index| !self.meta.files[*index].virtual_file)
            .any(|index| self.path_of(index).exists())
    }

    /// Creates the empty files among those chosen. No piece covers them, so nothing else
    /// would ever make them appear.
    pub fn create_empty_files(&self, only: Option<&[usize]>) -> io::Result<()> {
        if self.read_only {
            return Ok(());
        }
        for (index, file) in self.meta.files.iter().enumerate() {
            let chosen = only.is_none_or(|only| only.contains(&index));
            if chosen && file.len == 0 && !file.virtual_file {
                #[cfg(not(unix))]
                open_for_writing(&self.path_of(index), 0)?;
                #[cfg(unix)]
                self.native_file(index, true)?;
            }
        }
        Ok(())
    }

    /// One stamp for each file, or None for a file that isn't there.
    pub fn stamps(&self) -> Vec<Option<FileStamp>> {
        (0..self.meta.files.len())
            .map(|index| {
                #[cfg(not(unix))]
                {
                    stamp(&self.path_of(index))
                }
                #[cfg(unix)]
                {
                    file_stamp(&self.native_file(index, false).ok()?.metadata().ok()?)
                }
            })
            .collect()
    }

    /// Pushes everything written so far to the disk.
    pub fn sync(&self) -> io::Result<()> {
        let files: Vec<(Arc<File>, bool)> = self
            .open
            .lock()
            .files
            .values()
            .map(|entry| (entry.file.clone(), entry.writable))
            .collect();
        for (file, writable) in files {
            if writable {
                file.sync_all()?;
            }
        }
        Ok(())
    }

    /// Lets go of the open files, so that the user can move or delete them.
    pub fn close(&self) {
        self.open.lock().files.clear();
    }
}

#[cfg(not(unix))]
fn stamp(path: &Path) -> Option<FileStamp> {
    let info = fs::metadata(path).ok()?;
    file_stamp(&info)
}

fn file_stamp(info: &fs::Metadata) -> Option<FileStamp> {
    let modified = info.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(FileStamp {
        len: info.len(),
        modified: modified.as_nanos().to_string(),
    })
}

fn is_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::UnexpectedEof
    )
}

#[cfg(not(unix))]
fn open_for_writing(path: &Path, len: u64) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    match options.clone().create_new(true).open(path) {
        Ok(file) => {
            // Pieces arrive in any order, and without this Windows would fill the gap
            // before a far-away write with zeros while the download waits.
            mark_sparse(&file);
            mark_internet(path)?;
            file.set_len(len)?;
            Ok(file)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let file = options.open(path)?;
            if file.metadata()?.len() != len {
                file.set_len(len)?;
            }
            Ok(file)
        }
        Err(error) => Err(error),
    }
}

/// Files from a torrent came from the internet, and Windows should treat them that way.
/// This matches what the app does for its other downloads.
fn mark_internet(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use io::Write;
        let mut name = path.as_os_str().to_os_string();
        name.push(":Zone.Identifier");
        let mut marker = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(PathBuf::from(name))
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    "Cannot mark this file as an Internet download. Choose a local NTFS folder.",
                )
            })?;
        marker.write_all(b"[ZoneTransfer]\r\nZoneId=3\r\n")?;
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

#[cfg(windows)]
fn mark_sparse(file: &File) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;

    let mut returned = 0u32;
    // Not every file system supports this. The download still works, just more slowly.
    // SAFETY: the handle is open for as long as `file` lives, and this control code
    // takes no buffers.
    let _ = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
}

#[cfg(not(windows))]
fn mark_sparse(_file: &File) {}

#[cfg(windows)]
fn write_all_at(file: &File, mut data: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let written = file.seek_write(data, offset)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        data = &data[written..];
        offset += written as u64;
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buffer.is_empty() {
        let read = match file.seek_read(buffer, offset) {
            Ok(read) => read,
            // Windows reports reading at the end of a file this way.
            Err(error) if error.raw_os_error() == Some(38) => 0,
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buffer = &mut buffer[read..];
        offset += read as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn write_all_at(file: &File, data: &[u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, data, offset)
}

#[cfg(unix)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{TempFolder, meta_of, multi_file_torrent, single_file_torrent};

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn two_files() -> (Arc<TorrentMeta>, Vec<u8>, Vec<u8>) {
        let first = pattern(20_000, 1);
        let second = pattern(30_000, 2);
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["a.bin"], &first, false),
                (&["sub", "b.bin"], &second, false),
            ],
            16384,
        );
        (meta_of(&torrent), first, second)
    }

    #[test]
    fn test_a_write_across_two_files_lands_in_both() {
        let folder = TempFolder::new();
        let (meta, first, second) = two_files();
        let storage = Storage::new(meta, folder.path());
        let joined = [first.as_slice(), second.as_slice()].concat();
        storage.write(19_000, &joined[19_000..21_000]).unwrap();
        storage.close();

        let a = fs::read(folder.path().join("a.bin")).unwrap();
        let b = fs::read(folder.path().join("sub").join("b.bin")).unwrap();
        assert_eq!(a.len(), 20_000, "files get their full size");
        assert_eq!(b.len(), 30_000);
        assert_eq!(a[19_000..], first[19_000..]);
        assert_eq!(b[..1_000], second[..1_000]);
        assert_eq!(a[..19_000], vec![0u8; 19_000]);
    }

    #[test]
    fn test_a_read_across_two_files_returns_what_was_written() {
        let folder = TempFolder::new();
        let (meta, first, second) = two_files();
        let storage = Storage::new(meta, folder.path());
        let joined = [first.as_slice(), second.as_slice()].concat();
        storage.write(0, &joined).unwrap();
        let mut back = vec![0u8; 3_000];
        storage.read(18_500, &mut back).unwrap();
        assert_eq!(back, joined[18_500..21_500]);
    }

    #[test]
    fn test_the_hash_check_passes_good_data_and_fails_damage() {
        let folder = TempFolder::new();
        let (meta, first, second) = two_files();
        let storage = Storage::new(meta, folder.path());
        let joined = [first.as_slice(), second.as_slice()].concat();
        storage.write(0, &joined).unwrap();
        for piece in 0..4 {
            assert!(storage.piece_matches(piece).unwrap(), "piece {piece}");
        }
        storage.write(20_000, &[joined[20_000] ^ 1]).unwrap();
        assert!(!storage.piece_matches(1).unwrap());
        assert!(storage.piece_matches(0).unwrap());
        assert!(storage.piece_matches(2).unwrap());
    }

    #[test]
    fn test_a_missing_file_is_a_missing_piece_not_an_error() {
        let folder = TempFolder::new();
        let (meta, ..) = two_files();
        let storage = Storage::new(meta, folder.path());
        assert!(!storage.piece_matches(0).unwrap());
        assert!(!storage.piece_matches(99).unwrap());
    }

    #[test]
    fn test_a_short_file_is_a_missing_piece_not_an_error() {
        let folder = TempFolder::new();
        let (meta, first, _) = two_files();
        fs::write(folder.path().join("a.bin"), &first[..100]).unwrap();
        let storage = Storage::new(meta, folder.path());
        assert!(!storage.piece_matches(0).unwrap());
    }

    #[test]
    fn test_padding_never_reaches_the_disk_and_hashes_as_zeros() {
        let folder = TempFolder::new();
        let first = pattern(100, 1);
        let zeros = vec![0u8; 16284];
        let second = pattern(100, 2);
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["a.bin"], &first, false),
                (&[".pad", "16284"], &zeros, true),
                (&["b.bin"], &second, false),
            ],
            16384,
        );
        let storage = Storage::new(meta_of(&torrent), folder.path());
        let joined = [first.as_slice(), zeros.as_slice(), second.as_slice()].concat();
        storage.write(0, &joined).unwrap();
        assert!(!folder.path().join(".pad").exists());
        assert!(storage.piece_matches(0).unwrap());
        assert!(storage.piece_matches(1).unwrap());
    }

    #[test]
    fn test_empty_files_are_only_made_when_chosen() {
        let folder = TempFolder::new();
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["empty-one"], &[], false),
                (&["a.bin"], &[1u8; 100], false),
                (&["empty-two"], &[], false),
            ],
            16384,
        );
        let storage = Storage::new(meta_of(&torrent), folder.path());
        storage.create_empty_files(Some(&[0, 1])).unwrap();
        assert!(folder.path().join("empty-one").exists());
        assert!(!folder.path().join("empty-two").exists());
    }

    #[test]
    fn test_writes_around_empty_files_land_in_the_right_file() {
        let folder = TempFolder::new();
        let first = pattern(100, 1);
        let second = pattern(100, 2);
        let torrent = multi_file_torrent(
            "pack",
            &[
                (&["a"], &first, false),
                (&["empty"], &[], false),
                (&["b"], &second, false),
            ],
            16384,
        );
        let storage = Storage::new(meta_of(&torrent), folder.path());
        let joined = [first.as_slice(), second.as_slice()].concat();
        storage.write(0, &joined).unwrap();
        storage.close();
        assert_eq!(fs::read(folder.path().join("a")).unwrap(), first);
        assert_eq!(fs::read(folder.path().join("b")).unwrap(), second);
    }

    #[test]
    fn test_a_name_with_a_drive_stays_in_the_folder() {
        let folder = TempFolder::new();
        let torrent = multi_file_torrent("pack", &[(&["C:evil.dll"], &[1u8; 100], false)], 16384);
        let storage = Storage::new(meta_of(&torrent), folder.path());
        storage.write(0, &[1u8; 100]).unwrap();
        storage.close();
        let names: Vec<String> = fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["C_evil.dll"]);
    }

    #[test]
    fn test_more_files_than_may_stay_open_still_work() {
        let folder = TempFolder::new();
        let bodies: Vec<Vec<u8>> = (0..MAX_OPEN_FILES + 20)
            .map(|index| pattern(50, index as u8))
            .collect();
        let names: Vec<String> = (0..bodies.len()).map(|i| format!("{i}.bin")).collect();
        let parts: Vec<[&str; 1]> = names.iter().map(|name| [name.as_str()]).collect();
        let entries: Vec<(&[&str], &[u8], bool)> = parts
            .iter()
            .zip(&bodies)
            .map(|(part, body)| (&part[..], body.as_slice(), false))
            .collect();
        let torrent = multi_file_torrent("pack", &entries, 16384);
        let storage = Storage::new(meta_of(&torrent), folder.path());
        let joined = bodies.concat();
        storage.write(0, &joined).unwrap();
        assert!(storage.open.lock().files.len() <= MAX_OPEN_FILES);
        let mut back = vec![0u8; joined.len()];
        storage.read(0, &mut back).unwrap();
        assert_eq!(back, joined);
    }

    #[test]
    fn test_a_read_handle_is_upgraded_for_writing() {
        let folder = TempFolder::new();
        let (meta, first, second) = two_files();
        let joined = [first.as_slice(), second.as_slice()].concat();
        {
            let storage = Storage::new(meta.clone(), folder.path());
            storage.write(0, &joined).unwrap();
        }
        let storage = Storage::new(meta, folder.path());
        let mut back = [0u8; 10];
        storage.read(0, &mut back).unwrap();
        storage.write(0, &[9, 9]).unwrap();
        storage.read(0, &mut back).unwrap();
        assert_eq!(back[..2], [9, 9]);
    }

    #[test]
    fn test_stamps_change_when_a_file_changes() {
        let folder = TempFolder::new();
        let data = pattern(100, 1);
        let torrent = single_file_torrent("a.bin", &data, 16384);
        let storage = Storage::new(meta_of(&torrent), folder.path());
        assert_eq!(storage.stamps(), [None]);
        storage.write(0, &data).unwrap();
        storage.close();
        let before = storage.stamps();
        assert_eq!(before[0].as_ref().unwrap().len, 100);
        fs::write(folder.path().join("a.bin"), [0u8; 101]).unwrap();
        assert_ne!(storage.stamps(), before);
    }

    #[test]
    fn test_an_existing_file_of_the_wrong_size_is_corrected() {
        let folder = TempFolder::new();
        let data = pattern(100, 1);
        fs::write(folder.path().join("a.bin"), [7u8; 500]).unwrap();
        let storage = Storage::new(
            meta_of(&single_file_torrent("a.bin", &data, 16384)),
            folder.path(),
        );
        storage.write(0, &data).unwrap();
        storage.close();
        assert_eq!(fs::read(folder.path().join("a.bin")).unwrap(), data);
    }

    #[cfg(windows)]
    #[test]
    fn test_new_files_are_marked_as_from_the_internet() {
        let folder = TempFolder::new();
        let data = pattern(100, 1);
        let storage = Storage::new(
            meta_of(&single_file_torrent("a.bin", &data, 16384)),
            folder.path(),
        );
        storage.write(0, &data).unwrap();
        let marker = fs::read_to_string(format!(
            "{}:Zone.Identifier",
            folder.path().join("a.bin").display()
        ))
        .unwrap();
        assert!(marker.contains("ZoneId=3"));
    }

    #[cfg(windows)]
    #[test]
    fn test_new_files_are_sparse_on_windows() {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
        let folder = TempFolder::new();
        let data = pattern(100, 1);
        let storage = Storage::new(
            meta_of(&single_file_torrent("a.bin", &data, 16384)),
            folder.path(),
        );
        storage.write(0, &data).unwrap();
        let attributes = fs::metadata(folder.path().join("a.bin"))
            .unwrap()
            .file_attributes();
        assert_ne!(attributes & FILE_ATTRIBUTE_SPARSE_FILE, 0);
    }
}

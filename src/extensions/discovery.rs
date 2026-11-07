//! Read-only discovery of packaged extensions beside the application. Discovery
//! never opens a profile, installs code, grants permissions or runs Lua.
use super::{FILE_SUFFIX, MAX_PACKAGE_BYTES, Package, version};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 1024;
const MAX_PACKAGES: usize = 64;
const MAX_TOTAL_BYTES: usize = 16 * MAX_PACKAGE_BYTES;

#[derive(Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub package: Package,
    // The review must install exactly these bytes, even if the file changes.
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    New,
    Update,
    Current,
    Older,
    Changed,
}

impl Candidate {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn status(&self, installed: Option<&Package>) -> Result<Status> {
        let Some(installed) = installed else {
            return Ok(Status::New);
        };
        ensure!(installed.id == self.package.id, "Extension ID mismatch");
        Ok(
            match version(&self.package.version)?.cmp(&version(&installed.version)?) {
                std::cmp::Ordering::Greater => Status::Update,
                std::cmp::Ordering::Less => Status::Older,
                std::cmp::Ordering::Equal if installed == &self.package => Status::Current,
                std::cmp::Ordering::Equal => Status::Changed,
            },
        )
    }
}

#[derive(Debug)]
pub struct Issue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug)]
pub struct Report {
    pub directory: PathBuf,
    pub candidates: Vec<Candidate>,
    pub issues: Vec<Issue>,
}

/// Use application resources, never the current working directory or profile.
/// macOS resources live inside the app bundle; Windows/Linux use the exe folder.
pub fn directory() -> Result<PathBuf> {
    Ok(crate::paths::resource_directory()?.join("plugins"))
}

pub fn scan(directory: &Path) -> Result<Report> {
    let mut report = Report {
        directory: directory.to_owned(),
        candidates: Vec::new(),
        issues: Vec::new(),
    };
    let guard = match open_directory(directory) {
        Ok(guard) => guard,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error).context("Cannot read the plugins folder"),
    };
    let metadata = guard.metadata()?;
    ensure!(
        metadata.is_dir() && !is_link(&metadata),
        "Plugins folder must be a real directory"
    );
    let mut paths = Vec::new();
    for (index, entry) in fs::read_dir(directory)?.enumerate() {
        // Refuse an incomplete scan rather than arbitrarily picking one of two
        // packages with the same ID beyond a limit.
        ensure!(
            index < MAX_ENTRIES,
            "Plugins folder contains too many entries (limit: {MAX_ENTRIES})"
        );
        let path = entry?.path();
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(FILE_SUFFIX))
        {
            ensure!(
                paths.len() < MAX_PACKAGES,
                "Too many extension packages (limit: {MAX_PACKAGES})"
            );
            paths.push(path);
        }
    }
    paths.sort();
    let mut total = 0;
    for path in paths {
        let result = (|| -> Result<Candidate> {
            let file = read_file(&guard, &path)?;
            ensure!(
                file.metadata()?.len() <= MAX_PACKAGE_BYTES as u64,
                "Package exceeds the 1 MiB limit"
            );
            let mut bytes = Vec::new();
            file.take(MAX_PACKAGE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            total += bytes.len();
            ensure!(
                bytes.len() <= MAX_PACKAGE_BYTES,
                "Package exceeds the 1 MiB limit"
            );
            let package = super::store::checked(&bytes)?;
            Ok(Candidate {
                path: path.clone(),
                package,
                bytes,
            })
        })();
        ensure!(
            total <= MAX_TOTAL_BYTES,
            "Plugins folder exceeds the 16 MiB scan limit"
        );
        match result {
            Ok(candidate) => report.candidates.push(candidate),
            Err(error) => report.issues.push(Issue {
                path,
                message: format!("{error:#}"),
            }),
        }
    }
    let mut counts = BTreeMap::new();
    for candidate in &report.candidates {
        *counts.entry(candidate.package.id.clone()).or_insert(0) += 1;
    }
    report.candidates.retain(|candidate| {
        if counts[&candidate.package.id] == 1 {
            return true;
        }
        report.issues.push(Issue {
            path: candidate.path.clone(),
            message: format!(
                "Duplicate extension ID '{}'; keep only one package with this ID",
                candidate.package.id
            ),
        });
        false
    });
    Ok(report)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn open_directory(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        };
        // Do not follow a junction or let the scanned folder be replaced while
        // its files are read. Existing writable app directories remain usable.
        options
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        );
    }
    options.open(path)
}

fn read_file(_directory: &File, path: &Path) -> Result<File> {
    #[cfg(not(unix))]
    {
        crate::download_file::read_part_sync(path)
    }
    #[cfg(unix)]
    {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        };
        let name =
            std::ffi::CString::new(path.file_name().context("Missing package name")?.as_bytes())?;
        // Anchor to the opened directory; neither a root rename nor a leaf
        // symlink/FIFO can redirect or stall package discovery.
        let descriptor = unsafe {
            libc::openat(
                _directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        ensure!(
            descriptor >= 0,
            "Cannot open package: {}",
            std::io::Error::last_os_error()
        );
        let file = unsafe { File::from_raw_fd(descriptor) };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.nlink() == 1,
            "Package must be a regular file without links"
        );
        Ok(file)
    }
}

#[cfg(test)]
mod tests;

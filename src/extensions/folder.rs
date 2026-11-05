//! The extensions folder: packed extension files someone saved in the data
//! folder to install. Listing it only reads and checks those files. Each one
//! still goes through the normal install review, and nothing in the folder runs,
//! installs or updates by itself.
use super::{FILE_SUFFIX, MAX_PACKAGE_BYTES, Package};
use anyhow::{Context as _, Result, ensure};
use std::{fs, io::Read, path::Path};

/// Extension files past this many, in name order, are left out of the list.
pub const MAX_LISTED: usize = 100;
/// Characters kept of a reason a file can't be used. Decode errors can repeat
/// the file's own text, up to the whole file, which would stall the list.
const MAX_REASON: usize = 200;

/// The first line of `text`, cut to [`MAX_REASON`] characters.
pub fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    match line.char_indices().nth(MAX_REASON) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The file or folder name inside the extensions folder.
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A file that passes the same checks as an install. It is read again when
    /// someone reviews it, so the review shows the file as it is then.
    Package(Box<Package>),
    /// An extension file that can't be installed, and why.
    Unreadable(String),
    /// A folder holding a manifest.json: an extension that still needs packing.
    Unpacked,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// In name order.
    pub found: Vec<Found>,
    /// Other files and folders, links among them, which aren't read.
    pub skipped: usize,
    /// Extension files past [`MAX_LISTED`].
    pub left_out: usize,
}

/// A link, a junction or another reparse point. These are never followed, so
/// the folder can't be made to read something stored elsewhere.
fn link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// Files the system or a file browser hides, such as desktop.ini or .DS_Store.
fn hidden(name: &str, metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
        if metadata.file_attributes() & 0x6 != 0 {
            return true;
        }
    }
    #[cfg(not(windows))]
    let _ = metadata;
    name.starts_with('.')
}

fn extension_file(name: &str) -> bool {
    name.to_lowercase().ends_with(FILE_SUFFIX)
}

/// Opens a plain file without following a link and without waiting on a pipe
/// or a device. Its name was checked before, but what the name points to could
/// have been swapped since, so the file that was opened is checked too.
pub(super) fn open_plain(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Opens a link or junction itself, so the check below refuses it.
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A link fails to open, and a pipe opens at once so the check can refuse
        // it. Regular files are unaffected by O_NONBLOCK.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        !link(&metadata) && metadata.is_file(),
        "{} is not a plain file",
        path.display()
    );
    Ok(file)
}

/// Reads one extension file from the folder, without following links, and
/// checks it as an install would. `name` must be a plain file name.
pub fn read(folder: &Path, name: &str) -> Result<(Package, Vec<u8>)> {
    ensure!(
        Path::new(name).file_name().is_some_and(|file| file == name),
        "Not a file name in the extensions folder: {name}"
    );
    let path = folder.join(name);
    let metadata = fs::symlink_metadata(&path).with_context(|| format!("Can't read {name}"))?;
    ensure!(
        !link(&metadata) && metadata.is_file(),
        "Only files saved in the extensions folder are read. {name} is a link or a folder"
    );
    let file = open_plain(&path).with_context(|| format!("Can't read {name}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PACKAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Can't read {name}"))?;
    let package = super::store::checked(&bytes)?;
    Ok((package, bytes))
}

/// Lists the folder in name order. A folder that doesn't exist lists nothing.
pub fn scan(folder: &Path) -> Result<Listing> {
    let mut listing = Listing::default();
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(listing),
        Err(error) => {
            return Err(error).with_context(|| format!("Can't read {}", folder.display()));
        }
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("Can't read {}", folder.display()))?;
        // Neither of these follows a link.
        let Ok(metadata) = entry.metadata() else {
            listing.skipped += 1;
            continue;
        };
        // A name that isn't valid Unicode couldn't be found again to review it.
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            listing.skipped += 1;
            continue;
        };
        if hidden(&name, &metadata) {
            continue;
        }
        if link(&metadata) {
            listing.skipped += 1;
        } else if metadata.is_dir() {
            let manifest = fs::symlink_metadata(entry.path().join("manifest.json"));
            if manifest.is_ok_and(|manifest| manifest.is_file()) {
                candidates.push((name, true));
            } else {
                listing.skipped += 1;
            }
        } else if metadata.is_file() && extension_file(&name) {
            candidates.push((name, false));
        } else {
            listing.skipped += 1;
        }
    }
    candidates.sort_by(|(left, _), (right, _)| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    listing.left_out = candidates.len().saturating_sub(MAX_LISTED);
    candidates.truncate(MAX_LISTED);
    listing.found = candidates
        .into_iter()
        .map(|(name, unpacked)| {
            let kind = if unpacked {
                Kind::Unpacked
            } else {
                match read(folder, &name) {
                    Ok((package, _)) => Kind::Package(Box::new(package)),
                    Err(error) => Kind::Unreadable(short(&format!("{error:#}"))),
                }
            };
            Found { name, kind }
        })
        .collect();
    Ok(listing)
}

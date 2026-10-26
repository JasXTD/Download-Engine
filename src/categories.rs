//! Filename categories and destination rules used only when a download is created.
use crate::{model::Job, queue::batch_jobs, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Documents,
    Programs,
    Compressed,
    Music,
    Video,
    Other,
}
impl Category {
    pub const ALL: [Self; 6] = [
        Self::Documents,
        Self::Programs,
        Self::Compressed,
        Self::Music,
        Self::Video,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Documents => "Documents",
            Self::Programs => "Programs",
            Self::Compressed => "Compressed",
            Self::Music => "Music",
            Self::Video => "Video",
            Self::Other => "Other",
        }
    }
    pub fn examples(self) -> &'static str {
        match self {
            Self::Documents => "PDF, DOCX, XLSX, TXT, EPUB",
            Self::Programs => "EXE, MSI, MSIX, APK, DMG",
            Self::Compressed => "ZIP, 7Z, RAR, TAR, GZ",
            Self::Music => "MP3, FLAC, WAV, M4A, OGG",
            Self::Video => "MP4, MKV, WEBM, AVI, MOV",
            Self::Other => "Images, unknown types, and files without an extension",
        }
    }
    /// Use the final suffix only: report.pdf.exe is a program. Never treat a
    /// category as a safety verdict, and never let a filename supply a folder.
    pub fn from_filename(name: &str) -> Self {
        let extension = Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "csv" | "tsv" | "xls"
            | "xlsx" | "ods" | "ppt" | "pptx" | "odp" | "epub" | "mobi" => Self::Documents,
            "exe" | "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" | "msp" | "apk"
            | "dmg" | "pkg" | "deb" | "rpm" | "appimage" | "bat" | "cmd" | "ps1" | "com"
            | "scr" => Self::Programs,
            "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "zst"
            | "cab" => Self::Compressed,
            "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "wma" | "aiff"
            | "aif" | "alac" | "mid" | "midi" => Self::Music,
            "mp4" | "m4v" | "mkv" | "webm" | "ogv" | "avi" | "mov" | "wmv" | "flv" | "mpg"
            | "mpeg" | "3gp" | "m2ts" | "vob" => Self::Video,
            _ => Self::Other,
        }
    }
}
impl Job {
    /// Derive labels for old history without migrating or rewriting job data.
    pub fn category(&self) -> Category {
        if self.hls.is_some() {
            return Category::Video;
        }
        Category::from_filename(&self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadFolders {
    pub organize: bool,
    // Store only overrides so default folders follow a relocated portable app.
    pub overrides: BTreeMap<Category, PathBuf>,
}
impl Default for DownloadFolders {
    fn default() -> Self {
        Self {
            organize: true,
            overrides: BTreeMap::new(),
        }
    }
}
impl DownloadFolders {
    pub fn category_folder(&self, root: &Path, category: Category) -> PathBuf {
        self.overrides
            .get(&category)
            .cloned()
            .unwrap_or_else(|| root.join(category.label()))
    }
    pub fn destination(&self, root: &Path, name: &str) -> PathBuf {
        if self.organize {
            self.category_folder(root, Category::from_filename(name))
        } else {
            root.to_owned()
        }
    }
    fn validate(&self) -> Result<()> {
        for (category, path) in &self.overrides {
            validate_folder(path)
                .with_context(|| format!("{} folder is invalid", category.label()))?;
        }
        Ok(())
    }
}
pub(crate) fn validate_folder(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Choose an absolute download folder");
    ensure!(
        !path
            .as_os_str()
            .to_string_lossy()
            .chars()
            .any(char::is_control),
        "Folder paths cannot contain control characters"
    );
    #[cfg(windows)]
    ensure!(
        matches!(path.components().next(), Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_))),
        "Choose a local drive folder, such as C:\\Downloads"
    );
    Ok(())
}

fn same_folder_component(a: std::path::Component, b: std::path::Component) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
        // Unicode lowercasing conflates distinct Windows names (for example K
        // and the Kelvin sign). Compare the original UTF-16 with Windows rules.
        let a: Vec<u16> = a.as_os_str().encode_wide().collect();
        let b: Vec<u16> = b.as_os_str().encode_wide().collect();
        let (Ok(a_len), Ok(b_len)) = (i32::try_from(a.len()), i32::try_from(b.len())) else {
            return false;
        };
        // Both slices are live for this call; explicit lengths exclude any
        // terminator requirement. API failures never count as a match.
        unsafe { CompareStringOrdinal(a.as_ptr(), a_len, b.as_ptr(), b_len, 1) == CSTR_EQUAL }
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS download_folders (id INTEGER PRIMARY KEY CHECK(id=1), data TEXT NOT NULL);")?;
    conn.execute(
        "INSERT OR IGNORE INTO download_folders(id,data) VALUES(1,?1)",
        [serde_json::to_string(&DownloadFolders::default())?],
    )?;
    Ok(())
}
impl Store {
    /// Script-selected destinations stay under configured download roots.
    /// This performs no writes, and holds existing parents against redirection
    /// while a caller creates any missing child folders.
    pub(crate) fn guard_script_folder(&self, folder: &Path) -> Result<Vec<std::fs::File>> {
        validate_folder(folder)?;
        crate::download_file::validate_path(folder)?;
        let rules = self.download_folders()?;
        let inside = |root: &Path| {
            let mut parts = folder.components();
            root.components()
                .all(|r| parts.next().is_some_and(|p| same_folder_component(p, r)))
        };
        ensure!(
            std::iter::once(self.paths.downloads.as_path())
                .chain(rules.overrides.values().map(PathBuf::as_path))
                .any(inside),
            "Scripts can save or move files only into your download folder, a category folder, or a folder inside one"
        );
        let existing = folder
            .ancestors()
            .find(|path| path.exists())
            .context("The destination drive is unavailable")?;
        crate::download_file::guard_parents(&existing.join("check"))
    }

    pub fn download_folders(&self) -> Result<DownloadFolders> {
        let data: String =
            self.connect()?
                .query_row("SELECT data FROM download_folders WHERE id=1", [], |r| {
                    r.get(0)
                })?;
        let rules: DownloadFolders =
            serde_json::from_str(&data).context("Invalid download folder settings")?;
        rules.validate()?;
        Ok(rules)
    }
    /// One atomic update; invalid drafts cannot partly replace the saved rules.
    pub fn save_download_folders(&self, rules: &DownloadFolders) -> Result<()> {
        rules.validate()?;
        let count = self.connect()?.execute(
            "UPDATE download_folders SET data=?1 WHERE id=1",
            params![serde_json::to_string(rules)?],
        )?;
        ensure!(count == 1, "Download folder settings are unavailable");
        Ok(())
    }
    pub fn new_download(
        &self,
        url: &str,
        name: Option<&str>,
        folder: Option<&Path>,
        request_id: String,
    ) -> Result<Job> {
        let mut job = Job::new(
            url,
            name,
            folder.unwrap_or(&self.paths.downloads).to_owned(),
            request_id,
        )?;
        if let Some(folder) = folder {
            validate_folder(folder)?;
        } else {
            job.directory = self
                .download_folders()?
                .destination(&self.paths.downloads, &job.name);
        }
        Ok(job)
    }
    pub fn new_batch_downloads(&self, text: &str, folder: Option<&Path>) -> Result<Vec<Job>> {
        let mut jobs = batch_jobs(text, folder.unwrap_or(&self.paths.downloads))?;
        if let Some(folder) = folder {
            validate_folder(folder)?;
        } else {
            let rules = self.download_folders()?;
            for job in &mut jobs {
                job.directory = rules.destination(&self.paths.downloads, &job.name);
            }
        }
        Ok(jobs)
    }
}

#[cfg(all(test, windows))]
mod security_tests {
    use super::*;

    #[test]
    fn script_folder_matching_preserves_windows_case_rules() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let root = temp.path().join("Known");
        std::fs::create_dir(&root).unwrap();
        let mut rules = store.download_folders().unwrap();
        rules.overrides.insert(Category::Other, root);
        store.save_download_folders(&rules).unwrap();
        assert!(
            store
                .guard_script_folder(&temp.path().join("known").join("child"))
                .is_ok()
        );
        // Unicode lowercasing equates Kelvin-sign K with ASCII K, but Windows
        // treats these as different folders. A look-alike is outside the root.
        let lookalike = temp.path().join("\u{212a}nown");
        assert!(!lookalike.exists());
        assert!(store.guard_script_folder(&lookalike.join("child")).is_err());
        assert!(!lookalike.exists());
    }
}

//! Download list export and import, for backups and moving to another PC.
//!
//! A list carries links and organisation only: URL, file name, folder, queue
//! name, category and expected SHA-256. Logins, cookies, source pages, partial
//! data, resume plans and errors never leave the store. Imported entries are
//! untrusted: every one is validated, and nothing is replaced or started.
use crate::{
    categories::{Category, validate_folder},
    model::{Job, Status, validate_download_url},
    queue::{DownloadQueue, queue_name},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::TransactionBehavior;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// The `format` value of a Download Engine list (JSON).
pub const FORMAT: &str = "xdownloader.download-list";
/// Readers reject newer versions instead of guessing at unknown semantics.
pub const VERSION: u64 = 1;
/// Larger files are rejected before parsing.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Entries (JSON downloads or non-empty text lines) accepted per file.
pub const MAX_ENTRIES: usize = 10_000;
const MAX_FOLDER_CHARS: usize = 4096;
const EXCERPT_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A Download Engine list with names, folders, queues and checksums.
    Json,
    /// One link per line, the format Add batch and `de-cli batch` read.
    Text,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Text => "txt",
        }
    }
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "json" => Some(Self::Json),
            "txt" => Some(Self::Text),
            _ => None,
        }
    }
}

/// File-level problems. Callers can match them to show translated text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListError {
    TooLarge,
    TooManyEntries,
    NotUtf8,
    NotAList,
    NewerVersion,
    Empty,
    Exists,
}
impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "This file is larger than 16 MiB. Split the list into smaller files.",
            Self::TooManyEntries => {
                "This list has more than 10,000 entries. Split it into smaller files."
            }
            Self::NotUtf8 => "Save the list as UTF-8 text, then import it again.",
            Self::NotAList => "This file is not a Download Engine list or a text file of links.",
            Self::NewerVersion => {
                "This list was made by a newer version of Download Engine. Update Download Engine to import it."
            }
            Self::Empty => "This list has no downloads.",
            Self::Exists => "A file with this name already exists. Choose another name.",
        })
    }
}
impl std::error::Error for ListError {}

/// One exported download. Fields are copied one by one, so state added to
/// `Job` later can never leak into a list by accident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub url: String,
    pub name: String,
    pub folder: String,
    pub queue: String,
    pub category: Category,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Which downloads an export covers. Downloads are listed per queue, in queue order.
pub enum Scope {
    All,
    Queue(String),
    Jobs(HashSet<String>),
}

#[derive(Default)]
pub struct Collected {
    pub entries: Vec<Entry>,
    /// HLS, DASH and YouTube streams need Preview video; a plain link cannot add them.
    pub streams: usize,
}

/// Streams are left out of every list: Add batch and import reject their links.
pub fn is_stream(job: &Job) -> bool {
    job.hls.is_some() || crate::hls::is_playlist_url(&job.url)
}

#[derive(Serialize)]
struct Document<'a> {
    format: &'static str,
    version: u64,
    application: String,
    exported_utc: String,
    downloads: &'a [Entry],
}

/// Serialize entries. Lists that import would reject are refused here too.
pub fn render(entries: &[Entry], format: Format) -> Result<Vec<u8>> {
    anyhow::ensure!(!entries.is_empty(), ListError::Empty);
    anyhow::ensure!(entries.len() <= MAX_ENTRIES, ListError::TooManyEntries);
    let bytes = match format {
        Format::Json => {
            let mut bytes = serde_json::to_vec_pretty(&Document {
                format: FORMAT,
                version: VERSION,
                application: format!("Download Engine {}", env!("CARGO_PKG_VERSION")),
                exported_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                downloads: entries,
            })?;
            bytes.push(b'\n');
            bytes
        }
        Format::Text => entries
            .iter()
            .flat_map(|entry| [entry.url.as_bytes(), b"\r\n"])
            .flatten()
            .copied()
            .collect(),
    };
    anyhow::ensure!(bytes.len() <= MAX_BYTES, ListError::TooLarge);
    Ok(bytes)
}

/// Write beside the destination, then move it into place, so a reader never sees
/// a partial list. `replace` must be true only after the user confirmed replacing
/// an existing file; otherwise a file that exists by then is left untouched.
pub fn write_file(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    ensure!(path.is_absolute(), "Choose an absolute file path");
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .context("Choose an existing folder for the list")?;
    let temporary = parent.join(format!(
        "~DownloadEngine-list-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if replace {
            fs::rename(&temporary, path)?;
        } else if let Err(error) = move_new(&temporary, path) {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                return Err(ListError::Exists.into());
            }
            return Err(error.into());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let wide =
        |path: &Path| -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (from, to) = (wide(from), wide(to));
    // Without MOVEFILE_REPLACE_EXISTING the move fails if the destination exists.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::hard_link(from, to)?;
    fs::remove_file(from)
}

/// A valid entry read from a list, before checking it against this PC's store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Line number (text) or entry number (JSON), counting from 1.
    pub number: usize,
    pub url: String,
    pub name: Option<String>,
    pub folder: Option<PathBuf>,
    pub queue: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The link is rejected by download validation; the text is that error.
    Invalid(String),
    Stream,
    Checksum,
    /// The JSON entry is not an object with string fields and a URL.
    Format,
    /// The same link appears earlier in this list.
    Repeated,
    /// The link is already in the download list.
    Existing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub number: usize,
    /// A short, single-line excerpt of the entry's link for the details list.
    pub excerpt: String,
    pub problem: Problem,
}

pub struct Parsed {
    pub format: Format,
    pub candidates: Vec<Candidate>,
    pub rejected: Vec<Rejected>,
}

/// Read a list chosen by the user. The size is capped before parsing.
pub fn read_file(path: &Path) -> Result<Parsed> {
    ensure!(fs::metadata(path)?.is_file(), "Choose a list file");
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    parse(&bytes)
}

/// Validate a list without touching the store. JSON is recognised by content.
pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    anyhow::ensure!(bytes.len() <= MAX_BYTES, ListError::TooLarge);
    let text = std::str::from_utf8(bytes).map_err(|_| ListError::NotUtf8)?;
    let text = text.trim_start_matches('\u{feff}');
    let document: Value;
    // (number, fields or problem, the entry's link for the details list)
    let mut entries = Vec::new();
    let format = if text.trim_start().starts_with(['{', '[']) {
        document = serde_json::from_str(text).map_err(|_| ListError::NotAList)?;
        let object = document.as_object().ok_or(ListError::NotAList)?;
        anyhow::ensure!(
            object.get("format").and_then(Value::as_str) == Some(FORMAT),
            ListError::NotAList
        );
        match object.get("version").and_then(Value::as_u64) {
            Some(VERSION) => {}
            Some(version) if version > VERSION => return Err(ListError::NewerVersion.into()),
            _ => return Err(ListError::NotAList.into()),
        }
        let downloads = object
            .get("downloads")
            .and_then(Value::as_array)
            .ok_or(ListError::NotAList)?;
        anyhow::ensure!(downloads.len() <= MAX_ENTRIES, ListError::TooManyEntries);
        for (index, value) in downloads.iter().enumerate() {
            let raw = value.get("url").and_then(Value::as_str).unwrap_or("");
            entries.push((index + 1, json_fields(value), raw));
        }
        Format::Json
    } else {
        for (index, line) in text.lines().enumerate() {
            if !line.trim().is_empty() {
                anyhow::ensure!(entries.len() < MAX_ENTRIES, ListError::TooManyEntries);
                entries.push((index + 1, Ok(Fields::link(line.trim())), line));
            }
        }
        Format::Text
    };
    anyhow::ensure!(!entries.is_empty(), ListError::Empty);
    let mut parsed = Parsed {
        format,
        candidates: Vec::new(),
        rejected: Vec::new(),
    };
    let mut seen = HashSet::new();
    for (number, fields, raw) in entries {
        let result = fields.and_then(candidate).and_then(|candidate| {
            if seen.insert(candidate.url.clone()) {
                Ok(Candidate {
                    number,
                    ..candidate
                })
            } else {
                Err(Problem::Repeated)
            }
        });
        match result {
            Ok(candidate) => parsed.candidates.push(candidate),
            Err(problem) => parsed.rejected.push(Rejected {
                number,
                excerpt: excerpt(raw),
                problem,
            }),
        }
    }
    Ok(parsed)
}

struct Fields<'a> {
    url: &'a str,
    name: Option<&'a str>,
    folder: Option<&'a str>,
    queue: Option<&'a str>,
    sha256: Option<&'a str>,
}
impl<'a> Fields<'a> {
    fn link(url: &'a str) -> Self {
        Self {
            url,
            name: None,
            folder: None,
            queue: None,
            sha256: None,
        }
    }
}

/// Unknown fields are ignored, so later versions may add optional data.
fn json_fields(value: &Value) -> Result<Fields<'_>, Problem> {
    let object = value.as_object().ok_or(Problem::Format)?;
    let text = |key: &str| match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(Problem::Format),
    };
    // Category is derived from the file name on import; it is read only to
    // reject entries whose fields have the wrong type.
    text("category")?;
    Ok(Fields {
        url: text("url")?.ok_or(Problem::Format)?,
        name: text("name")?,
        folder: text("folder")?,
        queue: text("queue")?,
        sha256: text("sha256")?,
    })
}

fn candidate(fields: Fields<'_>) -> Result<Candidate, Problem> {
    let url = validate_download_url(fields.url)
        .map_err(|error| Problem::Invalid(error.to_string()))?
        .to_string();
    if crate::hls::is_playlist_url(&url) {
        return Err(Problem::Stream);
    }
    let sha256 = match fields.sha256 {
        Some(value) => crate::integrity::parse_expected(value).map_err(|_| Problem::Checksum)?,
        None => None,
    };
    // A folder or queue that cannot be used here falls back to the user's choice.
    let folder = fields
        .folder
        .map(str::trim)
        .filter(|folder| folder.chars().count() <= MAX_FOLDER_CHARS)
        .map(PathBuf::from)
        .filter(|folder| validate_folder(folder).is_ok());
    Ok(Candidate {
        number: 0,
        url,
        name: fields
            .name
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned),
        folder,
        queue: fields
            .queue
            .and_then(|name| queue_name(name).ok())
            .map(str::to_owned),
        sha256,
    })
}

/// A single-line, shortened link for the details list. A user name and password
/// written into a rejected link are hidden, so the dialog never shows them.
fn excerpt(raw: &str) -> String {
    let raw = raw.trim();
    let hidden;
    let raw = match raw.split_once("://") {
        Some((scheme, rest)) => {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            match rest[..end].rfind('@') {
                Some(at) => {
                    hidden = format!("{scheme}://***@{}", &rest[at + 1..]);
                    hidden.as_str()
                }
                None => raw,
            }
        }
        None => raw,
    };
    let text: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(EXCERPT_CHARS + 1)
        .collect();
    if text.chars().count() > EXCERPT_CHARS {
        format!("{}…", text.chars().take(EXCERPT_CHARS).collect::<String>())
    } else {
        text
    }
}

/// A parsed list checked against this PC's downloads, queues and folders.
pub struct Review {
    pub format: Format,
    /// Valid links that are not in the download list yet, in list order.
    pub candidates: Vec<Candidate>,
    pub rejected: Vec<Rejected>,
    /// Distinct queue names in the list, and how many match a queue here.
    pub queue_names: usize,
    pub matched_queues: usize,
    /// Distinct folders in the list that exist on this PC, and those that do not.
    pub folders: Vec<PathBuf>,
    pub missing_folders: usize,
}
impl Review {
    pub fn existing(&self) -> usize {
        self.rejected
            .iter()
            .filter(|r| matches!(r.problem, Problem::Existing | Problem::Repeated))
            .count()
    }
    pub fn invalid(&self) -> usize {
        self.rejected.len() - self.existing()
    }
}

pub struct ImportOptions {
    /// Queue for entries without a matching queue name. It must exist.
    pub queue: String,
    pub match_queues: bool,
    /// Use the list's folders when they exist on this PC; otherwise the
    /// download folder rules choose the destination.
    pub saved_folders: bool,
    /// Paused entries never start by themselves. Queued entries follow their
    /// queue, like Add batch.
    pub paused: bool,
}

pub struct Imported {
    pub added: Vec<Job>,
    /// Links added to the list by something else after the review.
    pub existing: usize,
}

fn find_queue<'a>(queues: &'a [DownloadQueue], name: &str) -> Option<&'a DownloadQueue> {
    let name = name.to_lowercase();
    queues
        .iter()
        .find(|queue| queue.name.to_lowercase() == name)
}

impl Store {
    pub fn export_list(&self, scope: &Scope, skip_complete: bool) -> Result<Collected> {
        let mut collected = Collected::default();
        for queue in self.queues()? {
            if matches!(scope, Scope::Queue(id) if *id != queue.id) {
                continue;
            }
            for job in self.queue_jobs(&queue.id)? {
                if matches!(scope, Scope::Jobs(ids) if !ids.contains(&job.id))
                    || (skip_complete && job.status == Status::Complete)
                {
                    continue;
                }
                if is_stream(&job) {
                    collected.streams += 1;
                    continue;
                }
                collected.entries.push(Entry {
                    category: job.category(),
                    url: job.url,
                    name: job.name,
                    folder: job.directory.to_string_lossy().into_owned(),
                    queue: queue.name.clone(),
                    sha256: job.integrity.expected,
                });
            }
        }
        Ok(collected)
    }

    fn listed_urls(conn: &rusqlite::Connection) -> Result<HashSet<String>> {
        let mut statement = conn.prepare("SELECT json_extract(data,'$.url') FROM jobs")?;
        let urls = statement
            .query_map([], |row| row.get::<_, Option<String>>(0))?
            .filter_map(|url| url.transpose())
            .collect::<rusqlite::Result<_>>()?;
        Ok(urls)
    }

    pub fn review_import(&self, parsed: Parsed) -> Result<Review> {
        let listed = Self::listed_urls(&self.connect()?)?;
        let queues = self.queues()?;
        let mut review = Review {
            format: parsed.format,
            candidates: Vec::new(),
            rejected: parsed.rejected,
            queue_names: 0,
            matched_queues: 0,
            folders: Vec::new(),
            missing_folders: 0,
        };
        for candidate in parsed.candidates {
            if listed.contains(&candidate.url) {
                review.rejected.push(Rejected {
                    number: candidate.number,
                    excerpt: excerpt(&candidate.url),
                    problem: Problem::Existing,
                });
            } else {
                review.candidates.push(candidate);
            }
        }
        review.rejected.sort_by_key(|rejected| rejected.number);
        let mut names = HashSet::new();
        let mut folders = HashSet::new();
        for candidate in &review.candidates {
            if let Some(name) = &candidate.queue
                && names.insert(name.to_lowercase())
                && find_queue(&queues, name).is_some()
            {
                review.matched_queues += 1;
            }
            if let Some(folder) = &candidate.folder
                && folders.insert(folder.clone())
            {
                if folder.is_dir() {
                    review.folders.push(folder.clone());
                } else {
                    review.missing_folders += 1;
                }
            }
        }
        review.queue_names = names.len();
        Ok(review)
    }

    /// Add reviewed entries in one transaction. Links already in the list are
    /// skipped again here, so an import never replaces or duplicates a download.
    pub fn import_list(
        &self,
        candidates: &[Candidate],
        options: &ImportOptions,
    ) -> Result<Imported> {
        ensure!(
            candidates.len() <= MAX_ENTRIES,
            "Import at most 10,000 downloads at a time"
        );
        let rules = self.download_folders()?;
        let queues = self.queues()?;
        ensure!(
            queues.iter().any(|queue| queue.id == options.queue),
            "Queue no longer exists"
        );
        let mut groups: Vec<(String, Vec<Job>)> = Vec::new();
        for candidate in candidates {
            let mut job = Job::new(
                &candidate.url,
                candidate.name.as_deref(),
                self.paths.downloads.clone(),
                uuid::Uuid::new_v4().to_string(),
            )?;
            job.directory = match &candidate.folder {
                Some(folder) if options.saved_folders && folder.is_dir() => folder.clone(),
                _ => rules.destination(&self.paths.downloads, &job.name),
            };
            job.integrity.expected = candidate.sha256.clone();
            if options.paused {
                job.status = Status::Paused;
            }
            let queue = candidate
                .queue
                .as_deref()
                .filter(|_| options.match_queues)
                .and_then(|name| find_queue(&queues, name))
                .map_or(options.queue.as_str(), |queue| queue.id.as_str());
            match groups.iter_mut().find(|(id, _)| id == queue) {
                Some((_, jobs)) => jobs.push(job),
                None => groups.push((queue.to_owned(), vec![job])),
            }
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut listed = Self::listed_urls(&tx)?;
        let mut imported = Imported {
            added: Vec::new(),
            existing: 0,
        };
        for (queue, jobs) in groups {
            let fresh: Vec<Job> = jobs
                .into_iter()
                .filter(|job| listed.insert(job.url.clone()))
                .collect();
            for chunk in fresh.chunks(1000) {
                imported
                    .added
                    .extend(crate::queue_store::enqueue_batch_in(&tx, chunk, &queue)?);
            }
        }
        imported.existing = candidates.len() - imported.added.len();
        tx.commit()?;
        Ok(imported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{paths::AppPaths, queue::MAIN_QUEUE};
    use rusqlite::params;
    use serde_json::json;
    use std::collections::BTreeSet;

    const SECRET: &str = "XD-PRIVATE-7f3a";

    fn store(root: &Path) -> Store {
        Store::open(AppPaths {
            data: root.join("data"),
            downloads: root.join("downloads"),
        })
        .unwrap()
    }
    fn add(store: &Store, url: &str, queue: &str) -> Job {
        let job = store
            .new_download(url, None, None, uuid::Uuid::new_v4().to_string())
            .unwrap();
        store.enqueue_batch(&[job], queue).unwrap().remove(0)
    }
    /// Streams cannot be added from a plain link, so store one the way a preview would.
    fn add_stream(store: &Store) -> Job {
        let job = Job::new(
            "https://example.com/live/master.m3u8",
            None,
            store.paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        let conn = store.connect().unwrap();
        conn.execute(
            "INSERT INTO jobs(id,request_id,created,data) VALUES(?1,?2,?3,?4)",
            params![
                job.id,
                job.request_id,
                job.created as i64,
                serde_json::to_string(&job).unwrap()
            ],
        )
        .unwrap();
        crate::queue_store::assign(&conn, &job.id, MAIN_QUEUE).unwrap();
        job
    }
    fn keys(value: &Value) -> BTreeSet<&str> {
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    }
    fn options(queue: &str) -> ImportOptions {
        ImportOptions {
            queue: queue.into(),
            match_queues: true,
            saved_folders: true,
            paused: true,
        }
    }
    fn error<T>(result: Result<T>) -> ListError {
        *result
            .err()
            .expect("the file is rejected")
            .downcast_ref::<ListError>()
            .expect("a file-level list error")
    }

    #[test]
    fn exports_contain_only_list_fields_and_never_private_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let queue = store.create_queue("Night").unwrap();
        let mut job = add(
            &store,
            "https://example.com/files/setup.exe?build=4",
            &queue.id,
        );
        job.status = Status::Paused;
        job.source_page = Some(format!("https://example.com/login?session={SECRET}"));
        job.authentication = Some(crate::auth::Binding {
            id: uuid::Uuid::new_v4(),
            origin: format!("https://{SECRET}.example"),
            kind: crate::auth::Kind::Bearer,
        });
        job.plan = Some(crate::model::Plan {
            size: 10,
            etag: SECRET.into(),
            final_url: format!("https://cdn.example/{SECRET}"),
            segments: vec![crate::model::Segment { start: 0, end: 9 }],
        });
        job.expected_file = Some(crate::capture::ExpectedFile {
            size: 10,
            mime: SECRET.into(),
            etag: Some(SECRET.into()),
        });
        job.integrity.expected = Some("ab".repeat(32));
        job.integrity.actual = Some(SECRET.into());
        job.error = Some(SECRET.into());
        job.saved_path = Some(temp.path().join(format!("{SECRET}.part")));
        job.downloaded = 5;
        job.total = Some(10);
        store.save(&job).unwrap();

        let collected = store.export_list(&Scope::All, false).unwrap();
        let json = String::from_utf8(render(&collected.entries, Format::Json).unwrap()).unwrap();
        assert!(!json.contains(SECRET), "{json}");
        let document: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            keys(&document),
            BTreeSet::from([
                "application",
                "downloads",
                "exported_utc",
                "format",
                "version"
            ])
        );
        assert_eq!(document["format"], FORMAT);
        assert_eq!(document["version"], 1);
        let entry = &document["downloads"][0];
        assert_eq!(
            keys(entry),
            BTreeSet::from(["category", "folder", "name", "queue", "sha256", "url"])
        );
        assert_eq!(entry["url"], "https://example.com/files/setup.exe?build=4");
        assert_eq!(entry["name"], "setup.exe");
        assert_eq!(entry["queue"], "Night");
        assert_eq!(entry["category"], "programs");
        assert_eq!(entry["sha256"], "ab".repeat(32));
        assert_eq!(
            entry["folder"],
            store.paths.downloads.join("Programs").display().to_string()
        );
        let text = render(&collected.entries, Format::Text).unwrap();
        assert_eq!(text, b"https://example.com/files/setup.exe?build=4\r\n");
    }

    #[test]
    fn exports_follow_their_scope_and_leave_out_streams() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let night = store.create_queue("Night").unwrap();
        let mut done = add(&store, "https://example.com/a.zip", MAIN_QUEUE);
        done.status = Status::Complete;
        store.save(&done).unwrap();
        let guide = add(&store, "https://example.com/b.pdf", MAIN_QUEUE);
        add(&store, "https://example.com/c.mp4", &night.id);
        add_stream(&store);
        let urls = |scope: Scope, skip: bool| -> (Vec<String>, usize) {
            let collected = store.export_list(&scope, skip).unwrap();
            let urls = collected.entries.into_iter().map(|e| e.url).collect();
            (urls, collected.streams)
        };
        assert_eq!(
            urls(Scope::All, false),
            (
                vec![
                    "https://example.com/a.zip".into(),
                    "https://example.com/b.pdf".into(),
                    "https://example.com/c.mp4".into()
                ],
                1
            )
        );
        assert_eq!(urls(Scope::All, true).0.len(), 2);
        assert_eq!(
            urls(Scope::Queue(night.id.clone()), false),
            (vec!["https://example.com/c.mp4".into()], 0)
        );
        assert_eq!(
            urls(Scope::Jobs(HashSet::from([guide.id])), false),
            (vec!["https://example.com/b.pdf".into()], 0)
        );
        assert_eq!(error(render(&[], Format::Json)), ListError::Empty);
        let entry = store.export_list(&Scope::All, false).unwrap().entries[0].clone();
        let many = vec![entry; MAX_ENTRIES + 1];
        assert_eq!(
            error(render(&many, Format::Text)),
            ListError::TooManyEntries
        );
        assert!(render(&many[..MAX_ENTRIES], Format::Json).is_ok());
    }

    #[test]
    fn lists_round_trip_to_another_store_paused_in_matching_queues() {
        let temp = tempfile::tempdir().unwrap();
        let source = store(&temp.path().join("source"));
        let night = source.create_queue("Night").unwrap();
        let mut checked = source
            .new_download(
                "https://example.com/b.pdf",
                Some("Guide 2.pdf"),
                None,
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap();
        checked.integrity.expected = Some("cd".repeat(32));
        source.enqueue_batch(&[checked], MAIN_QUEUE).unwrap();
        add(&source, "https://example.com/c.mp4", &night.id);
        let entries = source.export_list(&Scope::All, false).unwrap().entries;
        for entry in &entries {
            fs::create_dir_all(&entry.folder).unwrap();
        }

        // Text lists stay compatible with Add batch and `de-cli batch`.
        let text = String::from_utf8(render(&entries, Format::Text).unwrap()).unwrap();
        let batch = crate::queue::batch_jobs(&text, temp.path()).unwrap();
        assert_eq!(
            batch.iter().map(|job| &job.url).collect::<Vec<_>>(),
            entries.iter().map(|entry| &entry.url).collect::<Vec<_>>()
        );
        let parsed = parse(text.as_bytes()).unwrap();
        assert_eq!(parsed.format, Format::Text);
        assert_eq!(parsed.candidates.len(), 2);

        let target = store(&temp.path().join("target"));
        let night_here = target.create_queue("night").unwrap();
        let parsed = parse(&render(&entries, Format::Json).unwrap()).unwrap();
        assert_eq!(parsed.format, Format::Json);
        let review = target.review_import(parsed).unwrap();
        assert_eq!((review.queue_names, review.matched_queues), (2, 2));
        assert_eq!(review.folders.len(), 2);
        let imported = target
            .import_list(&review.candidates, &options(MAIN_QUEUE))
            .unwrap();
        assert_eq!((imported.added.len(), imported.existing), (2, 0));
        for (entry, job) in entries.iter().zip(&imported.added) {
            assert_eq!(job.url, entry.url);
            assert_eq!(job.name, entry.name);
            assert_eq!(job.directory, PathBuf::from(&entry.folder));
            assert_eq!(job.integrity.expected, entry.sha256);
            assert_eq!(job.status, Status::Paused);
            assert!(job.authentication.is_none() && job.source_page.is_none());
        }
        assert_eq!(imported.added[0].name, "Guide 2.pdf");
        assert_eq!(
            target.job_queue(&imported.added[0].id).unwrap().as_deref(),
            Some(MAIN_QUEUE)
        );
        assert_eq!(
            target.job_queue(&imported.added[1].id).unwrap(),
            Some(night_here.id)
        );
    }

    #[test]
    fn imports_validate_each_link_and_skip_links_already_listed() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let existing = add(&store, "https://example.com/existing.zip", MAIN_QUEUE);
        let before = serde_json::to_string(&store.get(&existing.id).unwrap()).unwrap();
        let text = "\u{feff}https://example.com/new.zip\n\nnot a link\nfile:///C:/Windows/win.ini\nhttps://user:pass@example.com/a.zip\r\nhttps://example.com/live/index.m3u8\nhttps://example.com/new.zip#again\n  https://example.com/existing.zip  \njavascript:alert(1)\n";
        let review = store
            .review_import(parse(text.as_bytes()).unwrap())
            .unwrap();
        assert_eq!(review.format, Format::Text);
        assert_eq!(
            review
                .candidates
                .iter()
                .map(|c| (c.number, c.url.as_str()))
                .collect::<Vec<_>>(),
            [(1, "https://example.com/new.zip")]
        );
        let problems: Vec<_> = review
            .rejected
            .iter()
            .map(|r| (r.number, r.problem.clone()))
            .collect();
        assert!(matches!(problems[0], (3, Problem::Invalid(_))));
        assert!(matches!(&problems[1], (4, Problem::Invalid(e)) if e.contains("HTTP")));
        assert!(matches!(&problems[2], (5, Problem::Invalid(e)) if e.contains("passwords")));
        assert_eq!(
            problems[3..6],
            [
                (6, Problem::Stream),
                (7, Problem::Repeated),
                (8, Problem::Existing)
            ]
        );
        assert!(matches!(problems[6], (9, Problem::Invalid(_))));
        assert_eq!((review.existing(), review.invalid()), (2, 5));
        assert_eq!(review.rejected[2].excerpt, "https://***@example.com/a.zip");

        let imported = store
            .import_list(&review.candidates, &options(MAIN_QUEUE))
            .unwrap();
        assert_eq!(imported.added.len(), 1);
        assert_eq!(store.list().unwrap().len(), 2);
        assert_eq!(
            serde_json::to_string(&store.get(&existing.id).unwrap()).unwrap(),
            before
        );
        // A link added after the review is skipped, never duplicated.
        let again = store
            .review_import(parse(b"https://example.com/later.zip\n").unwrap())
            .unwrap();
        add(&store, "https://example.com/later.zip", MAIN_QUEUE);
        let imported = store
            .import_list(&again.candidates, &options(MAIN_QUEUE))
            .unwrap();
        assert_eq!((imported.added.len(), imported.existing), (0, 1));
        assert_eq!(store.list().unwrap().len(), 3);
    }

    #[test]
    fn json_entries_are_checked_field_by_field_and_unknown_places_fall_back() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let night = store.create_queue("Night").unwrap();
        let kept = temp.path().join("kept");
        fs::create_dir(&kept).unwrap();
        let missing = temp.path().join("missing").join("sub");
        let document = json!({"format": FORMAT, "version": 1, "added_later": true, "downloads": [
            {"url": "https://example.com/a.zip", "name": "Renamed.zip", "folder": kept, "queue": "NIGHT",
             "sha256": "AB".repeat(32), "cookies": format!("session={SECRET}"), "category": "compressed"},
            {"url": "https://example.com/b.zip", "queue": "Gone", "folder": "relative\\folder"},
            {"url": "https://example.com/c.zip", "folder": missing, "name": "", "queue": "\u{7}"},
            {"url": "https://example.com/d.zip", "sha256": "xyz"},
            {"url": 5},
            {"name": "no-link.zip"},
            "https://example.com/e.zip",
            {"url": "https://example.com/f.zip", "name": ["not", "text"]},
            {"url": "https://example.com/g.zip", "category": 3},
        ]});
        let parsed = parse(document.to_string().as_bytes()).unwrap();
        assert_eq!(
            parsed
                .rejected
                .iter()
                .map(|r| (r.number, r.problem.clone()))
                .collect::<Vec<_>>(),
            [
                (4, Problem::Checksum),
                (5, Problem::Format),
                (6, Problem::Format),
                (7, Problem::Format),
                (8, Problem::Format),
                (9, Problem::Format)
            ]
        );
        let first = &parsed.candidates[0];
        assert_eq!(first.name.as_deref(), Some("Renamed.zip"));
        assert_eq!(first.folder.as_deref(), Some(kept.as_path()));
        assert_eq!(first.sha256, Some("ab".repeat(32)));
        assert_eq!(parsed.candidates[1].folder, None);
        assert_eq!(parsed.candidates[1].queue.as_deref(), Some("Gone"));
        assert_eq!(parsed.candidates[2].name, None);
        assert_eq!(parsed.candidates[2].queue, None);

        let review = store.review_import(parsed).unwrap();
        assert_eq!((review.queue_names, review.matched_queues), (2, 1));
        assert_eq!(
            (review.folders.clone(), review.missing_folders),
            (vec![kept.clone()], 1)
        );
        let imported = store
            .import_list(&review.candidates, &options(MAIN_QUEUE))
            .unwrap();
        let compressed = store.paths.downloads.join("Compressed");
        let placed: Vec<_> = imported
            .added
            .iter()
            .map(|job| {
                (
                    job.name.as_str(),
                    job.directory.clone(),
                    store.job_queue(&job.id).unwrap().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            placed,
            [
                ("Renamed.zip", kept.clone(), night.id.clone()),
                ("b.zip", compressed.clone(), MAIN_QUEUE.to_owned()),
                ("c.zip", compressed.clone(), MAIN_QUEUE.to_owned()),
            ]
        );
        assert!(!missing.exists(), "imports never create the list's folders");
        let saved = serde_json::to_string(&store.list().unwrap()).unwrap();
        assert!(!saved.contains(SECRET));

        // Without opting in to the list's folders and queues, the user's choices apply.
        let other = self::store(&temp.path().join("other"));
        other.create_queue("Night").unwrap();
        let review = other
            .review_import(parse(document.to_string().as_bytes()).unwrap())
            .unwrap();
        let imported = other
            .import_list(
                &review.candidates,
                &ImportOptions {
                    queue: MAIN_QUEUE.into(),
                    match_queues: false,
                    saved_folders: false,
                    paused: true,
                },
            )
            .unwrap();
        assert_eq!(
            imported.added[0].directory,
            other.paths.downloads.join("Compressed")
        );
        assert!(
            imported
                .added
                .iter()
                .all(|job| other.job_queue(&job.id).unwrap().as_deref() == Some(MAIN_QUEUE))
        );
        assert!(
            other
                .import_list(&review.candidates, &options("missing-queue"))
                .is_err()
        );
    }

    #[test]
    fn oversized_malformed_and_unsupported_files_are_rejected_whole() {
        assert_eq!(
            error(parse(&vec![b'a'; MAX_BYTES + 1])),
            ListError::TooLarge
        );
        let lines = |count: usize| -> String {
            (0..count)
                .map(|i| format!("https://example.com/{i}.zip\n"))
                .collect()
        };
        assert_eq!(
            error(parse(lines(MAX_ENTRIES + 1).as_bytes())),
            ListError::TooManyEntries
        );
        assert_eq!(
            parse(lines(MAX_ENTRIES).as_bytes())
                .unwrap()
                .candidates
                .len(),
            MAX_ENTRIES
        );
        let downloads = vec![json!({"url": "https://example.com/a.zip"}); MAX_ENTRIES + 1];
        let document = json!({"format": FORMAT, "version": 1, "downloads": downloads});
        assert_eq!(
            error(parse(document.to_string().as_bytes())),
            ListError::TooManyEntries
        );
        assert_eq!(error(parse(&[0xff, 0xfe, b'h', 0])), ListError::NotUtf8);
        for text in [
            "{not json".to_owned(),
            "[]".into(),
            json!({"format": "other", "version": 1, "downloads": []}).to_string(),
            json!({"format": FORMAT, "version": 0, "downloads": []}).to_string(),
            json!({"format": FORMAT, "version": "1", "downloads": []}).to_string(),
            json!({"format": FORMAT, "version": 1}).to_string(),
            json!({"format": FORMAT, "version": 1, "downloads": {}}).to_string(),
        ] {
            assert_eq!(error(parse(text.as_bytes())), ListError::NotAList, "{text}");
        }
        let newer = json!({"format": FORMAT, "version": 2, "downloads": []});
        assert_eq!(
            error(parse(newer.to_string().as_bytes())),
            ListError::NewerVersion
        );
        let empty = json!({"format": FORMAT, "version": 1, "downloads": []});
        for text in [empty.to_string(), String::new(), "\u{feff} \r\n\n".into()] {
            assert_eq!(error(parse(text.as_bytes())), ListError::Empty);
        }

        let temp = tempfile::tempdir().unwrap();
        let large = temp.path().join("large.txt");
        fs::write(&large, vec![b'\n'; MAX_BYTES + 1]).unwrap();
        assert_eq!(error(read_file(&large)), ListError::TooLarge);
        assert!(read_file(temp.path()).is_err());
        assert!(read_file(&temp.path().join("absent.json")).is_err());
    }

    #[test]
    fn list_files_are_written_atomically_and_replaced_only_when_confirmed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("list.json");
        write_file(&path, b"first", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(
            error(write_file(&path, b"second", false)),
            ListError::Exists
        );
        assert_eq!(fs::read(&path).unwrap(), b"first");
        write_file(&path, b"third", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"third");
        assert!(write_file(Path::new("relative.json"), b"x", false).is_err());
        assert!(write_file(&temp.path().join("absent").join("list.json"), b"x", false).is_err());
        let names: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["list.json"], "no temporary file is left behind");
        assert_eq!(
            Format::from_path(Path::new("C:\\a\\List.JSON")),
            Some(Format::Json)
        );
        assert_eq!(
            Format::from_path(Path::new("links.txt")),
            Some(Format::Text)
        );
        assert_eq!(Format::from_path(Path::new("list.ef2")), None);
    }

    #[test]
    fn paused_imports_never_start_a_queue_and_queued_imports_follow_add_batch() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        store.set_queue_running(MAIN_QUEUE, false).unwrap();
        let night = store.create_queue("Night").unwrap();
        let running = |id: &str| {
            store
                .queues()
                .unwrap()
                .into_iter()
                .find(|queue| queue.id == id)
                .unwrap()
                .running
        };
        let review = |text: &str| {
            store
                .review_import(parse(text.as_bytes()).unwrap())
                .unwrap()
        };
        let paused = review("https://example.com/a.zip\nhttps://example.com/b.zip\n");
        let imported = store
            .import_list(&paused.candidates, &options(MAIN_QUEUE))
            .unwrap();
        assert!(
            imported
                .added
                .iter()
                .all(|job| job.status == Status::Paused)
        );
        assert!(!running(MAIN_QUEUE));
        let mut queued = options(&night.id);
        queued.paused = false;
        let imported = store
            .import_list(&review("https://example.com/c.zip\n").candidates, &queued)
            .unwrap();
        assert_eq!(imported.added[0].status, Status::Queued);
        assert!(
            !running(&night.id),
            "custom queues stay paused until started"
        );
        // Like Add batch, a queued file wakes an idle, unscheduled Main queue.
        queued.queue = MAIN_QUEUE.into();
        store
            .import_list(&review("https://example.com/d.zip\n").candidates, &queued)
            .unwrap();
        assert!(running(MAIN_QUEUE));
    }
}

//! A manifest name is a lookup key. Only a reviewed job supplies a path to open.
use super::*;
use std::{collections::HashMap, path::PathBuf};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_ROWS: usize = 10_000;

#[derive(Clone, Debug)]
pub struct Target {
    id: String,
    pub path: PathBuf,
    expected: Option<String>,
}

#[derive(Debug)]
pub struct Entry {
    pub name: String,
    lookup: String,
    pub digest: String,
    pub target: Option<Target>,
    pub status: &'static str,
}

pub fn drop_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let versioned = name.rsplit_once('-').is_some_and(|(prefix, version)| {
        (prefix.eq_ignore_ascii_case("SHA256SUMS")
            || prefix.eq_ignore_ascii_case("SHA256SUMS-MSIX"))
            && crate::updates::parse_version(version).is_ok()
    });
    path.is_absolute()
        && (versioned
            || name.eq_ignore_ascii_case("SHA256SUMS")
            || name.eq_ignore_ascii_case("SHA256SUMS.txt")
            || path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sha256")))
}

fn parse(text: &str) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for (index, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        ensure!(entries.len() < MAX_ROWS, "Too many checksum entries");
        entries.push(
            parse_line(line)
                .with_context(|| format!("Invalid checksum file at line {}", index + 1))?,
        );
    }
    ensure!(!entries.is_empty(), "No SHA-256 checksums found");
    Ok(entries)
}

fn parse_line(line: &str) -> Result<Entry> {
    let (line, escaped) = line
        .strip_prefix('\\')
        .map_or((line, false), |line| (line, true));
    let digest = line
        .get(..64)
        .and_then(|s| super::parse_expected(s).ok().flatten())
        .context("Expected a SHA-256 digest")?;
    let name = line[64..]
        .strip_prefix("  ")
        .or_else(|| line[64..].strip_prefix(" *"))
        .context("Expected two spaces or a space and an asterisk before the filename")?;
    let name = name.strip_prefix("./").unwrap_or(name);
    ensure!(
        !name.contains(':')
            && !name.chars().any(char::is_control)
            && name.split('/').all(|part| !matches!(part, "" | "." | "..")),
        "Expected a plain relative filename"
    );
    let lookup = if escaped {
        let mut decoded = String::with_capacity(name.len());
        let mut chars = name.chars();
        while let Some(c) = chars.next() {
            decoded.push(if c == '\\' {
                match chars.next() {
                    Some('\\') => '\\',
                    Some('n') => '\n',
                    Some('r') => '\r',
                    _ => anyhow::bail!("Invalid checksum filename escape"),
                }
            } else {
                c
            });
        }
        key(&decoded)
    } else {
        ensure!(
            !name.contains('\\'),
            "Expected a GNU-style relative filename"
        );
        key(name)
    };
    Ok(Entry {
        name: name.to_owned(),
        lookup,
        digest,
        target: None,
        status: "No matching download",
    })
}

fn key(name: &str) -> String {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    if cfg!(windows) {
        leaf.to_lowercase()
    } else {
        leaf.to_owned()
    }
}

pub fn preview(store: &Store, queue: &str, path: &Path) -> Result<Vec<Entry>> {
    #[cfg(windows)]
    ensure!(
        matches!(path.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_))),
        "Choose a checksum file on a local drive"
    );
    let file = download_file::read_completed(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Checksum file exceeds 1 MiB"
    );
    let mut entries =
        parse(std::str::from_utf8(&bytes).context("The checksum file must be UTF-8 text")?)?;
    let mut names = HashMap::new();
    for entry in &entries {
        *names.entry(entry.lookup.clone()).or_insert(0usize) += 1;
    }
    let mut jobs = HashMap::new();
    for job in store.queue_jobs(queue)? {
        if job.status != Status::Complete {
            continue;
        }
        let Some(ref path) = job.saved_path else {
            continue;
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let key = key(name);
        if !names.contains_key(&key) {
            continue;
        }
        if job.torrent.is_some() {
            let files = crate::torrent::completed_files(store, &job)?;
            if !matches!(files.as_slice(), [(file, _)] if file == path) {
                continue;
            }
        }
        let target = Target {
            id: job.id,
            path: path.clone(),
            expected: job.integrity.expected,
        };
        jobs.entry(key)
            .and_modify(|value| *value = None)
            .or_insert(Some(target));
    }
    for entry in &mut entries {
        entry.status = match jobs.get(&entry.lookup) {
            _ if names[&entry.lookup] > 1 => "Ambiguous filename",
            None => "No matching download",
            Some(None) => "Ambiguous filename",
            Some(Some(target))
                if target
                    .expected
                    .as_ref()
                    .is_some_and(|hash| hash != &entry.digest) =>
            {
                "Expected checksum differs"
            }
            Some(Some(target)) => {
                entry.target = Some(target.clone());
                "Ready"
            }
        };
    }
    Ok(entries)
}

pub fn verify(
    store: &Store,
    queue: &str,
    target: &Target,
    digest: &str,
    cancel: &CancellationToken,
) -> Result<State> {
    let job = store
        .get(&target.id)?
        .context("This download was removed after preview")?;
    ensure!(
        job.saved_path.as_ref() == Some(&target.path)
            && (job.integrity.expected == target.expected
                || job.integrity.expected.as_deref() == Some(digest))
            && store.job_queue(&target.id)?.as_deref() == Some(queue),
        "The download changed after preview"
    );
    super::verify_in_queue(store, &job, digest, cancel, Some(queue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn checksum_drops_recognize_versioned_release_manifests() {
        let folder = std::env::temp_dir();
        for name in [
            "SHA256SUMS",
            "SHA256SUMS.txt",
            "download.sha256",
            "SHA256SUMS-3.20.1",
            "sha256sums-msix-12.10.0",
        ] {
            assert!(drop_path(&folder.join(name)), "{name}");
            assert!(!drop_path(Path::new(name)), "{name}");
        }
        for name in [
            "SHA256SUMS-",
            "SHA256SUMS-3.20",
            "SHA256SUMS-3.20.1.exe",
            "SHA256SUMS-notes",
            "SHA256SUMS-notes-3.20.1",
            "notes-3.20.1",
        ] {
            assert!(!drop_path(&folder.join(name)), "{name}");
        }
    }

    #[test]
    fn manifest_names_never_select_files_outside_the_queue() {
        for name in ["../a", "/a", "C:/a", "a/../b", "", "name\0", "a\\b"] {
            assert!(parse(&format!("{ABC}  {name}")).is_err());
        }
        assert!(parse(&format!("{ABC}one")).is_err());
        let escaped = parse(&format!("\\{ABC}  file\\nname")).unwrap();
        assert_eq!(escaped[0].lookup, "file\nname");
        assert_eq!(escaped[0].name, r"file\nname");
        assert!(parse(&format!("\\{ABC}  file\\q")).is_err());
        assert!(parse(&format!("{ABC}  a\n").repeat(MAX_ROWS + 1)).is_err());
        let rows = parse(&format!(
            "\u{feff}{ABC} *./release/first file\r\n# comment\n{ABC}  second"
        ))
        .unwrap();
        assert_eq!(rows[0].name, "release/first file");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn manifest_preview_and_verification_use_only_unique_reviewed_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("out"),
        })
        .unwrap();
        let queue = crate::queue::MAIN_QUEUE;
        let mut completed = vec![];
        for (i, name) in [
            "match.bin",
            "wrong.bin",
            "duplicate",
            "duplicate",
            "twice",
            "conflict",
        ]
        .iter()
        .enumerate()
        {
            let folder = temp.path().join(i.to_string());
            fs::create_dir(&folder).unwrap();
            let mut job = Job::new(
                &format!("https://example.invalid/{i}"),
                Some(name),
                folder.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .unwrap();
            job.status = Status::Complete;
            job.saved_path = Some(folder.join(name));
            fs::write(
                job.saved_path.as_ref().unwrap(),
                if i == 1 { b"bad" } else { b"abc" },
            )
            .unwrap();
            if i == 5 {
                job.integrity.expected = Some("00".repeat(32));
            }
            completed.push(store.enqueue(&job).unwrap());
        }
        let file = temp.path().join("SHA256SUMS");
        let names = [
            "./match.bin",
            "wrong.bin",
            "duplicate",
            "a/twice",
            "b/twice",
            "absent",
            "conflict",
        ];
        fs::write(&file, names.map(|s| format!("{ABC}  {s}\r\n")).concat()).unwrap();
        let rows = preview(&store, queue, &file).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.status).collect::<Vec<_>>(),
            [
                "Ready",
                "Ready",
                "Ambiguous filename",
                "Ambiguous filename",
                "Ambiguous filename",
                "No matching download",
                "Expected checksum differs"
            ]
        );
        assert!(rows[2..].iter().all(|r| r.target.is_none()));
        let token = CancellationToken::new();
        let run = |i: usize| verify(&store, queue, rows[i].target.as_ref().unwrap(), ABC, &token);
        assert_eq!(run(0).unwrap().matches(), Some(true));
        assert_eq!(run(1).unwrap().matches(), Some(false));
        assert!(run(0).is_ok());
        let other = store.create_queue("Other").unwrap();
        store.move_job(&completed[0].id, &other.id).unwrap();
        assert!(run(0).is_err());
        store.move_job(&completed[0].id, queue).unwrap();
        let mut moved = store.get(&completed[0].id).unwrap().unwrap();
        moved.saved_path = Some(temp.path().join("replaced"));
        store.save(&moved).unwrap();
        assert!(run(0).is_err());
        token.cancel();
        assert!(run(1).is_err());
    }
}

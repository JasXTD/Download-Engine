use super::*;
use crate::{extensions::Grants, paths::AppPaths, store::Store};
use serde_json::json;

fn package(id: &str, version: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format":"download-engine-extension", "format_version":1,
        "id":id, "name":"Discovery sample", "version":version,
        "publisher":"Fixture", "description":"Never executed during discovery",
        "api_version":1,
        "jobs":[{"id":"startup", "name":"Startup", "entry":"main.lua",
            "timeout_seconds":10, "trigger":{"kind":"startup"}}],
        "files":{"main.lua":"error('discovery must never run this')"}
    }))
    .unwrap()
}

fn write(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(format!("{name}{FILE_SUFFIX}"));
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn discovers_only_top_level_packages_without_creating_or_changing_a_profile() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("plugins");
    assert!(scan(&root).unwrap().candidates.is_empty());
    assert!(!root.exists(), "discovery creates no directory");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    write(
        &root.join("nested"),
        "nested",
        &package("sample.nested", "1.0.0"),
    );
    fs::write(root.join("unpacked.lua"), "error('must never run')").unwrap();
    write(&root, "valid", &package("sample.valid", "1.0.0"));
    let store = Store::open(AppPaths {
        data: temp.path().join("profile/data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let report = scan(&root).unwrap();
    assert_eq!(report.candidates.len(), 1);
    assert!(report.issues.is_empty());
    assert!(store.extensions().unwrap().is_empty());
    assert!(store.scripts().unwrap().is_empty());
    assert!(store.script_runs(None).unwrap().is_empty());
    assert_eq!(
        directory().unwrap(),
        crate::paths::resource_directory().unwrap().join("plugins")
    );
}

#[test]
fn discovery_accepts_bundled_examples_but_rejects_impersonations() {
    let temp = tempfile::tempdir().unwrap();
    let bundled = crate::extensions::examples::FILES[0];
    write(temp.path(), "bundled", bundled);
    write(
        temp.path(),
        "impersonation",
        &package("examples.impersonation", "1.0.0"),
    );
    let report = scan(temp.path()).unwrap();
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(report.candidates[0].bytes(), bundled);
    assert_eq!(report.issues.len(), 1);
    assert!(report.issues[0].message.contains("Only the examples"));
}

#[test]
fn rejects_invalid_large_linked_and_directory_packages_but_keeps_valid_ones() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "valid", &package("sample.valid", "1.0.0"));
    write(root, "invalid", b"{\"unknown\":true}");
    write(root, "large", &vec![b' '; MAX_PACKAGE_BYTES + 1]);
    fs::create_dir(root.join(format!("directory{FILE_SUFFIX}"))).unwrap();
    let source = root.join("linked-source");
    fs::write(&source, package("sample.linked", "1.0.0")).unwrap();
    fs::hard_link(&source, root.join(format!("hardlink{FILE_SUFFIX}"))).unwrap();
    let report = scan(root).unwrap();
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(report.issues.len(), 4);
}

#[test]
fn duplicate_ids_are_all_blocked_and_count_limits_never_return_a_partial_scan() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "a", &package("sample.same", "1.0.0"));
    write(root, "b", &package("sample.same", "2.0.0"));
    let report = scan(root).unwrap();
    assert!(report.candidates.is_empty());
    assert_eq!(report.issues.len(), 2);
    for index in 0..MAX_PACKAGES {
        write(root, &index.to_string(), b"{}");
    }
    assert!(
        scan(root)
            .unwrap_err()
            .to_string()
            .contains("Too many extension packages")
    );
}

#[test]
fn compares_versions_and_content_and_preserves_reviewed_bytes_after_file_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = write(temp.path(), "sample", &package("sample.example", "1.2.0"));
    let report = scan(temp.path()).unwrap();
    let candidate = &report.candidates[0];
    assert_eq!(candidate.status(None).unwrap(), Status::New);
    assert_eq!(
        candidate.status(Some(&candidate.package)).unwrap(),
        Status::Current
    );
    let mut installed = candidate.package.clone();
    installed.version = "1.1.0".into();
    assert_eq!(candidate.status(Some(&installed)).unwrap(), Status::Update);
    installed.version = "1.10.0".into();
    assert_eq!(candidate.status(Some(&installed)).unwrap(), Status::Older);
    installed.version = "1.2.0".into();
    installed
        .files
        .insert("main.lua".into(), "print('changed')".into());
    assert_eq!(candidate.status(Some(&installed)).unwrap(), Status::Changed);
    fs::write(&path, package("sample.example", "9.0.0")).unwrap();
    fs::remove_file(path).unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let installed = store
        .install_extension(candidate.bytes(), &Grants::default())
        .unwrap();
    assert_eq!(installed.package.version, "1.2.0");
    assert!(!installed.enabled);
    assert!(scan(temp.path()).unwrap().candidates.is_empty());
    assert_eq!(
        store.extension("sample.example").unwrap().package.version,
        "1.2.0",
        "removing the source never uninstalls it"
    );
}

#[test]
fn directory_and_total_read_limits_are_bounded() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..=MAX_ENTRIES {
        fs::write(temp.path().join(index.to_string()), b"").unwrap();
    }
    assert!(
        scan(temp.path())
            .unwrap_err()
            .to_string()
            .contains("too many entries")
    );
    let temp = tempfile::tempdir().unwrap();
    for index in 0..17 {
        write(
            temp.path(),
            &index.to_string(),
            &vec![b' '; MAX_PACKAGE_BYTES],
        );
    }
    assert!(
        scan(temp.path())
            .unwrap_err()
            .to_string()
            .contains("16 MiB")
    );
}

#[cfg(unix)]
#[test]
fn rejects_symbolic_links_and_fifos_without_reading_their_targets() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("plugins");
    fs::create_dir(&root).unwrap();
    let source = temp.path().join("source");
    fs::write(&source, package("sample.linked", "1.0.0")).unwrap();
    symlink(&source, root.join(format!("link{FILE_SUFFIX}"))).unwrap();
    let fifo = std::ffi::CString::new(
        root.join(format!("fifo{FILE_SUFFIX}"))
            .as_os_str()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let report = scan(&root).unwrap();
    assert!(report.candidates.is_empty());
    assert_eq!(report.issues.len(), 2);
    let alias = temp.path().join("alias");
    symlink(&root, &alias).unwrap();
    assert!(scan(&alias).is_err());
}

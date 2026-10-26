//! Controlled computer/folder changes plus a read-only check of the real OS.
//! No environment variables or Windows folder registrations are changed.
use super::*;
use std::cell::{Cell, RefCell};

#[test]
fn every_start_refreshes_downloads_without_rewriting_existing_jobs_or_custom_folders() {
    use crate::{categories::Category, model::Status, store::Store};
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("LocalAppData");
    let executable = temp.path().join("app").join("de.exe");
    let first = temp.path().join("First drive").join("下载");
    let second = temp.path().join("Another drive").join("Downloads");
    let current = RefCell::new(first.clone());
    let calls = Cell::new(0);
    let discover = || {
        AppPaths::discover_with(
            None,
            || Ok(executable.clone()),
            || {
                calls.set(calls.get() + 1);
                Ok((local.clone(), current.borrow().clone()))
            },
        )
    };
    let paths = discover().unwrap();
    assert_eq!(paths.downloads, first.join("Download Engine"));
    assert_eq!(calls.get(), 1);
    assert!(!first.exists(), "Discovery must not create the destination");
    let store = Store::open(paths.clone()).unwrap();
    let mut job = store
        .new_download(
            "https://example.com/file.bin",
            None,
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    job.status = Status::Paused;
    job.downloaded = 512;
    job.total = Some(1024);
    store.enqueue(&job).unwrap();
    let part = paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&part).unwrap();
    let part = part.join("saved.part");
    std::fs::write(&part, vec![37; 512]).unwrap();
    let custom = temp.path().join("Custom music");
    let mut rules = store.download_folders().unwrap();
    rules.overrides.insert(Category::Music, custom.clone());
    store.save_download_folders(&rules).unwrap();
    drop(store);

    *current.borrow_mut() = second.clone();
    let refreshed = discover().unwrap();
    assert_eq!(
        calls.get(),
        2,
        "Each launch must ask for the current folders"
    );
    assert_eq!(refreshed.data, paths.data);
    assert_eq!(refreshed.downloads, second.join("Download Engine"));
    assert!(!second.exists());
    let store = Store::open(refreshed.clone()).unwrap();
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.directory, job.directory);
    assert_eq!(saved.status, Status::Paused);
    assert_eq!(saved.downloaded, 512);
    assert_eq!(std::fs::read(&part).unwrap(), vec![37; 512]);
    let next = store
        .new_download(
            "https://example.com/next.bin",
            None,
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(next.directory, refreshed.downloads.join("Other"));
    let song = store
        .new_download(
            "https://example.com/song.mp3",
            None,
            None,
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(song.directory, custom);
}

#[test]
fn legacy_profiles_use_the_current_root_and_an_unavailable_lookup_is_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("LocalAppData");
    let legacy = local.join("IDM_EX");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("queue.sqlite3"), b"profile presence fixture").unwrap();
    let executable = temp.path().join("de.exe");
    for name in ["Old drive", "New drive"] {
        let downloads = temp.path().join(name).join("Downloads");
        let paths = AppPaths::discover_with(
            None,
            || Ok(executable.clone()),
            || Ok((local.clone(), downloads.clone())),
        )
        .unwrap();
        assert_eq!(paths.data, legacy);
        assert_eq!(paths.downloads, downloads.join("IDM EX"));
        assert!(!downloads.exists());
    }
    let error = AppPaths::discover_with(
        None,
        || Ok(executable.clone()),
        || anyhow::bail!("Windows Downloads lookup failed"),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Windows Downloads lookup failed")
    );
    assert!(!local.join("Download Engine").exists());
}

#[test]
fn portable_roots_follow_the_current_executable_and_explicit_profiles_take_precedence() {
    let temp = tempfile::tempdir().unwrap();
    // The same portable configuration is unpacked in two different locations.
    for name in ["Portable on first PC", "Portable on another PC"] {
        let base = temp.path().join(name);
        std::fs::create_dir(&base).unwrap();
        let executable = base.join("de.exe");
        std::fs::write(
            base.join("portable.json"),
            br#"{"data":"data","downloads":"downloads"}"#,
        )
        .unwrap();
        let paths = AppPaths::discover_with(
            None,
            || Ok(executable.clone()),
            || panic!("Portable mode must not query installed folders"),
        )
        .unwrap();
        assert_eq!(paths.data, base.join("data"));
        assert_eq!(paths.downloads, base.join("downloads"));
        let explicit = temp.path().join("Chosen downloads");
        std::fs::write(
            base.join("portable.json"),
            serde_json::to_vec(&serde_json::json!({"data":"data","downloads":explicit})).unwrap(),
        )
        .unwrap();
        let paths = AppPaths::discover_with(
            None,
            || Ok(executable.clone()),
            || panic!("Custom portable path must stay explicit"),
        )
        .unwrap();
        assert_eq!(paths.downloads, explicit);
        std::fs::write(base.join("portable.json"), b"invalid fixture").unwrap();
        assert!(
            AppPaths::discover_with(
                None,
                || Ok(executable),
                || panic!("Invalid portable config must not fall back")
            )
            .is_err()
        );
    }
    let explicit = temp.path().join("Explicit profile");
    let paths = AppPaths::discover_with(
        Some(explicit.as_os_str()),
        || panic!("Profile override must not inspect the executable"),
        || panic!("Profile override must not query installed folders"),
    )
    .unwrap();
    assert_eq!(paths.data, explicit.join("data"));
    assert_eq!(paths.downloads, explicit.join("downloads"));
    assert!(!explicit.exists());
    let error = AppPaths::discover_with(
        Some(std::ffi::OsStr::new("relative")),
        || panic!(),
        || panic!(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("must be absolute"));
}

#[test]
fn native_lookup_matches_windows_shell_without_changing_its_registration() {
    use std::process::Command;
    let powershell = file_browser()
        .unwrap()
        .parent()
        .unwrap()
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = Command::new(powershell).args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
        "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $shell=New-Object -ComObject Shell.Application; try { $folder=$shell.NameSpace('shell:Downloads'); try { [Console]::Write($folder.Self.Path) } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($folder) } } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell) }"])
        .output().unwrap();
    assert!(
        output.status.success(),
        "Windows Shell query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert!(expected.is_absolute());
    let detected = user_downloads().unwrap();
    assert_eq!(detected, expected);
    assert_eq!(user_downloads().unwrap(), expected);
    println!(
        "Read-only Windows Downloads lookup matched Explorer: {}",
        detected.display()
    );
}

//! The extensions folder only lists and checks what is saved there. Nothing is
//! installed, and links are never followed.
use super::{
    FILE_SUFFIX, Package,
    folder::{Kind, Listing, MAX_LISTED, read, scan},
};
use serde_json::{Value, json};
use std::fs;

fn package(id: &str, version: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "download-engine-extension",
        "format_version": 1,
        "id": id,
        "name": "Tidy downloads",
        "version": version,
        "publisher": "Example",
        "description": "Moves finished downloads into folders.",
        "api_version": 1,
        "permissions": { "network": [] },
        "settings": [],
        "jobs": [
            { "id": "tidy", "name": "Tidy", "entry": "tidy.lua", "timeout_seconds": 30,
              "trigger": { "kind": "download_completed", "queue": null } }
        ],
        "files": { "tidy.lua": "print('tidy')\n" }
    }))
    .unwrap()
}

fn names(listing: &Listing) -> Vec<&str> {
    listing
        .found
        .iter()
        .map(|found| found.name.as_str())
        .collect()
}

fn package_in(listing: &Listing, name: &str) -> Package {
    match &listing
        .found
        .iter()
        .find(|found| found.name == name)
        .unwrap()
        .kind
    {
        Kind::Package(package) => (**package).clone(),
        other => panic!("{name}: {other:?}"),
    }
}

fn unreadable(listing: &Listing, name: &str) -> String {
    match &listing
        .found
        .iter()
        .find(|found| found.name == name)
        .unwrap()
        .kind
    {
        Kind::Unreadable(why) => why.clone(),
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn a_missing_folder_lists_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let listing = scan(&temp.path().join("Extensions")).unwrap();
    assert_eq!(listing, Listing::default());
    assert!(
        !temp.path().join("Extensions").exists(),
        "listing creates nothing"
    );
}

#[test]
fn extension_files_are_listed_in_name_order_and_checked_like_an_install() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path();
    fs::write(
        folder.join("b-tidy.de-extension.json"),
        package("test.b", "1.0.0"),
    )
    .unwrap();
    fs::write(
        folder.join("A-TIDY.DE-EXTENSION.JSON"),
        package("test.a", "2.1.0"),
    )
    .unwrap();
    fs::write(folder.join("broken.de-extension.json"), b"{").unwrap();
    fs::write(folder.join("notes.txt"), b"not an extension").unwrap();
    fs::write(
        folder.join(".hidden.de-extension.json"),
        package("test.h", "1.0.0"),
    )
    .unwrap();
    fs::create_dir(folder.join("my-reader")).unwrap();
    fs::write(folder.join("my-reader/manifest.json"), b"{}").unwrap();
    fs::create_dir(folder.join("pictures")).unwrap();

    let listing = scan(folder).unwrap();
    assert_eq!(
        names(&listing),
        [
            "A-TIDY.DE-EXTENSION.JSON",
            "b-tidy.de-extension.json",
            "broken.de-extension.json",
            "my-reader"
        ]
    );
    assert_eq!(
        package_in(&listing, "A-TIDY.DE-EXTENSION.JSON").id,
        "test.a"
    );
    assert_eq!(
        package_in(&listing, "b-tidy.de-extension.json").version,
        "1.0.0"
    );
    assert!(
        unreadable(&listing, "broken.de-extension.json")
            .contains("This is not a valid extension file")
    );
    assert_eq!(listing.found[3].kind, Kind::Unpacked);
    // notes.txt and the folder without a manifest; hidden files aren't counted.
    assert_eq!(listing.skipped, 2);
    assert_eq!(listing.left_out, 0);
}

#[test]
fn a_file_over_1_mib_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(format!("big{FILE_SUFFIX}")),
        vec![b' '; 2 * 1024 * 1024],
    )
    .unwrap();
    let listing = scan(temp.path()).unwrap();
    assert!(unreadable(&listing, "big.de-extension.json").contains("at most 1 MiB"));
}

/// A decode error can quote the file, so a reason is cut to one short line.
#[test]
fn a_reason_never_repeats_a_whole_file() {
    let temp = tempfile::tempdir().unwrap();
    let text = format!(
        "{{\"format\":\"download-engine-extension\",\"format_version\":\"{}\"}}",
        "A".repeat(500_000)
    );
    fs::write(temp.path().join("long.de-extension.json"), text).unwrap();
    let why = unreadable(&scan(temp.path()).unwrap(), "long.de-extension.json");
    assert!(
        why.chars().count() <= 201,
        "{} characters",
        why.chars().count()
    );
    assert!(
        why.starts_with("This is not a valid extension file"),
        "{why}"
    );
}

#[test]
fn only_the_examples_that_come_with_the_app_can_use_an_examples_id() {
    let temp = tempfile::tempdir().unwrap();
    let shipped = super::examples::FILES[0];
    fs::write(temp.path().join("copy.de-extension.json"), shipped).unwrap();
    let mut changed: Value = serde_json::from_slice(shipped).unwrap();
    changed["version"] = json!("9.0.0");
    fs::write(
        temp.path().join("changed.de-extension.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let listing = scan(temp.path()).unwrap();
    assert_eq!(
        package_in(&listing, "copy.de-extension.json").id,
        "examples.internet-archive"
    );
    assert!(unreadable(&listing, "changed.de-extension.json").contains("\"examples.\""));
}

#[test]
fn at_most_100_files_are_listed_and_the_rest_are_counted() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..MAX_LISTED + 2 {
        fs::write(temp.path().join(format!("f{index:03}{FILE_SUFFIX}")), b"{").unwrap();
    }
    let listing = scan(temp.path()).unwrap();
    assert_eq!(listing.found.len(), MAX_LISTED);
    assert_eq!(listing.left_out, 2);
    assert_eq!(listing.found[0].name, "f000.de-extension.json");
    assert_eq!(listing.found[MAX_LISTED - 1].name, "f099.de-extension.json");
}

#[test]
fn a_review_reads_the_file_as_it_is_now_and_only_inside_the_folder() {
    let temp = tempfile::tempdir().unwrap();
    let name = "tidy.de-extension.json";
    fs::write(temp.path().join(name), package("test.tidy", "1.0.0")).unwrap();
    assert_eq!(
        package_in(&scan(temp.path()).unwrap(), name).version,
        "1.0.0"
    );
    fs::write(temp.path().join(name), package("test.tidy", "1.1.0")).unwrap();
    let (current, bytes) = read(temp.path(), name).unwrap();
    assert_eq!(current.version, "1.1.0");
    assert_eq!(bytes, package("test.tidy", "1.1.0"));

    fs::create_dir(temp.path().join("inner")).unwrap();
    fs::write(
        temp.path().join("inner").join(name),
        package("test.tidy", "2.0.0"),
    )
    .unwrap();
    for outside in [
        format!("inner/{name}"),
        format!("..{}{name}", std::path::MAIN_SEPARATOR),
        "inner".to_owned(),
    ] {
        assert!(read(temp.path(), &outside).is_err(), "{outside}");
    }
}

/// A link is skipped, so the folder can't be made to list or read a file kept
/// elsewhere. Windows only lets some accounts make links.
#[test]
fn links_are_skipped_and_never_read() {
    let temp = tempfile::tempdir().unwrap();
    let elsewhere = temp.path().join("elsewhere.de-extension.json");
    fs::write(&elsewhere, package("test.elsewhere", "1.0.0")).unwrap();
    let folder = temp.path().join("Extensions");
    fs::create_dir(&folder).unwrap();
    let linked = folder.join("linked.de-extension.json");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&elsewhere, &linked);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&elsewhere, &linked);
    if let Err(error) = made {
        eprintln!("skipped: can't make a link here: {error}");
        return;
    }
    let listing = scan(&folder).unwrap();
    assert!(listing.found.is_empty(), "{listing:?}");
    assert_eq!(listing.skipped, 1);
    assert!(read(&folder, "linked.de-extension.json").is_err());
}

/// Any account can make a junction, so this runs where links can't be made.
#[cfg(windows)]
#[test]
fn a_junction_to_an_unpacked_extension_elsewhere_is_skipped() {
    let temp = tempfile::tempdir().unwrap();
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::write(elsewhere.join("manifest.json"), b"{}").unwrap();
    let folder = temp.path().join("Extensions");
    fs::create_dir(&folder).unwrap();
    let junction = folder.join("linked");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&elsewhere)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    assert!(
        junction.join("manifest.json").is_file(),
        "the junction leads there"
    );
    let listing = scan(&folder).unwrap();
    assert!(listing.found.is_empty(), "{listing:?}");
    assert_eq!(listing.skipped, 1);
}

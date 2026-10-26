//! Explicit opt-in live check. Never run downloaded content or read a browser profile.
use download_engine::{offline_website, website_grabber};
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires explicitly authorized GitHub network capture"]
async fn github_home_live_capture() {
    let folder = std::path::PathBuf::from(
        std::env::var("IDM_EX_GRABBER_LIVE_OUTPUT").expect("Set a fresh absolute output folder"),
    );
    assert!(folder.is_absolute());
    std::fs::create_dir_all(&folder).unwrap();
    let mut scans = Vec::new();
    for (url, types) in [
        ("https://github.com/uBlockOrigin/uBOL-home", "md,png,zip"),
        (
            "https://github.com/uBlockOrigin/uBOL-home/releases",
            "zip,xpi,tar.gz",
        ),
    ] {
        let report = website_grabber::scan(
            website_grabber::Config::new(url, types, 0, 50).unwrap(),
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        println!(
            "Scan {url}: {} files, {} pages, {} failures",
            report.files.len(),
            report.pages_read,
            report.pages_failed
        );
        scans.push(serde_json::json!({"url":url,"pages":report.pages_read,"failures":report.pages_failed,"files":report.files,"issues":report.issues.iter().map(|i|serde_json::json!({"page":i.page,"message":i.message})).collect::<Vec<_>>() }));
        assert!(report.pages_read >= 1);
        assert_eq!(report.pages_failed, 0);
        if url.ends_with("/releases") {
            for suffix in ["chromium.zip", "edge.zip", "firefox.signed.xpi"] {
                assert!(
                    report.files.iter().any(|file| file.name.ends_with(suffix)),
                    "Missing {suffix}"
                );
            }
        } else {
            let readme = report
                .files
                .iter()
                .find(|file| file.name == "README.md")
                .expect("README was not found");
            assert_eq!(
                readme.url,
                "https://github.com/uBlockOrigin/uBOL-home/raw/main/README.md"
            );
        }
    }
    std::fs::write(
        folder.join("discoveries.json"),
        serde_json::to_vec_pretty(&scans).unwrap(),
    )
    .unwrap();
    let mut config = offline_website::Config::new(
        "https://github.com/uBlockOrigin/uBOL-home",
        folder.clone(),
        0,
        1,
    )
    .unwrap();
    config.megabytes = 20;
    let report = offline_website::save(config, CancellationToken::new(), |p| {
        if !p.current.is_empty() {
            println!(
                "Capture: {} pages, {} assets; {}",
                p.pages,
                p.assets,
                p.current.split('?').next().unwrap_or("")
            );
        }
    })
    .await
    .unwrap();
    println!(
        "Capture: {} pages, {} assets, {} bytes, {} notes",
        report.pages, report.assets, report.bytes, report.issue_count
    );
    std::fs::write(
        folder.join("capture.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert_eq!(report.pages, 1);
    assert!(report.assets > 0);
}

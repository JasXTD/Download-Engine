//! The real Lua worker in site reader mode: what it returns, and what it refuses.
use download_engine::{paths::AppPaths, script_automation::Reading};
use serde_json::json;
use std::process::Command;

struct Run {
    status: std::process::ExitStatus,
    stderr: String,
    result: Option<Reading>,
}

fn read(source: &str) -> Run {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    let store = download_engine::store::Store::open(paths.clone()).unwrap();
    store
        .save_proxy_settings(
            &store.proxy_settings().unwrap(),
            download_engine::proxy::Settings {
                mode: download_engine::proxy::Mode::Direct,
                ..Default::default()
            },
            download_engine::proxy::LoginChange::Remove,
        )
        .unwrap();
    let folder = temp.path().join("run");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("script.lua"), source).unwrap();
    std::fs::write(
        folder.join("event.json"),
        json!({"trigger": "site_reader", "url": "https://www.example.com/watch/a1", "downloads": []}).to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_de-cli"))
        .arg("--lua-worker")
        .env("IDM_EX_SCRIPT_RUN", uuid::Uuid::new_v4().to_string())
        .env(
            "IDM_EX_SCRIPT_PROFILE",
            serde_json::to_string(&paths).unwrap(),
        )
        .env("XDOWNLOADER_SCRIPT_FILE", folder.join("script.lua"))
        .env("XDOWNLOADER_EVENT_FILE", folder.join("event.json"))
        .env("XDOWNLOADER_RESULT_FILE", folder.join("result.json"))
        .output()
        .unwrap();
    let result = std::fs::read(folder.join("result.json"))
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap());
    Run {
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        result,
    }
}

#[test]
fn a_reader_returns_its_findings_for_the_opened_page() {
    let run = read(
        r#"
        local id = event.url:match("/watch/(%w+)")
        print("Reading", id)
        return {
            title = "Video " .. id,
            items = {
                { kind = "hls", url = "https://cdn.example.com/" .. id .. "/master.m3u8", label = "All qualities" },
                { kind = "file", url = "https://cdn.example.com/" .. id .. ".mp4", size = 1024 },
            },
        }
        "#,
    );
    assert!(run.status.success(), "{}", run.stderr);
    let found = run.result.unwrap();
    assert_eq!(found.title.as_deref(), Some("Video a1"));
    assert_eq!(found.items.len(), 2);
    assert_eq!(found.items[1].size, Some(1024));
}

#[test]
fn a_reader_cannot_reach_local_addresses_or_touch_downloads() {
    for (source, expected) in [
        ("return xd.fetch('http://127.0.0.1:9/')", "public internet"),
        ("return xd.fetch('http://localhost/')", "public internet"),
        (
            "return xd.add('https://example.com/file.zip')",
            "cannot see or change your downloads",
        ),
        (
            "return xd.downloads()",
            "cannot see or change your downloads",
        ),
        (
            "return { items = { { kind = 'file', url = 'file:///C:/Windows/win.ini' } } }",
            "HTTP",
        ),
        ("return 42", "unexpected value"),
        ("error('Not a video page')", "Not a video page"),
    ] {
        let run = read(source);
        assert!(!run.status.success(), "{source}");
        assert!(run.stderr.contains(expected), "{source}: {}", run.stderr);
        assert!(run.result.is_none(), "{source}");
    }
}

//! The real CLI and Lua worker against isolated profiles. Queues stay paused.
use download_engine::{
    extensions::{Grants, Reach},
    paths::AppPaths,
    script_automation::{Run, Script},
    store::Store,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

const ID: &str = "example.videohost";

fn manifest() -> Value {
    json!({
        "format": "download-engine-extension",
        "format_version": 1,
        "id": ID,
        "name": "VideoHost",
        "version": "1.2.0",
        "publisher": "Example",
        "description": "Adds VideoHost pages to Find videos.",
        "api_version": 1,
        "permissions": { "network": ["videohost.example"] },
        "settings": [
            { "id": "quality", "type": "choice", "label": "Preferred quality",
              "options": ["1080p", "720p"], "default": "1080p" },
            { "id": "api_token", "type": "secret", "label": "API token" }
        ],
        "jobs": [
            { "id": "reader", "name": "VideoHost pages", "entry": "reader.lua", "timeout_seconds": 30,
              "trigger": { "kind": "site_reader", "pages": ["videohost.example/watch/*"] } },
            { "id": "report", "name": "Report", "entry": "report.lua", "timeout_seconds": 30,
              "trigger": { "kind": "manual" } }
        ]
    })
}
fn files() -> [(&'static str, &'static str); 3] {
    [
        (
            "lib/parse.lua",
            "local M = {}\nfunction M.label(q) return 'Quality ' .. q end\nreturn M\n",
        ),
        (
            "reader.lua",
            "local parse = require('lib.parse')\nprint(parse.label(xd.setting('quality')))\nprint(xd.setting('api_token'))\nlocal reply = xd.fetch(event.url)\nreturn { items = {} }\n",
        ),
        (
            "report.lua",
            "local parse = require('lib.parse')\nprint(parse.label(xd.setting('quality')))\nprint(xd.supports('xd.fetch'))\nprint(pcall(xd.setting, 'api_token'))\nprint(pcall(xd.command, 'automation', 'list'))\n",
        ),
    ]
}
fn package() -> Vec<u8> {
    let mut package = manifest();
    package["files"] = json!(
        files()
            .iter()
            .map(|(path, source)| (path.to_string(), json!(source)))
            .collect::<serde_json::Map<_, _>>()
    );
    serde_json::to_vec(&package).unwrap()
}
fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

struct Fixture {
    temp: tempfile::TempDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        store.set_queue_running("main", false).unwrap();
        store
            .install_extension(&package(), &Grants::default())
            .unwrap();
        store.set_extension_enabled(ID, true).unwrap();
        Self { temp, store }
    }
    fn worker(&self, run: &Run, script: &Script, reach: Reach, source: &str) -> Output {
        let context = self
            .store
            .extension_context(script, reach)
            .unwrap()
            .unwrap();
        let file = self.temp.path().join(format!("{}.lua", run.id));
        let event = self.temp.path().join(format!("{}.json", run.id));
        std::fs::write(&file, source).unwrap();
        std::fs::write(&event, serde_json::to_vec(&run.context).unwrap()).unwrap();
        let mut child = quiet(&mut Command::new(env!("CARGO_BIN_EXE_de-cli")))
            .arg("--lua-worker")
            .env("IDM_EX_SCRIPT_RUN", &run.id)
            .env(
                "IDM_EX_SCRIPT_PROFILE",
                serde_json::to_string(&self.store.paths).unwrap(),
            )
            .env("XDOWNLOADER_SCRIPT_FILE", file)
            .env("XDOWNLOADER_EVENT_FILE", event)
            .env("XDOWNLOADER_EXTENSION", "1")
            .env(
                "XDOWNLOADER_RESULT_FILE",
                self.temp.path().join("result.json"),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&context).unwrap())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

#[test]
fn an_automation_worker_gets_modules_and_settings_but_no_secrets_or_rule_commands() {
    let f = Fixture::new();
    f.store
        .set_extension_setting(ID, "quality", &json!("720p"))
        .unwrap();
    f.store
        .set_extension_secret(ID, "api_token", "private-token")
        .unwrap();
    let script = f.store.extension_job(ID, "report").unwrap();
    let run = f
        .store
        .run_script(&script.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let output = f.worker(&run, &script, Reach::Automation, &script.body);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Quality 720p"), "{stdout}");
    assert!(stdout.contains("false"), "no web access: {stdout}");
    assert!(
        stdout.contains("not a setting this job can read"),
        "{stdout}"
    );
    assert!(stdout.contains("not available to extensions"), "{stdout}");
    assert!(!stdout.contains("private-token"), "{stdout}");
}

#[test]
fn a_reader_worker_can_read_its_secret_and_cannot_reach_undeclared_hosts() {
    let f = Fixture::new();
    f.store
        .set_extension_secret(ID, "api_token", "reader-token")
        .unwrap();
    let script = f.store.extension_job(ID, "reader").unwrap();
    let run = f
        .store
        .run_site_reader(
            &script.id,
            "https://undeclared.example/watch/1",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    let output = f.worker(&run, &script, Reach::Web, &script.body);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stdout}");
    assert!(stdout.contains("Quality 1080p"), "{stdout}");
    assert!(stdout.contains("reader-token"), "{stdout}");
    assert!(
        stderr.contains("did not declare undeclared.example"),
        "{stderr}"
    );
}

fn cli(home: &Path, args: &[&str]) -> (Value, Output) {
    let output = quiet(&mut Command::new(env!("CARGO_BIN_EXE_de-cli")))
        .args(args)
        .arg("--no-start")
        .env("DOWNLOAD_ENGINE_HOME", home)
        .env_remove("IDM_EX_HOME")
        .env_remove("IDM_EX_SCRIPT_RUN")
        .env_remove("IDM_EX_SCRIPT_PROFILE")
        .output()
        .unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (value, output)
}

#[test]
fn the_cli_packs_installs_lists_and_removes_an_extension() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let folder = temp.path().join("videohost");
    std::fs::create_dir_all(folder.join("lib")).unwrap();
    std::fs::write(
        folder.join("manifest.json"),
        serde_json::to_vec(&manifest()).unwrap(),
    )
    .unwrap();
    for (path, source) in files() {
        std::fs::write(folder.join(path), source).unwrap();
    }
    let out = temp.path().join("videohost.de-extension.json");
    let (out_text, folder_text) = (out.to_str().unwrap(), folder.to_str().unwrap());

    let (reply, output) = cli(
        &home,
        &["extensions", "pack", folder_text, "--out", out_text],
    );
    assert_eq!(
        reply["ok"],
        true,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A second pack never replaces the first file.
    let (_, output) = cli(
        &home,
        &["extensions", "pack", folder_text, "--out", out_text],
    );
    assert!(!output.status.success());

    let (reply, _) = cli(&home, &["extensions", "review", out_text]);
    assert_eq!(reply["result"]["package"]["id"], ID);
    assert_eq!(reply["result"]["update"], Value::Null);

    let (reply, output) = cli(&home, &["extensions", "install", out_text]);
    assert_eq!(
        reply["ok"],
        true,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(reply["result"]["enabled"], false);

    let (reply, _) = cli(&home, &["extensions", "list"]);
    assert_eq!(reply["result"][0]["id"], ID);
    assert_eq!(reply["result"][0]["jobs"], 2);
    let (reply, _) = cli(&home, &["scripts", "list"]);
    assert_eq!(
        reply["result"],
        json!([]),
        "jobs stay out of the script list"
    );

    let (reply, _) = cli(&home, &["extensions", "enable", ID]);
    assert_eq!(reply["ok"], true);
    let (reply, _) = cli(&home, &["extensions", "list"]);
    assert_eq!(reply["result"][0]["enabled"], true);

    // A mistyped option stops the command before it does anything.
    let (_, output) = cli(&home, &["extensions", "remove", ID, "--name", "x"]);
    assert!(!output.status.success());
    let (reply, _) = cli(&home, &["extensions", "list"]);
    assert_eq!(reply["result"][0]["id"], ID);

    let (reply, _) = cli(&home, &["extensions", "remove", ID]);
    assert_eq!(reply["ok"], true);
    let (reply, _) = cli(&home, &["extensions", "list"]);
    assert_eq!(reply["result"], json!([]));
}

#[test]
fn scripts_cannot_manage_extensions_through_the_cli() {
    let f = Fixture::new();
    let output = quiet(&mut Command::new(env!("CARGO_BIN_EXE_de-cli")))
        .args(["extensions", "remove", ID])
        .env("IDM_EX_SCRIPT_RUN", uuid::Uuid::new_v4().to_string())
        .env(
            "IDM_EX_SCRIPT_PROFILE",
            serde_json::to_string(&f.store.paths).unwrap(),
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot manage extensions"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(f.store.extensions().unwrap().len(), 1);
}

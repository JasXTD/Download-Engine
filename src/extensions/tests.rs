use super::*;
use crate::{
    control::{self, Action},
    paths::AppPaths,
    script_automation::{self, Language, Script, Trigger, execute_job, execute_lua},
    store::Store,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const KIB: usize = 1024;
const KEY: &str = "00112233445566778899aabbccddeeff";

fn example() -> Value {
    json!({
        "format": "download-engine-extension",
        "format_version": 1,
        "id": "example.videohost",
        "name": "VideoHost",
        "version": "1.2.0",
        "publisher": "Example",
        "description": "Adds VideoHost pages to Find videos.",
        "homepage": "https://example.com/videohost",
        "license": "MIT",
        "api_version": 1,
        "provider_api": 1,
        "permissions": { "network": ["videohost.example", "*.cdn.videohost.example"] },
        "settings": [
            { "id": "quality", "type": "choice", "label": "Preferred quality",
              "options": ["1080p", "720p", "480p"], "default": "1080p" },
            { "id": "api_token", "type": "secret", "label": "API token" },
            { "id": "content_key", "type": "aes_key", "label": "Content key" }
        ],
        "jobs": [
            { "id": "reader", "name": "VideoHost pages", "entry": "reader.lua", "timeout_seconds": 30,
              "trigger": { "kind": "site_reader", "pages": ["videohost.example/watch/*"] } },
            { "id": "decrypt", "name": "Decrypt episodes", "entry": "provider.lua", "timeout_seconds": 60,
              "trigger": { "kind": "provider" } },
            { "id": "tidy", "name": "Tidy finished files", "entry": "tidy.lua", "timeout_seconds": 60,
              "change_files": true, "trigger": { "kind": "download_completed", "queue": null } }
        ],
        "files": {
            "reader.lua": "local parse = require('lib.parse')\nreturn parse.read(xd.fetch(event.url))\n",
            "provider.lua": "return {}\n",
            "tidy.lua": "print('tidy')\n",
            "lib/parse.lua": "local M = {}\nfunction M.read(page) return { items = {} } end\nreturn M\n"
        }
    })
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn decode(value: &Value) -> anyhow::Result<Package> {
    Package::decode(&bytes(value))
}
fn fixture() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running("main", false).unwrap();
    (temp, store)
}
fn big(size: usize) -> Value {
    json!("x".repeat(size))
}
fn job(trigger: Value) -> Value {
    json!({ "id": "extra", "name": "Extra", "entry": "tidy.lua", "timeout_seconds": 5, "trigger": trigger })
}
fn version_of(store: &Store) -> (i64, i64) {
    store
        .connect()
        .unwrap()
        .query_row(
            "SELECT data_version,min_writer_version FROM profile_format",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
}

#[test]
fn the_published_example_and_its_variations_pass() {
    decode(&example()).unwrap();
    let mut automation_only = example();
    automation_only
        .as_object_mut()
        .unwrap()
        .retain(|key, _| key != "permissions" && key != "provider_api");
    let tidy = automation_only["jobs"][2].clone();
    automation_only["jobs"] = json!([tidy]);
    decode(&automation_only).unwrap();

    let mut queue = example();
    queue["jobs"]
        .as_array_mut()
        .unwrap()
        .push(job(json!({"kind":"queue_completed","queue":""})));
    decode(&queue).unwrap();

    let mut wildcard = example();
    wildcard["permissions"]["network"] = json!(["*.videohost.example"]);
    wildcard["jobs"][0]["trigger"]["pages"] = json!(["www.videohost.example/watch/*"]);
    decode(&wildcard).unwrap();
    for page in ["*.videohost.example/watch/*", "*.cdn.videohost.example/*"] {
        wildcard["jobs"][0]["trigger"]["pages"] = json!([page]);
        decode(&wildcard).unwrap();
    }

    let mut limit = example();
    limit["files"]["big.lua"] = big(64 * KIB);
    decode(&limit).unwrap();

    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(bytes(&example()));
    Package::decode(&bom).unwrap();
}

#[test]
fn every_broken_package_is_refused_for_the_right_reason() {
    type Change = fn(&mut Value);
    let cases: Vec<(&str, Change, &str)> = vec![
        (
            "unknown top-level field",
            |p| p["language"] = json!("powershell"),
            "not a valid extension file",
        ),
        (
            "unknown job field",
            |p| p["jobs"][2]["language"] = json!("powershell"),
            "not a valid extension file",
        ),
        (
            "unknown trigger field",
            |p| p["jobs"][1]["trigger"]["extra"] = json!(true),
            "not a valid extension file",
        ),
        (
            "unknown setting field",
            |p| p["settings"][1]["default"] = json!("leaked"),
            "not a valid extension file",
        ),
        (
            "unknown permission",
            |p| p["permissions"]["cookies"] = json!(true),
            "not a valid extension file",
        ),
        (
            "wrong format",
            |p| p["format"] = json!("download-engine-script"),
            "not an extension file",
        ),
        (
            "newer format",
            |p| p["format_version"] = json!(2),
            "uses format 2",
        ),
        (
            "newer Lua API",
            |p| p["api_version"] = json!(2),
            "needs Lua API 2",
        ),
        (
            "upper-case id",
            |p| p["id"] = json!("Example.VideoHost"),
            "Invalid extension id",
        ),
        (
            "id with no dot",
            |p| p["id"] = json!("videohost"),
            "Invalid extension id",
        ),
        (
            "id over 64",
            |p| p["id"] = json!(format!("a.{}", "b".repeat(70))),
            "Invalid extension id",
        ),
        (
            "short version",
            |p| p["version"] = json!("1.2"),
            "must look like",
        ),
        (
            "leading-zero version",
            |p| p["version"] = json!("1.02.0"),
            "must look like",
        ),
        (
            "name with control character",
            |p| p["name"] = json!("Video\nHost"),
            "Invalid extension name",
        ),
        (
            "http homepage",
            |p| p["homepage"] = json!("http://example.com"),
            "https://",
        ),
        (
            "parent path",
            |p| p["files"]["../evil.lua"] = json!("x"),
            "Invalid file path",
        ),
        (
            "nested parent path",
            |p| p["files"]["lib/../../evil.lua"] = json!("x"),
            "Invalid file path",
        ),
        (
            "backslash path",
            |p| p["files"]["lib\\evil.lua"] = json!("x"),
            "Invalid file path",
        ),
        (
            "absolute path",
            |p| p["files"]["/evil.lua"] = json!("x"),
            "Invalid file path",
        ),
        (
            "drive path",
            |p| p["files"]["c:/evil.lua"] = json!("x"),
            "Invalid file path",
        ),
        (
            "upper-case path",
            |p| p["files"]["Reader.lua"] = json!("x"),
            "Invalid file path",
        ),
        (
            "non-Lua file",
            |p| p["files"]["evil.dll"] = json!("x"),
            "Invalid file path",
        ),
        (
            "binary chunk",
            |p| p["files"]["evil.lua"] = json!("\u{1b}LuaT"),
            "must be Lua text",
        ),
        (
            "NUL in source",
            |p| p["files"]["evil.lua"] = json!("a\u{0}b"),
            "must be Lua text",
        ),
        (
            "file over 64 KiB",
            |p| p["files"]["big.lua"] = big(64 * KIB + 1),
            "over 64 KiB",
        ),
        (
            "Lua over 512 KiB",
            |p| {
                for index in 0..9 {
                    p["files"][format!("f{index}.lua")] = big(60 * KIB);
                }
            },
            "at most 512 KiB",
        ),
        (
            "too many files",
            |p| {
                for index in 0..64 {
                    p["files"][format!("f{index}.lua")] = json!("x");
                }
            },
            "1 to 64 Lua files",
        ),
        (
            "missing entry",
            |p| p["jobs"][0]["entry"] = json!("missing.lua"),
            "not in this package",
        ),
        (
            "duplicate job",
            |p| p["jobs"][1]["id"] = json!("reader"),
            "listed twice",
        ),
        (
            "too many jobs",
            |p| {
                for index in 0..16 {
                    let mut extra = job(json!({"kind":"manual"}));
                    extra["id"] = json!(format!("j{index}"));
                    p["jobs"].as_array_mut().unwrap().push(extra);
                }
            },
            "1 to 16 jobs",
        ),
        (
            "no jobs",
            |p| {
                p["jobs"] = json!([]);
                p.as_object_mut().unwrap().remove("provider_api");
            },
            "1 to 16 jobs",
        ),
        (
            "bad job timeout",
            |p| p["jobs"][0]["timeout_seconds"] = json!(3601),
            "is not valid",
        ),
        (
            "bare wildcard",
            |p| p["permissions"]["network"] = json!(["*"]),
            "Invalid host",
        ),
        (
            "IP address host",
            |p| {
                p["permissions"]["network"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("203.0.113.7"))
            },
            "Invalid host",
        ),
        (
            "host with port",
            |p| {
                p["permissions"]["network"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("videohost.example:8443"))
            },
            "Invalid host",
        ),
        (
            "host with path",
            |p| {
                p["permissions"]["network"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("videohost.example/api"))
            },
            "Invalid host",
        ),
        (
            "undeclared page host",
            |p| p["jobs"][0]["trigger"]["pages"] = json!(["other.example/watch/*"]),
            "not in permissions.network",
        ),
        (
            "page wider than declared",
            |p| p["permissions"]["network"] = json!(["*.cdn.videohost.example"]),
            "not in permissions.network",
        ),
        (
            "web job with no network",
            |p| p["permissions"]["network"] = json!([]),
            "not in permissions.network",
        ),
        (
            "provider without provider_api",
            |p| {
                p.as_object_mut().unwrap().remove("provider_api");
            },
            "provider_api",
        ),
        (
            "provider_api with no provider",
            |p| {
                p["jobs"].as_array_mut().unwrap().remove(1);
            },
            "provider_api",
        ),
        (
            "change_files on a reader",
            |p| p["jobs"][0]["change_files"] = json!(true),
            "is not valid",
        ),
        (
            "change_files on a provider",
            |p| p["jobs"][1]["change_files"] = json!(true),
            "is not valid",
        ),
        (
            "named queue",
            |p| p["jobs"][2]["trigger"]["queue"] = json!("main"),
            "can't name a queue",
        ),
        (
            "queue_completed with a queue",
            |p| {
                p["jobs"]
                    .as_array_mut()
                    .unwrap()
                    .push(job(json!({"kind":"queue_completed","queue":"main"})))
            },
            "leave the queue empty",
        ),
        (
            "wildcard page declared by one named host",
            |p| {
                p["permissions"]["network"] = json!(["x.videohost.example"]);
                p["jobs"][0]["trigger"]["pages"] = json!(["*.videohost.example/*"]);
            },
            "not in permissions.network",
        ),
        (
            "provider package too large for a download's copy of it",
            |p| {
                for index in 0..6 {
                    p["files"][format!("big{index}.lua")] = big(64 * KIB);
                }
                p["files"]["provider.lua"] = json!(format!("--{}", "x".repeat(64 * KIB - 2)));
            },
            "must fit in 496 KiB",
        ),
        (
            "choice default not in options",
            |p| p["settings"][0]["default"] = json!("4K"),
            "not one of its options",
        ),
        (
            "duplicate setting",
            |p| p["settings"][1]["id"] = json!("quality"),
            "listed twice",
        ),
        (
            "unknown setting type",
            |p| p["settings"][0]["type"] = json!("file"),
            "not a valid extension file",
        ),
    ];
    assert_eq!(cases.len(), 50);
    for (title, change, expected) in cases {
        let mut package = example();
        change(&mut package);
        let error = format!("{:#}", decode(&package).expect_err(title));
        assert!(error.contains(expected), "{title}: {error}");
    }
}

#[test]
fn a_package_over_the_file_limit_is_refused_before_parsing() {
    assert!(Package::decode(&vec![b' '; MAX_PACKAGE_BYTES + 1]).is_err());
}

#[test]
fn hosts_match_exactly_and_wildcards_exclude_the_bare_domain() {
    let hosts = vec!["a.example".to_owned(), "*.b.example".to_owned()];
    assert!(host_allowed(&hosts, "a.example"));
    assert!(host_allowed(&hosts, "x.b.example"));
    assert!(host_allowed(&hosts, "y.x.b.example"));
    assert!(!host_allowed(&hosts, "b.example"));
    assert!(!host_allowed(&hosts, "sub.a.example"));
    assert!(!host_allowed(&hosts, "evilb.example"));
    assert!(!host_allowed(&hosts, "a.example.evil.test"));
    assert!(url_allowed(&hosts, "https://a.example:8443/x"));
    assert!(!url_allowed(&hosts, "not a url"));
    assert!(!url_allowed(&hosts, "https://evil.test/?a.example"));
    assert!(!url_allowed(&hosts, "https://a.example@evil.test/"));
}

#[test]
fn versions_compare_as_numbers() {
    assert!(version("1.10.0").unwrap() > version("1.9.9").unwrap());
    assert!(version("2.0.0").unwrap() > version("1.99.99").unwrap());
    assert!(version("1.2").is_err());
    assert!(version("1.2.3.4").is_err());
    assert!(version("1.2.x").is_err());
}

#[test]
fn installing_starts_off_keeps_jobs_out_of_the_script_list_and_guards_the_profile() {
    let (_temp, store) = fixture();
    assert_eq!(
        version_of(&store),
        (1, 1),
        "opening never raises the version"
    );
    let installed = store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    assert!(!installed.enabled);
    assert_eq!(version_of(&store), (4, 4));
    assert!(store.scripts().unwrap().is_empty());
    let tidy = store.extension_job("example.videohost", "tidy").unwrap();
    assert!(!tidy.enabled, "starts off");
    assert!(!tidy.change_files, "file access is only a request");
    assert_eq!(tidy.language, Language::Lua);
    assert_eq!(tidy.body, "print('tidy')\n");
    let link = tidy.extension.as_ref().unwrap();
    assert_eq!(link.hosts, ["videohost.example", "*.cdn.videohost.example"]);
    assert!(link.limit_results, "a package with a secret is limited");
    assert!(
        store
            .site_reader_for("https://videohost.example/watch/1")
            .unwrap()
            .is_none()
    );

    store
        .set_extension_enabled("example.videohost", true)
        .unwrap();
    let reader = store
        .site_reader_for("https://videohost.example/watch/1")
        .unwrap()
        .unwrap();
    assert_eq!(reader.extension.unwrap().job, "reader");
    assert!(
        store
            .extension_job("example.videohost", "tidy")
            .unwrap()
            .enabled
    );
}

#[test]
fn extension_jobs_can_only_be_changed_through_their_extension() {
    let (_temp, store) = fixture();
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    let mut tidy = store.extension_job("example.videohost", "tidy").unwrap();
    let error = store.save_script(&tidy).unwrap_err().to_string();
    assert!(error.contains("belongs to an extension"), "{error}");
    tidy.extension = None;
    let error = store.save_script(&tidy).unwrap_err().to_string();
    assert!(error.contains("belongs to an extension"), "{error}");
    let error = store.remove_script(&tidy.id).unwrap_err().to_string();
    assert!(error.contains("belongs to an extension"), "{error}");
    let error = store
        .run_script(&tidy.id, &uuid::Uuid::new_v4().to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("Turn this extension on"), "{error}");
}

#[test]
fn updates_keep_ids_settings_and_stored_values_and_ask_for_new_access() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    let grants = Grants {
        file_access: BTreeSet::from(["tidy".to_owned()]),
        ..Default::default()
    };
    store
        .install_extension(&bytes(&example()), &grants)
        .unwrap();
    store.set_extension_enabled(id, true).unwrap();
    store
        .set_extension_setting(id, "quality", &json!("720p"))
        .unwrap();
    store.set_extension_secret(id, "content_key", KEY).unwrap();
    let before = store.extension_job(id, "tidy").unwrap();
    assert!(before.change_files);
    store
        .set_script_value(&before.id, "seen", &json!(3))
        .unwrap();

    let mut next = example();
    next["version"] = json!("1.3.0");
    next["files"]["tidy.lua"] = json!("print('tidier')\n");
    next["permissions"]["network"]
        .as_array_mut()
        .unwrap()
        .push(json!("api.vh-media.example"));
    let review = store.review_extension(&bytes(&next)).unwrap();
    let changes = review.update.unwrap();
    assert_eq!(changes.from_version, "1.2.0");
    assert_eq!(changes.added_hosts, ["api.vh-media.example"]);
    assert_eq!(changes.changed_files, ["tidy.lua"]);
    assert!(changes.needs_approval());

    let error = store
        .install_extension(&bytes(&next), &Grants::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("needs your approval"), "{error}");
    assert_eq!(store.extension(id).unwrap().package.version, "1.2.0");

    let approved = Grants {
        approved: true,
        ..Default::default()
    };
    let updated = store.install_extension(&bytes(&next), &approved).unwrap();
    assert!(updated.enabled, "an update keeps it on");
    let after = store.extension_job(id, "tidy").unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.revision, before.revision + 1);
    assert!(after.change_files, "the earlier grant still stands");
    assert_eq!(after.body, "print('tidier')\n");
    assert_eq!(
        store.script_value(&after.id, "seen").unwrap(),
        Some(json!(3))
    );
    assert_eq!(
        store.extension_settings(id).unwrap().get("quality"),
        Some(&json!("720p"))
    );

    let error = store
        .install_extension(&bytes(&next), &approved)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not newer"), "{error}");
    next["version"] = json!("1.2.9");
    assert!(store.install_extension(&bytes(&next), &approved).is_err());
}

#[test]
fn an_update_that_only_changes_code_needs_no_approval_but_new_powers_do() {
    let (_temp, store) = fixture();
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    let mut code_only = example();
    code_only["version"] = json!("1.2.1");
    code_only["files"]["tidy.lua"] = json!("print('new')\n");
    store
        .install_extension(&bytes(&code_only), &Grants::default())
        .unwrap();

    let mut reader_only = example();
    reader_only["version"] = json!("1.0.0");
    reader_only["id"] = json!("example.reader");
    reader_only["jobs"] = json!([reader_only["jobs"][0]]);
    reader_only.as_object_mut().unwrap().remove("provider_api");
    store
        .install_extension(&bytes(&reader_only), &Grants::default())
        .unwrap();
    let mut grows = reader_only.clone();
    grows["version"] = json!("1.1.0");
    grows["jobs"]
        .as_array_mut()
        .unwrap()
        .push(job(json!({"kind":"manual"})));
    let changes = store
        .review_extension(&bytes(&grows))
        .unwrap()
        .update
        .unwrap();
    assert_eq!(changes.new_kinds, ["automation"]);
    assert_eq!(changes.added_jobs, ["Extra"]);
    assert!(changes.needs_approval());
    assert!(
        store
            .install_extension(&bytes(&grows), &Grants::default())
            .is_err()
    );
}

#[test]
fn an_update_cannot_change_how_a_job_runs_or_who_publishes_it_unseen() {
    let (_temp, store) = fixture();
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    type Change = fn(&mut Value);
    let cases: [(&str, Change); 3] = [
        ("Tidy finished files", |p| {
            p["jobs"][2]["trigger"] = json!({"kind":"startup"})
        }),
        ("VideoHost pages", |p| {
            p["jobs"][0]["trigger"]["pages"] = json!(["videohost.example/*"])
        }),
        ("Tidy finished files", |p| {
            p["jobs"][2]["entry"] = json!("lib/parse.lua")
        }),
    ];
    for (name, change) in cases {
        let mut next = example();
        next["version"] = json!("1.3.0");
        change(&mut next);
        let changes = store
            .review_extension(&bytes(&next))
            .unwrap()
            .update
            .unwrap();
        assert_eq!(changes.changed_jobs, [name]);
        assert!(changes.needs_approval());
        let error = store
            .install_extension(&bytes(&next), &Grants::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("needs your approval"), "{error}");
    }

    let mut next = example();
    next["version"] = json!("1.3.0");
    next["publisher"] = json!("Someone else");
    let changes = store
        .review_extension(&bytes(&next))
        .unwrap()
        .update
        .unwrap();
    assert_eq!(changes.new_publisher.as_deref(), Some("Someone else"));
    assert!(changes.needs_approval());
    assert!(
        store
            .install_extension(&bytes(&next), &Grants::default())
            .is_err()
    );
}

#[test]
fn an_update_resets_only_the_settings_it_can_no_longer_use() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    store
        .set_extension_setting(id, "quality", &json!("720p"))
        .unwrap();
    store
        .set_extension_secret(id, "api_token", "private-token")
        .unwrap();
    store.set_extension_secret(id, "content_key", KEY).unwrap();
    let mut next = example();
    next["version"] = json!("1.3.0");
    next["settings"][0]["options"] = json!(["1080p", "480p"]);
    next["settings"][1] = json!({ "id": "api_token", "type": "string", "label": "API token" });
    let changes = store
        .review_extension(&bytes(&next))
        .unwrap()
        .update
        .unwrap();
    assert_eq!(changes.dropped_settings, ["API token", "Preferred quality"]);
    assert!(!changes.needs_approval());
    store
        .install_extension(&bytes(&next), &Grants::default())
        .unwrap();
    let saved = store.extension_settings(id).unwrap();
    assert_eq!(saved.keys().collect::<Vec<_>>(), ["content_key"]);

    store.set_extension_enabled(id, true).unwrap();
    let job = store.extension_job(id, "tidy").unwrap();
    let context = store
        .extension_context(&job, Reach::Automation)
        .unwrap()
        .unwrap();
    assert_eq!(context.settings["quality"], json!("1080p"));
    assert_eq!(context.settings["api_token"], Value::Null);
}

#[test]
fn a_dropped_job_goes_away_with_its_values_and_its_settings() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    store
        .set_extension_setting(id, "quality", &json!("720p"))
        .unwrap();
    let tidy = store.extension_job(id, "tidy").unwrap();
    store.set_script_value(&tidy.id, "seen", &json!(1)).unwrap();
    let mut next = example();
    next["version"] = json!("2.0.0");
    next["jobs"].as_array_mut().unwrap().remove(2);
    next["settings"].as_array_mut().unwrap().remove(0);
    let changes = store
        .review_extension(&bytes(&next))
        .unwrap()
        .update
        .unwrap();
    assert_eq!(changes.dropped_settings, ["Preferred quality"]);
    store
        .install_extension(&bytes(&next), &Grants::default())
        .unwrap();
    assert!(store.extension_job(id, "tidy").is_err());
    assert!(store.script(&tidy.id).is_err());
    assert!(store.script_values(&tidy.id).unwrap().is_empty());
    assert!(store.extension_settings(id).unwrap().is_empty());
}

#[test]
fn queue_jobs_need_a_queue_chosen_by_the_person() {
    let (_temp, store) = fixture();
    let mut package = example();
    package["jobs"]
        .as_array_mut()
        .unwrap()
        .push(job(json!({"kind":"queue_completed","queue":""})));
    let error = store
        .install_extension(&bytes(&package), &Grants::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("Choose a queue"), "{error}");
    let queue = store.create_queue("Chosen").unwrap().id;
    let grants = Grants {
        queues: BTreeMap::from([("extra".to_owned(), queue.clone())]),
        ..Default::default()
    };
    store.install_extension(&bytes(&package), &grants).unwrap();
    assert_eq!(
        store
            .extension_job("example.videohost", "extra")
            .unwrap()
            .trigger,
        Trigger::QueueCompleted { queue }
    );
}

#[test]
fn a_job_only_receives_the_settings_its_kind_may_hold() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    store.set_extension_enabled(id, true).unwrap();
    store
        .set_extension_secret(id, "api_token", "private-token")
        .unwrap();
    store.set_extension_secret(id, "content_key", KEY).unwrap();
    let job = store.extension_job(id, "tidy").unwrap();

    let web = store.extension_context(&job, Reach::Web).unwrap().unwrap();
    assert_eq!(web.settings["quality"], json!("1080p"));
    assert_eq!(web.settings["api_token"], json!("private-token"));
    assert!(web.keys.is_empty() && web.key_scope.is_none());

    let automation = store
        .extension_context(&job, Reach::Automation)
        .unwrap()
        .unwrap();
    assert_eq!(automation.settings["quality"], json!("1080p"));
    assert!(!automation.settings.contains_key("api_token"));
    assert!(automation.keys.is_empty() && automation.key_scope.is_none());

    let processor = store
        .extension_context(&job, Reach::Processor)
        .unwrap()
        .unwrap();
    assert!(!processor.settings.contains_key("api_token"));
    assert!(processor.keys["content_key"].starts_with("provider-key-v1:"));
    assert!(processor.key_scope.is_some());

    // Secrets are sealed on disk and never come back through the settings list.
    let raw: Vec<u8> = store
        .connect()
        .unwrap()
        .query_row(
            "SELECT value FROM extension_settings WHERE key='api_token'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!raw.windows(13).any(|w| w == b"private-token"));
    assert_eq!(
        store.extension_settings(id).unwrap().get("api_token"),
        Some(&Value::Null)
    );
    store.clear_extension_setting(id, "api_token").unwrap();
    assert!(
        !store
            .extension_settings(id)
            .unwrap()
            .contains_key("api_token")
    );
}

#[test]
fn settings_are_checked_against_the_manifest() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    assert!(
        store
            .set_extension_setting(id, "quality", &json!("4K"))
            .is_err()
    );
    assert!(
        store
            .set_extension_setting(id, "quality", &json!(5))
            .is_err()
    );
    assert!(
        store
            .set_extension_setting(id, "missing", &json!("x"))
            .is_err()
    );
    assert!(
        store
            .set_extension_setting(id, "api_token", &json!("x"))
            .is_err()
    );
    assert!(store.set_extension_secret(id, "quality", "x").is_err());
    assert!(
        store
            .set_extension_secret(id, "content_key", "12ab")
            .is_err()
    );
    assert!(store.set_extension_secret(id, "api_token", "").is_err());
}

#[test]
fn personal_readers_go_first_and_a_conflict_is_reported() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    store.set_extension_enabled(id, true).unwrap();
    assert_eq!(store.extension_conflict(id).unwrap(), None);
    store
        .save_script(&Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: "My reader".into(),
            language: Language::Lua,
            body: "return { items = {} }".into(),
            enabled: true,
            trigger: Trigger::SiteReader {
                pages: vec!["videohost.example/*".into()],
            },
            timeout_seconds: 30,
            change_files: false,
            extension: None,
        })
        .unwrap();
    let reader = store
        .site_reader_for("https://videohost.example/watch/1")
        .unwrap()
        .unwrap();
    assert_eq!(reader.name, "My reader", "created later, still first");
    assert_eq!(
        store.extension_conflict(id).unwrap().as_deref(),
        Some("My reader")
    );
}

#[test]
fn removing_refuses_while_a_download_needs_it_and_then_clears_everything() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    let provider = store.extension_job(id, "decrypt").unwrap();
    let error = store
        .add_provider_download(
            &provider.id,
            provider.revision,
            "https://videohost.example/a.bin",
            None,
            "main",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("Turn on VideoHost"), "{error}");

    store.set_extension_enabled(id, true).unwrap();
    let provider = store.extension_job(id, "decrypt").unwrap();
    let download = store
        .add_provider_download(
            &provider.id,
            provider.revision,
            "https://videohost.example/a.bin",
            None,
            "main",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    store
        .set_extension_setting(id, "quality", &json!("720p"))
        .unwrap();

    // The download keeps the code it started with, even after an update.
    let mut next = example();
    next["version"] = json!("1.3.0");
    next["files"]["lib/parse.lua"] = json!("return { changed = true }\n");
    store
        .install_extension(&bytes(&next), &Grants::default())
        .unwrap();
    let frozen = store.get(&download.id).unwrap().unwrap();
    let frozen = &frozen.provider.as_ref().unwrap().script;
    let link = frozen.extension.as_ref().unwrap();
    assert_eq!(link.version, "1.2.0");
    assert!(link.files["lib/parse.lua"].contains("M.read"));
    let context = store
        .extension_context(frozen, Reach::Web)
        .unwrap()
        .unwrap();
    assert!(context.files["lib/parse.lua"].contains("M.read"));

    let error = store.remove_extension(id).unwrap_err().to_string();
    assert!(error.contains("1 unfinished downloads"), "{error}");
    store.remove(&download.id).unwrap();
    store.remove_extension(id).unwrap();
    assert!(store.extensions().unwrap().is_empty());
    assert!(store.script(&provider.id).is_err());
    let conn = store.connect().unwrap();
    for table in ["extension_jobs", "extension_settings"] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

fn add_download(store: &Store, id: &str) -> crate::model::Job {
    let provider = store.extension_job(id, "decrypt").unwrap();
    store
        .add_provider_download(
            &provider.id,
            provider.revision,
            "https://videohost.example/a.bin",
            None,
            "main",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
}

#[test]
fn a_turned_off_extension_cannot_start_a_download_and_stops_a_running_one() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    store.set_extension_enabled(id, true).unwrap();
    let download = add_download(&store, id);
    let running = add_download(&store, id);
    store
        .connect()
        .unwrap()
        .execute(
            "UPDATE jobs SET data=json_set(data,'$.status','downloading') WHERE id=?1",
            [&running.id],
        )
        .unwrap();
    let job = store.get(&download.id).unwrap().unwrap();
    let frozen = &job.provider.as_ref().unwrap().script;
    store.extension_context(frozen, Reach::Web).unwrap();

    store.set_extension_enabled(id, false).unwrap();
    for reach in [Reach::Web, Reach::Processor, Reach::Automation] {
        let error = store
            .extension_context(frozen, reach)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Turn on VideoHost"), "{error}");
    }
    let requests = store
        .connect()
        .unwrap()
        .prepare("SELECT action FROM control_requests")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(requests[0].contains(&running.id), "{requests:?}");

    store.set_extension_enabled(id, true).unwrap();
    store.extension_context(frozen, Reach::Web).unwrap();
}

#[test]
fn a_frozen_download_follows_a_secret_added_after_it_started() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    let mut first = example();
    first["settings"].as_array_mut().unwrap().remove(1);
    store
        .install_extension(&bytes(&first), &Grants::default())
        .unwrap();
    store.set_extension_enabled(id, true).unwrap();
    let job = add_download(&store, id);
    let frozen = &job.provider.as_ref().unwrap().script;
    let context = store.extension_context(frozen, Reach::Web).unwrap();
    assert!(!context.unwrap().limit_results);

    let mut next = example();
    next["version"] = json!("1.3.0");
    store
        .install_extension(&bytes(&next), &Grants::default())
        .unwrap();
    let context = store.extension_context(frozen, Reach::Web).unwrap();
    assert!(context.unwrap().limit_results);
}

#[test]
fn modules_load_from_the_package_only_once_and_never_from_disk() {
    let context = Context {
        id: "example.videohost".into(),
        version: "1.2.0".into(),
        job: "tidy".into(),
        files: BTreeMap::from([
            (
                "lib/a.lua".to_owned(),
                "COUNT = (COUNT or 0) + 1\nreturn { quality = xd.setting('quality') }".to_owned(),
            ),
            (
                "lib/loop.lua".to_owned(),
                "return require('lib.loop')".to_owned(),
            ),
            ("lib/none.lua".to_owned(), "local x = 1".to_owned()),
        ]),
        settings: BTreeMap::from([("quality".to_owned(), json!("720p"))]),
        ..Default::default()
    };
    let run = |source: &str| {
        execute_job(source, &json!({}), Some(&context), |_| Ok(Value::Null))
            .map_err(|e| format!("{e:#}"))
    };
    run(r#"
        local a = require("lib.a")
        assert(a.quality == "720p")
        assert(require("lib.a") == a)
        assert(COUNT == 1)
        assert(require("lib.none") == true)
        assert(xd.extension.id == "example.videohost")
        assert(xd.extension.job == "tidy")
        assert(xd.supports("xd.setting") and xd.supports("require"))
        assert(not xd.supports("xd.fetch"))
    "#)
    .unwrap();
    for (source, expected) in [
        ("require('lib.missing')", "not in this extension"),
        ("require('../secret')", "invalid module name"),
        ("require('C:\\\\Windows\\\\win')", "invalid module name"),
        ("require('lib.loop')", "in a loop"),
        ("xd.setting('api_token')", "not a setting this job can read"),
    ] {
        let error = run(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
    }
    // A personal script has none of this.
    let error = format!(
        "{:#}",
        execute_lua("require('lib.a')", &json!({}), |_| Ok(Value::Null)).unwrap_err()
    );
    assert!(error.contains("require"), "{error}");
}

#[test]
fn extensions_cannot_use_commands_that_belong_to_the_person() {
    let context = Context::default();
    for command in ["automation", "request", "cancel", "restart"] {
        let source = format!("xd.command('{command}', 'list')");
        let error = format!(
            "{:#}",
            execute_job(&source, &json!({}), Some(&context), |_| Ok(Value::Null)).unwrap_err()
        );
        assert!(error.contains("not available to extensions"), "{error}");
        execute_lua(&source, &json!({}), |_| Ok(Value::Null)).unwrap();
    }
    execute_job("xd.command('list')", &json!({}), Some(&context), |_| {
        Ok(Value::Null)
    })
    .unwrap();
}

#[test]
fn an_extension_job_can_only_add_downloads_from_declared_hosts() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    store.set_extension_enabled(id, true).unwrap();
    let tidy = store.extension_job(id, "tidy").unwrap();
    // Download completed jobs run from events, so this tests the job through a run.
    let run = store
        .run_script(&tidy.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    let add = |url: &str| {
        control::execute_stored(
            &store,
            &uuid::Uuid::new_v4().to_string(),
            Action::Add {
                urls: vec![url.to_owned()],
                queue: "main".into(),
                folder: None,
                name: None,
                script_run: Some(run.id.clone()),
            },
        )
    };
    let error = add("https://evil.example/?names=secret")
        .unwrap_err()
        .to_string();
    assert!(error.contains("hosts it declares"), "{error}");
    assert!(store.list().unwrap().is_empty());
    add("https://media.cdn.videohost.example/a.mp4").unwrap();
    let added = store.list().unwrap();
    assert_eq!(added.len(), 1);
    assert!(
        added[0].public_only,
        "a declared name must not reach the local network"
    );
}

#[test]
fn a_personal_script_can_still_add_any_download() {
    let (_temp, store) = fixture();
    let script = store
        .save_script(&Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: "Mine".into(),
            language: Language::Lua,
            body: "print('x')".into(),
            enabled: false,
            trigger: Trigger::Manual,
            timeout_seconds: 5,
            change_files: false,
            extension: None,
        })
        .unwrap();
    let run = store
        .run_script(&script.id, &uuid::Uuid::new_v4().to_string())
        .unwrap();
    control::execute_stored(
        &store,
        &uuid::Uuid::new_v4().to_string(),
        Action::Add {
            urls: vec!["https://anywhere.example/a.bin".into()],
            queue: "main".into(),
            folder: None,
            name: None,
            script_run: Some(run.id),
        },
    )
    .unwrap();
    assert!(!store.list().unwrap()[0].public_only);
}

#[test]
fn a_reader_in_a_package_with_a_secret_cannot_return_other_hosts() {
    let (_temp, store) = fixture();
    let id = "example.videohost";
    store
        .install_extension(&bytes(&example()), &Grants::default())
        .unwrap();
    let reader = store.extension_job(id, "reader").unwrap();
    let run = store
        .run_site_reader(
            &reader.id,
            "https://videohost.example/watch/1",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    let reading = |url: &str| {
        script_automation::Reading::from_value(json!({"items":[{"kind":"file","url":url}]}))
            .unwrap()
    };
    let error = store
        .save_script_result(&run.id, &reading("https://evil.example/?token=abc"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("declared hosts"), "{error}");
    store
        .save_script_result(
            &run.id,
            &reading("https://media.cdn.videohost.example/a.mp4"),
        )
        .unwrap();

    // Without a secret, results may point at media hosts nobody can list ahead of time.
    let mut plain = example();
    plain["id"] = json!("example.plain");
    plain["settings"] = json!([]);
    store
        .install_extension(&bytes(&plain), &Grants::default())
        .unwrap();
    let reader = store.extension_job("example.plain", "reader").unwrap();
    let run = store
        .run_site_reader(
            &reader.id,
            "https://videohost.example/watch/1",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    store
        .save_script_result(&run.id, &reading("https://media.elsewhere.example/a.mp4"))
        .unwrap();
}

#[test]
fn pack_builds_a_package_from_a_folder_and_refuses_anything_unexpected() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("videohost");
    std::fs::create_dir_all(folder.join("lib")).unwrap();
    let mut manifest = example();
    let files = manifest.as_object_mut().unwrap().remove("files").unwrap();
    for (path, source) in files.as_object().unwrap() {
        std::fs::write(folder.join(path), source.as_str().unwrap()).unwrap();
    }
    std::fs::write(folder.join("manifest.json"), bytes(&manifest)).unwrap();
    let (name, packed) = pack(&folder).unwrap();
    assert_eq!(name, "example.videohost-1.2.0.de-extension.json");
    assert_eq!(
        Package::decode(&packed).unwrap(),
        decode(&example()).unwrap()
    );

    std::fs::write(folder.join("notes.txt"), "x").unwrap();
    let error = pack(&folder).unwrap_err().to_string();
    assert!(error.contains("Only lower-case .lua files"), "{error}");
    std::fs::remove_file(folder.join("notes.txt")).unwrap();

    manifest["files"] = json!({});
    std::fs::write(folder.join("manifest.json"), bytes(&manifest)).unwrap();
    let error = pack(&folder).unwrap_err().to_string();
    assert!(error.contains("must not list files"), "{error}");
}

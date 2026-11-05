//! The starter pack in examples/extensions. Each package must be its folder,
//! packed; it installs turned off through the normal review; and its code works
//! against local copies of the sites' answers. Nothing here reaches the network.
use super::*;
use crate::{
    paths::AppPaths,
    script_automation::{
        FetchReply, FetchRequest, Reading, Script, Trigger, execute_job, execute_reader_job,
    },
    store::Store,
};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, rc::Rc};

const EXAMPLES: [&str; 5] = [
    "internet-archive",
    "wikimedia-commons",
    "github-releases",
    "sort-by-type",
    "queue-finished",
];
const ARCHIVE: &str = include_str!("../../tests/fixtures/extensions/archive-org-metadata.json");
const COMMONS: &str = include_str!("../../tests/fixtures/extensions/commons-imageinfo.json");
const GITHUB: &str = include_str!("../../tests/fixtures/extensions/github-release.json");

fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/extensions")
}
fn packed(name: &str) -> Vec<u8> {
    std::fs::read(folder().join(format!("{name}{FILE_SUFFIX}"))).unwrap()
}
/// A copy of an example under the author's own id, as docs/first-extension.md
/// says to make one, changed by `edit`.
fn own_copy(name: &str, edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(&packed(name)).unwrap();
    value["id"] = json!(format!("test.{name}"));
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
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

/// Installs an example, turns it on and returns its job's code with the
/// context the app gives that job's worker.
fn installed(name: &str, reach: Reach, grants: &Grants) -> (tempfile::TempDir, Store, Script, Context) {
    let (temp, store) = fixture();
    let installed = store.install_extension(&packed(name), grants).unwrap();
    let id = installed.package.id.clone();
    store.set_extension_enabled(&id, true).unwrap();
    let script = store
        .extension_job(&id, &installed.package.jobs[0].id)
        .unwrap();
    let context = store.extension_context(&script, reach).unwrap().unwrap();
    (temp, store, script, context)
}

/// Runs a site reader the way the CLI worker does, but answers each fetch from
/// `answers` by its exact address. Like the worker, it refuses undeclared hosts.
fn read(
    script: &Script,
    context: &Context,
    page: &str,
    answers: &[(&str, u16, &str)],
) -> (anyhow::Result<Reading>, Vec<FetchRequest>) {
    let answers: BTreeMap<String, (u16, String)> = answers
        .iter()
        .map(|(url, status, body)| ((*url).to_owned(), (*status, (*body).to_owned())))
        .collect();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let hosts = context.hosts.clone();
    let result = execute_reader_job(
        &script.body,
        &json!({"trigger": "site_reader", "url": page, "downloads": []}),
        Some(context),
        move |request: FetchRequest| {
            anyhow::ensure!(
                url_allowed(&hosts, &request.url),
                "{} is not on a declared host",
                request.url
            );
            log.borrow_mut().push(request.clone());
            let (status, body) = answers
                .get(&request.url)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("No local answer for {}", request.url))?;
            Ok(FetchReply {
                status,
                url: request.url,
                headers: BTreeMap::new(),
                body,
            })
        },
    )
    .and_then(Reading::from_value);
    let requests = seen.borrow().clone();
    (result, requests)
}

/// Runs an automation job, answering its app commands with `reply` and
/// recording each one, as the CLI contract would receive them.
fn run(
    script: &Script,
    context: &Context,
    event: Value,
    reply: impl Fn(&[String]) -> anyhow::Result<Value> + 'static,
) -> (anyhow::Result<()>, Vec<Vec<String>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let result = execute_job(&script.body, &event, Some(context), move |args| {
        log.borrow_mut().push(args.clone());
        reply(&args)
    });
    let commands = seen.borrow().clone();
    (result, commands)
}

fn items(reading: &Reading) -> Value {
    serde_json::to_value(&reading.items).unwrap()
}

#[test]
fn each_example_package_is_its_folder_packed() {
    for name in EXAMPLES {
        let (file, bytes) = pack(&folder().join(name)).unwrap();
        assert!(file.starts_with("examples."), "{file}");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            String::from_utf8(packed(name)).unwrap(),
            "{name} changed: rebuild it with de-cli extensions pack examples/extensions/{name} --out examples/extensions/{name}{FILE_SUFFIX}"
        );
    }
}

#[test]
fn every_example_installs_turned_off_through_the_normal_review() {
    let (_temp, store) = fixture();
    let queue = store.create_queue("Nightly").unwrap().id;
    let mut hosts = BTreeMap::new();
    for name in EXAMPLES {
        let bytes = packed(name);
        let review = store.review_extension(&bytes).unwrap();
        assert!(review.update.is_none(), "{name}");
        let package = review.package;
        assert_eq!(package.publisher, "Download Engine examples", "{name}");
        assert_eq!(package.version, "1.0.0", "{name}");
        assert!(
            package.jobs.iter().all(|job| job.trigger != Trigger::Provider),
            "{name}: the starter pack has no provider jobs"
        );
        assert!(package.settings.iter().all(|s| !s.protected()), "{name}");
        let grants = Grants {
            queues: package
                .jobs
                .iter()
                .filter(|job| matches!(job.trigger, Trigger::QueueCompleted { .. }))
                .map(|job| (job.id.clone(), queue.clone()))
                .collect(),
            ..Default::default()
        };
        let installed = store.install_extension(&bytes, &grants).unwrap();
        assert!(!installed.enabled, "{name} must install turned off");
        for job in &package.jobs {
            let script = store.extension_job(&package.id, &job.id).unwrap();
            assert!(!script.enabled, "{name}");
            assert!(!script.change_files, "{name}: file access stays off unless ticked");
        }
        hosts.insert(name, package.permissions.network.clone());
    }
    assert_eq!(hosts["internet-archive"], ["archive.org"]);
    assert_eq!(hosts["wikimedia-commons"], ["commons.wikimedia.org"]);
    assert_eq!(hosts["github-releases"], ["github.com", "api.github.com"]);
    assert!(hosts["sort-by-type"].is_empty());
    assert!(hosts["queue-finished"].is_empty());
    assert_eq!(
        store
            .extension_job("examples.queue-finished", "notify")
            .unwrap()
            .trigger,
        Trigger::QueueCompleted { queue }
    );
}

#[test]
fn sort_finished_files_asks_for_file_access_and_gets_it_only_when_ticked() {
    let bytes = packed("sort-by-type");
    let package = Package::decode(&bytes).unwrap();
    assert!(package.jobs[0].change_files);
    assert_eq!(
        package.jobs[0].trigger,
        Trigger::DownloadCompleted { queue: None }
    );
    let (_temp, store) = fixture();
    store
        .install_extension(
            &bytes,
            &Grants {
                file_access: ["sort".to_owned()].into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        store
            .extension_job("examples.sort-by-type", "sort")
            .unwrap()
            .change_files
    );
}

#[test]
fn queue_finished_needs_a_queue_chosen_in_the_review() {
    let (_temp, store) = fixture();
    let error = store
        .install_extension(&packed("queue-finished"), &Grants::default())
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("Choose a queue for Notify when the queue finishes"),
        "{error:#}"
    );
    assert!(store.extensions().unwrap().is_empty());
}

#[test]
fn internet_archive_lists_original_files_from_the_metadata_answer() {
    let (_temp, _store, script, context) =
        installed("internet-archive", Reach::Web, &Grants::default());
    let api = "https://archive.org/metadata/example-public-domain-film";
    let (reading, requests) = read(
        &script,
        &context,
        "https://archive.org/details/example-public-domain-film/Example+Film.mp4?tab=about",
        &[(api, 200, ARCHIVE)],
    );
    let reading = reading.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, api);
    assert_eq!(reading.title.as_deref(), Some("Example Public Domain Film"));
    let base = "https://archive.org/download/example-public-domain-film";
    assert_eq!(
        items(&reading),
        json!([
            {"kind": "file", "url": format!("{base}/Example%20Film.mp4"), "label": "MPEG4",
             "filename": "Example Film.mp4", "size": 73400320},
            {"kind": "file", "url": format!("{base}/extras/Making%20of%20%231.mp4"), "label": "MPEG4",
             "filename": "Making of #1.mp4", "size": 10485760},
            {"kind": "file", "url": format!("{base}/notes.txt"), "label": "Text",
             "filename": "notes.txt", "size": null}
        ]),
        "copies, metadata, private files and playlists are left out, and an empty size is dropped"
    );
}

#[test]
fn internet_archive_can_also_list_copies_and_reports_failures() {
    let (_temp, store, _script, _context) =
        installed("internet-archive", Reach::Web, &Grants::default());
    store
        .set_extension_setting("examples.internet-archive", "include_derivatives", &json!(true))
        .unwrap();
    let script = store
        .extension_job("examples.internet-archive", "item-files")
        .unwrap();
    let context = store.extension_context(&script, Reach::Web).unwrap().unwrap();
    let api = "https://archive.org/metadata/example-public-domain-film";
    let page = "https://archive.org/details/example-public-domain-film";
    let (reading, _) = read(&script, &context, page, &[(api, 200, ARCHIVE)]);
    let names: Vec<_> = reading
        .unwrap()
        .items
        .iter()
        .map(|item| item.filename.clone().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Example Film.mp4",
            "Example Film.ogv",
            "Example Film_000001.jpg",
            "Making of #1.mp4",
            "notes.txt"
        ]
    );

    let (reading, _) = read(&script, &context, page, &[(api, 503, "")]);
    assert!(
        format!("{:#}", reading.unwrap_err()).contains("archive.org answered with HTTP 503")
    );
    let (reading, requests) = read(&script, &context, "https://archive.org/search?query=film", &[]);
    assert!(format!("{:#}", reading.unwrap_err()).contains("Open an archive.org item page"));
    assert!(requests.is_empty());
}

#[test]
fn wikimedia_commons_returns_the_full_size_original() {
    let (_temp, _store, script, context) =
        installed("wikimedia-commons", Reach::Web, &Grants::default());
    let api = "https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2\
               &prop=imageinfo&iiprop=url%7Csize%7Cmime&titles=File%3ACaf%C3%A9_at_night.jpg";
    let (reading, requests) = read(
        &script,
        &context,
        "https://commons.wikimedia.org/wiki/File:Caf%C3%A9_at_night.jpg",
        &[(api, 200, COMMONS)],
    );
    let reading = reading.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(reading.title.as_deref(), Some("File:Café at night.jpg"));
    assert_eq!(
        items(&reading),
        json!([{
            "kind": "file",
            "url": "https://upload.wikimedia.org/wikipedia/commons/a/ab/Caf%C3%A9_at_night.jpg",
            "label": "image/jpeg",
            "filename": "Café at night.jpg",
            "size": 4718592
        }])
    );

    let missing = r#"{"batchcomplete":true,"query":{"pages":[{"ns":6,"title":"File:Nothing here.jpg","missing":true}]}}"#;
    let api = "https://commons.wikimedia.org/w/api.php?action=query&format=json&formatversion=2\
               &prop=imageinfo&iiprop=url%7Csize%7Cmime&titles=File%3ANothing_here.jpg";
    let (reading, _) = read(
        &script,
        &context,
        "https://commons.wikimedia.org/wiki/File:Nothing_here.jpg",
        &[(api, 200, missing)],
    );
    assert!(
        format!("{:#}", reading.unwrap_err())
            .contains("Commons has no file called File:Nothing_here.jpg")
    );
}

#[test]
fn github_releases_lists_a_releases_files_by_tag_or_the_latest_one() {
    let (_temp, store, script, context) =
        installed("github-releases", Reach::Web, &Grants::default());
    let repo = "https://github.com/example-org/example-tool";
    let api = "https://api.github.com/repos/example-org/example-tool/releases";
    let tagged = format!("{api}/tags/v2.1.0");
    let latest = format!("{api}/latest");
    let (reading, requests) = read(
        &script,
        &context,
        &format!("{repo}/releases/tag/v2.1.0"),
        &[(tagged.as_str(), 200, GITHUB)],
    );
    let reading = reading.unwrap();
    assert_eq!(
        requests[0].headers,
        [(
            "Accept".to_owned(),
            "application/vnd.github+json".to_owned()
        )]
    );
    assert_eq!(reading.title.as_deref(), Some("example-tool Example Tool 2.1"));
    let download = format!("{repo}/releases/download/v2.1.0");
    assert_eq!(
        items(&reading),
        json!([
            {"kind": "file", "url": format!("{download}/example-tool-2.1.0-windows-x64.zip"),
             "label": null, "filename": "example-tool-2.1.0-windows-x64.zip", "size": 5242880},
            {"kind": "file", "url": format!("{download}/example-tool-2.1.0-linux-x64.tar.gz"),
             "label": null, "filename": "example-tool-2.1.0-linux-x64.tar.gz", "size": 4194304},
            {"kind": "file", "url": format!("{download}/empty-placeholder.txt"),
             "label": null, "filename": "empty-placeholder.txt", "size": null}
        ])
    );
    for page in ["releases", "releases/", "releases/latest", "releases/latest/"] {
        let (reading, requests) = read(
            &script,
            &context,
            &format!("{repo}/{page}"),
            &[(latest.as_str(), 200, GITHUB)],
        );
        assert_eq!(reading.unwrap().items.len(), 3, "{page}");
        assert_eq!(requests[0].url, latest, "{page}");
    }

    store
        .set_extension_setting("examples.github-releases", "include_source", &json!(true))
        .unwrap();
    let context = store.extension_context(&script, Reach::Web).unwrap().unwrap();
    let (reading, _) = read(
        &script,
        &context,
        &format!("{repo}/releases/latest"),
        &[(latest.as_str(), 200, GITHUB)],
    );
    let found = items(&reading.unwrap());
    let sources = json!([
        {"kind": "file", "url": format!("{repo}/archive/refs/tags/v2.1.0.zip"),
         "label": "Source code (zip)", "filename": "example-tool-v2.1.0.zip", "size": null},
        {"kind": "file", "url": format!("{repo}/archive/refs/tags/v2.1.0.tar.gz"),
         "label": "Source code (tar.gz)", "filename": "example-tool-v2.1.0.tar.gz", "size": null}
    ]);
    assert_eq!(
        &found.as_array().unwrap()[3..],
        &sources.as_array().unwrap()[..]
    );

    let (reading, _) = read(
        &script,
        &context,
        &format!("{repo}/releases/latest"),
        &[(latest.as_str(), 403, "{}")],
    );
    assert!(format!("{:#}", reading.unwrap_err()).contains("GitHub is limiting requests"));
    let (reading, requests) = read(&script, &context, &format!("{repo}/releases/new"), &[]);
    assert!(format!("{:#}", reading.unwrap_err()).contains("Open one release"));
    assert!(requests.is_empty());
}

#[test]
fn github_releases_keeps_a_large_release_within_the_app_limit() {
    let (_temp, store, script, _context) =
        installed("github-releases", Reach::Web, &Grants::default());
    store
        .set_extension_setting("examples.github-releases", "include_source", &json!(true))
        .unwrap();
    let context = store.extension_context(&script, Reach::Web).unwrap().unwrap();
    let assets: Vec<Value> = (0..1001)
        .map(|i| {
            json!({
                "name": format!("part-{i}.bin"),
                "size": 1,
                "browser_download_url":
                    format!("https://github.com/example-org/big/releases/download/v1/part-{i}.bin")
            })
        })
        .collect();
    let body = json!({ "tag_name": "v1", "name": "Big", "assets": assets }).to_string();
    let (reading, _) = read(
        &script,
        &context,
        "https://github.com/example-org/big/releases/latest",
        &[(
            "https://api.github.com/repos/example-org/big/releases/latest",
            200,
            body.as_str(),
        )],
    );
    assert_eq!(reading.unwrap().items.len(), 1000);
}

#[test]
fn sort_finished_files_moves_known_types_into_folders_and_skips_the_rest() {
    let (_temp, _store, script, context) = installed(
        "sort-by-type",
        Reach::Automation,
        &Grants {
            file_access: ["sort".to_owned()].into(),
            ..Default::default()
        },
    );
    let event = json!({
        "trigger": "download_completed",
        "queue": null,
        "downloads": [
            {"id": "a", "queue": "main", "name": "holiday.mp4", "saved_path": "C:\\Downloads\\holiday.mp4"},
            {"id": "b", "queue": "main", "name": "report.PDF", "saved_path": "C:\\Downloads\\report.PDF"},
            {"id": "c", "queue": "main", "name": "song.mp3", "saved_path": "/home/someone/Downloads/Music/song.mp3"},
            {"id": "d", "queue": "main", "name": "clip.mp4", "saved_path": "C:\\Downloads\\videos\\clip.mp4"},
            {"id": "e", "queue": "main", "name": "data.bin", "saved_path": "C:\\Downloads\\data.bin"},
            {"id": "f", "queue": "main", "name": null, "saved_path": null},
            {"id": "g", "queue": "main", "name": "photo.jpg", "saved_path": "/home/someone/Downloads/photo.jpg"},
            {"id": "h", "queue": "main", "name": "setup.exe", "saved_path": "D:\\Elsewhere\\setup.exe"},
            {"id": "i", "queue": "main", "name": "book.epub", "saved_path": "C:\\Downloads\\book.epub"}
        ]
    });
    let (result, commands) = run(&script, &context, event, |args| {
        // A move outside the download folders fails; the rest must still run.
        anyhow::ensure!(args[1] != "h", "Choose a folder inside your download folders");
        Ok(json!({"ok": true}))
    });
    result.unwrap();
    let mv = |id: &str, folder: &str| -> Vec<String> {
        ["move", id, folder, "--wait", "120"]
            .map(str::to_owned)
            .to_vec()
    };
    assert_eq!(
        commands,
        [
            mv("a", "C:\\Downloads\\Videos"),
            mv("b", "C:\\Downloads\\Documents"),
            mv("g", "/home/someone/Downloads/Pictures"),
            mv("h", "D:\\Elsewhere\\Programs"),
            mv("i", "C:\\Downloads\\Documents"),
        ]
    );
}

#[test]
fn sort_finished_files_needs_its_file_access_granted_to_move_anything() {
    // Without the grant, the app's own check refuses each move; the script logs
    // a warning per file and the run still finishes.
    let (_temp, store, script, context) =
        installed("sort-by-type", Reach::Automation, &Grants::default());
    assert!(!script.change_files);
    let event = json!({
        "trigger": "download_completed",
        "queue": null,
        "downloads": [
            {"id": "a", "queue": "main", "name": "holiday.mp4", "saved_path": "C:\\Downloads\\holiday.mp4"},
            {"id": "b", "queue": "main", "name": "report.pdf", "saved_path": "C:\\Downloads\\report.pdf"}
        ]
    });
    let id = script.id.clone();
    let store = Rc::new(store);
    let saved = store.clone();
    let (result, commands) = run(&script, &context, event, move |_| {
        let script = saved.script(&id)?;
        anyhow::ensure!(script.change_files, "This script can't change downloaded files.");
        Ok(json!({"ok": true}))
    });
    result.unwrap();
    assert_eq!(commands.len(), 2, "every move was tried and refused: {commands:?}");
}

#[test]
fn queue_finished_notifies_and_starts_the_next_queue_when_set() {
    let (_temp, store) = fixture();
    let nightly = store.create_queue("Nightly").unwrap().id;
    store
        .install_extension(
            &packed("queue-finished"),
            &Grants {
                queues: [("notify".to_owned(), nightly.clone())].into(),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .set_extension_enabled("examples.queue-finished", true)
        .unwrap();
    let script = store
        .extension_job("examples.queue-finished", "notify")
        .unwrap();
    let event = json!({"trigger": "queue_completed", "queue": nightly, "downloads": []});
    let answers = {
        let nightly = nightly.clone();
        move |args: &[String]| -> anyhow::Result<Value> {
            Ok(match args[0].as_str() {
                "queues" => json!([
                    {"id": "main", "name": "Main", "running": false},
                    {"id": nightly, "name": "Nightly", "running": false}
                ]),
                "list" => json!([
                    {"id": "1", "queue": nightly, "status": "complete"},
                    {"id": "2", "queue": nightly, "status": "complete"},
                    {"id": "3", "queue": nightly, "status": "failed"},
                    {"id": "4", "queue": "main", "status": "complete"}
                ]),
                _ => json!({"ok": true}),
            })
        }
    };
    let strings = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();

    let context = store.extension_context(&script, Reach::Automation).unwrap().unwrap();
    assert_eq!(context.settings["next_queue"], json!(""));
    let (result, commands) = run(&script, &context, event.clone(), answers.clone());
    result.unwrap();
    assert_eq!(
        commands,
        [
            strings(&["queues"]),
            strings(&["list"]),
            strings(&["notify", "Queue finished: Nightly", "2 downloads are complete"]),
        ]
    );

    store
        .set_extension_setting("examples.queue-finished", "next_queue", &json!("Morning"))
        .unwrap();
    let context = store.extension_context(&script, Reach::Automation).unwrap().unwrap();
    let (result, commands) = run(&script, &context, event, answers);
    result.unwrap();
    assert_eq!(commands[3], strings(&["queue", "start", "Morning"]));
}

#[test]
fn only_the_shipped_files_can_use_an_examples_id() {
    for name in EXAMPLES {
        assert!(examples::is_example(&packed(name)), "{name}");
    }
    let (_temp, store) = fixture();
    let changed = |version: &str| {
        let mut value: Value = serde_json::from_slice(&packed("sort-by-type")).unwrap();
        value["version"] = json!(version);
        value["files"]["sort.lua"] = json!("print('not the example')\n");
        serde_json::to_vec(&value).unwrap()
    };
    // A changed copy can't install under the example's id...
    for error in [
        store.review_extension(&changed("1.0.0")).unwrap_err(),
        store
            .install_extension(&changed("1.0.0"), &Grants::default())
            .unwrap_err(),
    ] {
        assert!(format!("{error:#}").contains("examples."), "{error:#}");
    }
    assert!(store.extensions().unwrap().is_empty());

    // ...and can't replace the real one, which would keep the access given to it.
    store
        .install_extension(
            &packed("sort-by-type"),
            &Grants {
                file_access: ["sort".to_owned()].into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        store
            .install_extension(&changed("1.0.1"), &Grants::default())
            .is_err()
    );
    let job = store.extension_job("examples.sort-by-type", "sort").unwrap();
    assert!(job.body.contains("folder_for"), "the example's own code stays");
    assert!(job.change_files);

    // Under the author's own id, the same change is an ordinary package.
    store
        .install_extension(&own_copy("sort-by-type", |_| {}), &Grants::default())
        .unwrap();
}

#[test]
fn file_access_can_be_granted_after_install_and_taken_back() {
    let (_temp, store) = fixture();
    store
        .install_extension(&own_copy("sort-by-type", |_| {}), &Grants::default())
        .unwrap();
    let id = "test.sort-by-type";
    let before = store.extension_job(id, "sort").unwrap();
    let cursor = |store: &Store| -> i64 {
        store
            .connect()
            .unwrap()
            .query_row(
                "SELECT cursor FROM automation_scripts WHERE id=?1",
                [&before.id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let saved_cursor = cursor(&store);

    store.set_extension_file_access(id, "sort", true).unwrap();
    let granted = store.extension_job(id, "sort").unwrap();
    assert!(granted.change_files);
    assert_eq!(granted.revision, before.revision + 1);
    assert_eq!(granted.body, before.body);
    assert_eq!(granted.trigger, before.trigger);
    assert_eq!(cursor(&store), saved_cursor, "pending events are kept");
    // Granting again changes nothing.
    store.set_extension_file_access(id, "sort", true).unwrap();
    assert_eq!(store.extension_job(id, "sort").unwrap().revision, granted.revision);

    // An update that still asks keeps the grant without asking again.
    let next = own_copy("sort-by-type", |value| value["version"] = json!("1.0.1"));
    let review = store.review_extension(&next).unwrap();
    assert!(!review.update.as_ref().unwrap().needs_approval());
    store.install_extension(&next, &Grants::default()).unwrap();
    assert!(store.extension_job(id, "sort").unwrap().change_files);

    store.set_extension_file_access(id, "sort", false).unwrap();
    assert!(!store.extension_job(id, "sort").unwrap().change_files);
}

#[test]
fn file_access_is_never_granted_to_a_job_that_does_not_ask() {
    let (_temp, store) = fixture();
    let queue = store.create_queue("Nightly").unwrap().id;
    store
        .install_extension(
            &packed("queue-finished"),
            &Grants {
                queues: [("notify".to_owned(), queue)].into(),
                ..Default::default()
            },
        )
        .unwrap();
    let error = store
        .set_extension_file_access("examples.queue-finished", "notify", true)
        .unwrap_err();
    assert!(format!("{error:#}").contains("doesn't ask to change downloaded files"));
    assert!(
        !store
            .extension_job("examples.queue-finished", "notify")
            .unwrap()
            .change_files
    );
    // Taking back what was never granted is harmless.
    store
        .set_extension_file_access("examples.queue-finished", "notify", false)
        .unwrap();
    for (id, job) in [("examples.queue-finished", "missing"), ("examples.none", "notify")] {
        assert!(store.set_extension_file_access(id, job, true).is_err(), "{id} {job}");
    }
}

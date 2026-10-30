use super::super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;
use serde_json::{Value, json};

const ID: &str = "example.videohost";

fn package(version: &str, hosts: &[&str]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "download-engine-extension",
        "format_version": 1,
        "id": ID,
        "name": "VideoHost",
        "version": version,
        "publisher": "Example",
        "description": "Adds VideoHost pages to Find videos.",
        "api_version": 1,
        "permissions": { "network": hosts },
        "settings": [
            { "id": "quality", "type": "choice", "label": "Preferred quality",
              "options": ["1080p", "720p"], "default": "1080p" },
            { "id": "api_token", "type": "secret", "label": "API token" }
        ],
        "jobs": [
            { "id": "reader", "name": "VideoHost pages", "entry": "reader.lua", "timeout_seconds": 30,
              "trigger": { "kind": "site_reader", "pages": ["videohost.example/watch/*"] } },
            { "id": "tidy", "name": "Tidy finished files", "entry": "tidy.lua", "timeout_seconds": 30,
              "change_files": true, "trigger": { "kind": "download_completed", "queue": null } }
        ],
        "files": {
            "reader.lua": "return { items = {} }\n",
            "tidy.lua": "print('tidy')\n"
        }
    }))
    .unwrap()
}
fn open(app: &mut Desktop) {
    app.view = View::Automation;
    app.refresh_automation();
}
fn review(app: &mut Desktop, bytes: Vec<u8>) {
    let reviewed = app.store.review_extension(&bytes).unwrap();
    app.review_extension(bytes, reviewed);
}
fn size() -> egui::Vec2 {
    egui::vec2(1280.0, 860.0)
}
fn has_text(output: &egui::FullOutput, label: &str) -> bool {
    output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == label)
    })
}

#[test]
fn the_page_opens_on_installed_and_keeps_scripts_and_rules_as_tabs() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "No extensions yet");
    click(&mut app, &ctx, "Scripts");
    let output = frame(&mut app, &ctx, size(), vec![]);
    assert!(!has_text(&output, "No extensions yet"));
    click(&mut app, &ctx, "Rules (0)");
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "+ New rule");
    click(&mut app, &ctx, "Run history");
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    assert!(!has_text(&output, "+ New rule"));
}

#[test]
fn installing_goes_through_a_review_and_starts_turned_off() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    review(&mut app, package("1.2.0", &["videohost.example"]));
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Install VideoHost?");
    text_rect(&output, "Read videohost.example");
    text_rect(
        &output,
        "It installs turned off. Turn it on after you review it.",
    );
    assert!(
        app.store.extensions().unwrap().is_empty(),
        "nothing saved yet"
    );

    click(&mut app, &ctx, "Install");
    assert!(app.dialog.is_none());
    let installed = app.store.extension(ID).unwrap();
    assert!(!installed.enabled);
    assert!(
        !app.store.extension_job(ID, "tidy").unwrap().change_files,
        "file access stays off unless ticked"
    );
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "1.2.0 · Example · Publisher not verified");
    text_rect(&output, "Disabled");
    text_rect(&output, "What it can do");
    text_rect(&output, "Use your browser logins or cookies");
    text_rect(&output, "Tidy finished files");

    click(&mut app, &ctx, "Enabled");
    assert!(app.store.extension(ID).unwrap().enabled);
}

#[test]
fn the_jobs_table_becomes_a_list_when_it_would_not_fit() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    let long = "This job name cannot share a row with three other columns";
    let bytes = String::from_utf8(package("1.2.0", &["videohost.example"]))
        .unwrap()
        .replace("VideoHost pages", long);
    review(&mut app, bytes.into_bytes());
    click(&mut app, &ctx, "Install");

    let small = egui::vec2(860.0, 1200.0);
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    assert!(!has_text(&output, "Type"));
    text_rect(&output, "Site reader · On matching pages · Not run yet");
    assert!(text_rect(&output, long).right() <= small.x);

    let wide = egui::vec2(1800.0, 860.0);
    frame(&mut app, &ctx, wide, vec![]);
    let output = frame(&mut app, &ctx, wide, vec![]);
    text_rect(&output, "Type");
}

#[test]
fn installing_shows_the_new_extension_and_drops_what_was_typed_for_the_last_one() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Settings");
    app.extensions
        .secrets
        .insert("api_token".into(), "half-typed".into());
    let other = String::from_utf8(package("1.0.0", &["videohost.example"]))
        .unwrap()
        .replace(ID, "example.other")
        .replace("VideoHost", "Other");
    review(&mut app, other.into_bytes());
    click(&mut app, &ctx, "Install");
    assert_eq!(app.extensions.selected.as_deref(), Some("example.other"));
    assert!(app.extensions.detail == Detail::Overview);
    assert!(app.extensions.secrets.is_empty());
    assert_eq!(app.extensions.values["quality"], "1080p");
}

#[test]
fn ticking_file_access_is_what_grants_it() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    review(&mut app, package("1.2.0", &["videohost.example"]));
    click(
        &mut app,
        &ctx,
        "Let Tidy finished files rename, move and delete any downloaded file",
    );
    click(&mut app, &ctx, "Install");
    assert!(app.store.extension_job(ID, "tidy").unwrap().change_files);
}

#[test]
fn cancelling_a_review_changes_nothing() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    review(&mut app, package("1.2.0", &["videohost.example"]));
    click(&mut app, &ctx, "Cancel");
    assert!(app.dialog.is_none());
    assert!(app.store.extensions().unwrap().is_empty());
}

#[test]
fn an_update_shows_new_access_and_not_now_keeps_the_old_version() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    let next = package("1.3.0", &["videohost.example", "api.vh-media.example"]);
    review(&mut app, next.clone());
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Update VideoHost?");
    text_rect(&output, "New access");
    text_rect(&output, "Read api.vh-media.example");
    text_rect(&output, "Approve and update");
    click(&mut app, &ctx, "Not now");
    assert_eq!(app.store.extension(ID).unwrap().package.version, "1.2.0");

    review(&mut app, next);
    click(&mut app, &ctx, "Approve and update");
    assert_eq!(app.store.extension(ID).unwrap().package.version, "1.3.0");
}

#[test]
fn an_update_names_a_new_publisher_changed_jobs_and_reset_settings() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    app.store
        .set_extension_setting(ID, "quality", &json!("720p"))
        .unwrap();
    open(&mut app);
    let mut next: Value =
        serde_json::from_slice(&package("1.3.0", &["videohost.example"])).unwrap();
    next["publisher"] = json!("Someone else");
    next["jobs"][1]["trigger"] = json!({"kind":"startup"});
    next["settings"][0]["options"] = json!(["1080p"]);
    review(&mut app, serde_json::to_vec(&next).unwrap());
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Publisher changes to Someone else");
    text_rect(&output, "Tidy finished files changes when or how it runs");
    text_rect(&output, "Settings reset: Preferred quality");
    text_rect(
        &output,
        "Your other settings, keys and stored values are kept.",
    );
    text_rect(&output, "Approve and update");
}

#[test]
fn an_update_keeps_the_queue_chosen_when_it_was_installed() {
    let (_temp, mut app, ctx) = fixture();
    let with_queue = |version: &str| {
        let mut value: Value =
            serde_json::from_slice(&package(version, &["videohost.example"])).unwrap();
        value["jobs"][1]["trigger"] = json!({"kind":"queue_completed","queue":""});
        serde_json::to_vec(&value).unwrap()
    };
    let queue = app.store.create_queue("Chosen").unwrap().id;
    app.store
        .install_extension(
            &with_queue("1.2.0"),
            &Grants {
                queues: [("tidy".to_owned(), queue.clone())].into(),
                ..Default::default()
            },
        )
        .unwrap();
    open(&mut app);
    review(&mut app, with_queue("1.3.0"));
    click(&mut app, &ctx, "Update");
    assert_eq!(app.store.extension(ID).unwrap().package.version, "1.3.0");
    assert_eq!(
        app.store.extension_job(ID, "tidy").unwrap().trigger,
        Trigger::QueueCompleted { queue }
    );
}

#[test]
fn a_broken_file_never_opens_a_review() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    assert!(app.store.review_extension(b"not json").is_err());
    frame(&mut app, &ctx, size(), vec![]);
    assert!(app.dialog.is_none());
}

#[test]
fn settings_save_plain_values_and_seal_secrets() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Settings");
    app.extensions
        .values
        .insert("quality".into(), "720p".into());
    app.extensions
        .secrets
        .insert("api_token".into(), "private-token".into());
    click(&mut app, &ctx, "Save settings");
    let saved = app.store.extension_settings(ID).unwrap();
    assert_eq!(saved["quality"], json!("720p"));
    assert_eq!(saved["api_token"], Value::Null);
    assert!(app.extensions.secrets.values().all(String::is_empty));
}

#[test]
fn typing_in_settings_survives_a_background_refresh() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Settings");
    app.extensions
        .values
        .insert("quality".into(), "720p".into());
    app.extensions
        .secrets
        .insert("api_token".into(), "half-typed".into());
    app.refresh_automation();
    assert_eq!(app.extensions.values["quality"], "720p");
    assert_eq!(app.extensions.secrets["api_token"], "half-typed");
    assert!(app.extensions.detail == Detail::Settings);
}

#[test]
fn a_refused_setting_stays_on_screen_to_be_corrected() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Settings");
    app.extensions.values.insert("quality".into(), "4K".into());
    click(&mut app, &ctx, "Save settings");
    assert!(app.extensions.settings_error.is_some());
    assert_eq!(app.extensions.values["quality"], "4K");
    assert!(
        !app.store
            .extension_settings(ID)
            .unwrap()
            .contains_key("quality")
    );
}

#[test]
fn code_is_read_only_and_can_be_copied_to_a_script() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Code");
    click(&mut app, &ctx, "tidy.lua");
    click(&mut app, &ctx, "Copy to my scripts");
    let draft = app.scripts.draft().unwrap();
    assert_eq!(draft.name, "tidy.lua");
    assert_eq!(draft.body, "print('tidy')\n");
    assert!(draft.extension.is_none());
    assert_eq!(
        app.store.extension_job(ID, "tidy").unwrap().body,
        "print('tidy')\n"
    );
}

#[test]
fn copying_code_never_replaces_a_script_being_edited() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    app.scripts.new_script_from("Mine", "print('unsaved work')");
    click(&mut app, &ctx, "Code");
    click(&mut app, &ctx, "Copy to my scripts");
    assert_eq!(app.scripts.draft().unwrap().name, "Mine");
    assert_eq!(app.scripts.draft().unwrap().body, "print('unsaved work')");
    assert_eq!(
        app.extensions.error.as_deref(),
        Some("Save or discard your open script first")
    );
}

#[test]
fn removing_asks_first() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Remove");
    assert_eq!(app.dialog, Some(Dialog::ExtensionRemove));
    click(&mut app, &ctx, "Cancel");
    assert_eq!(app.store.extensions().unwrap().len(), 1);
    click(&mut app, &ctx, "Remove");
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Remove VideoHost?");
    app.store.remove_extension(ID).unwrap();
}

#[test]
fn a_conflict_is_explained_on_the_page() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    app.store.set_extension_enabled(ID, true).unwrap();
    app.store
        .save_script(&Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: "My reader".into(),
            language: download_engine::script_automation::Language::Lua,
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
    open(&mut app);
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Never runs: page conflict");
    text_rect(
        &output,
        "My reader handles the same pages first, so this job never runs.",
    );
}

#[test]
fn the_page_fits_the_smallest_window() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    let small = egui::vec2(860.0, 600.0);
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    let enabled = text_rect(&output, "Enabled");
    assert!(enabled.right() <= small.x, "{enabled:?}");
    let remove = text_rect(&output, "Remove");
    assert!(remove.right() <= small.x, "{remove:?}");
}

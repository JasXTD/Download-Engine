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

/// One automation job that asks for file access. Its Access list is in view in
/// the window the click helper uses, without scrolling.
fn tidy_only() -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(&package("1.2.0", &[])).unwrap();
    value["jobs"] = json!([value["jobs"][1].clone()]);
    value["settings"] = json!([]);
    value["files"] = json!({ "tidy.lua": "print('tidy')\n" });
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn the_page_opens_on_installed_and_keeps_scripts_and_rules_as_tabs() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "What do you want Download Engine to do?");
    click(&mut app, &ctx, "Scripts");
    let output = frame(&mut app, &ctx, size(), vec![]);
    assert!(!has_text(
        &output,
        "What do you want Download Engine to do?"
    ));
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
    text_rect(&output, "Reads videohost.example");
    text_rect(&output, "Runs when you open one of these pages:");
    text_rect(&output, "videohost.example/watch/*");
    text_rect(&output, "Runs when any download finishes.");
    text_rect(
        &output,
        "Let it rename, move and delete any downloaded file",
    );
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
    text_rect(&output, "Turned off");
    text_rect(&output, "Turned off. Nothing runs until you turn it on.");
    text_rect(&output, "What it does");
    text_rect(
        &output,
        "Never: your browser logins or cookies, or other programs.",
    );
    text_rect(&output, "Tidy finished files");
    text_rect(&output, "Not allowed");

    click(&mut app, &ctx, "Enabled");
    assert!(app.store.extension(ID).unwrap().enabled);
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    assert!(!has_text(
        &output,
        "Turned off. Nothing runs until you turn it on."
    ));
    text_rect(&output, "Turned on");
}

#[test]
fn turn_on_in_the_banner_turns_the_extension_on() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Turn on");
    assert!(app.store.extension(ID).unwrap().enabled);
}

#[test]
fn long_job_names_and_page_lists_stay_inside_a_small_window() {
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
    assert!(text_rect(&output, long).right() <= small.x);
    assert!(text_rect(&output, "videohost.example/watch/*").right() <= small.x);
    text_rect(&output, "Runs when you open one of these pages:");
    text_rect(&output, "2 jobs");
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
        "Let it rename, move and delete any downloaded file",
    );
    click(&mut app, &ctx, "Install");
    assert!(app.store.extension_job(ID, "tidy").unwrap().change_files);
}

#[test]
fn file_access_can_be_allowed_after_install_and_taken_back() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(&tidy_only(), &Grants::default())
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Allow…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionFileAccess));
    assert!(
        !app.store.extension_job(ID, "tidy").unwrap().change_files,
        "nothing changes until the person confirms"
    );
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Allow VideoHost to change files?");
    click(&mut app, &ctx, "Allow");
    assert!(app.dialog.is_none());
    assert!(app.store.extension_job(ID, "tidy").unwrap().change_files);

    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(
        &output,
        "Rename, move and delete any downloaded file. You allowed this.",
    );
    click(&mut app, &ctx, "Take back");
    assert!(!app.store.extension_job(ID, "tidy").unwrap().change_files);
}

#[test]
fn closing_the_file_access_question_grants_nothing() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(&tidy_only(), &Grants::default())
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Allow…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionFileAccess));
    let size = size();
    frame(&mut app, &ctx, size, vec![]);
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(&mut app, &ctx, size, vec![]);
    assert!(app.dialog.is_none());
    assert!(!app.store.extension_job(ID, "tidy").unwrap().change_files);
}

#[test]
fn the_empty_page_points_each_goal_to_the_right_place() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    click(&mut app, &ctx, "+ New rule");
    assert_eq!(app.dialog, Some(Dialog::Automation));
    assert!(app.automation.tab() == automation::Tab::Rules);
    app.dialog = None;

    app.automation.show_installed();
    click(&mut app, &ctx, "Write a script");
    assert!(app.automation.tab() == automation::Tab::Scripts);
    assert!(app.scripts.draft().is_some_and(|draft| draft.revision == 0));

    app.automation.show_installed();
    click(&mut app, &ctx, "See examples");
    assert_eq!(app.dialog, Some(Dialog::ExtensionExamples));
}

#[test]
fn an_example_opens_the_normal_review_and_installs_turned_off() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    click(&mut app, &ctx, "Examples…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionExamples));
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    for name in [
        "Internet Archive files",
        "Wikimedia Commons originals",
        "GitHub release files",
        "Sort finished files",
        "Queue finished",
        "Reads archive.org",
        "Asks to change files",
        "You pick the queue in the review",
    ] {
        text_rect(&output, name);
    }
    assert!(
        app.store.extensions().unwrap().is_empty(),
        "nothing installs from the list"
    );

    // With a dialog open, a click goes to the top-most match: the last
    // example's Review…, Queue finished.
    click(&mut app, &ctx, "Review…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionReview));
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Install Queue finished?");
    text_rect(&output, "Runs when the queue you choose finishes.");
    text_rect(&output, "Choose a queue");
    text_rect(&output, "Can't rename, move or delete them.");
    click(&mut app, &ctx, "Install");
    let installed = app.store.extension("examples.queue-finished").unwrap();
    assert!(!installed.enabled);
    assert_eq!(
        app.store
            .extension_job("examples.queue-finished", "notify")
            .unwrap()
            .trigger,
        Trigger::QueueCompleted {
            queue: MAIN_QUEUE.into()
        }
    );
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    let main = automation::queue_label(&app.queues, MAIN_QUEUE);
    text_rect(&output, &format!("Runs when {main} finishes."));

    click(&mut app, &ctx, "Examples…");
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Installed");
}

/// Reviewed as a new install that starts turned off. If another program installs
/// and turns on an older copy meanwhile, Install must not quietly update it.
#[test]
fn installing_after_the_extension_changed_elsewhere_asks_for_a_new_review() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    review(&mut app, package("1.3.0", &["videohost.example"]));
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    app.store.set_extension_enabled(ID, true).unwrap();
    click(&mut app, &ctx, "Install");
    assert_eq!(app.dialog, Some(Dialog::ExtensionReview));
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_containing(&output, "This extension changed after you reviewed it");
    assert_eq!(app.store.extension(ID).unwrap().package.version, "1.2.0");
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
    for label in [
        "Enabled",
        "Remove",
        "Turn on",
        "Examples…",
        "Install from file…",
    ] {
        let rect = text_rect(&output, label);
        assert!(rect.right() <= small.x, "{label}: {rect:?}");
    }
}

#[test]
fn german_labels_fit_the_smallest_window() {
    use download_engine::i18n::Language;
    let german = |text: &str| {
        let _scope = download_engine::i18n::scope(Language::German);
        tr(text).to_owned()
    };
    let fits = |output: &egui::FullOutput, size: egui::Vec2, labels: &[&str]| {
        for label in labels {
            let rect = text_rect(output, &german(label));
            assert!(
                rect.right() <= size.x && rect.bottom() <= size.y,
                "{label}: {rect:?}"
            );
        }
    };
    let small = egui::vec2(860.0, 600.0);

    // Nothing installed: the goal cards fit across, and their buttons too once
    // the window is tall enough not to scroll.
    let (_temp, mut app, ctx) = fixture();
    app.interface.language = Language::German;
    open(&mut app);
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    fits(
        &output,
        small,
        &[
            "Examples…",
            "Install from file…",
            "Get files from a website",
            "Do something after a download",
            "Send downloads to the right queue",
        ],
    );
    let taller = egui::vec2(860.0, 900.0);
    frame(&mut app, &ctx, taller, vec![]);
    let output = frame(&mut app, &ctx, taller, vec![]);
    fits(
        &output,
        taller,
        &["See examples", "Write a script", "+ New rule"],
    );

    // The Examples list.
    app.open_extension_examples();
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    fits(&output, small, &["Review…", "Internet Archive files"]);
    app.dialog = None;

    // An installed extension: the header, the banner and the Access list.
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    app.refresh_extensions();
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    fits(&output, small, &["Enabled", "Remove", "Turn on"]);
    let tall = egui::vec2(860.0, 1400.0);
    frame(&mut app, &ctx, tall, vec![]);
    let output = frame(&mut app, &ctx, tall, vec![]);
    fits(
        &output,
        tall,
        &["Allow…", "Not allowed", "Your files", "Your downloads"],
    );
}

#[test]
fn the_access_list_fits_the_smallest_window() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    open(&mut app);
    // Tall enough to reach the Access group without scrolling.
    let small = egui::vec2(860.0, 1400.0);
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    for label in ["Allow…", "Not allowed", "Your files", "Your downloads"] {
        let rect = text_rect(&output, label);
        assert!(rect.right() <= small.x, "{label}: {rect:?}");
    }
    // A chip squeezed next to wrapped text once stacked its letters one per line
    // and pushed the group past the panel.
    let chip = text_rect(&output, "Not allowed");
    assert!(chip.width() > chip.height() * 2.0, "{chip:?}");
    let allow = text_rect(&output, "Allow…");
    let panel = text_rect(&output, "Runs when any download finishes.");
    assert!(allow.right() < small.x - 24.0, "{allow:?} {panel:?}");
}

/// Draws the page, waits for any folder listing that started, and draws what it found.
fn settled(app: &mut Desktop, ctx: &egui::Context) -> egui::FullOutput {
    frame(app, ctx, size(), vec![]);
    app.wait_for_io();
    frame(app, ctx, size(), vec![]);
    frame(app, ctx, size(), vec![])
}
fn text_containing(output: &egui::FullOutput, part: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(part) => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("No rendered text containing: {part}"))
}
fn save_in_folder(app: &Desktop, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let folder = app.store.paths.extensions();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(name), bytes).unwrap();
    folder
}
const ONE_TO_REVIEW: &str = "Your extensions folder has 1 extension to review.";

#[test]
fn a_file_in_the_folder_is_only_listed_until_reviewed_and_installs_turned_off() {
    let (_temp, mut app, ctx) = fixture();
    save_in_folder(
        &app,
        "videohost.de-extension.json",
        &package("1.2.0", &["videohost.example"]),
    );
    open(&mut app);
    let output = settled(&mut app, &ctx);
    text_rect(&output, ONE_TO_REVIEW);
    assert!(
        app.store.extensions().unwrap().is_empty(),
        "nothing installs from the folder by itself"
    );

    click(&mut app, &ctx, "Review…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionFolder));
    let output = settled(&mut app, &ctx);
    text_rect(&output, "VideoHost");
    text_rect(&output, "1.2.0 · Example");
    text_rect(&output, "Asks to change files");
    text_rect(
        &output,
        "Nothing in this folder runs until you review it. A new extension installs turned off, and an update stays on or off as it was.",
    );
    // The top-most Review… is the row's, above the strip behind the list.
    click(&mut app, &ctx, "Review…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionReview));
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Install VideoHost?");
    text_rect(
        &output,
        "Let it rename, move and delete any downloaded file",
    );
    click(&mut app, &ctx, "Install");
    let installed = app.store.extension(ID).unwrap();
    assert!(!installed.enabled);
    assert!(
        !app.store.extension_job(ID, "tidy").unwrap().change_files,
        "file access stays off unless ticked"
    );

    let output = settled(&mut app, &ctx);
    assert!(!has_text(&output, ONE_TO_REVIEW));
    click(&mut app, &ctx, "Extensions folder…");
    let output = settled(&mut app, &ctx);
    text_rect(&output, "Installed");
    assert!(!has_text(&output, "Review…"));
}

#[test]
fn a_newer_file_in_the_folder_is_an_update_that_asks_for_new_access() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    save_in_folder(
        &app,
        "videohost-1.3.0.de-extension.json",
        &package("1.3.0", &["videohost.example", "api.vh-media.example"]),
    );
    open(&mut app);
    let output = settled(&mut app, &ctx);
    text_rect(&output, ONE_TO_REVIEW);
    click(&mut app, &ctx, "Extensions folder…");
    let output = settled(&mut app, &ctx);
    text_rect(&output, "You have 1.2.0. This file is newer.");
    click(&mut app, &ctx, "Review update…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionReview));
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Update VideoHost?");
    text_rect(&output, "Read api.vh-media.example");
    text_rect(&output, "Approve and update");
    assert_eq!(
        app.store.extension(ID).unwrap().package.version,
        "1.2.0",
        "nothing updates before the approval"
    );
    click(&mut app, &ctx, "Approve and update");
    assert_eq!(app.store.extension(ID).unwrap().package.version, "1.3.0");
    let output = settled(&mut app, &ctx);
    assert!(!has_text(&output, ONE_TO_REVIEW));
}

#[test]
fn a_changed_file_without_a_newer_version_cannot_update() {
    let (_temp, mut app, ctx) = fixture();
    let installed = package("1.2.0", &["videohost.example"]);
    app.store
        .install_extension(&installed, &Grants::default())
        .unwrap();
    let mut changed: Value = serde_json::from_slice(&installed).unwrap();
    changed["description"] = json!("Changed, but still 1.2.0.");
    save_in_folder(
        &app,
        "a-changed.de-extension.json",
        &serde_json::to_vec(&changed).unwrap(),
    );
    save_in_folder(
        &app,
        "b-older.de-extension.json",
        &package("1.1.0", &["videohost.example"]),
    );
    save_in_folder(&app, "c-same.de-extension.json", &installed);
    open(&mut app);
    let output = settled(&mut app, &ctx);
    assert!(!has_text(&output, ONE_TO_REVIEW));

    app.open_extension_folder();
    let output = settled(&mut app, &ctx);
    text_rect(
        &output,
        "Differs from the 1.2.0 you installed. Raise its version to update.",
    );
    text_rect(&output, "Older than the 1.2.0 you installed.");
    text_rect(&output, "Installed");
    assert!(!has_text(&output, "Review…"));
    assert!(!has_text(&output, "Review update…"));
}

#[test]
fn two_files_of_one_new_extension_count_once() {
    let (_temp, mut app, ctx) = fixture();
    save_in_folder(
        &app,
        "videohost-1.2.0.de-extension.json",
        &package("1.2.0", &["videohost.example"]),
    );
    save_in_folder(
        &app,
        "videohost-1.3.0.de-extension.json",
        &package("1.3.0", &["videohost.example"]),
    );
    open(&mut app);
    let output = settled(&mut app, &ctx);
    text_rect(&output, ONE_TO_REVIEW);
}

#[test]
fn a_file_removed_after_listing_says_why_it_cant_be_reviewed() {
    let (_temp, mut app, ctx) = fixture();
    let folder = save_in_folder(
        &app,
        "videohost.de-extension.json",
        &package("1.2.0", &["videohost.example"]),
    );
    open(&mut app);
    app.open_extension_folder();
    let output = settled(&mut app, &ctx);
    text_rect(&output, "VideoHost");
    std::fs::remove_file(folder.join("videohost.de-extension.json")).unwrap();

    click(&mut app, &ctx, "Review…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionFolder));
    // The listing that starts right after must not wipe the reason.
    let output = settled(&mut app, &ctx);
    text_containing(
        &output,
        "Can't read this file: videohost.de-extension.json:",
    );
    text_rect(&output, "No extension files in this folder yet.");
    assert!(app.store.extensions().unwrap().is_empty());
}

#[test]
fn broken_files_and_unpacked_folders_are_explained_and_never_reviewed() {
    let (_temp, mut app, ctx) = fixture();
    let folder = save_in_folder(&app, "broken.de-extension.json", b"{");
    std::fs::create_dir(folder.join("my-reader")).unwrap();
    std::fs::write(folder.join("my-reader").join("manifest.json"), b"{}").unwrap();
    std::fs::write(folder.join("notes.txt"), b"not an extension").unwrap();
    open(&mut app);
    let output = settled(&mut app, &ctx);
    assert!(!has_text(&output, ONE_TO_REVIEW));

    app.open_extension_folder();
    let output = settled(&mut app, &ctx);
    text_rect(&output, "broken.de-extension.json");
    text_containing(
        &output,
        "Can't read this file: This is not a valid extension file",
    );
    text_rect(&output, &format!("my-reader{}", std::path::MAIN_SEPARATOR));
    text_rect(
        &output,
        "Folders aren't read. Pack it into a file with de-cli extensions pack first.",
    );
    text_rect(&output, "Skipped 1 other file.");
    assert!(!has_text(&output, "Review…"));
}

#[test]
fn the_folder_list_creates_the_folder_and_scan_now_finds_a_new_file() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    click(&mut app, &ctx, "Extensions folder…");
    assert_eq!(app.dialog, Some(Dialog::ExtensionFolder));
    let output = settled(&mut app, &ctx);
    let folder = app.store.paths.extensions();
    assert!(folder.is_dir(), "listing creates the folder");
    text_rect(&output, &folder.display().to_string());
    text_rect(&output, "No extension files in this folder yet.");

    save_in_folder(
        &app,
        "videohost.de-extension.json",
        &package("1.2.0", &["videohost.example"]),
    );
    let output = settled(&mut app, &ctx);
    assert!(
        !has_text(&output, "VideoHost"),
        "nothing watches the folder"
    );
    click(&mut app, &ctx, "Scan now");
    let output = settled(&mut app, &ctx);
    text_rect(&output, "VideoHost");

    click(&mut app, &ctx, "Open folder");
    assert_eq!(
        app.extensions.folder.opened.as_deref(),
        Some(folder.as_path())
    );
    assert_eq!(
        app.dialog,
        Some(Dialog::ExtensionFolder),
        "the list stays open"
    );
}

#[test]
fn coming_back_to_the_installed_tab_looks_in_the_folder_again() {
    let (_temp, mut app, ctx) = fixture();
    open(&mut app);
    let output = settled(&mut app, &ctx);
    assert!(!has_text(&output, ONE_TO_REVIEW));
    save_in_folder(
        &app,
        "videohost.de-extension.json",
        &package("1.2.0", &["videohost.example"]),
    );
    let output = settled(&mut app, &ctx);
    assert!(
        !has_text(&output, ONE_TO_REVIEW),
        "nothing watches the folder"
    );

    app.view = View::Downloads;
    frame(&mut app, &ctx, size(), vec![]);
    app.view = View::Automation;
    let output = settled(&mut app, &ctx);
    text_rect(&output, ONE_TO_REVIEW);
}

#[test]
fn german_header_strip_and_folder_list_fit_the_smallest_window() {
    use download_engine::i18n::Language;
    let german = |text: &str| {
        let _scope = download_engine::i18n::scope(Language::German);
        tr(text).to_owned()
    };
    let small = egui::vec2(860.0, 600.0);
    let fits = |output: &egui::FullOutput, labels: &[&str]| {
        for label in labels {
            let rect = text_rect(output, &german(label));
            assert!(
                rect.right() <= small.x && rect.bottom() <= small.y,
                "{label}: {rect:?}"
            );
        }
    };
    let (_temp, mut app, ctx) = fixture();
    app.interface.language = Language::German;
    save_in_folder(
        &app,
        "videohost.de-extension.json",
        &package("1.2.0", &["videohost.example"]),
    );
    open(&mut app);
    frame(&mut app, &ctx, small, vec![]);
    app.wait_for_io();
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    fits(
        &output,
        &[
            "Examples…",
            "Extensions folder…",
            "Install from file…",
            ONE_TO_REVIEW,
            "Review…",
        ],
    );
    // Three buttons leave the title less room: its subtitle wraps instead of
    // running under them.
    let subtitle = text_rect(
        &output,
        &german("Install extensions, write your own scripts and route downloads with rules."),
    );
    let buttons = text_rect(&output, &german("Examples…"));
    assert!(
        subtitle.right() < buttons.left(),
        "{subtitle:?} {buttons:?}"
    );

    app.open_extension_folder();
    frame(&mut app, &ctx, small, vec![]);
    app.wait_for_io();
    frame(&mut app, &ctx, small, vec![]);
    let output = frame(&mut app, &ctx, small, vec![]);
    fits(
        &output,
        &["Open folder", "Scan now", "Your extensions folder"],
    );
}

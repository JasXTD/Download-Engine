use super::super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;
use serde_json::{Value, json};

const ID: &str = "example.videohost";

#[test]
fn plugin_account_actions_render_and_start_only_after_enable_and_stop_without_downloads() {
    let (_temp, mut app, ctx) = fixture();
    let mut local: Value =
        serde_json::from_slice(download_engine::extensions::examples::PLUGINS[0]).unwrap();
    local["id"] = json!("test.account-actions");
    local["files"]["main.lua"] = json!("xd.sleep(60000); return {items={}}");
    let bytes = serde_json::to_vec(&local).unwrap();
    let downloads_before = app.store.list().unwrap().len();
    let installed = app
        .store
        .install_extension(&bytes, &Grants::default())
        .unwrap();
    open(&mut app);
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Plugin actions");
    text_rect(&output, "Connect account");
    text_rect(&output, "Turn on this plugin to use its actions.");
    click(&mut app, &ctx, "Connect account");
    assert!(app.store.script_runs(None).unwrap().is_empty());
    app.store
        .set_extension_enabled(&installed.package.id, true)
        .unwrap();
    app.refresh_extensions();
    frame(&mut app, &ctx, size(), vec![]);
    click(&mut app, &ctx, "Connect account");
    let runs = app.store.script_runs(None).unwrap();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].active());
    frame(&mut app, &ctx, size(), vec![]);
    app.extension_action(Act::StopAction(runs[0].id.clone()));
    for _ in 0..40 {
        if !app.store.script_run(&runs[0].id).unwrap().active() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!app.store.script_run(&runs[0].id).unwrap().active());
    assert_eq!(app.store.list().unwrap().len(), downloads_before);
}

#[test]
fn plugin_action_buttons_fit_the_shared_panel_at_two_window_sizes() {
    for dimensions in [egui::vec2(1280.0, 860.0), egui::vec2(1000.0, 760.0)] {
        let (_temp, mut app, ctx) = fixture();
        app.store
            .install_extension(
                download_engine::extensions::examples::PLUGINS[1],
                &Grants::default(),
            )
            .unwrap();
        open(&mut app);
        frame(&mut app, &ctx, dimensions, vec![]);
        let output = frame(&mut app, &ctx, dimensions, vec![]);
        let connect = text_rect(&output, "Connect account");
        let check = text_rect(&output, "Check account");
        let disconnect = text_rect(&output, "Disconnect");
        assert!(!connect.intersects(check) && !check.intersects(disconnect));
        assert!(disconnect.right() <= dimensions.x);
    }
}

#[test]
fn plugin_result_opens_selection_without_starting_a_download() {
    // The CLI integration suite executes the worker. Seed its completed receipt
    // before this layout fixture starts its scheduler; test binaries do not have
    // the installed de-cli sibling that the production scheduler launches.
    let (_temp, mut app, ctx) = super::super::interaction_tests::fixture_with_store_setup(
        |store| {
            let package = store
                .install_extension(
                    download_engine::extensions::examples::PLUGINS[1],
                    &Grants::default(),
                )
                .unwrap();
            store
                .set_extension_enabled(&package.package.id, true)
                .unwrap();
            let job = store
                .extension_job(&package.package.id, "generate")
                .unwrap();
            let mut run = store
                .run_site_reader(
                    &job.id,
                    "https://source.example/file",
                    &Uuid::new_v4().to_string(),
                )
                .unwrap();
            run.state = "succeeded".into();
            let db = super::super::fixture_database::open(store.paths.data.join("queue.sqlite3"))
                .unwrap();
            db.execute(
                "UPDATE script_runs SET state='succeeded',data=?2 WHERE id=?1",
                rusqlite::params![run.id, serde_json::to_string(&run).unwrap()],
            )
            .unwrap();
            let reading = json!({"title":"Generated fixture","single_connection":true,"items":[{"kind":"file","url":"https://cdn.real-debrid.com/fixture.bin","filename":"fixture.bin","size":32}]});
            db.execute(
                "INSERT INTO script_results(run_id,data) VALUES(?1,?2)",
                rusqlite::params![run.id, reading.to_string()],
            )
            .unwrap();
        },
    );
    open(&mut app);
    let before = app.store.list().unwrap().len();
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Review generated links");
    click(&mut app, &ctx, "Review generated links");
    assert_eq!(app.dialog, Some(Dialog::BrowserVideo));
    let pending = app.store.next_browser_video_review().unwrap().unwrap();
    assert!(pending.review.single_connection);
    assert_eq!(pending.review.items.len(), 1);
    assert_eq!(app.store.list().unwrap().len(), before);
}

pub(super) fn package(version: &str, hosts: &[&str]) -> Vec<u8> {
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
    let overview = text_rect(&output, "What it does");
    text_rect(&output, "Tidy finished files");
    // The plugins panel leaves less vertical space. Reach the lower access
    // details through the real scroll area instead of assuming they are in view.
    frame(
        &mut app,
        &ctx,
        size(),
        vec![
            egui::Event::PointerMoved(overview.center()),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, -500.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(
        &output,
        "Never: your browser logins or cookies, or other programs.",
    );
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
fn session_permission_distinguishes_supplied_credentials_from_browser_imports() {
    let mut data: Value =
        serde_json::from_slice(&package("1.2.0", &["videohost.example"])).unwrap();
    let supplied = "Use login cookies and headers you supply, on the declared hosts";
    let imported = "Import logins or cookies from your browser";
    for permitted in [false, true] {
        data["permissions"]["http_sessions"] = json!(permitted);
        let package = Package::decode(&serde_json::to_vec(&data).unwrap()).unwrap();
        let can = access::web(&package);
        assert_eq!(can.iter().any(|text| text == supplied), permitted);
        assert!(!can.iter().any(|text| text == imported));
    }
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
    frame(&mut app, &ctx, size(), vec![]);
    let output = frame(&mut app, &ctx, size(), vec![]);
    text_rect(&output, "Install");
    assert!(
        !has_text(&output, "Cancel"),
        "The header × already closes it"
    );
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert!(app.store.extensions().unwrap().is_empty());
}

#[test]
fn an_update_shows_new_access_and_closing_keeps_the_old_version() {
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
    text_rect(
        &output,
        "Closing this keeps the installed version exactly as it is.",
    );
    assert!(
        !has_text(&output, "Not now"),
        "The header × already closes it"
    );
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
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
    click(&mut app, &ctx, "Approve and update");
    assert_eq!(app.extensions.values["quality"], "1080p");
    assert!(!app.extensions.settings_dirty());
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
fn review_regression_empty_setting_is_saved_and_failed_save_changes_nothing() {
    let (_temp, mut app, ctx) = fixture();
    let mut value: Value =
        serde_json::from_slice(&package("1.2.0", &["videohost.example"])).unwrap();
    value["settings"] = json!([
        {"id":"next_queue","type":"string","label":"Next queue","default":"","max_length":64},
        {"id":"count","type":"integer","label":"Count","default":1,"min":1,"max":10}
    ]);
    app.store
        .install_extension(&serde_json::to_vec(&value).unwrap(), &Grants::default())
        .unwrap();
    app.store
        .set_extension_setting(ID, "next_queue", &json!("Morning"))
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Settings");
    app.extensions
        .values
        .insert("next_queue".into(), String::new());
    click(&mut app, &ctx, "Save settings");
    assert_eq!(
        app.store.extension_settings(ID).unwrap()["next_queue"],
        json!("")
    );
    app.extensions
        .values
        .insert("next_queue".into(), "Evening".into());
    app.extensions
        .values
        .insert("count".into(), "not a number".into());
    click(&mut app, &ctx, "Save settings");
    assert!(app.extensions.settings_error.is_some());
    assert_eq!(
        app.store.extension_settings(ID).unwrap()["next_queue"],
        json!("")
    );
    assert_eq!(app.extensions.values["next_queue"], "Evening");
}

#[test]
fn review_regression_switching_extensions_keeps_unsaved_settings_and_errors() {
    let (_temp, mut app, ctx) = fixture();
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    let mut other: Value =
        serde_json::from_slice(&package("1.2.0", &["videohost.example"])).unwrap();
    other["id"] = json!("example.other");
    other["name"] = json!("Other extension");
    app.store
        .install_extension(&serde_json::to_vec(&other).unwrap(), &Grants::default())
        .unwrap();
    open(&mut app);
    click(&mut app, &ctx, "Settings");
    app.extensions.values.insert("quality".into(), "4K".into());
    app.extensions
        .secrets
        .insert("api_token".into(), "unsaved-test-token".into());
    click(&mut app, &ctx, "Save settings");
    let error = app.extensions.settings_error.clone();
    assert!(error.is_some());
    click(&mut app, &ctx, "Other extension");
    assert!(app.extensions.secrets.is_empty());
    app.extension_action(Act::Select(ID.into()));
    assert_eq!(app.extensions.values["quality"], "4K");
    assert_eq!(app.extensions.secrets["api_token"], "unsaved-test-token");
    assert_eq!(app.extensions.settings_error, error);
    app.extension_action(Act::Select(ID.into()));
    assert_eq!(app.extensions.values["quality"], "4K");
    app.extensions
        .values
        .insert("quality".into(), "720p".into());
    app.extension_action(Act::SaveSettings(ID.into()));
    app.extension_action(Act::Select("example.other".into()));
    app.extension_action(Act::Select(ID.into()));
    assert_eq!(app.extensions.values["quality"], "720p");
    assert!(app.extensions.secrets.is_empty());
    assert!(app.extensions.settings_error.is_none());
}

#[test]
fn review_regression_settings_save_stays_visible_at_minimum_size() {
    for language in [
        download_engine::i18n::Language::English,
        download_engine::i18n::Language::German,
    ] {
        for bundled in [false, true] {
            let (_temp, mut app, ctx) = fixture();
            let bytes = if bundled {
                include_bytes!("../../../examples/extensions/queue-finished.de-extension.json")
                    .to_vec()
            } else {
                package("1.2.0", &["videohost.example"])
            };
            let first_field = Package::decode(&bytes).unwrap().settings[0]
                .label()
                .to_owned();
            let fields_end_above = |output: &egui::FullOutput, bottom: f32| {
                let field = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == first_field)).unwrap();
                assert!(
                    field.clip_rect.intersects(text_rect(output, &first_field)),
                    "The setting must be reachable by scrolling"
                );
                assert!(
                    field.clip_rect.bottom() <= bottom,
                    "Settings paint over the fixed footer: {:?}, footer starts at {bottom}",
                    field.clip_rect
                );
            };
            let mut grants = Grants::default();
            if bundled {
                grants.queues.insert("notify".into(), MAIN_QUEUE.into());
            }
            app.store.install_extension(&bytes, &grants).unwrap();
            app.interface.language = language;
            open(&mut app);
            app.extensions.detail = Detail::Settings;
            let small = egui::vec2(860.0, 600.0);
            let scroll_form = |app: &mut Desktop, save: egui::Rect| {
                frame(
                    app,
                    &ctx,
                    small,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(save.center().x, save.top() - 80.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            phase: egui::TouchPhase::Move,
                            delta: egui::vec2(0.0, -1000.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                for _ in 0..8 {
                    frame(app, &ctx, small, vec![]);
                }
                frame(app, &ctx, small, vec![])
            };
            for _ in 0..8 {
                frame(&mut app, &ctx, small, vec![]);
            }
            let output = frame(&mut app, &ctx, small, vec![]);
            let _scope = download_engine::i18n::scope(language);
            let label = tr("Save settings");
            let rect = text_rect(&output, label);
            let clipped = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == label)).unwrap();
            assert!(
                clipped.clip_rect.contains_rect(rect),
                "Save is clipped: {rect:?}"
            );
            assert!(
                rect.bottom() < 570.0,
                "Save is under the status bar: {rect:?}"
            );
            let output = scroll_form(&mut app, rect);
            assert_eq!(
                text_rect(&output, label),
                rect,
                "Save must stay fixed while scrolling"
            );
            fields_end_above(&output, rect.top());
            app.extensions.settings_error =
                Some("Correct the invalid setting and try saving again.".into());
            app.extensions.values.insert(
                if bundled { "next_queue" } else { "quality" }.into(),
                "720p".into(),
            );
            for _ in 0..8 {
                frame(&mut app, &ctx, small, vec![]);
            }
            let output = frame(&mut app, &ctx, small, vec![]);
            let save = text_rect(&output, label);
            let error = text_rect(&output, "Correct the invalid setting and try saving again.");
            let clip = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.trim() == label)).unwrap().clip_rect;
            assert!(clip.contains_rect(save) && save.bottom() < 570.0);
            assert!(error.bottom() < save.top());
            let output = scroll_form(&mut app, save);
            assert_eq!(text_rect(&output, label), save);
            fields_end_above(&output, error.top());
        }
    }
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

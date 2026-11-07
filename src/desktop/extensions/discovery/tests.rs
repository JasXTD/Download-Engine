use super::super::super::interaction_tests::{click, fixture, frame, text_rect};
use super::super::tests::package;
use super::*;

fn load(app: &mut Desktop, ctx: &egui::Context, root: PathBuf) {
    app.extensions.discovery = State {
        directory: Some(root),
        shown: true,
        ..State::default()
    };
    app.view = View::Automation;
    app.refresh_automation();
    app.start_plugin_discovery();
    wait(app, ctx);
}

fn wait(app: &mut Desktop, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.extensions.discovery.task.is_some() {
        assert!(Instant::now() < deadline, "Discovery did not finish");
        app.poll_plugin_discovery(ctx);
        std::thread::yield_now();
    }
}

fn render(app: &mut Desktop, ctx: &egui::Context) -> egui::FullOutput {
    let size = vec2(1280.0, 860.0);
    frame(app, ctx, size, vec![]);
    frame(app, ctx, size, vec![])
}

fn escape(app: &mut Desktop, ctx: &egui::Context) {
    frame(
        app,
        ctx,
        vec2(1280.0, 860.0),
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
}

#[test]
fn discovered_install_is_reviewed_and_disabled_and_uses_the_reviewed_snapshot() {
    let (temp, mut app, ctx) = fixture();
    let root = temp.path().join("app/plugins");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("sample.de-extension.json");
    let bytes = package("1.2.0", &["videohost.example"]);
    std::fs::write(&path, &bytes).unwrap();
    load(&mut app, &ctx, root);
    text_rect(&render(&mut app, &ctx), "Review install");
    assert!(app.store.extensions().unwrap().is_empty());
    assert!(app.store.script_runs(None).unwrap().is_empty());
    click(&mut app, &ctx, "Review install");
    text_rect(&render(&mut app, &ctx), "Reads videohost.example");
    escape(&mut app, &ctx);
    assert!(app.dialog.is_none());
    assert!(app.store.extensions().unwrap().is_empty());
    click(&mut app, &ctx, "Review install");
    render(&mut app, &ctx);
    std::fs::write(&path, package("9.0.0", &["changed.example"])).unwrap();
    app.start_plugin_discovery();
    wait(&mut app, &ctx);
    click(&mut app, &ctx, "Install");
    let saved = app.store.extension("example.videohost").unwrap();
    assert_eq!(saved.package, Package::decode(&bytes).unwrap());
    assert!(!saved.enabled);
    assert!(
        !app.store
            .extension_job("example.videohost", "tidy")
            .unwrap()
            .change_files
    );
    assert!(app.store.script_runs(None).unwrap().is_empty());
}

#[test]
fn discovered_update_needs_review_and_removing_source_keeps_installed_copy() {
    let (temp, mut app, ctx) = fixture();
    let id = "example.videohost";
    app.store
        .install_extension(
            &package("1.2.0", &["videohost.example"]),
            &Grants::default(),
        )
        .unwrap();
    let root = temp.path().join("app/plugins");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("sample.de-extension.json");
    std::fs::write(
        &path,
        package("1.3.0", &["videohost.example", "cdn.example"]),
    )
    .unwrap();
    load(&mut app, &ctx, root);
    text_rect(&render(&mut app, &ctx), "Review update");
    assert_eq!(app.store.extension(id).unwrap().package.version, "1.2.0");
    click(&mut app, &ctx, "Review update");
    text_rect(&render(&mut app, &ctx), "Approve and update");
    escape(&mut app, &ctx);
    assert_eq!(app.store.extension(id).unwrap().package.version, "1.2.0");
    click(&mut app, &ctx, "Review update");
    render(&mut app, &ctx);
    click(&mut app, &ctx, "Approve and update");
    assert_eq!(app.store.extension(id).unwrap().package.version, "1.3.0");
    std::fs::remove_file(path).unwrap();
    click(&mut app, &ctx, "Refresh");
    wait(&mut app, &ctx);
    assert!(
        app.extensions
            .discovery
            .report
            .as_ref()
            .unwrap()
            .candidates
            .is_empty()
    );
    assert_eq!(app.store.extension(id).unwrap().package.version, "1.3.0");
}

#[test]
fn discovery_errors_are_visible_refresh_recovers_and_scan_is_deduplicated() {
    let (temp, mut app, ctx) = fixture();
    let root = temp.path().join("app/plugins");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("broken.de-extension.json");
    std::fs::write(&path, b"broken").unwrap();
    load(&mut app, &ctx, root);
    assert_eq!(
        app.extensions
            .discovery
            .report
            .as_ref()
            .unwrap()
            .issues
            .len(),
        1
    );
    let output = render(&mut app, &ctx);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains("broken.de-extension.json"))));
    std::fs::write(path, package("1.2.0", &["videohost.example"])).unwrap();
    click(&mut app, &ctx, "Refresh");
    // Calling again must keep the same inflight result, never launch another scan.
    let task = app.extensions.discovery.task.as_ref().unwrap() as *const _;
    app.start_plugin_discovery();
    assert_eq!(
        app.extensions.discovery.task.as_ref().unwrap() as *const _,
        task
    );
    wait(&mut app, &ctx);
    assert!(
        app.extensions
            .discovery
            .report
            .as_ref()
            .unwrap()
            .issues
            .is_empty()
    );
    text_rect(&render(&mut app, &ctx), "Review install");
    // At the smaller supported window size, review and refresh remain reachable.
    let size = vec2(940.0, 680.0);
    frame(&mut app, &ctx, size, vec![]);
    let output = frame(&mut app, &ctx, size, vec![]);
    for label in ["Review install", "Refresh", "Open folder"] {
        let rect = text_rect(&output, label);
        assert!(
            rect.min.x >= 0.0 && rect.max.x <= size.x && rect.min.y >= 0.0 && rect.max.y <= size.y,
            "{label}: {rect:?}"
        );
    }
}

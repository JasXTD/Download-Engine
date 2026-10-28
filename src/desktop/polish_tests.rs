use super::interaction_tests::{click, fixture, frame, text_rect};
use super::*;

const SIZE: egui::Vec2 = egui::vec2(1080.0, 800.0);
const CTRL: egui::Modifiers = egui::Modifiers {
    ctrl: true,
    command: true,
    ..egui::Modifiers::NONE
};

fn key(app: &mut Desktop, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            SIZE,
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            }],
        );
    }
}
fn select(app: &mut Desktop, ctx: &egui::Context, label: &str, modifiers: egui::Modifiers) {
    frame(app, ctx, SIZE, vec![]);
    let output = frame(app, ctx, SIZE, vec![]);
    // The details panel repeats the selected filename; click the actual table row.
    let pos = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.job.text == label && t.pos.y < 500.0 => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("Visible filename in the download table");
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            SIZE,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                },
            ],
        );
    }
}
fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "Scheduler did not acknowledge the action"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn persist(app: &mut Desktop) {
    // Keep action tests deterministic even when Resume wakes an idle Main queue.
    let now = chrono::Utc::now().timestamp();
    app.store
        .schedule_queue(MAIN_QUEUE, now + 3600, now)
        .unwrap();
    for job in &app.jobs {
        app.store.enqueue(job).unwrap();
    }
    app.refresh_and_wait();
}

#[test]
fn modifier_clicks_toggle_ranges_and_clear_selection_without_stale_details() {
    let (_root, mut app, ctx) = fixture();
    select(&mut app, &ctx, "first.zip", egui::Modifiers::NONE);
    select(&mut app, &ctx, "failed.zip", CTRL);
    assert_eq!(app.selected_jobs().len(), 2);
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    text_rect(&output, "2 selected");
    select(&mut app, &ctx, "first.zip", CTRL);
    assert_eq!(app.selected_jobs().len(), 1);
    select(&mut app, &ctx, "failed.zip", CTRL);
    app.reconcile_selection();
    assert!(app.selected_jobs().is_empty());
    assert!(app.selected.is_none());
    select(&mut app, &ctx, "first.zip", egui::Modifiers::NONE);
    select(&mut app, &ctx, "failed.zip", egui::Modifiers::SHIFT);
    assert_eq!(app.selected_jobs().len(), 3);
    select(&mut app, &ctx, "guide.pdf", egui::Modifiers::SHIFT);
    assert_eq!(app.selected_jobs().len(), 2);
    app.search = "guide".into();
    app.reconcile_selection();
    assert_eq!(app.selected_jobs().len(), 1);
    app.search.clear();
    app.reconcile_selection();
    assert_eq!(
        app.selected_jobs().len(),
        1,
        "Hidden selections must not return"
    );
    app.set_selection(None);
    app.jobs.clear();
    app.reconcile_selection();
    assert!(app.selected_jobs().is_empty());
}

#[test]
fn keyboard_ranges_select_all_and_search_focus_do_not_trigger_download_actions() {
    let (_root, mut app, ctx) = fixture();
    select(&mut app, &ctx, "first.zip", egui::Modifiers::NONE);
    key(&mut app, &ctx, egui::Key::ArrowDown, egui::Modifiers::SHIFT);
    assert_eq!(app.selected_jobs().len(), 2);
    assert_eq!(app.selected.as_deref(), Some(app.jobs[1].id.as_str()));
    key(&mut app, &ctx, egui::Key::End, egui::Modifiers::SHIFT);
    assert_eq!(app.selected_jobs().len(), 3);
    key(&mut app, &ctx, egui::Key::Home, egui::Modifiers::NONE);
    assert_eq!(app.selected_jobs().len(), 1);
    key(&mut app, &ctx, egui::Key::A, CTRL);
    assert_eq!(app.selected_jobs().len(), 3);
    key(&mut app, &ctx, egui::Key::F, CTRL);
    frame(&mut app, &ctx, SIZE, vec![egui::Event::Text("zip".into())]);
    let before = app.selected_jobs().len();
    key(&mut app, &ctx, egui::Key::A, CTRL);
    assert_eq!(app.selected_jobs().len(), before);
    key(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(app.search.is_empty());
    assert!(app.dialog.is_none());
    app.open_add();
    key(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(app.dialog, Some(Dialog::Add));
}

#[test]
fn keyboard_end_scrolls_virtualized_rows_into_view() {
    let (_root, mut app, ctx) = fixture();
    let sample = app.jobs[0].clone();
    app.jobs = (0..90)
        .map(|i| {
            let mut j = sample.clone();
            j.id = Uuid::new_v4().to_string();
            j.name = format!("download-{i:03}.zip");
            j
        })
        .collect();
    app.set_selection(Some(app.jobs[0].id.clone()));
    select(&mut app, &ctx, "download-000.zip", egui::Modifiers::NONE);
    key(&mut app, &ctx, egui::Key::End, egui::Modifiers::NONE);
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    let visible_rows: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::epaint::Shape::Text(t) if t.galley.job.text == "download-089.zip" => Some((
                s.clip_rect,
                egui::Rect::from_min_size(t.pos, t.galley.size()),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(app.selected.as_deref(), Some(app.jobs[89].id.as_str()));
    let details_top = text_rect(&output, "File actions").top();
    assert!(
        visible_rows
            .iter()
            .any(|(clip, rect)| rect.top() < details_top && clip.contains_rect(*rect)),
        "Focused row must be rendered inside the table: {visible_rows:?}"
    );
    key(&mut app, &ctx, egui::Key::Home, egui::Modifiers::NONE);
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t)
        if t.galley.job.text == "download-000.zip" && t.pos.y < 400.0))
    );
}

#[test]
fn bulk_toolbar_pause_and_resume_apply_to_all_eligible_selected_jobs() {
    let (_root, mut app, ctx) = fixture();
    persist(&mut app);
    select(&mut app, &ctx, "first.zip", egui::Modifiers::NONE);
    select(&mut app, &ctx, "failed.zip", CTRL);
    click(&mut app, &ctx, "Resume");
    wait_for(|| {
        app.store
            .queue_jobs(MAIN_QUEUE)
            .unwrap()
            .iter()
            .filter(|j| j.status == Status::Queued)
            .count()
            == 2
    });
    app.refresh_and_wait();
    assert_eq!(app.selected_jobs().len(), 2);
    click(&mut app, &ctx, "Pause");
    wait_for(|| {
        app.store
            .queue_jobs(MAIN_QUEUE)
            .unwrap()
            .iter()
            .filter(|j| j.status == Status::Paused)
            .count()
            == 2
    });
    assert_eq!(
        app.store
            .get(&app.jobs.iter().find(|j| j.name == "guide.pdf").unwrap().id)
            .unwrap()
            .unwrap()
            .status,
        Status::Complete
    );
}

#[test]
fn removing_partial_downloads_requires_one_confirmation_and_cancel_preserves_bytes() {
    let (_root, mut app, ctx) = fixture();
    persist(&mut app);
    let job = app
        .jobs
        .iter()
        .find(|j| j.name == "first.zip")
        .unwrap()
        .clone();
    let parts = app.store.paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&parts).unwrap();
    let partial = parts.join("0.part");
    std::fs::write(&partial, b"partial bytes").unwrap();
    select(&mut app, &ctx, "first.zip", egui::Modifiers::NONE);
    click(&mut app, &ctx, "Remove");
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    assert!(app.store.get(&job.id).unwrap().is_some());
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none() && app.confirmation.is_none());
    assert_eq!(std::fs::read(&partial).unwrap(), b"partial bytes");
    select(&mut app, &ctx, "failed.zip", CTRL);
    click(&mut app, &ctx, "Remove");
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    click(&mut app, &ctx, "Remove and discard progress");
    wait_for(|| app.store.list().unwrap().len() == 1);
    assert!(!partial.exists());
    assert_eq!(app.store.list().unwrap()[0].name, "guide.pdf");
}

#[test]
fn restart_checks_disk_data_even_with_zero_recorded_progress_and_revalidates_state() {
    let (_root, mut app, ctx) = fixture();
    persist(&mut app);
    let mut job = app
        .jobs
        .iter()
        .find(|j| j.name == "first.zip")
        .unwrap()
        .clone();
    job.downloaded = 0;
    app.store.save(&job).unwrap();
    let parts = app.store.paths.parts(&job.id).unwrap();
    std::fs::create_dir_all(&parts).unwrap();
    std::fs::write(
        parts.join("0.part"),
        b"saved despite stale progress counter",
    )
    .unwrap();
    app.command(Command::Restart(job.id.clone()));
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    job.status = Status::Downloading;
    app.store.save(&job).unwrap();
    click(&mut app, &ctx, "Restart and discard progress");
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    assert!(app.form_error.as_ref().unwrap().contains("Pause"));
    assert!(parts.join("0.part").exists());
    job.status = Status::Paused;
    app.store.save(&job).unwrap();
    click(&mut app, &ctx, "Restart and discard progress");
    wait_for(|| app.store.get(&job.id).unwrap().unwrap().status == Status::Queued);
    assert!(!parts.exists());
}

#[cfg(windows)]
#[test]
fn completed_file_menu_confirms_deletion_and_cancel_keeps_file_and_history() {
    let (root, mut app, ctx) = fixture();
    persist(&mut app);
    let mut job = app
        .jobs
        .iter()
        .find(|j| j.status == Status::Complete)
        .unwrap()
        .clone();
    let saved = root.path().join("delete me.pdf");
    std::fs::write(&saved, b"completed bytes").unwrap();
    job.saved_path = Some(saved.clone());
    app.store.save(&job).unwrap();
    app.refresh_and_wait();
    select(&mut app, &ctx, "guide.pdf", egui::Modifiers::NONE);
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Delete file…");
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    text_rect(&output, "Delete file?");
    text_rect(&output, &saved.to_string_lossy());
    assert!(app.store.get(&job.id).unwrap().is_some());
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert_eq!(std::fs::read(&saved).unwrap(), b"completed bytes");
    app.command(Command::DeleteFile {
        id: job.id.clone(),
        path: saved.clone(),
    });
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    click(&mut app, &ctx, "×");
    assert_eq!(std::fs::read(&saved).unwrap(), b"completed bytes");
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Delete file…");
    click(&mut app, &ctx, "Delete file");
    wait_for(|| app.store.get(&job.id).unwrap().is_none());
    assert!(!saved.exists());
    assert_eq!(app.store.list().unwrap().len(), 2);
}

#[test]
fn delete_file_confirmation_rechecks_the_saved_path() {
    let (root, mut app, ctx) = fixture();
    persist(&mut app);
    let mut job = app
        .jobs
        .iter()
        .find(|j| j.status == Status::Complete)
        .unwrap()
        .clone();
    let original = root.path().join("original.pdf");
    let replacement = root.path().join("replacement.pdf");
    std::fs::write(&original, b"original").unwrap();
    std::fs::write(&replacement, b"replacement").unwrap();
    job.saved_path = Some(original.clone());
    app.store.save(&job).unwrap();
    app.refresh_and_wait();
    select(&mut app, &ctx, "guide.pdf", egui::Modifiers::NONE);
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Delete file…");
    job.saved_path = Some(replacement.clone());
    app.store.save(&job).unwrap();
    click(&mut app, &ctx, "Delete file");
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    assert!(
        app.form_error
            .as_ref()
            .unwrap()
            .contains("location changed")
    );
    assert_eq!(std::fs::read(original).unwrap(), b"original");
    assert_eq!(std::fs::read(replacement).unwrap(), b"replacement");
    assert!(app.store.get(&job.id).unwrap().is_some());
}

#[cfg(windows)]
#[test]
fn delete_file_failure_is_visible_and_keeps_the_history_entry() {
    let (root, mut app, ctx) = fixture();
    persist(&mut app);
    let mut job = app
        .jobs
        .iter()
        .find(|j| j.status == Status::Complete)
        .unwrap()
        .clone();
    job.saved_path = Some(root.path().join("missing.pdf"));
    app.store.save(&job).unwrap();
    app.refresh_and_wait();
    select(&mut app, &ctx, "guide.pdf", egui::Modifiers::NONE);
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Delete file…");
    click(&mut app, &ctx, "Delete file");
    wait_for(|| {
        app.refresh_and_wait();
        app.notice
            .as_ref()
            .is_some_and(|notice| notice.contains("Cannot delete guide.pdf"))
    });
    assert_eq!(
        app.notice.as_ref().unwrap().kind,
        interactions::NoticeKind::Error
    );
    assert!(app.store.get(&job.id).unwrap().is_some());
}

#[cfg(windows)]
#[test]
fn delete_files_selection_requires_confirmation_and_escape_preserves_every_file() {
    let (root, mut app, ctx) = fixture();
    persist(&mut app);
    let mut jobs: Vec<_> = app.jobs.iter().take(2).cloned().collect();
    for job in &mut jobs {
        job.status = Status::Complete;
        let path = root.path().join(&job.name);
        std::fs::write(&path, b"keep until confirmed").unwrap();
        job.saved_path = Some(path);
        app.store.save(job).unwrap();
    }
    app.refresh_and_wait();
    select(&mut app, &ctx, &jobs[0].name, egui::Modifiers::NONE);
    select(&mut app, &ctx, &jobs[1].name, CTRL);
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Delete files…");
    assert_eq!(app.dialog, Some(Dialog::Confirm));
    key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(app.dialog.is_none());
    for job in &jobs {
        assert_eq!(
            std::fs::read(job.saved_path.as_ref().unwrap()).unwrap(),
            b"keep until confirmed"
        );
        assert!(app.store.get(&job.id).unwrap().is_some());
    }
    click(&mut app, &ctx, "File actions");
    click(&mut app, &ctx, "Delete files…");
    click(&mut app, &ctx, "Delete files");
    wait_for(|| app.store.list().unwrap().len() == 1);
    for job in &jobs {
        assert!(!job.saved_path.as_ref().unwrap().exists());
    }
}

#[test]
fn empty_downloads_and_completed_history_do_not_need_confirmation() {
    let (root, mut app, _ctx) = fixture();
    persist(&mut app);
    let mut job = app
        .jobs
        .iter()
        .find(|j| j.name == "first.zip")
        .unwrap()
        .clone();
    job.downloaded = 0;
    app.store.save(&job).unwrap();
    app.command(Command::Remove(job.id.clone()));
    assert!(app.dialog.is_none());
    wait_for(|| app.store.get(&job.id).unwrap().is_none());
    let mut complete = app
        .jobs
        .iter()
        .find(|j| j.name == "guide.pdf")
        .unwrap()
        .clone();
    let saved = root.path().join("keep.pdf");
    std::fs::write(&saved, b"keep me").unwrap();
    complete.saved_path = Some(saved.clone());
    app.store.save(&complete).unwrap();
    app.command(Command::Remove(complete.id.clone()));
    assert!(app.dialog.is_none());
    wait_for(|| app.store.get(&complete.id).unwrap().is_none());
    assert_eq!(std::fs::read(saved).unwrap(), b"keep me");
}

#[test]
fn folder_browse_select_cancel_and_failure_preserve_the_add_draft() {
    let (root, mut app, ctx) = fixture();
    app.open_add();
    app.url = "https://example.invalid/file.zip".into();
    app.folder = "unchanged".into();
    app.folder_picker.response = Some(Ok(None));
    click(&mut app, &ctx, "Browse…");
    assert_eq!(app.folder, "unchanged");
    assert_eq!(app.dialog, Some(Dialog::Add));
    app.folder_picker.response = Some(Err(anyhow::anyhow!("test picker error")));
    click(&mut app, &ctx, "Browse…");
    assert!(
        app.form_error
            .as_ref()
            .unwrap()
            .contains("test picker error")
    );
    assert_eq!(app.folder, "unchanged");
    let picked = root
        .path()
        .join("下載 résumé")
        .to_string_lossy()
        .into_owned();
    app.folder_picker.response = Some(Ok(Some(picked.clone())));
    click(&mut app, &ctx, "Browse…");
    assert_eq!(app.folder, picked);
    assert!(app.form_error.is_none());
    assert!(app.store.list().unwrap().is_empty());
    click(&mut app, &ctx, "×");
    assert_eq!(app.folder, picked);
}

#[test]
fn dialog_close_buttons_cancel_drafts_and_pending_save_cannot_be_dismissed() {
    let (_root, mut app, ctx) = fixture();
    app.open_download_folders();
    app.folders_draft.as_mut().unwrap().organize = false;
    click(&mut app, &ctx, "×");
    assert!(app.folders_draft.is_none());
    assert!(app.store.download_folders().unwrap().organize);
    app.open_columns();
    click(&mut app, &ctx, "Size");
    click(&mut app, &ctx, "×");
    assert!(app.dialog.is_none());
    assert!(app.interface.columns[3]);
    app.open_speed_limits();
    app.speed_limits.as_mut().unwrap().request = Some(Uuid::new_v4());
    click(&mut app, &ctx, "×");
    assert_eq!(app.dialog, Some(Dialog::SpeedLimits));
    app.speed_limits.as_mut().unwrap().request = None;
    click(&mut app, &ctx, "×");
    assert!(app.speed_limits.is_none() && app.dialog.is_none());
}

#[test]
fn dialogs_offer_only_the_header_close_button() {
    type Open = fn(&mut Desktop);
    let (_root, mut app, ctx) = fixture();
    let dialogs: [(&str, Open); 12] = [
        ("Add download", |app| app.open_add()),
        ("Add batch", |app| app.open_queue_dialog(Dialog::Batch)),
        ("New queue", |app| app.open_queue_dialog(Dialog::NewQueue)),
        ("Schedule", |app| app.open_queue_dialog(Dialog::Schedule)),
        ("Preferences", |app| app.open_preferences_window()),
        ("Download folders", |app| app.open_download_folders()),
        ("Table columns", |app| app.open_columns()),
        ("Speed limits", |app| app.open_speed_limits()),
        ("Data quotas", |app| app.open_data_quotas()),
        ("Proxy", |app| app.open_proxy()),
        ("Connections", |app| app.open_connections()),
        ("Antivirus settings", |app| app.open_antivirus_settings()),
    ];
    for (name, open) in dialogs {
        open(&mut app);
        frame(&mut app, &ctx, SIZE, vec![]);
        let output = frame(&mut app, &ctx, SIZE, vec![]);
        assert!(app.dialog.is_some(), "{name} did not open");
        text_rect(&output, "×");
        for duplicate in ["Cancel", "Close"] {
            if name == "Preferences" && duplicate == "Cancel" {
                continue;
            }
            assert!(
                !output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == duplicate)),
                "{name} repeats its × with {duplicate}"
            );
        }
        click(&mut app, &ctx, "×");
        assert!(app.dialog.is_none(), "× did not close {name}");
    }
}

#[test]
fn successful_saves_and_errors_render_distinct_feedback_in_both_themes() {
    for dark in [false, true] {
        let (_root, mut app, ctx) = fixture();
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        app.open_speed_limits();
        let request = Uuid::new_v4();
        app.speed_limits.as_mut().unwrap().request = Some(request);
        app.bandwidth_event(bandwidth::Event {
            request,
            result: Ok(bandwidth::Limits::default()),
        });
        assert_eq!(
            app.notice.as_ref().unwrap().kind,
            interactions::NoticeKind::Success
        );
        let output = frame(&mut app, &ctx, SIZE, vec![]);
        let message = &app.notice.as_ref().unwrap().message;
        let color = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) if &t.galley.job.text == message => {
                    Some(t.galley.job.sections[0].format.color)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(color, Palette::new(dark).success);
        assert_ne!(
            color,
            Notice::error("error".into()).color(Palette::new(dark))
        );
        assert_eq!(
            Notice::outcome(Some("warning".into()), "success".into()).kind,
            interactions::NoticeKind::Error
        );
    }
}

#[test]
fn add_advanced_is_collapsed_by_default_and_preserves_configured_values() {
    let (_root, mut app, ctx) = fixture();
    app.open_add();
    frame(&mut app, &ctx, SIZE, vec![]);
    let output = frame(&mut app, &ctx, SIZE, vec![]);
    assert!(!output.shapes.iter().any(|s| matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.job.text=="Expected SHA-256 (optional)")));
    click(&mut app, &ctx, "Advanced");
    click(&mut app, &ctx, "Use login");
    assert!(app.add_authentication);
    app.expected_sha256 = "a".repeat(64);
    click(&mut app, &ctx, "Advanced");
    click(&mut app, &ctx, "×");
    app.open_add();
    assert!(app.add_authentication);
    assert_eq!(app.expected_sha256, "a".repeat(64));
    assert!(app.store.list().unwrap().is_empty());
}

//! Opt-in native Windows allocation counters around repeated egui lifecycles.
//! Runs in its own test process; no personal data, downloads, or OS settings.
use super::interaction_tests::{click, fixture, frame};
use super::*;
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Threading::{GetCurrentProcess, GetProcessHandleCount},
};

#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    size: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    peak_paged: usize,
    paged: usize,
    peak_nonpaged: usize,
    nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private_bytes: usize,
}
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(process: HANDLE, counters: *mut MemoryCounters, size: u32) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn GetGuiResources(process: HANDLE, flags: u32) -> u32;
}
fn sample(cycle: usize) -> serde_json::Value {
    let mut counters = MemoryCounters {
        size: std::mem::size_of::<MemoryCounters>() as u32,
        ..Default::default()
    };
    let mut handles = 0;
    let (gdi, user) = unsafe {
        let process = GetCurrentProcess();
        assert_ne!(
            GetProcessMemoryInfo(
                process,
                &mut counters,
                std::mem::size_of::<MemoryCounters>() as u32
            ),
            0
        );
        assert_ne!(GetProcessHandleCount(process, &mut handles), 0);
        (GetGuiResources(process, 0), GetGuiResources(process, 1))
    };
    let point = serde_json::json!({"cycle":cycle,"privateBytes":counters.private_bytes,
        "workingSet":counters.working_set,"handles":handles,"gdi":gdi,"user":user});
    println!("MEMORY_SAMPLE {point}");
    point
}

#[test]
#[ignore = "Bounded memory soak; run alone with --ignored --exact --nocapture --test-threads=1"]
fn repeated_pages_dialogs_and_history_cleanup() {
    let (temp, mut app, ctx) = fixture();
    let size = egui::vec2(1080.0, 800.0);
    app.interface.auto_download_status = false;
    app.force_refresh();
    let mut samples = Vec::new();
    for cycle in 0..300 {
        let mut job = Job::new(
            &format!("https://example.invalid/memory-{cycle}.bin"),
            None,
            temp.path().join("downloads"),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = Status::Complete;
        job.downloaded = 1024;
        job.total = Some(1024);
        app.store.enqueue(&job).unwrap();
        app.force_refresh();
        for view in [
            View::Downloads,
            View::Queues,
            View::Scheduler,
            View::Automation,
            View::Logs,
            View::Guide,
            View::Changelog,
        ] {
            app.view = view;
            app.force_refresh();
            app.background(&ctx);
            for _ in 0..2 {
                drop(frame(&mut app, &ctx, size, vec![]));
            }
        }
        app.view = View::Downloads;
        app.open_preferences_window();
        drop(frame(&mut app, &ctx, size, vec![]));
        app.close_preferences_window(&ctx);
        for dialog in [
            Dialog::Add,
            Dialog::Batch,
            Dialog::NewQueue,
            Dialog::About,
            Dialog::Mcp,
        ] {
            match dialog {
                Dialog::Add => app.open_add(),
                Dialog::About => app.dialog = Some(dialog),
                Dialog::Mcp => app.open_mcp(),
                _ => app.open_queue_dialog(dialog),
            }
            for _ in 0..2 {
                drop(frame(&mut app, &ctx, size, vec![]));
            }
            for pressed in [true, false] {
                drop(frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                ));
            }
            assert!(app.dialog.is_none(), "{dialog:?} did not close");
        }
        click(&mut app, &ctx, "Clear finished");
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.store.get(&job.id).unwrap().is_some() {
            assert!(Instant::now() < deadline, "History removal did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
        app.force_refresh();
        app.background(&ctx);
        for _ in 0..3 {
            drop(frame(&mut app, &ctx, size, vec![]));
        }
        assert!(app.jobs.is_empty());
        assert!(app.rates.is_empty());
        assert!(app.store.activity.get(&job.id).is_none());
        if cycle % 25 == 24 {
            samples.push(sample(cycle + 1));
        }
    }
    // The first 100 cycles warm fonts, layouts and allocator arenas. Inspect the
    // emitted samples as well; these generous bounds flag persistent retention,
    // not small allocator/OS fluctuations. This check is never a default CI test.
    let warm = &samples[3];
    let last = samples.last().unwrap();
    assert!(
        last["privateBytes"].as_u64().unwrap()
            <= warm["privateBytes"].as_u64().unwrap() + 16 * 1024 * 1024
    );
    assert!(last["handles"].as_u64().unwrap() <= warm["handles"].as_u64().unwrap() + 8);
    assert_eq!(last["gdi"], warm["gdi"]);
    assert_eq!(last["user"], warm["user"]);
    drop(app);
    drop(ctx);
    temp.close().unwrap();
}

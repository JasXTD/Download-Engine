#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod clipboard_links;
mod desktop;
mod drop_target;
mod startup;
mod tray;
mod url_drop;
use download_engine::{
    diagnostics::{self, Component, Details, Event, Level},
    paths::AppPaths,
    scheduler::Scheduler,
    store::Store,
};
use eframe::egui;
use std::fs::OpenOptions;

fn main() {
    // Request the Explorer scan action in an isolated helper, before any
    // desktop startup, queue access, or interactive startup-error handling.
    let helper_args: Vec<_> = std::env::args_os().collect();
    if helper_args
        .get(1)
        .is_some_and(|a| a == "--antivirus-scan-request" || a == "--malwarebytes-scan-request")
    {
        let result = (|| -> anyhow::Result<download_engine::antivirus::Provider> {
            let legacy = helper_args[1] == "--malwarebytes-scan-request";
            anyhow::ensure!(
                helper_args.len() == if legacy { 3 } else { 4 },
                "Expected a provider and one local file for the antivirus scan request"
            );
            let provider = if legacy {
                download_engine::antivirus::Provider::Malwarebytes
            } else {
                helper_args[2]
                    .to_str()
                    .and_then(download_engine::antivirus::Provider::from_helper_id)
                    .ok_or_else(|| anyhow::anyhow!("Unsupported external scanner"))?
            };
            download_engine::antivirus::external_helper_request(
                provider,
                std::path::Path::new(&helper_args[if legacy { 2 } else { 3 }]),
            )?;
            Ok(provider)
        })();
        match result {
            Ok(provider) => {
                if helper_args[1] == "--malwarebytes-scan-request" {
                    println!("IDM_EX_MALWAREBYTES_REQUESTED_V1");
                } else {
                    println!("IDM_EX_AV_REQUESTED_V1:{}", provider.helper_id().unwrap());
                }
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        }
    }
    if let Err(error) = start() {
        diagnostics::error(Event::StartupFailed, &error);
        let message = format!("Download Engine could not start:\n{error:#}");
        eprintln!("{message}");
        #[cfg(windows)]
        {
            let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "Download Engine".encode_utf16().chain(Some(0)).collect();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    0x10,
                );
            }
        }
    } else {
        diagnostics::record(Level::Info, Event::Stopped, Details::default());
    }
    if diagnostics::flush().is_err() {
        eprintln!("Some diagnostics could not be saved before exit.");
    }
}
fn start() -> anyhow::Result<()> {
    let paths = AppPaths::discover()?;
    if diagnostics::init(&paths, Component::Desktop).is_err() {
        eprintln!(
            "Download Engine diagnostics are unavailable. Check the data folder permissions."
        );
    }
    let _installation = download_engine::paths::installation_guard()?;
    // A portable copy updated in place has no installer to do this. A failure
    // leaves the connection marked for repair in Browser setup.
    match download_engine::browser_setup::BrowserSetup::discover()
        .and_then(|setup| setup.migrate_legacy_host())
    {
        Ok(migrated) if !migrated.is_empty() => {
            diagnostics::record(Level::Info, Event::BrowserRegistered, Details::default())
        }
        Ok(_) => {}
        Err(error) => diagnostics::error(Event::BrowserSetupFailed, &error),
    }
    let show_browser_setup = std::env::args().any(|arg| arg == "--browser-setup");
    let autostart = std::env::args().any(|arg| arg == startup::ARGUMENT);
    paths.prepare()?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.data.join("desktop.lock"))?;
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {}
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            // A sign-in launch must not bring an already running copy to the front.
            if !autostart {
                tray::show_existing(&paths, show_browser_setup)?;
            }
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }
    let store = Store::open(paths)?;
    let hidden = startup::launch_hidden(autostart, store.startup_preferences());
    let scheduler = Scheduler::start(store.clone())?;
    diagnostics::record(Level::Info, Event::Ready, Details::default());
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 860.0])
        .with_min_inner_size([860.0, 600.0])
        .with_drag_and_drop(false)
        .with_icon(tray::icon());
    #[cfg(feature = "screenshot")]
    let viewport = if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some() {
        let size = std::env::var("IDM_EX_TEST_SIZE")
            .ok()
            .and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?])
            })
            .filter(|s| (860.0..=1920.0).contains(&s[0]) && (600.0..=1200.0).contains(&s[1]));
        if let Some(size) = size {
            viewport.with_inner_size(size)
        } else {
            viewport
        }
    } else {
        viewport
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Download Engine",
        options,
        Box::new(move |cc| {
            desktop::configure(&cc.egui_ctx);
            let mut app = desktop::Desktop::new(store, scheduler, lock);
            app.configure_appearance(&cc.egui_ctx);
            #[cfg(windows)]
            app.sync_window_frame(&cc.egui_ctx, cc);
            app.attach_tray(&cc.egui_ctx);
            if hidden {
                app.start_in_tray(cc);
            }
            app.attach_url_drop(cc);
            app.attach_clipboard(&cc.egui_ctx);
            app.attach_drop_target(&cc.egui_ctx);
            if show_browser_setup {
                app.open_browser_setup();
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

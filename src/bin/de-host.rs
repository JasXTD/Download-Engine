use anyhow::{Context, Result};
use download_engine::{
    diagnostics::{self, Component, Details, Event, Level},
    native,
    paths::AppPaths,
    store::Store,
};
use std::io::{self, Write};

#[path = "../desktop_process.rs"]
mod desktop_process;

fn main() {
    if let Err(error) = run() {
        diagnostics::error(Event::NativeHostFailed, &error);
        let _ = writeln!(io::stderr(), "Download Engine native host: {error:#}");
        let _ = diagnostics::flush();
        std::process::exit(1);
    }
    diagnostics::record(Level::Info, Event::Stopped, Details::default());
    if diagnostics::flush().is_err() {
        let _ = writeln!(
            io::stderr(),
            "Some diagnostics could not be saved before exit."
        );
    }
}
fn run() -> Result<()> {
    let paths = AppPaths::discover();
    if let Ok(paths) = &paths
        && diagnostics::init(paths, Component::NativeHost).is_err()
    {
        let _ = writeln!(
            io::stderr(),
            "Download Engine diagnostics are unavailable. Check the data folder permissions."
        );
    }
    if setup_command()? {
        return Ok(());
    }
    let paths = paths?;
    let _installation = download_engine::paths::installation_guard()?;
    #[cfg(windows)]
    prevent_pipe_inheritance()?;
    let store = Store::open(paths.clone())?;
    diagnostics::record(Level::Info, Event::Ready, Details::default());
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut sequence = 0u64;
    while let Some(data) = native::read_frame(&mut input)? {
        sequence = sequence.saturating_add(1);
        let started = std::time::Instant::now();
        let request = serde_json::from_slice::<native::Request>(&data);
        drop(data); // Wipe the raw native frame before queue work or desktop startup.
        let details = Details {
            action: request
                .as_ref()
                .ok()
                .map(native::Request::diagnostic_action),
            request: Some(sequence),
            ..Default::default()
        };
        diagnostics::record(Level::Info, Event::NativeRequestReceived, details.clone());
        let manual_add = matches!(
            &request,
            Ok(native::Request::Add { .. }
                | native::Request::AddBatch { .. }
                | native::Request::AddBrowserLogin { .. }
                | native::Request::PreviewVideo { .. }
                | native::Request::ReviewBrowserVideo { .. }
                | native::Request::PreviewHls { .. })
        );
        let status_window = matches!(
            &request,
            Ok(native::Request::Add { .. }
                | native::Request::AddBrowserLogin { .. }
                | native::Request::CaptureCommit { .. })
        );
        let mut reply = match request {
            Ok(request @ native::Request::CaptureCommit { .. }) => match ensure_desktop(&paths) {
                Ok(()) => native::handle_request(&store, request),
                Err(error) => {
                    diagnostics::error(Event::DesktopLaunchFailed, &error);
                    native::Reply::failure(error)
                }
            },
            Ok(request) => native::handle_request(&store, request),
            Err(_) => {
                diagnostics::record(Level::Warn, Event::NativeMessageInvalid, details.clone());
                native::Reply::failure("Invalid native request")
            }
        };
        if manual_add
            && reply.ok
            && (reply.job_id.is_some() || !reply.job_ids.is_empty() || reply.preview_id.is_some())
            && let Err(error) = ensure_desktop(&paths)
        {
            diagnostics::error(Event::DesktopLaunchFailed, &error);
            // A saved job remains successful even if its desktop launch fails.
            reply.message = if reply.preview_id.is_some() {
                format!("Video preview is waiting. Open Download Engine to review it ({error}).")
            } else {
                format!("Saved to queue. Open Download Engine to start it ({error}).")
            };
        }
        if status_window
            && reply.ok
            && let Some(id) = &reply.job_id
            && let Err(error) = store.request_download_status(id)
        {
            // The durable handoff remains successful if only its UI request fails.
            diagnostics::error(Event::UiError, &error);
        }
        diagnostics::record(
            if reply.ok { Level::Info } else { Level::Warn },
            if reply.ok {
                Event::NativeRequestSucceeded
            } else {
                Event::NativeRequestFailed
            },
            Details {
                elapsed_ms: Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
                job: reply
                    .job_id
                    .as_deref()
                    .and_then(|id| uuid::Uuid::parse_str(id).ok()),
                ..details
            },
        );
        native::write_frame(&mut output, &reply)?;
    }
    Ok(())
}
fn ensure_desktop(paths: &AppPaths) -> Result<()> {
    desktop_process::ensure_running(paths)
}

// Chrome/Firefox give the host inheritable standard handles. A long-lived GUI
// must not keep those original pipes open after this short-lived host exits.
#[cfg(windows)]
fn prevent_pipe_inheritance() -> Result<()> {
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // These borrowed process handles remain owned by the standard streams.
        let handle = unsafe { GetStdHandle(kind) };
        if !handle.is_null()
            && handle != INVALID_HANDLE_VALUE
            && unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0
        {
            return Err(io::Error::last_os_error())
                .context("Cannot isolate the native messaging pipes");
        }
    }
    Ok(())
}

// These management actions never open the download database or launch the desktop.
fn setup_command() -> Result<bool> {
    use download_engine::browser_setup::{Browser, BrowserSetup};
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(action) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    match action {
        "--register-browser" => {
            anyhow::ensure!(
                (2..=3).contains(&args.len()),
                "Usage: --register-browser chrome|edge|brave ID, or --register-browser firefox"
            );
            let browser = Browser::parse(&args[1])?;
            BrowserSetup::discover()?.register(browser, args.get(2).map_or("", String::as_str))?;
            diagnostics::record(Level::Info, Event::BrowserRegistered, Details::default());
            println!(
                "{}: registered. Use Check connection in the extension popup.",
                browser.label()
            );
        }
        "--unregister-browser" => {
            anyhow::ensure!(
                args.len() == 2,
                "Usage: --unregister-browser chrome|edge|brave|firefox"
            );
            let browser = Browser::parse(&args[1])?;
            let removed = BrowserSetup::discover()?.unregister(browser)?;
            diagnostics::record(Level::Info, Event::BrowserUnregistered, Details::default());
            println!(
                "{}: {}",
                browser.label(),
                if removed {
                    "disconnected"
                } else {
                    "no registration owned by this copy"
                }
            );
        }
        // The installer runs this after an upgrade renamed the host program.
        "--migrate-browser-registrations" => {
            anyhow::ensure!(args.len() == 1, "Unexpected arguments");
            for browser in BrowserSetup::discover()?.migrate_legacy_host()? {
                println!("{}: connection updated", browser.label());
            }
        }
        "--unregister-all-browsers" => {
            anyhow::ensure!(args.len() == 1, "Unexpected arguments");
            let setup = BrowserSetup::discover()?;
            for browser in Browser::ALL {
                setup.unregister(browser)?;
            }
            diagnostics::record(Level::Info, Event::BrowserUnregistered, Details::default());
        }
        _ => return Ok(false), // Browser-supplied origin and parent-window arguments.
    }
    Ok(true)
}

//! Desktop controls for the same localhost server used by the CLI.
use super::*;
use download_engine::mcp::{self, Server, ServerStatus, Settings};
use std::thread::JoinHandle;
use zeroize::Zeroizing;

mod ui;

const FIRST_PROMPT: &str = "Read get_usage_guide, then show my queues.";

enum Completed {
    Saved(Settings, bool),
    Copied(Zeroizing<String>),
    Checked(usize),
}

#[derive(Clone, Copy)]
enum Action {
    Save,
    Start,
    Stop,
    CopyConfig,
    CopyHeader,
    CopyUrl,
    CopyPrompt,
    Check,
    Guide,
    Logs,
}

struct Pending {
    action: Action,
    worker: JoinHandle<anyhow::Result<Completed>>,
}

pub(super) struct State {
    // Drop the server before releasing the desktop's scheduler and profile lock.
    server: Server,
    pending: Option<Pending>,
    status: ServerStatus,
    saved: Settings,
    port: String,
    read_only: bool,
    start_with_app: bool,
    error: Option<String>,
    notice: Option<String>,
}

impl State {
    pub(super) fn new(store: &Store) -> Self {
        let (saved, error) = match store.mcp_settings() {
            Ok(settings) => (settings, None),
            Err(error) => (
                Settings::default(),
                Some(tf!("Cannot load MCP settings: {error}", error = error)),
            ),
        };
        let mut state = Self {
            server: Server::default(),
            pending: None,
            status: ServerStatus::Stopped,
            saved,
            port: saved.port.to_string(),
            read_only: saved.read_only,
            start_with_app: saved.start_with_app,
            error,
            notice: None,
        };
        if saved.start_with_app && state.error.is_none() {
            state.start(store);
        }
        state
    }

    fn draft(&self) -> anyhow::Result<Settings> {
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| anyhow::anyhow!(tr("Enter a port from 1 to 65535")))?;
        Ok(Settings {
            port,
            read_only: self.read_only,
            start_with_app: self.start_with_app,
        })
    }

    fn url(&self) -> Option<String> {
        let port = if self.status.active() {
            self.saved.port
        } else {
            self.draft().ok()?.port
        };
        Some(format!("http://127.0.0.1:{port}/mcp"))
    }

    fn start(&mut self, store: &Store) {
        let context = mcp::Context {
            store: store.clone(),
            read_only: self.saved.read_only,
            no_start: true,
            start: |_| anyhow::bail!(tr("Open Download Engine before using its controls")),
        };
        match self.server.start(context, self.saved.port) {
            Ok(()) => self.status = ServerStatus::Starting,
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(super) fn poll(&mut self, store: &Store, ctx: &egui::Context) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.worker.is_finished())
        {
            let pending = self.pending.take().unwrap();
            let stale_check = matches!(pending.action, Action::Check)
                && self.server.status() != ServerStatus::Running;
            // Stop remains available while a check is pending. Its old result,
            // including a disconnect error caused by Stop, must not update the page.
            let completed = pending.worker.join();
            if !stale_check {
                match completed {
                    Ok(Ok(Completed::Saved(settings, start))) => {
                        self.saved = settings;
                        self.port = settings.port.to_string();
                        self.notice = (!start).then(|| tr("Settings saved.").into());
                        if start {
                            self.start(store);
                        }
                    }
                    Ok(Ok(Completed::Copied(text))) => {
                        ctx.copy_text(text.to_string());
                        self.notice = Some(
                            tr("Copied. Paste into your AI client's connection settings.").into(),
                        );
                    }
                    Ok(Ok(Completed::Checked(count))) => {
                        if self.server.status() == ServerStatus::Running {
                            self.notice = Some(tf!(
                                "Connection checked. {count} tools available to your AI.",
                                count = count
                            ));
                        }
                    }
                    Ok(Err(error)) => self.error = Some(format!("{error:#}")),
                    Err(_) => {
                        self.error =
                            Some(tr("MCP operation stopped unexpectedly. Try again.").into())
                    }
                }
            }
        }
        let status = self.server.status();
        if status != self.status {
            use download_engine::diagnostics::{self, Details, Event, Level};
            match &status {
                ServerStatus::Running => {
                    diagnostics::record(Level::Info, Event::McpStarted, Details::default())
                }
                ServerStatus::Stopped => {
                    diagnostics::record(Level::Info, Event::McpStopped, Details::default())
                }
                ServerStatus::Failed(error) => {
                    diagnostics::error(Event::McpFailed, &anyhow::anyhow!(error.clone()))
                }
                _ => {}
            }
            self.status = status;
        }
        if self.pending.is_some()
            || matches!(self.status, ServerStatus::Starting | ServerStatus::Stopping)
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub(super) fn label(&self) -> &'static str {
        match self.status {
            ServerStatus::Stopped => tr("Stopped"),
            ServerStatus::Starting => tr("Starting…"),
            ServerStatus::Running => tr("Running"),
            ServerStatus::Stopping => tr("Stopping…"),
            ServerStatus::Failed(_) => tr("Could not start"),
        }
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn capture_ready(&self) -> bool {
        self.pending.is_none()
            && !matches!(self.status, ServerStatus::Starting | ServerStatus::Stopping)
    }

    fn act(&mut self, action: Action, store: &Store, ctx: &egui::Context) {
        if matches!(action, Action::Stop) {
            self.server.stop();
            self.notice = None;
            return;
        }
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        self.notice = None;
        let port = self.saved.port;
        let store = store.clone();
        let task: Box<dyn FnOnce() -> anyhow::Result<Completed> + Send> = match action {
            Action::Start | Action::Save => {
                if self.status.active() {
                    return;
                }
                let settings = match self.draft() {
                    Ok(settings) => settings,
                    Err(error) => {
                        self.error = Some(error.to_string());
                        return;
                    }
                };
                Box::new(move || {
                    store.save_mcp_settings(settings)?;
                    Ok(Completed::Saved(settings, matches!(action, Action::Start)))
                })
            }
            Action::CopyConfig | Action::CopyHeader if self.status == ServerStatus::Running => {
                Box::new(move || {
                    let config = mcp::client_config(&store.paths, port)?;
                    let text = if matches!(action, Action::CopyHeader) {
                        config["mcpServers"]["download-engine"]["headers"]["Authorization"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!(tr("Missing connection header")))?
                            .to_owned()
                    } else {
                        serde_json::to_string_pretty(&config)?
                    };
                    Ok(Completed::Copied(Zeroizing::new(text)))
                })
            }
            Action::Check if self.status == ServerStatus::Running => Box::new(move || {
                Ok(Completed::Checked(mcp::check_connection(
                    &store.paths,
                    port,
                )?))
            }),
            Action::CopyUrl => {
                let Some(url) = self.url() else { return };
                ctx.copy_text(url);
                self.notice = Some(tr("Server URL copied.").into());
                return;
            }
            Action::CopyPrompt => {
                ctx.copy_text(FIRST_PROMPT.into());
                self.notice =
                    Some(tr("Prompt copied. Paste it into your connected AI client.").into());
                return;
            }
            _ => return,
        };
        match std::thread::Builder::new()
            .name("mcp-settings".into())
            .spawn(task)
        {
            Ok(worker) => self.pending = Some(Pending { action, worker }),
            Err(error) => {
                self.error = Some(tf!("Cannot start MCP operation: {error}", error = error))
            }
        }
        ctx.request_repaint();
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let active = self.server.status().active();
        self.server.stop();
        if let Some(pending) = self.pending.take() {
            let _ = pending.worker.join();
        }
        drop(std::mem::take(&mut self.server));
        if active {
            use download_engine::diagnostics::{self, Details, Event, Level};
            diagnostics::record(Level::Info, Event::McpStopped, Details::default());
        }
    }
}

#[cfg(all(test, windows))]
#[path = "mcp_tests.rs"]
mod tests;

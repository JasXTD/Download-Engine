use download_engine::{i18n::tr, tf};
mod about;
mod add_download;
mod add_login;
mod authentication;
mod automation;
mod background_io;
mod browser_video;
mod browsers;
mod clipboard;
mod diagnostics;
mod download_status;
mod drop_target;
mod extensions;
#[cfg(any(test, feature = "screenshot"))]
mod fixture_database;
mod folder_picker;
mod form_dialog;
mod formatting;
mod guide;
mod interactions;
mod scripts;
use interactions::{Confirmation, DestructiveAction, Notice, Selection};
mod link;
mod link_fallback;
mod list_transfer;
mod logs;
mod mcp;
mod new_queue;
mod preferences_window;
mod queue_overview;
mod queues;
#[cfg(test)]
mod responsiveness_tests;
mod settings;
mod settings_page;
mod speed_limits;
mod theme;
mod theme_assets;
mod updates;
use formatting::*;
use theme::Palette;
pub use theme::configure;
use view::SquareCheckbox;

mod antivirus;
mod connections;
mod extras;
mod grabber;
mod hls;
mod power;
mod proxy;
mod quotas;
mod shell;
mod table;
mod view;
#[cfg(windows)]
mod window_frame;
mod zip_preview;

use crate::tray::{Action as TrayAction, Tray};
use download_engine::{
    auth::{self, Credentials, Kind as AuthKind},
    bandwidth,
    browser_setup::{Browser, BrowserSetup, RegistrationStatus},
    categories::{Category, DownloadFolders},
    link_recovery::{Command as LinkCommand, Event as LinkEvent, Outcome as LinkOutcome},
    model::{Job, Status, validate_url},
    preferences::{Appearance, InterfacePreferences, Preferences},
    queue::{DownloadQueue, MAIN_QUEUE, local_schedule},
    queue_schedule::{DAY_NAMES, EVERY_DAY, RecurringSchedule, clock_label},
    quota,
    scheduler::{Command, Scheduler},
    store::Store,
};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::PathBuf,
    time::{Duration, Instant},
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Downloading,
    Queued,
    Unfinished,
    Complete,
    Failed,
}
impl Filter {
    fn accepts(self, status: Status) -> bool {
        match self {
            Self::All => true,
            Self::Downloading => status == Status::Downloading,
            Self::Queued => status == Status::Queued,
            Self::Unfinished => status != Status::Complete,
            Self::Complete => status == Status::Complete,
            Self::Failed => status == Status::Failed,
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::All => tr("All downloads"),
            Self::Downloading => tr("Downloading"),
            Self::Queued => tr("Queued"),
            Self::Unfinished => tr("Unfinished"),
            Self::Complete => tr("Completed"),
            Self::Failed => tr("Failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialog {
    Automation,
    Confirm,
    About,
    CheckUpdates,
    Legal(about::LegalPage),
    Proxy,
    Connections,
    ZipPreview,
    AntivirusSettings,
    AntivirusResult,
    AntivirusScan,
    Checksum,
    Columns,
    Add,
    Browsers,
    UpdateLink,
    Authentication,
    SpeedLimits,
    DataQuotas,
    PreferencesWindow,
    Mcp,
    DownloadFolders,
    Batch,
    Grabber,
    LinkFallback,
    Hls,
    BrowserVideo,
    NewQueue,
    RenameQueue,
    Schedule,
    ListTransfer,
    ScriptSample,
    ScriptReplace,
    ExtensionReview,
    ExtensionRemove,
}

enum LinkState {
    Editing,
    Checking { checked: u64, total: u64 },
    Ready(LinkOutcome),
    Applying,
}
struct LinkDialog {
    job: String,
    name: String,
    url: String,
    request: Option<Uuid>,
    state: LinkState,
    page: String,
    browser: Option<download_engine::browser_recovery::Lease>,
    browser_poll: Instant,
}

struct AuthenticationDialog {
    job: String,
    name: String,
    url: String,
    method: Option<AuthKind>,
    username: Zeroizing<String>,
    secret: Zeroizing<String>,
    restart_needed: bool,
    restart_confirmed: bool,
    resume: bool,
    request: Option<Uuid>,
}

struct SpeedField {
    enabled: bool,
    kib: String,
}
impl SpeedField {
    fn new(rate: Option<u64>) -> Self {
        Self {
            enabled: rate.is_some(),
            kib: (rate.unwrap_or(1024 * 1024) as f64 / 1024.0).to_string(),
        }
    }
    fn rate(&self) -> anyhow::Result<Option<u64>> {
        if !self.enabled {
            return Ok(None);
        }
        let value = self
            .kib
            .trim()
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!(tr("Enter a speed in KiB/s.")))?
            * 1024.0;
        anyhow::ensure!(
            value.is_finite()
                && value.fract() == 0.0
                && value >= bandwidth::MIN_RATE as f64
                && value <= bandwidth::MAX_RATE as f64,
            tr("Enter a speed from 1 to 1048576 KiB/s.")
        );
        Ok(Some(value as u64))
    }
    /// Rows of one form pass the same `label_width` so their inputs line up.
    fn draw(&mut self, ui: &mut egui::Ui, label: &str, label_width: f32) {
        ui.horizontal(|ui| {
            view::column_checkbox(ui, label_width, &mut self.enabled, label);
            ui.add_enabled(
                self.enabled,
                egui::TextEdit::singleline(&mut self.kib)
                    .desired_width(100.0)
                    .char_limit(20),
            );
            ui.label("KiB/s");
        });
    }
}
struct SpeedDialog {
    global: SpeedField,
    download: Option<(String, String, SpeedField)>,
    request: Option<Uuid>,
}
fn speed_limit_label(rate: u64) -> String {
    if rate >= 1024 * 1024 {
        format!("{:.1} MiB/s", rate as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KiB/s", rate as f64 / 1024.0)
    }
}

struct FoldersDialog {
    organize: bool,
    paths: [String; 6],
}
impl FoldersDialog {
    fn new(rules: &DownloadFolders) -> Self {
        Self {
            organize: rules.organize,
            paths: Category::ALL.map(|category| {
                rules
                    .overrides
                    .get(&category)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
        }
    }
    fn rules(&self) -> DownloadFolders {
        DownloadFolders {
            organize: self.organize,
            overrides: Category::ALL
                .into_iter()
                .zip(&self.paths)
                .filter(|(_, path)| !path.trim().is_empty())
                .map(|(category, path)| (category, PathBuf::from(path.trim())))
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
    #[default]
    Downloads,
    Queues,
    Scheduler,
    Automation,
    Logs,
    Guide,
    Changelog,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScheduleMode {
    Once,
    Daily,
    SelectedDays,
}

pub struct Desktop {
    download_status: download_status::State,
    mcp: mcp::State,
    guide_topic: usize,
    automation: automation::State,
    extensions: extensions::State,
    scripts: scripts::State,
    clipboard: clipboard::State,
    drop_target: drop_target::State,
    #[cfg(windows)]
    window_frame: window_frame::State,
    antivirus: antivirus::State,
    extras: extras::Extras,
    grabber: Option<grabber::State>,
    link_fallback: link_fallback::State,
    hls: Option<hls::State>,
    browser_video: Option<browser_video::State>,
    browser_video_requests: browser_video::Requests,
    hls_requests: hls::Requests,
    proxy: Option<proxy::Form>,
    connections: Option<connections::Form>,
    zip_preview: Option<zip_preview::State>,
    hls_request_check: Instant,
    browser_video_request_check: Instant,
    grabber_summaries: Vec<download_engine::grabber_schedule::Summary>,
    table: table::State,
    expected_sha256: String,
    view: View,
    search: String,
    interface: InterfacePreferences,
    tray: Option<Tray>,
    preferences: Preferences,
    startup: crate::startup::Settings,
    hide_on_start: bool,
    #[cfg(windows)]
    cloaked_start: bool,
    download_folders: Option<DownloadFolders>,
    folders_draft: Option<FoldersDialog>,
    exit_requested: bool,
    close_handled: bool,
    pending_review: Option<String>,
    scheduler: Scheduler,
    store: Store,
    jobs: Vec<Job>,
    queues: Vec<DownloadQueue>,
    queue_id: String,
    batch: String,
    batch_pattern: queues::PatternDraft,
    queue_name: String,
    queue_synchronize: bool,
    schedule_date: String,
    schedule_time: String,
    schedule_stop: String,
    schedule_mode: ScheduleMode,
    schedule_days: [bool; 7],
    selected: Option<String>,
    selection: Selection,
    confirmation: Option<Confirmation>,
    folder_picker: folder_picker::FolderPicker,
    filter: Filter,
    category: Option<Category>,
    dialog: Option<Dialog>,
    link: Option<LinkDialog>,
    authentication: Option<AuthenticationDialog>,
    add_authentication: bool,
    add_queue: Option<String>,
    add_login: add_login::Draft,
    add_request: Option<background_io::Task<add_download::Added>>,
    settings_summary: settings_page::Summary,
    preferences_window: Option<preferences_window::State>,
    queue_summaries: HashMap<String, Result<queue_overview::Summary, String>>,
    immediate_downloads: Vec<String>,
    add_ftp_security: Option<download_engine::ftp::Security>,
    speed_limits: Option<SpeedDialog>,
    limits: bandwidth::Limits,
    quota_status: quota::Status,
    data_quotas: Option<quotas::Form>,
    url: String,
    name: String,
    folder: String,
    focus_url: bool,
    browser: Browser,
    browser_ids: [String; 3],
    /// The ID this copy's extension folder gets in the selected Chromium browser.
    browser_expected: Option<String>,
    browser_status: Option<RegistrationStatus>,
    browser_notice: Option<(bool, String)>,
    form_error: Option<String>,
    notice: Option<Notice>,
    diagnostics: diagnostics::State,
    list_transfer: list_transfer::State,
    logs: logs::State,
    background_io: background_io::State,
    theme_assets: theme_assets::State,
    last_refresh: Instant,
    rates: HashMap<String, (u64, f64)>,
    #[cfg(feature = "screenshot")]
    screenshot_requested: bool,
    _lock: File,
}

impl Desktop {
    pub fn new(store: Store, scheduler: Scheduler, lock: File) -> Self {
        let _language = download_engine::i18n::scope(
            store
                .interface_preferences()
                .map(|p| p.language)
                .unwrap_or_default(),
        );
        let (preferences, preference_error) = match store.preferences() {
            Ok(value) => (value, None),
            Err(error) => (
                Preferences::default(),
                Some(tf!("Cannot load preferences: {error}", error = error)),
            ),
        };
        let (interface, interface_error) = match store.interface_preferences() {
            Ok(value) => (value, None),
            Err(error) => (
                InterfacePreferences::default(),
                Some(tf!("Cannot load appearance: {error}", error = error)),
            ),
        };
        let (startup, startup_error) = crate::startup::Settings::load(&store);
        let mut app = Self {
            download_status: download_status::State::default(),
            mcp: mcp::State::new(&store),
            guide_topic: 0,
            automation: automation::State::default(),
            extensions: extensions::State::default(),
            scripts: scripts::State::default(),
            clipboard: clipboard::State::default(),
            drop_target: drop_target::State::default(),
            #[cfg(windows)]
            window_frame: window_frame::State::default(),
            antivirus: antivirus::State::default(),
            extras: extras::Extras::default(),
            grabber: None,
            link_fallback: link_fallback::State::default(),
            hls: None,
            browser_video: None,
            browser_video_requests: browser_video::Requests::default(),
            hls_requests: hls::Requests::default(),
            proxy: None,
            connections: None,
            zip_preview: None,
            hls_request_check: Instant::now() - Duration::from_secs(2),
            browser_video_request_check: Instant::now() - Duration::from_secs(2),
            grabber_summaries: Vec::new(),
            table: table::State::default(),
            expected_sha256: String::new(),
            view: View::Downloads,
            search: String::new(),
            interface,
            tray: None,
            preferences,
            startup,
            hide_on_start: false,
            #[cfg(windows)]
            cloaked_start: false,
            download_folders: None,
            folders_draft: None,
            exit_requested: false,
            close_handled: false,
            pending_review: None,
            folder: String::new(),
            store,
            scheduler,
            jobs: Vec::new(),
            queues: Vec::new(),
            queue_id: MAIN_QUEUE.into(),
            batch: String::new(),
            batch_pattern: Default::default(),
            queue_name: String::new(),
            queue_synchronize: false,
            schedule_date: String::new(),
            schedule_time: String::new(),
            schedule_stop: "07:00".into(),
            schedule_mode: ScheduleMode::Once,
            schedule_days: [true, true, true, true, true, false, false],
            selected: None,
            selection: Selection::default(),
            confirmation: None,
            folder_picker: folder_picker::FolderPicker::default(),
            filter: Filter::All,
            category: None,
            dialog: None,
            link: None,
            authentication: None,
            add_authentication: false,
            add_queue: None,
            add_login: add_login::Draft::default(),
            add_request: None,
            settings_summary: settings_page::Summary::default(),
            preferences_window: None,
            queue_summaries: HashMap::new(),
            immediate_downloads: Vec::new(),
            add_ftp_security: None,
            speed_limits: None,
            limits: bandwidth::Limits::default(),
            quota_status: quota::Status::default(),
            data_quotas: None,
            url: String::new(),
            name: String::new(),
            focus_url: false,
            browser: Browser::Chrome,
            browser_ids: Default::default(),
            browser_expected: None,
            browser_status: None,
            browser_notice: None,
            form_error: None,
            notice: preference_error
                .or(interface_error)
                .or(startup_error)
                .map(Notice::error),
            diagnostics: diagnostics::State::default(),
            list_transfer: list_transfer::State::default(),
            logs: logs::State::default(),
            background_io: background_io::State::default(),
            theme_assets: theme_assets::State::default(),
            last_refresh: Instant::now() - Duration::from_secs(1),
            rates: HashMap::new(),
            #[cfg(feature = "screenshot")]
            screenshot_requested: false,
            _lock: lock,
        };
        app.initialize_snapshot();
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && let Ok(state) = std::env::var("IDM_EX_TEST_LINK_DIALOG")
            && let Some(job) = app
                .jobs
                .iter()
                .find(|j| matches!(j.status, Status::Paused | Status::Failed))
                .cloned()
        {
            app.open_link(&job);
            if let Some(link) = &mut app.link {
                link.url = "https://example.com/downloads/Studio-installer.exe?fresh-link".into();
                link.state = match state.as_str() {
                    "resume" => LinkState::Ready(LinkOutcome::Resume {
                        saved: job.downloaded,
                        total: job.total.unwrap_or(0),
                    }),
                    "restart" => LinkState::Ready(LinkOutcome::RestartRequired(
                        tr("The replacement does not match the saved data.").into(),
                    )),
                    _ => LinkState::Editing,
                };
            }
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && let Ok(method) = std::env::var("IDM_EX_TEST_AUTH_DIALOG")
            && let Some(job) = app
                .jobs
                .iter()
                .find(|j| matches!(j.status, Status::Paused | Status::Failed))
                .cloned()
        {
            app.open_authentication(&job);
            if let Some(form) = &mut app.authentication {
                form.method = Some(if method == "bearer" {
                    AuthKind::Bearer
                } else {
                    AuthKind::Basic
                });
                *form.username = "example-user".into();
                *form.secret = "masked-preview-secret".into();
                form.restart_needed = method == "reset";
            }
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && std::env::var_os("IDM_EX_TEST_SPEED_DIALOG").is_some()
        {
            app.selected = app
                .jobs
                .iter()
                .find(|j| j.status != Status::Complete)
                .map(|j| j.id.clone());
            app.open_speed_limits();
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && std::env::var_os("IDM_EX_TEST_PREFERENCES").is_some()
        {
            app.open_preferences_window();
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && std::env::var_os("IDM_EX_TEST_FOLDERS").is_some()
        {
            app.open_download_folders();
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some() {
            app.view = match std::env::var("IDM_EX_TEST_VIEW").as_deref() {
                Ok("queues") => View::Queues,
                Ok("scheduler") => View::Scheduler,
                Ok(
                    "automation"
                    | "automation-empty"
                    | "automation-history"
                    | "automation-edit"
                    | "automation-error"
                    | "automation-installed"
                    | "automation-installed-settings"
                    | "automation-installed-code"
                    | "automation-installed-review"
                    | "automation-scripts"
                    | "automation-scripts-empty"
                    | "automation-scripts-error"
                    | "automation-scripts-logs"
                    | "automation-scripts-values"
                    | "automation-scripts-sample"
                    | "automation-scripts-replace",
                ) => View::Automation,
                Ok("diagnostics" | "logs-empty" | "logs-error") => View::Logs,
                _ => View::Downloads,
            };
            if std::env::var("IDM_EX_TEST_VIEW").as_deref() == Ok("help") {
                app.dialog = Some(Dialog::About);
            }
            if let Ok(mode) = std::env::var("IDM_EX_TEST_VIEW") {
                if mode.starts_with("preferences") || mode.starts_with("settings") {
                    use preferences_window::Category;
                    app.open_preferences_window();
                    if let Some(state) = &mut app.preferences_window {
                        state.category = match mode.as_str() {
                            "preferences-downloads" => Category::Downloads,
                            "preferences-connection" => Category::Connection,
                            "preferences-browser" | "settings-bottom" => Category::Browser,
                            "preferences-mcp" => Category::Browser,
                            "preferences-antivirus" => Category::Antivirus,
                            "preferences-startup" | "settings-startup" => Category::Startup,
                            _ => Category::Appearance,
                        };
                    }
                    if mode == "preferences-mcp" {
                        app.dialog = Some(Dialog::Mcp);
                    }
                }
                match mode.as_str() {
                    "mcp" | "mcp-running" | "mcp-error" => app.open_mcp(),
                    "guide" => app.open_guide("Getting started"),
                    "guide-cli" => app.open_guide("Command line"),
                    "guide-automation" => app.open_guide("Scripting"),
                    "guide-lua" => app.open_guide("Lua scripting"),
                    "guide-mcp" => app.open_guide("MCP integration"),
                    "guide-troubleshooting" => app.open_guide("Troubleshooting"),
                    "changelog" => app.view = View::Changelog,
                    "legal" => app.dialog = Some(Dialog::Legal(about::LegalPage::Application)),
                    "legal-notices" => {
                        app.dialog = Some(Dialog::Legal(about::LegalPage::ThirdParty));
                    }
                    _ => {}
                }
            }
            if let Ok(mode) = std::env::var("IDM_EX_TEST_VIEW")
                && mode.starts_with("automation")
            {
                app.prepare_automation_capture(&mode);
            }
            if let Ok(mode) = std::env::var("IDM_EX_TEST_VIEW")
                && let Some(browser) = mode.strip_prefix("browsers")
            {
                // "browsers-brave" and the like open Browser setup on that browser.
                if let Some(slug) = browser.strip_prefix('-') {
                    app.browser = Browser::parse(slug).unwrap_or_default();
                }
                app.open_browser_setup();
            }
            if let Ok(mode) = std::env::var("IDM_EX_TEST_VIEW")
                && matches!(
                    mode.as_str(),
                    "new-queue" | "new-queue-sync" | "new-queue-error"
                )
            {
                app.view = View::Queues;
                app.open_queue_dialog(Dialog::NewQueue);
                app.queue_synchronize = mode == "new-queue-sync";
                if app.queue_synchronize {
                    app.queue_name = tr("Current files").into();
                } else if mode == "new-queue-error" {
                    app.submit_queue_dialog(Dialog::NewQueue);
                }
            }
            if std::env::var("IDM_EX_TEST_VIEW").as_deref() == Ok("logs-empty") {
                app.logs.search = tr("no matching event").into();
            }
            if let Ok(appearance) = std::env::var("IDM_EX_TEST_APPEARANCE") {
                app.interface.appearance = if appearance == "light" {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
            }
            if let Ok(preset) = std::env::var("IDM_EX_TEST_CUSTOM_THEME") {
                use download_engine::preferences::{CustomTheme, ThemePreset};
                let preset = match preset.as_str() {
                    "reactor" => ThemePreset::ReactorNight,
                    "aurora" => ThemePreset::Aurora,
                    "moonveil" => ThemePreset::Moonveil,
                    _ => ThemePreset::NeonCourier,
                };
                app.interface.appearance = Appearance::Custom;
                app.interface.custom_theme = CustomTheme {
                    preset,
                    accent: preset.accent(),
                    ..Default::default()
                };
                if let Some(state) = &mut app.preferences_window {
                    state.interface = app.interface;
                }
            }
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && std::env::var_os("IDM_EX_TEST_SCHEDULE").is_some()
        {
            app.open_queue_dialog(Dialog::Schedule);
            app.schedule_mode = ScheduleMode::SelectedDays;
            app.schedule_time = "23:00".into();
            app.schedule_stop = "07:00".into();
        }
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            && let Ok(mode) = std::env::var("IDM_EX_TEST_FEATURE")
        {
            app.feature_preview(&mode);
        }
        app
    }

    pub fn attach_tray(&mut self, ctx: &egui::Context) {
        let _language = download_engine::i18n::scope(self.interface.language);
        // Screenshot fixtures must not place sample notifications on the user's desktop.
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            || std::env::var_os("EFRAME_SCREENSHOT_TO").is_some()
        {
            return;
        }
        match Tray::new(&self.store.paths, ctx.clone()) {
            Ok(tray) => self.tray = Some(tray),
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Tray unavailable: {error}. Closing the window will exit and pause downloads.",
                    error = error
                )))
            }
        }
    }
    fn tray_available(&self) -> bool {
        self.tray.as_ref().is_some_and(Tray::available)
    }
    /// A Windows sign-in launch with "Start in the tray" hides the window in the first
    /// frame. eframe shows the window after painting that frame, so it stays cloaked
    /// (not drawn, no taskbar button) until the hide has been applied.
    pub fn start_in_tray(&mut self, owner: &impl raw_window_handle::HasWindowHandle) {
        self.hide_on_start = true;
        // Without cloaking the window can flash for one frame; hiding still works.
        #[cfg(windows)]
        {
            self.cloaked_start = window_frame::set_cloaked(owner, true).is_ok();
        }
        #[cfg(not(windows))]
        let _ = owner;
    }
    fn show_window(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        if self.tray_available() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else {
            self.notice = Some(Notice::error(
                tr("The tray is unavailable. Keep this window open to continue downloading.")
                    .into(),
            ));
            self.show_window(ctx);
        }
    }
    fn exit(&mut self, ctx: &egui::Context) {
        self.grabber = None;
        self.antivirus.service.cancel_all();
        self.exit_requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }
    fn tray_action(&mut self, action: TrayAction, ctx: &egui::Context) {
        match action {
            TrayAction::Open => self.show_window(ctx),
            TrayAction::BrowserSetup => {
                self.show_window(ctx);
                if self.dialog.is_none() {
                    self.open_browser_setup();
                } else {
                    self.notice = Some(Notice::info(
                        tr("Finish the current dialog, then choose Browser setup below.").into(),
                    ));
                }
            }
            TrayAction::Settings => {
                self.show_window(ctx);
                if self.dialog.is_none() {
                    self.open_preferences_window();
                } else {
                    self.notice = Some(Notice::info(
                        tr("Finish the current dialog, then open Preferences.").into(),
                    ));
                }
            }
            TrayAction::PauseAll => self.command(Command::PauseAll),
            TrayAction::Exit => self.exit(ctx),
            TrayAction::ReviewQueue(id) => self.review_queue(id, ctx),
            TrayAction::ScriptRun(run) => {
                self.show_window(ctx);
                if self.dialog.is_none() {
                    self.view = View::Automation;
                    self.automation.show_scripts();
                    self.scripts.show_run(run);
                    scripts::refresh(&self.store, &mut self.scripts);
                } else {
                    self.notice = Some(Notice::info(
                        tr("Finish the current dialog, then open the Scripts tab to see this run.")
                            .into(),
                    ));
                }
            }
            TrayAction::Review(id) => {
                self.pending_review = Some(id);
                self.show_window(ctx);
            }
            TrayAction::DropTarget => self.set_drop_target(ctx, !self.interface.drop_target),
            TrayAction::ClipboardLink => {
                self.show_window(ctx);
                self.review_clipboard_link();
            }
            TrayAction::Unavailable(message) => {
                self.show_window(ctx);
                self.notice = Some(Notice::error(tf!(
                    "Tray unavailable: {message}. This window will stay available.",
                    message = message
                )));
            }
        }
    }
    fn background(&mut self, ctx: &egui::Context) {
        let _language = download_engine::i18n::scope(self.interface.language);
        self.poll_theme_assets(ctx);
        self.poll_background_io(ctx);
        self.poll_browser_recovery(ctx);
        self.mcp.poll(&self.store, ctx);
        self.poll_diagnostic_export();
        self.poll_list_transfer();
        self.poll_browser_video_requests(ctx);
        self.poll_hls_requests(ctx);
        self.clipboard_background();
        self.drop_target_background(ctx);
        let actions = self.tray.as_ref().map(Tray::events).unwrap_or_default();
        for action in actions {
            self.tray_action(action, ctx);
        }
        // eframe shows the window after painting its first frame and applies this
        // frame's commands right after that, so the window does not stay on screen.
        // Without a tray icon it stays open with a notice instead of becoming unreachable.
        if std::mem::take(&mut self.hide_on_start) {
            self.hide_to_tray(ctx);
        }
        self.refresh();
        self.poll_link_fallback(ctx);
        if self.dialog.is_none()
            && let Some(id) = self.pending_review.take()
        {
            match self
                .store
                .get(&id)
                .and_then(|job| Ok((job, self.store.job_queue(&id)?)))
            {
                Ok((Some(job), Some(queue))) => {
                    self.queue_id = queue;
                    self.filter = Filter::All;
                    self.search.clear();
                    self.view = View::Downloads;
                    self.category = None;
                    self.last_refresh = Instant::now() - Duration::from_secs(1);
                    self.refresh();
                    self.select_download(ctx, &job);
                }
                Ok(_) => {
                    self.notice = Some(Notice::info(
                        tr("This download has been removed from history.").into(),
                    ))
                }
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
        }
        let finished: Vec<_> = self.scheduler.finished.try_iter().collect();
        for event in finished {
            match self.store.get(&event.job) {
                Ok(Some(job)) => {
                    if event.status == Status::Complete {
                        self.scan_finished_download(&job);
                    }
                    if let Some(failed) = event.outcome(&job) {
                        self.extras.sound.record(failed);
                    }
                    if let Some(notice) = event.notice(&job, self.preferences)
                        && let Some(tray) = &self.tray
                        && tray.available()
                        && let Err(error) = tray.notify(notice)
                    {
                        self.notice = Some(Notice::error(error.to_string()));
                    }
                }
                Ok(None) => {}
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
        }
        let script_notices: Vec<_> = self.scheduler.script_notices.try_iter().collect();
        for notice in script_notices {
            // The run log keeps the message even when notifications are off.
            if self.preferences.notify_scripts
                && let Some(tray) = &self.tray
                && tray.available()
                && let Err(error) = tray.notify_script(notice)
            {
                self.notice = Some(Notice::error(error.to_string()));
            }
        }
        self.antivirus_background();
        self.extras_background(ctx);
        let closing = ctx.input(|input| input.viewport().close_requested());
        if !closing {
            self.close_handled = false;
        }
        if closing
            && !self.exit_requested
            && self.preferences.close_to_tray
            && self.tray_available()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // Hidden egui logic retains the last input until another visible pass.
            // Do not hide again when a tray action is restoring that same window.
            if !self.close_handled {
                self.hide_to_tray(ctx);
                self.close_handled = true;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    fn command(&mut self, command: Command) {
        let starting = matches!(command, Command::Resume(_) | Command::StartImmediately(_));
        if let Command::Resume(id) | Command::StartImmediately(id) = &command
            && let Some(job) = self
                .jobs
                .iter()
                .find(|job| &job.id == id)
                .cloned()
                .or_else(|| self.cached_status_job(id))
            && self.open_link_fallback(&job)
        {
            return;
        }
        self.cancel_status_request(&command);
        let action = match &command {
            Command::Remove(id) => Some((DestructiveAction::Remove, id)),
            Command::Restart(id) => Some((DestructiveAction::Restart, id)),
            Command::DeleteFile { id, .. } => Some((DestructiveAction::DeleteFile, id)),
            _ => None,
        };
        if let Some((action, id)) = action {
            match self.store.get(id) {
                Ok(Some(job)) => self.request_destructive(action, vec![job]),
                Ok(None) => {}
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
            return;
        }
        if let Err(error) = self.scheduler.send(command) {
            self.notice = Some(Notice::error(error.to_string()));
        } else if starting {
            self.notice = Some(Notice::info(tr("Start requested.").into()));
            self.force_refresh();
        }
    }

    fn refresh(&mut self) {
        self.poll_download_status();
        let events: Vec<_> = self.scheduler.quota_events.try_iter().collect();
        for event in events {
            self.quota_event(event);
        }
        let events: Vec<_> = self.scheduler.bandwidth_events.try_iter().collect();
        for event in events {
            self.bandwidth_event(event);
        }
        let events: Vec<_> = self.scheduler.auth_events.try_iter().collect();
        for event in events {
            self.authentication_event(event);
        }
        let events: Vec<_> = self.scheduler.link_events.try_iter().collect();
        for event in events {
            self.link_event(event);
        }
        let errors: Vec<_> = self.scheduler.errors.try_iter().collect();
        for error in errors {
            self.status_command_failed(&error);
            self.notice = Some(Notice::error(error));
        }
        match self.scheduler.quota_snapshot() {
            Ok(status) => self.quota_status = status,
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot read data quotas: {error}",
                    error = error
                )))
            }
        }
        self.refresh_snapshot();
    }

    fn accepts_category(&self, job: &Job) -> bool {
        self.category
            .is_none_or(|category| job.category() == category)
    }
    #[cfg(test)]
    fn accepts(&self, job: &Job) -> bool {
        self.accepts_with_search(job, &self.search.trim().to_lowercase())
    }
    fn accepts_with_search(&self, job: &Job, search: &str) -> bool {
        self.filter.accepts(job.status)
            && self.accepts_category(job)
            && Self::matches_search(job, search)
    }
    fn matches_search(job: &Job, search: &str) -> bool {
        // The usual, unfiltered list does not need an allocated lowercase filename.
        search.is_empty() || job.name.to_lowercase().contains(search)
    }
    fn reconcile_selection(&mut self) {
        let visible = self.visible_job_ids();
        let visible_set: HashSet<_> = visible.iter().map(String::as_str).collect();
        self.selection
            .additional
            .retain(|id| visible_set.contains(id.as_str()));
        if !self
            .selected
            .as_ref()
            .is_some_and(|id| visible_set.contains(id.as_str()))
        {
            self.selected = visible
                .iter()
                .find(|id| self.selection.additional.contains(*id))
                .cloned()
                .or_else(|| {
                    (!self.selection.empty_by_choice)
                        .then(|| visible.first().cloned())
                        .flatten()
                });
        }
        if let Some(id) = &self.selected {
            self.selection.additional.remove(id);
        }
        if !self
            .selection
            .anchor
            .as_ref()
            .is_some_and(|id| visible_set.contains(id.as_str()))
        {
            self.selection.anchor = self.selected.clone();
        }
    }

    fn rate(&self, job: &Job) -> f64 {
        if job.status == Status::Downloading {
            self.rates.get(&job.id).map(|v| v.1).unwrap_or(0.0)
        } else {
            0.0
        }
    }

    fn folder_override(&self) -> Option<PathBuf> {
        let value = self.folder.trim();
        (!value.is_empty()).then(|| PathBuf::from(value))
    }
    fn load_download_folders(&mut self) {
        match self.store.download_folders() {
            Ok(rules) => self.download_folders = Some(rules),
            Err(error) => {
                self.download_folders = None;
                self.form_error = Some(tf!(
                    "Cannot load download folders: {error:#}",
                    error = error
                ));
            }
        }
    }
    fn show_folder(&mut self, job: &Job) {
        self.open_folder_async(job.directory.clone());
    }

    fn summary(&self) -> String {
        if self.filter == Filter::Complete {
            let count = self
                .jobs
                .iter()
                .filter(|j| j.status == Status::Complete)
                .count();
            return tf!(
                "{count} file{} saved and ready",
                if count == 1
                    || download_engine::i18n::current() != download_engine::i18n::Language::English
                {
                    ""
                } else {
                    "s"
                },
                count = count
            );
        }
        let mut parts = Vec::new();
        for (status, label) in [
            (Status::Downloading, tr("downloading")),
            (Status::Queued, tr("queued")),
            (Status::Paused, tr("paused")),
            (Status::Failed, tr("need attention")),
        ] {
            let count = self
                .jobs
                .iter()
                .filter(|j| {
                    j.status == status && !(status == Status::Queued && j.retry.next_at.is_some())
                })
                .count();
            if count > 0 {
                parts.push(format!(
                    "{count} {}",
                    if count == 1 && status == Status::Failed {
                        tr("needs attention")
                    } else {
                        label
                    }
                ));
            }
        }
        let waiting = self
            .jobs
            .iter()
            .filter(|j| j.status == Status::Queued && j.retry.next_at.is_some())
            .count();
        if waiting > 0 {
            parts.push(tf!("{waiting} waiting to retry", waiting = waiting));
        }
        if parts.is_empty() {
            tr("Your downloads are up to date").into()
        } else {
            parts.join("  ·  ")
        }
    }

    fn force_refresh(&mut self) {
        self.invalidate_snapshot();
        self.refresh();
    }
}

impl eframe::App for Desktop {
    #[cfg(feature = "screenshot")]
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_none() {
            return;
        }
        if std::env::var("IDM_EX_TEST_VIEW").as_deref() == Ok("settings-bottom") {
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(500.0, 450.0)));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, -1500.0),
                modifiers: egui::Modifiers::NONE,
            });
            return;
        }
        let menu_path = std::env::var("IDM_EX_TEST_MENU").ok();
        if std::env::var_os("IDM_EX_TEST_TOOLBAR").is_none() && menu_path.is_none() {
            return;
        }
        // Feed real egui input in isolated screenshot builds; never move the OS pointer.
        // Native cursor events would otherwise interrupt the simulated press.
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::PointerMoved(_)
                    | egui::Event::PointerButton { .. }
                    | egui::Event::PointerGone
            )
        });
        if menu_path.is_some() {
            if let Some((rect, _, _)) = ctx.data(|data| {
                data.get_temp::<(egui::Rect, usize, u64)>(egui::Id::new("menu-capture-target"))
            }) {
                input.events.push(egui::Event::PointerMoved(rect.center()));
            }
            return;
        }
        if let Some((id, rect)) = ctx.data(|data| {
            data.get_temp::<(egui::Id, egui::Rect)>(egui::Id::new("toolbar-capture-target"))
        }) {
            let mode = std::env::var("IDM_EX_TEST_TOOLBAR_STATE").unwrap_or_default();
            if mode == "focus" {
                // Simulate a focused test window without taking OS focus from the user.
                input.focused = true;
                if let Some(viewport) = input.viewports.get_mut(&input.viewport_id) {
                    viewport.focused = Some(true);
                }
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::WindowFocused(_)));
                input.events.push(egui::Event::PointerGone);
                ctx.memory_mut(|memory| memory.request_focus(id));
            } else {
                input.events.push(egui::Event::PointerMoved(rect.center()));
                if mode == "pressed"
                    && ctx
                        .read_response(id)
                        .is_some_and(|response| response.hovered())
                    && !ctx.input(|state| state.pointer.primary_down())
                {
                    input.events.push(egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
    }
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Uncloak in the frame after the sign-in hide was applied. A failure is retried:
        // a cloaked window would stay invisible when opened from the tray.
        #[cfg(windows)]
        if self.cloaked_start && !self.hide_on_start {
            self.cloaked_start = window_frame::set_cloaked(frame, false).is_err();
        }
        #[cfg(not(windows))]
        let _ = frame;
        self.background(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.folder_picker.set_owner(frame);
        self.draw(ui);
        #[cfg(windows)]
        self.sync_window_frame(ui.ctx(), frame);
        #[cfg(feature = "screenshot")]
        if !std::env::var("IDM_EX_TEST_FEATURE").is_ok_and(|mode| mode.starts_with("status-native"))
        {
            self.capture_settled_test_frame(ui.ctx());
        }
    }
}

// Test-only GPU capture after modal sizing has settled. No screen-reading API
// or additional image dependency is needed; the smoke script converts PPM to PNG.
#[cfg(feature = "screenshot")]
impl Desktop {
    fn capture_settled_test_frame(&mut self, ctx: &egui::Context) {
        use std::io::Write;
        let Some(path) = std::env::var_os("IDM_EX_SCREENSHOT_TO") else {
            return;
        };
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        let captured = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = captured {
            let mut file = File::create(path).expect("Cannot create test screenshot");
            write!(file, "P6\n{} {}\n255\n", image.width(), image.height())
                .expect("Cannot write screenshot header");
            let bytes: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| [pixel.r(), pixel.g(), pixel.b()])
                .collect();
            file.write_all(&bytes)
                .expect("Cannot write screenshot pixels");
            file.sync_all().expect("Cannot save test screenshot");
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        } else if !self.screenshot_requested
            && !self.theme_assets.busy()
            && (self.view != View::Logs || self.logs.loaded())
            && (self.dialog != Some(Dialog::Mcp) || self.mcp.capture_ready())
            && (!std::env::var("IDM_EX_TEST_VIEW")
                .is_ok_and(|view| view.starts_with("automation-scripts"))
                || self.scripts.capture_ready())
        {
            if let Ok(path) = std::env::var("IDM_EX_TEST_MENU") {
                let target = ctx.data(|data| {
                    data.get_temp::<(egui::Rect, usize, u64)>(egui::Id::new("menu-capture-target"))
                });
                let ready = target.is_some_and(|(rect, depth, frame)| {
                    depth + 1 == path.split('|').count()
                        && frame == ctx.cumulative_frame_nr()
                        && ctx.input(|input| {
                            input
                                .pointer
                                .hover_pos()
                                .is_some_and(|pos| rect.contains(pos))
                        })
                });
                if !ready {
                    ctx.data_mut(|data| {
                        data.remove::<u64>(egui::Id::new("screenshot-ready-frame"));
                    });
                    ctx.request_repaint();
                    return;
                }
            }
            // Async pages need settled frames after their data arrives too.
            let frame = ctx.cumulative_frame_nr();
            let ready = ctx.data_mut(|data| {
                *data.get_temp_mut_or_insert_with(egui::Id::new("screenshot-ready-frame"), || frame)
            });
            if frame >= ready + 3 {
                self.screenshot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod interaction_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ux_tests;

#[cfg(test)]
mod polish_tests;

#[cfg(test)]
mod cards_login_tests;

#[cfg(test)]
mod alignment_tests;

#[cfg(all(test, windows))]
mod add_flow_tests;

#[cfg(all(test, windows))]
mod memory_tests;

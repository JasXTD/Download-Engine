use super::*;
use download_engine::antivirus::{
    self as av, OpenPermit, Provider, Report, Service, Settings, Verdict,
};
mod batch;
pub(super) use batch::ScanMode;

struct Draft {
    settings: Settings,
    arguments: String,
    clean: String,
    detected: String,
    setup: Option<Result<std::path::PathBuf, String>>,
}

impl Draft {
    fn new(settings: Settings) -> Self {
        Self {
            arguments: settings.arguments.join("\n"),
            clean: settings
                .clean_exit
                .map(|n| n.to_string())
                .unwrap_or_default(),
            detected: settings
                .detected_exit
                .map(|n| n.to_string())
                .unwrap_or_default(),
            settings,
            setup: None,
        }
    }
    fn value(&self) -> anyhow::Result<Settings> {
        let mut value = self.settings.clone();
        value.arguments = self.arguments.lines().map(str::to_owned).collect();
        if value.provider == Provider::Custom && value.enabled {
            value.clean_exit = Some(self.clean.trim().parse().map_err(|_| {
                anyhow::anyhow!(tr("Enter the scanner's numeric no-threat exit code"))
            })?);
            value.detected_exit = Some(self.detected.trim().parse().map_err(|_| {
                anyhow::anyhow!(tr("Enter the scanner's numeric threat exit code"))
            })?);
        } else {
            value.clean_exit = self.clean.trim().parse().ok();
            value.detected_exit = self.detected.trim().parse().ok();
        }
        Ok(value)
    }
}
struct Review {
    job: Job,
    report: Option<Report>,
    permit: Option<OpenPermit>,
    error: Option<String>,
    waiting: bool,
    external_ready: bool,
    external_confirmed: bool,
}
#[derive(Default)]
pub(super) struct State {
    pub service: Service,
    draft: Option<Draft>,
    review: Option<Review>,
    // Only the selected download's details use this cache. Other reports stay
    // in SQLite, so visiting/removing downloads cannot retain them indefinitely.
    report: Option<Report>,
    loaded_selection: Option<String>,
    loading_report: Option<(String, background_io::Task<Option<Report>>)>,
    batch: Option<batch::Batch>,
}

impl Desktop {
    pub(super) fn poll_antivirus_report(&mut self) {
        let Some((id, task)) = &mut self.antivirus.loading_report else {
            return;
        };
        let Some(result) = task.poll() else {
            return;
        };
        let current = self.selected.as_ref() == Some(id);
        self.antivirus.loading_report = None;
        if current {
            match result {
                Ok(report) if self.antivirus.report.is_none() => self.antivirus.report = report,
                Ok(_) => {}
                Err(error) => {
                    self.notice = Some(Notice::error(tf!(
                        "Cannot read scan history: {error}",
                        error = error
                    )))
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn antivirus_report_pending(&self) -> bool {
        self.antivirus.loading_report.is_some()
    }

    pub(super) fn antivirus_background(&mut self) {
        self.poll_antivirus_report();
        if self.exit_requested {
            self.antivirus.service.cancel_all();
            return;
        }
        let mut close_review = false;
        if let Some((id, owner, result)) = self.antivirus.service.poll_owned(&self.store) {
            if let Some(batch) = &mut self.antivirus.batch
                && owner == Some(batch.id)
            {
                batch.completed(&id, &result);
            }
            match result {
                Ok(mut outcome) => {
                    if self.selected.as_deref() == Some(&id) {
                        self.antivirus.report = Some(outcome.report.clone());
                    }
                    if let Some(review) = &mut self.antivirus.review
                        && review.job.id == id
                    {
                        if outcome.report.verdict == Verdict::ScanRequested
                            && outcome.permit.is_some()
                        {
                            match outcome.permit.take().unwrap().open(&self.store) {
                                Ok(()) => close_review = true,
                                Err(error) => {
                                    review.error =
                                        Some(tf!("Cannot open file: {error}", error = error));
                                    review.external_ready = false;
                                    review.external_confirmed = false;
                                    review.waiting = false;
                                }
                            }
                        } else {
                            review.external_ready =
                                outcome.report.verdict == Verdict::ScanRequested;
                            review.external_confirmed = false;
                            review.report = Some(outcome.report);
                            review.permit = outcome.permit;
                            review.waiting = self.antivirus.service.status(&id).is_some();
                        }
                    }
                }
                Err(error) => {
                    if let Some(review) = &mut self.antivirus.review
                        && review.job.id == id
                    {
                        review.error = Some(error.to_string());
                        review.permit = None;
                        review.external_ready = false;
                        review.external_confirmed = false;
                        review.waiting = self.antivirus.service.status(&id).is_some();
                    }
                    self.notice = Some(Notice::error(tf!("Antivirus: {error}", error = error)));
                }
            }
        }
        if close_review {
            self.close_scan_review();
        }
        self.advance_antivirus_batch();
        if self.selected != self.antivirus.loaded_selection {
            self.antivirus.report = None;
            if self.antivirus.loading_report.is_none() {
                self.antivirus.loaded_selection = self.selected.clone();
                if let Some(id) = self.selected.clone() {
                    let store = self.store.clone();
                    let job = id.clone();
                    match background_io::Task::spawn(
                        self.background_io.context.clone(),
                        move || store.antivirus_report(&job),
                    ) {
                        Ok(task) => self.antivirus.loading_report = Some((id, task)),
                        Err(error) => {
                            self.notice = Some(Notice::error(tf!(
                                "Cannot read scan history: {error}",
                                error = error
                            )))
                        }
                    }
                }
            }
        }
    }
    pub(super) fn antivirus_label(&self, job: &Job) -> String {
        if let Some(status) = self.antivirus.service.status(&job.id) {
            return status.into();
        }
        self.antivirus
            .report
            .as_ref()
            .filter(|report| report.matches_file(job))
            .map(|r| {
                if r.verdict == Verdict::ScanRequested {
                    tr(r.verdict.label()).into()
                } else {
                    tf!("{} (last scan)", tr(r.verdict.label()))
                }
            })
            .unwrap_or_else(|| tr("Not scanned").into())
    }
    pub(super) fn scan_finished_download(&mut self, job: &Job) {
        if self.exit_requested || job.status != Status::Complete {
            return;
        }
        match self.store.antivirus_settings() {
            Ok(settings) if settings.enabled => {
                if job.synchronization.as_ref().is_some_and(|state| {
                    state.phase == download_engine::synchronization::Phase::UpToDate
                }) {
                    // An unchanged check is not a new download. Do still scan if
                    // completion raced the first automatic scan or its result
                    // belongs to another saved version.
                    match self.store.antivirus_report(&job.id) {
                        Ok(Some(report))
                            if report.matches_file(job)
                                && report.sha256.is_some()
                                && report.sha256 == job.integrity.actual =>
                        {
                            return;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            self.notice = Some(Notice::error(tf!(
                                "Cannot read scan history: {error}",
                                error = error
                            )));
                            return;
                        }
                    }
                }
                self.antivirus.service.enqueue(job.clone(), settings, false)
            }
            Ok(_) => {}
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Automatic scan unavailable: {error}",
                    error = error
                )))
            }
        }
    }
    #[cfg(feature = "screenshot")]
    pub(super) fn antivirus_settings_preview(&mut self, mode: &str) {
        self.open_antivirus_settings();
        let draft = self.antivirus.draft.as_mut().unwrap();
        draft.settings.provider = match mode {
            "scan-settings-kaspersky" => Provider::Kaspersky,
            "scan-settings-bitdefender" => Provider::Bitdefender,
            "scan-settings-malwarebytes" => Provider::Malwarebytes,
            "scan-settings-custom" => Provider::Custom,
            _ => Provider::Defender,
        };
    }
    #[cfg(feature = "screenshot")]
    pub(super) fn antivirus_result_preview(&mut self) {
        if let Some(job) = self
            .jobs
            .iter()
            .find(|job| job.status == Status::Complete)
            .cloned()
        {
            self.review_scan(&job, false);
            let review = self.antivirus.review.as_mut().unwrap();
            review.external_ready = true;
            review.report = Some(Report {
                job_id: job.id.clone(),
                path: job.saved_path.clone().unwrap(),
                scanner: Provider::Kaspersky.label().into(),
                settings_key: "preview".into(),
                verdict: Verdict::ScanRequested,
                message: tr("Illustrative request for UI preview only").into(),
                sha256: None,
                finished_at: 1_790_000_000,
            });
        }
    }
    pub(super) fn open_antivirus_settings(&mut self) {
        if self.dialog.is_some() {
            self.notice = Some(Notice::info(
                tr("Finish the current dialog before opening scan settings.").into(),
            ));
            return;
        }
        let settings = match self.store.antivirus_settings() {
            Ok(settings) => {
                self.form_error = None;
                settings
            }
            Err(error) => {
                self.form_error = Some(tf!(
                    "{error}. Choose valid settings and save to repair them.",
                    error = error
                ));
                Settings::default()
            }
        };
        self.antivirus.draft = Some(Draft::new(settings));
        self.dialog = Some(Dialog::AntivirusSettings);
    }
    pub(super) fn review_scan(&mut self, job: &Job, start: bool) {
        if self.dialog.is_some() && self.dialog != Some(Dialog::AntivirusResult) {
            self.notice = Some(Notice::info(
                tr("Finish the current dialog before scanning this file.").into(),
            ));
            return;
        }
        let (report, error) = match self.store.antivirus_report(&job.id) {
            Ok(report) => (report.filter(|report| report.matches_file(job)), None),
            Err(error) => (None, Some(error.to_string())),
        };
        self.antivirus.review = Some(Review {
            job: job.clone(),
            report,
            permit: None,
            error,
            waiting: false,
            external_ready: false,
            external_confirmed: false,
        });
        self.dialog = Some(Dialog::AntivirusResult);
        if start {
            self.start_review_scan();
        }
    }
    fn start_review_scan(&mut self) {
        let Some(review) = &mut self.antivirus.review else {
            return;
        };
        review.permit = None;
        review.error = None;
        review.external_ready = false;
        review.external_confirmed = false;
        match self.store.antivirus_settings().and_then(|s| {
            s.validate()?;
            Ok(s)
        }) {
            Ok(settings) => {
                self.antivirus
                    .service
                    .enqueue(review.job.clone(), settings, true);
                review.waiting = true;
            }
            Err(error) => {
                review.waiting = false;
                review.error = Some(error.to_string());
            }
        }
    }
    fn close_scan_review(&mut self) {
        if let Some(review) = self.antivirus.review.take()
            && review.waiting
        {
            self.antivirus.service.cancel(&review.job.id);
        }
        self.dialog = None;
    }
    pub(super) fn antivirus_settings_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(draft) = &mut self.antivirus.draft else {
            self.dialog = None;
            return;
        };
        let (mut save, mut cancel, mut open_app, mut check) = (false, false, false, false);
        let response = form_dialog::modal("antivirus-settings", p).show(ctx, |ui| {
                ui.set_width(510.0_f32.min(ctx.content_rect().width()-64.0));
                cancel = form_dialog::title(ui, tr("Antivirus scanning"), form_dialog::Icon::Symbol("\u{ea18}"), p, true);
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("scan-settings-scroll").max_height((ctx.content_rect().height()-190.0).max(180.0)).show(ui, |ui| {
                    let external = draft.settings.provider.external_review();
                    ui.square_checkbox(&mut draft.settings.enabled, if external { tr("Automatically request antivirus scans") } else { tr("Scan downloads before opening") });
                    ui.label(RichText::new(if external { tr("Requests a scan when a download finishes. Open file requests a fresh scan, then asks you to confirm that you reviewed the result in the antivirus app.") } else { tr("Also scans new downloads on completion. Open file runs a fresh scan and shows its result for your confirmation.") }).small().color(p.muted));
                    ui.add_space(10.0);
                    ui.label(tr("Scanner"));
                    let previous = draft.settings.provider;
                    egui::ComboBox::from_id_salt("av-provider").selected_text(tr(draft.settings.provider.label())).width(310.0).show_ui(ui, |ui| {
                        for provider in Provider::ALL {
                            ui.selectable_value(&mut draft.settings.provider, provider, tr(provider.label()));
                        }
                    });
                    if previous != draft.settings.provider { draft.settings.custom_confirmed = false; draft.setup = None; self.form_error = None; }
                    ui.horizontal(|ui| {
                        check = ui.button(tr("Check setup")).clicked();
                        if draft.settings.provider.external_review() { open_app = ui.button(tr("Open antivirus")).on_hover_text(draft.settings.provider.name()).clicked(); }
                        let guide = draft.settings.provider.guide_url();
                        if !guide.is_empty() { ui.hyperlink_to(tr("Scanner guide"), guide); }
                    });
                    if let Some(result) = &draft.setup {
                        match result {
                            Ok(path) => {
                                ui.label(RichText::new(tr("Integration found; scan availability is checked when requested.")).small().color(p.accent));
                                ui.add(egui::Label::new(RichText::new(path.display().to_string()).small().color(p.muted)).truncate()).on_hover_text(path.display().to_string());
                            }
                            Err(error) => { ui.label(RichText::new(error).small().color(p.warning)); }
                        }
                    }
                    ui.add_space(8.0);
                    match draft.settings.provider {
                        Provider::Defender => {
                            ui.label(RichText::new(tr("Uses Microsoft's installed scanner without changing your antivirus settings. If Defender is inactive, the result will explain that scanning is unavailable.")).small().color(p.muted));
                        }
                        Provider::Malwarebytes | Provider::Kaspersky | Provider::Bitdefender => {
                            ui.label(RichText::new(tr("Review the result in the selected antivirus app.")).small().color(p.muted));
                            ui.label(RichText::new(tr("Requires the English-language antivirus Explorer integration. The timeout limits the handoff, not the scan. Cancel scans in the antivirus app.")).small().color(p.muted));
                            ui.label(RichText::new(tr("Default installation folder required.")).small().color(p.muted));
                            ui.label(RichText::new(tr("The antivirus app controls protection and quarantine. A request is not a clean result.")).small().color(p.muted));
                        }
                        Provider::ClamAv | Provider::Custom => {
                            ui.label(if draft.settings.provider == Provider::ClamAv { tr("Full path to clamscan.exe") } else { tr("Full path to the antivirus scanner .exe") });
                            if ui.add(view::line_edit(&mut draft.settings.executable).desired_width(f32::INFINITY).hint_text(r"C:\Program Files\ClamAV\clamscan.exe")).changed() { draft.settings.custom_confirmed = false; draft.setup = None; }
                            if draft.settings.provider == Provider::ClamAv {
                                ui.label(RichText::new(tr("Install ClamAV and update its signatures with FreshClam first. Encrypted files and scan-limit alerts block opening.")).small().color(p.muted));
                            } else {
                                ui.label(tr("Arguments — one per line; include {file} once"));
                                if ui.add(egui::TextEdit::multiline(&mut draft.arguments).desired_rows(3).desired_width(f32::INFINITY)).changed() { draft.settings.custom_confirmed = false; draft.setup = None; }
                                ui.label(RichText::new(tr("Do not add shell quoting around {file}. Each line is passed as one argument. Use the vendor's single-file, scan-only command.")).small().color(p.muted));
                                ui.horizontal(|ui| {
                                    ui.label(tr("No-threat exit code"));
                                    if ui.add(view::line_edit(&mut draft.clean).desired_width(48.0)).changed() { draft.settings.custom_confirmed = false; draft.setup = None; }
                                    ui.label(tr("Threat exit code"));
                                    if ui.add(view::line_edit(&mut draft.detected).desired_width(48.0)).changed() { draft.settings.custom_confirmed = false; draft.setup = None; }
                                });
                                ui.square_checkbox(&mut draft.settings.custom_confirmed, tr("I verified these scan arguments and result codes"));
                                ui.label(RichText::new(tr("The scanner must wait until scanning finishes, and report skipped or incomplete scans as errors. GUI launchers cannot provide this result.")).small().color(p.muted));
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr("Timeout (seconds)"));
                        ui.add(egui::DragValue::new(&mut draft.settings.timeout_seconds).range(10..=3600));
                    });
                    ui.add_space(10.0);
                    ui.separator();
                    if let Some(error) = &self.form_error { ui.label(RichText::new(error).color(p.warning)); }
                });
                save = form_dialog::footer(ui, tr("Save scan settings"), None, p);
            });
        if check {
            draft.setup = Some(
                draft
                    .value()
                    .and_then(|settings| av::check_setup(&settings))
                    .map_err(|error| error.to_string()),
            );
        }
        if open_app && let Err(error) = av::open_antivirus(draft.settings.provider) {
            self.form_error = Some(error.to_string());
        }
        if cancel || response.should_close() {
            self.antivirus.draft = None;
            self.dialog = None;
            self.form_error = None;
        } else if save {
            match draft.value().and_then(|s| {
                self.store.save_antivirus_settings(&s)?;
                Ok(s)
            }) {
                Ok(_) => {
                    self.antivirus.service.cancel_all();
                    self.note_advanced_preferences_saved();
                    self.antivirus.review = None;
                    self.antivirus.draft = None;
                    self.dialog = None;
                    self.form_error = None;
                    self.notice = Some(Notice::success(tr("Scan settings saved. They apply to new downloads and the next Open file action.").into()));
                }
                Err(error) => self.form_error = Some(error.to_string()),
            }
        }
    }
    pub(super) fn antivirus_result_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(review) = &mut self.antivirus.review else {
            self.dialog = None;
            return;
        };
        let provider = review.report.as_ref().and_then(Provider::from_report);
        let waiting = review.waiting;
        let external = review.external_ready;
        let can_open = !waiting
            && review.error.is_none()
            && (review.permit.is_some() || (external && review.external_confirmed));
        let (mut open, mut scan, mut cancel, mut close, mut folder, mut open_app) =
            (false, false, false, false, false, false);
        let response = form_dialog::modal("antivirus-result", p).show(ctx, |ui| {
                ui.set_width(510.0_f32.min(ctx.content_rect().width()-64.0));
                close = form_dialog::title(ui, tr("Antivirus scan result"), form_dialog::Icon::Symbol("\u{ea18}"), p, true);
                ui.label(RichText::new(&review.job.name).strong());
                egui::ScrollArea::vertical().id_salt("scan-result-scroll").max_height((ctx.content_rect().height()-230.0).max(150.0)).show(ui, |ui| {
                    if waiting {
                        ui.horizontal(|ui| { ui.spinner(); ui.label(self.antivirus.service.status(&review.job.id).unwrap_or(tr("Preparing scan…"))); });
                        ui.label(tr("The file will stay closed."));
                    } else if let Some(report) = &review.report {
                        ui.label(RichText::new(tr(report.verdict.label())).strong().color(if report.verdict == Verdict::NoThreatsFound { p.text } else { p.warning }));
                        let at = chrono::DateTime::from_timestamp(report.finished_at, 0).map(|v| v.with_timezone(&chrono::Local).format("%d %b %Y, %H:%M:%S").to_string()).unwrap_or_else(|| tr("Unknown time").into());
                        ui.label(format!("{} · {at}", report.scanner));
                        ui.label(&report.message);
                        if let Some(hash) = &report.sha256 { ui.collapsing(if report.verdict == Verdict::ScanRequested { tr("File SHA-256 when requested") } else { tr("Scanned file SHA-256") }, |ui| { ui.label(hash); }); }
                    } else { ui.label(tr("This file has not been scanned.")); }
                    if let Some(error) = &review.error { ui.label(RichText::new(error).color(p.warning)); }
                    if !waiting && external {
                        ui.square_checkbox(&mut review.external_confirmed, tr("I reviewed this file's result in the antivirus app"));
                        ui.label(RichText::new(tr("Opening after this confirmation relies on your review. Download Engine cannot verify the external scan result.")).small().color(p.muted));
                    } else if !waiting && !can_open { ui.label(RichText::new(tr("Request a fresh scan before opening. External scanners require your review in their app.")).small().color(p.muted)); }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        folder = ui.button(tr("Open folder")).clicked();
                        if let Some(provider) = provider.filter(|provider| provider.external_review()) {
                            open_app = ui.button(tr("Open antivirus")).on_hover_text(provider.name()).clicked();
                        }
                    });
                });
                form_dialog::controls(ui, |ui| {
                    open = form_dialog::primary_enabled(ui, can_open, if external { tr("Open after review") } else { tr("Open file") }).clicked();
                    if waiting { cancel = form_dialog::secondary_button(ui, tr("Cancel scan")).clicked(); }
                    else { scan = form_dialog::secondary_button(ui, tr("Scan again")).clicked(); }
                });
            });
        if folder
            && let Some(path) = &review.job.saved_path
            && let Some(parent) = path.parent()
        {
            let result = download_engine::paths::file_browser()
                .and_then(|exe| Ok(std::process::Command::new(exe).arg(parent).spawn()?));
            if let Err(error) = result {
                review.error = Some(error.to_string());
            }
        }
        if open_app
            && let Some(provider) = provider
            && let Err(error) = av::open_antivirus(provider)
        {
            review.error = Some(error.to_string());
        }
        if close || response.should_close() {
            self.close_scan_review();
        } else if cancel {
            self.antivirus.service.cancel(&review.job.id);
            review.permit = None;
            review.external_ready = false;
            review.external_confirmed = false;
            review.waiting = false;
            review.error = Some(tr("Scan cancelled. Scan again before opening.").into());
        } else if scan {
            self.start_review_scan();
        } else if open && external && review.external_confirmed {
            if let Some(report) = review.report.clone() {
                self.antivirus
                    .service
                    .enqueue_external_review(review.job.clone(), report);
                review.waiting = true;
                review.error = None;
                review.permit = None;
            } else if let Some(permit) = review.permit.take() {
                review.error = Some(tr("Missing scan request").into());
                drop(permit);
            } else {
                review.error = Some(tr("Missing scan request").into());
            }
        } else if open {
            let result = if let Some(permit) = review.permit.take() {
                permit.open(&self.store)
            } else {
                Err(anyhow::anyhow!(tr("Review a fresh scan before opening")))
            };
            match result {
                Ok(()) => self.close_scan_review(),
                Err(error) => review.error = Some(tf!("Cannot open file: {error}", error = error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;

    #[test]
    fn settings_show_only_selected_provider_actions_and_cancel_preserves_settings() {
        let (_temp, mut app, ctx) = fixture();
        let original = app.store.antivirus_settings().unwrap();
        app.open_antivirus_settings();
        for provider in Provider::ALL {
            let current = app.antivirus.draft.as_ref().unwrap().settings.provider;
            if current != provider {
                click(&mut app, &ctx, current.label());
                click(&mut app, &ctx, provider.label());
            }
            frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
            let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
            text_rect(&output, "Check setup");
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                labels.contains(&"Open antivirus"),
                provider.external_review()
            );
            assert!(!labels.contains(&"Open Malwarebytes"));
        }
        // A setup check validates configuration without invoking any antivirus.
        click(&mut app, &ctx, "Check setup");
        assert!(
            app.antivirus
                .draft
                .as_ref()
                .unwrap()
                .setup
                .as_ref()
                .unwrap()
                .is_err()
        );
        assert!(!app.antivirus.service.busy());
        click(&mut app, &ctx, "×");
        assert_eq!(app.store.antivirus_settings().unwrap(), original);
    }

    fn saved_job(app: &mut Desktop) -> Job {
        // Keep the background details-panel Open button out of text hit testing.
        app.interface.show_details = false;
        let mut job = app
            .jobs
            .iter()
            .find(|j| j.status == Status::Complete)
            .unwrap()
            .clone();
        let path = app.store.paths.downloads.join("scan-ui-fixture.txt");
        std::fs::write(&path, b"harmless UI fixture").unwrap();
        job.saved_path = Some(path);
        app.store.enqueue(&job).unwrap();
        job
    }
    #[test]
    fn removed_download_releases_cached_scan_report() {
        let (temp, mut app, _ctx) = fixture();
        let job = saved_job(&mut app);
        let report = Report {
            job_id: job.id.clone(),
            path: job.saved_path.clone().unwrap(),
            scanner: "Isolated test result".into(),
            settings_key: "fixture".into(),
            verdict: Verdict::NoThreatsFound,
            message: "Saved scan details".repeat(1024),
            sha256: None,
            finished_at: 1_790_000_000,
        };
        let connection =
            crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
                .unwrap();
        connection
            .execute(
                "INSERT INTO antivirus_reports(job_id,data) VALUES(?1,?2)",
                rusqlite::params![job.id, serde_json::to_string(&report).unwrap()],
            )
            .unwrap();
        drop(connection);
        app.selected = Some(job.id.clone());
        app.antivirus_background();
        app.wait_for_io();
        assert!(app.antivirus_label(&job).contains("last scan"));
        app.selected = None;
        app.antivirus_background();
        app.wait_for_io();
        assert!(app.antivirus.report.is_none());
        // Releasing the UI cache must not discard the persisted scan result.
        app.selected = Some(job.id.clone());
        app.antivirus_background();
        app.wait_for_io();
        assert!(app.antivirus_label(&job).contains("last scan"));
        app.command(Command::Remove(job.id.clone()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.store.get(&job.id).unwrap().is_some() {
            assert!(Instant::now() < deadline, "Removal did not settle");
            std::thread::sleep(Duration::from_millis(10));
        }
        app.refresh_and_wait();
        app.antivirus_background();
        app.wait_for_io();
        assert!(app.store.antivirus_report(&job.id).unwrap().is_none());
        assert!(
            app.antivirus.report.is_none(),
            "Removed scan details remain in memory"
        );
        drop(app);
        temp.close().unwrap();
    }
    #[test]
    fn scan_settings_cancel_and_save_are_separate_and_persist() {
        let (_temp, mut app, ctx) = fixture();
        app.open_antivirus_settings();
        click(&mut app, &ctx, "Scan downloads before opening");
        click(&mut app, &ctx, "×");
        assert!(!app.store.antivirus_settings().unwrap().enabled);
        app.open_antivirus_settings();
        click(&mut app, &ctx, "Scan downloads before opening");
        click(&mut app, &ctx, "Save scan settings");
        assert!(app.store.antivirus_settings().unwrap().enabled);
        assert!(app.dialog.is_none());
    }
    #[test]
    fn opening_with_scanning_enabled_queues_a_scan_and_keeps_the_file_closed() {
        let (_temp, mut app, ctx) = fixture();
        let job = saved_job(&mut app);
        app.store
            .save_antivirus_settings(&Settings {
                enabled: true,
                ..Default::default()
            })
            .unwrap();
        app.open_file(&job);
        assert_eq!(app.dialog, Some(Dialog::AntivirusResult));
        assert_eq!(
            app.antivirus.service.status(&job.id),
            Some("Waiting to scan…")
        );
        assert!(app.antivirus.review.as_ref().unwrap().permit.is_none());
        click(&mut app, &ctx, "Open file");
        assert_eq!(app.dialog, Some(Dialog::AntivirusResult));
        click(&mut app, &ctx, "Cancel scan");
        assert!(!app.antivirus.service.busy());
        assert!(
            app.antivirus
                .review
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("cancelled")
        );
    }
    #[test]
    fn historical_clean_result_is_visible_but_does_not_enable_open() {
        let (_temp, mut app, ctx) = fixture();
        let job = saved_job(&mut app);
        app.review_scan(&job, false);
        let review = app.antivirus.review.as_mut().unwrap();
        review.report = Some(Report {
            job_id: job.id.clone(),
            path: job.saved_path.clone().unwrap(),
            scanner: "Fixture scanner".into(),
            settings_key: "fixture".into(),
            verdict: Verdict::NoThreatsFound,
            message: "Historical fixture result".into(),
            sha256: None,
            finished_at: 1_790_000_000,
        });
        frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        let output = frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        text_rect(&output, "No threats found");
        click(&mut app, &ctx, "Open file");
        assert_eq!(app.dialog, Some(Dialog::AntivirusResult));
        assert!(app.antivirus.review.as_ref().unwrap().permit.is_none());
    }
    #[test]
    fn unreadable_settings_block_open_and_can_be_repaired_explicitly() {
        let (_temp, mut app, ctx) = fixture();
        let job = saved_job(&mut app);
        let conn =
            crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
                .unwrap();
        conn.execute("UPDATE antivirus_settings SET data='invalid'", [])
            .unwrap();
        app.open_file(&job);
        assert!(app.dialog.is_none());
        assert!(app.notice.as_ref().unwrap().contains("Cannot open file"));
        app.open_antivirus_settings();
        assert!(app.form_error.is_some());
        click(&mut app, &ctx, "Save scan settings");
        assert!(!app.store.antivirus_settings().unwrap().enabled);
    }
    #[test]
    fn malwarebytes_option_saves_and_completed_downloads_queue_external_requests() {
        for external_provider in [
            Provider::Malwarebytes,
            Provider::Kaspersky,
            Provider::Bitdefender,
        ] {
            let (_temp, mut app, ctx) = fixture();
            let job = saved_job(&mut app);
            app.open_antivirus_settings();
            app.antivirus.draft.as_mut().unwrap().settings.provider = external_provider;
            click(&mut app, &ctx, "Automatically request antivirus scans");
            click(&mut app, &ctx, "Save scan settings");
            let settings = app.store.antivirus_settings().unwrap();
            assert!(settings.enabled);
            assert_eq!(settings.provider, external_provider);
            app.scan_finished_download(&job);
            assert_eq!(
                app.antivirus.service.status(&job.id),
                Some("Waiting to request antivirus scan…")
            );
            app.antivirus.service.cancel_all();
        }
    }
    #[test]
    fn external_request_needs_explicit_manual_review_and_saved_reports_never_enable_open() {
        for external_provider in [
            Provider::Malwarebytes,
            Provider::Kaspersky,
            Provider::Bitdefender,
        ] {
            let (_temp, mut app, ctx) = fixture();
            let job = saved_job(&mut app);
            app.review_scan(&job, false);
            let review = app.antivirus.review.as_mut().unwrap();
            review.report = Some(Report {
                job_id: job.id.clone(),
                path: job.saved_path.clone().unwrap(),
                scanner: external_provider.label().into(),
                settings_key: "fixture".into(),
                verdict: Verdict::ScanRequested,
                message: "Review in Malwarebytes".into(),
                sha256: None,
                finished_at: 1_790_000_000,
            });
            click(&mut app, &ctx, "Open file");
            assert!(app.antivirus.review.as_ref().unwrap().error.is_none());
            assert!(!app.antivirus.review.as_ref().unwrap().external_ready);
            app.antivirus.review.as_mut().unwrap().external_ready = true;
            click(&mut app, &ctx, "Open after review");
            assert!(app.antivirus.review.as_ref().unwrap().error.is_none());
            click(
                &mut app,
                &ctx,
                "I reviewed this file's result in the antivirus app",
            );
            assert!(app.antivirus.review.as_ref().unwrap().external_confirmed);
            // Current scanner settings differ: the acknowledged button reaches the
            // backend recheck and fails without launching a file or any application.
            click(&mut app, &ctx, "Open after review");
            // The recheck worker took up to 4 s with CPU near 100%. This
            // deadline only detects a worker that never reports.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while app
                .antivirus
                .review
                .as_ref()
                .is_some_and(|review| review.error.is_none())
            {
                assert!(std::time::Instant::now() < deadline);
                app.antivirus_background();
                app.wait_for_io();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(
                app.antivirus
                    .review
                    .as_ref()
                    .unwrap()
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("settings changed")
            );
        }
    }
    #[test]
    fn settings_and_result_controls_fit_the_minimum_window_in_both_themes() {
        for external_provider in [
            Provider::Malwarebytes,
            Provider::Kaspersky,
            Provider::Bitdefender,
        ] {
            for dark in [false, true] {
                let (_temp, mut app, ctx) = fixture();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let job = saved_job(&mut app);
                app.open_antivirus_settings();
                for provider in [
                    Provider::Defender,
                    external_provider,
                    Provider::ClamAv,
                    Provider::Custom,
                ] {
                    app.antivirus.draft.as_mut().unwrap().settings.provider = provider;
                    frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
                    let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
                    for label in ["Antivirus scanning", "Save scan settings", "×"] {
                        let rect = text_rect(&output, label);
                        assert!(
                            rect.min.y >= 0.0
                                && rect.max.y <= 600.0
                                && rect.min.x >= 0.0
                                && rect.max.x <= 860.0,
                            "{label}: {rect:?}"
                        );
                    }
                }
                app.dialog = None;
                app.review_scan(&job, false);
                frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
                let output = frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
                for label in ["Antivirus scan result", "Open file", "Scan again", "×"] {
                    assert!(text_rect(&output, label).max.y <= 600.0);
                }
            }
        }
    }
}

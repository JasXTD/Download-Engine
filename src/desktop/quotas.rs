use super::*;
use anyhow::Context;

struct Field {
    enabled: bool,
    mib: String,
}
impl Field {
    fn new(limit: Option<u64>) -> Self {
        Self {
            enabled: limit.is_some(),
            mib: limit.map_or_else(
                || "1024".into(),
                |n| (n as f64 / quota::MIN_LIMIT as f64).to_string(),
            ),
        }
    }
    fn limit(&self) -> anyhow::Result<Option<u64>> {
        if !self.enabled {
            return Ok(None);
        }
        let amount: f64 = self
            .mib
            .trim()
            .parse()
            .context(tr("Enter a data limit in MiB."))?;
        let bytes = amount * quota::MIN_LIMIT as f64;
        anyhow::ensure!(
            bytes.is_finite()
                && bytes >= quota::MIN_LIMIT as f64
                && bytes <= quota::MAX_LIMIT as f64
                && bytes.fract() == 0.0,
            tr("Choose 1 to 1,073,741,824 MiB, with no fraction of a byte.")
        );
        let bytes = bytes as u64;
        quota::Limits {
            hourly: Some(bytes),
            daily: None,
        }
        .validate()?;
        Ok(Some(bytes))
    }
    fn draw(
        &mut self,
        ui: &mut egui::Ui,
        (label, label_width): (&str, f32),
        used: u64,
        reset: Option<i64>,
        p: Palette,
    ) {
        ui.horizontal(|ui| {
            view::column_checkbox(ui, label_width, &mut self.enabled, label);
            ui.add_enabled(
                self.enabled,
                view::line_edit(&mut self.mib).desired_width(105.0),
            );
            ui.label("MiB");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(tf!(
                        "{} used · {}",
                        bytes(used),
                        reset.map_or_else(
                            || tr("Starts when enabled").into(),
                            |at| tf!("Resets {}", reset_label(at))
                        )
                    ))
                    .small()
                    .color(p.muted),
                );
            });
        });
    }
}
pub(super) struct Form {
    hourly: Field,
    daily: Field,
    request: Option<Uuid>,
}
pub(super) fn reset_label(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or_else(
        || tr("at the next window").into(),
        |date| {
            date.with_timezone(&chrono::Local)
                .format("%d %b, %H:%M:%S")
                .to_string()
        },
    )
}
impl Desktop {
    pub(super) fn open_data_quotas(&mut self) {
        match self.scheduler.quota_snapshot() {
            Ok(status) => self.quota_status = status,
            Err(error) => {
                self.notice = Some(Notice::error(error.to_string()));
                return;
            }
        }
        self.data_quotas = Some(Form {
            hourly: Field::new(self.quota_status.limits.hourly),
            daily: Field::new(self.quota_status.limits.daily),
            request: None,
        });
        self.form_error = None;
        self.dialog = Some(Dialog::DataQuotas);
    }
    fn save_data_quotas(&mut self) {
        let Some(form) = &mut self.data_quotas else {
            return;
        };
        if form.request.is_some() {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let limits = quota::Limits {
                hourly: form.hourly.limit()?,
                daily: form.daily.limit()?,
            };
            let request = Uuid::new_v4();
            self.scheduler
                .send(Command::Quota(quota::Update { request, limits }))?;
            form.request = Some(request);
            Ok(())
        })();
        self.form_error = result.err().map(|error| error.to_string());
    }
    pub(super) fn quota_event(&mut self, event: quota::Event) {
        let Some(form) = &mut self.data_quotas else {
            return;
        };
        if form.request != Some(event.request) {
            return;
        }
        match event.result {
            Ok(status) => {
                self.quota_status = status;
                self.note_advanced_preferences_saved();
                self.data_quotas = None;
                self.dialog = None;
                self.form_error = None;
                self.notice = Some(Notice::success(
                    tr("Data quotas saved. Current usage is preserved.").into(),
                ));
            }
            Err(error) => {
                form.request = None;
                self.form_error = Some(error);
            }
        }
    }
    pub(super) fn data_quotas_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(form) = &mut self.data_quotas else {
            self.dialog = None;
            return;
        };
        let saving = form.request.is_some();
        let mut submit = false;
        let mut cancel = false;
        let response = form_dialog::modal("data-quotas", p).show(ctx, |ui| {
            ui.set_width(470.0);
            cancel = form_dialog::title(
                ui,
                tr("Data quotas"),
                form_dialog::Icon::Symbol("\u{e9d9}"),
                p,
                !saving,
            );
            form_dialog::body(ui, "quotas-body", |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let labels = [tr("Hourly limit"), tr("Daily limit")];
                let label_width = view::checkbox_column(ui, &labels);
                ui.add_enabled_ui(!saving, |ui| {
                    egui::Frame::new()
                        .fill(p.background)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 8.0;
                            ui.push_id("hourly-quota", |ui| {
                                form.hourly.draw(
                                    ui,
                                    (labels[0], label_width),
                                    self.quota_status.hourly_used,
                                    self.quota_status.hour_start.map(|at| at + quota::HOUR),
                                    p,
                                )
                            });
                            ui.separator();
                            ui.push_id("daily-quota", |ui| {
                                form.daily.draw(
                                    ui,
                                    (labels[1], label_width),
                                    self.quota_status.daily_used,
                                    self.quota_status.day_start.map(|at| at + quota::DAY),
                                    p,
                                )
                            });
                        });
                });
                if let Some(at) = self.quota_status.resumes_at() {
                    ui.label(
                        RichText::new(tf!(
                            "Quota reached · Available again {}",
                            reset_label(at)
                        ))
                        .color(p.warning),
                    );
                }
                ui.label(
                    RichText::new(
                        tr("Shared across all downloads (1,024 MiB = 1 GiB). Active transfers pause when a limit is reached and resume after reset."),
                    )
                    .small()
                    .color(p.muted),
                );
                if let Some(error) = &self.form_error {
                    ui.label(RichText::new(error).color(p.warning));
                }
            });
            form_dialog::controls(ui, |ui| {
                submit = form_dialog::primary_enabled(ui, !saving, tr("Apply quotas")).clicked();
                if saving {
                    ui.spinner();
                }
            });
        });
        if !saving && (cancel || response.should_close()) {
            self.data_quotas = None;
            self.dialog = None;
            self.form_error = None;
        } else if submit {
            self.save_data_quotas();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame, text_rect};
    use super::*;
    #[test]
    fn opening_quota_dialog_does_not_wait_for_a_database_writer() {
        let (_temp, mut app, _ctx) = fixture();
        let conn =
            crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3"))
                .unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let started = Instant::now();
        app.open_data_quotas();
        let elapsed = started.elapsed();
        conn.execute_batch("ROLLBACK").unwrap();
        assert!(
            elapsed < Duration::from_millis(250),
            "UI waited {elapsed:?} on a writer"
        );
        assert_eq!(app.dialog, Some(Dialog::DataQuotas));
    }
    fn await_save(app: &mut Desktop) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .data_quotas
            .as_ref()
            .is_some_and(|form| form.request.is_some())
        {
            app.refresh();
            assert!(
                Instant::now() < deadline,
                "Quota settings were not acknowledged"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn quota_settings_validate_acknowledge_preserve_usage_and_cancel() {
        let (_temp, mut app, ctx) = fixture();
        app.open_preferences_window();
        app.preferences_window.as_mut().unwrap().category = preferences_window::Category::Downloads;
        let size = egui::vec2(1080.0, 800.0);
        frame(&mut app, &ctx, size, vec![]);
        frame(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(egui::pos2(600.0, 450.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -1000.0),
                    modifiers: egui::Modifiers::default(),
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        click(&mut app, &ctx, "Data quotas");
        assert_eq!(app.dialog, Some(Dialog::DataQuotas));
        assert!(!app.data_quotas.as_ref().unwrap().hourly.enabled);
        for input in ["", "NaN", "inf", "0", "-1", "1073741825", "1.0001"] {
            let form = app.data_quotas.as_mut().unwrap();
            form.hourly.enabled = true;
            form.hourly.mib = input.into();
            app.save_data_quotas();
            assert!(app.form_error.is_some(), "{input}");
            assert!(app.data_quotas.as_ref().unwrap().request.is_none());
        }
        let form = app.data_quotas.as_mut().unwrap();
        form.hourly.mib = "1024".into();
        form.daily.enabled = true;
        form.daily.mib = "4096".into();
        click(&mut app, &ctx, "Apply quotas");
        let request = app.data_quotas.as_ref().unwrap().request;
        app.quota_event(quota::Event {
            request: Uuid::new_v4(),
            result: Err("stale".into()),
        });
        assert_eq!(app.data_quotas.as_ref().unwrap().request, request);
        await_save(&mut app);
        assert!(app.data_quotas.is_none(), "{:?}", app.form_error);
        assert_eq!(
            app.store.quota_status().unwrap().limits,
            quota::Limits {
                hourly: Some(1024 * quota::MIN_LIMIT),
                daily: Some(4096 * quota::MIN_LIMIT)
            }
        );
        app.store
            .record_quota_bytes_at(1234567, chrono::Utc::now().timestamp())
            .unwrap();
        app.open_data_quotas();
        app.data_quotas.as_mut().unwrap().hourly.mib = "2048".into();
        click(&mut app, &ctx, "×");
        assert_eq!(
            app.store.quota_status().unwrap().limits.hourly,
            Some(1024 * quota::MIN_LIMIT)
        );
        app.open_data_quotas();
        app.data_quotas.as_mut().unwrap().hourly.enabled = false;
        app.data_quotas.as_mut().unwrap().daily.enabled = false;
        click(&mut app, &ctx, "Apply quotas");
        await_save(&mut app);
        assert!(!app.store.quota_status().unwrap().limits.enabled());
        assert_eq!(app.store.quota_status().unwrap().daily_used, 1234567);
    }
    #[test]
    fn quota_controls_and_errors_fit_small_window_in_both_themes() {
        let (_temp, mut app, ctx) = fixture();
        let size = egui::vec2(860.0, 600.0);
        app.store
            .set_quota_limits_at(
                quota::Limits {
                    hourly: Some(quota::MIN_LIMIT),
                    daily: Some(2 * quota::MIN_LIMIT),
                },
                chrono::Utc::now().timestamp(),
            )
            .unwrap();
        app.store
            .record_quota_bytes_at(2 * quota::MIN_LIMIT, chrono::Utc::now().timestamp())
            .unwrap();
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            ctx.set_theme(theme);
            app.open_data_quotas();
            app.form_error =
                Some("Cannot save data quotas. Check the database and try again.".into());
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for label in [
                "Hourly limit",
                "Daily limit",
                "Apply quotas",
                app.form_error.as_deref().unwrap(),
            ] {
                let rect = text_rect(&output, label);
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                    "{label} outside the window: {rect:?}"
                );
                let clip = output.shapes.iter().find(|clipped| matches!(&clipped.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == label)).unwrap().clip_rect;
                assert!(clip.expand(1.0).contains_rect(rect), "{label} clipped");
            }
        }
    }
    #[test]
    fn quota_fields_round_trip_saved_byte_limits() {
        for value in [
            quota::MIN_LIMIT,
            quota::MIN_LIMIT + 1,
            123456789,
            quota::MAX_LIMIT,
        ] {
            assert_eq!(Field::new(Some(value)).limit().unwrap(), Some(value));
        }
    }

    #[test]
    fn quota_status_bar_keeps_table_and_speed_controls_visible_and_opens_settings() {
        let (_temp, mut app, ctx) = fixture();
        app.store
            .set_quota_limits_at(
                quota::Limits {
                    hourly: Some(quota::MIN_LIMIT),
                    daily: None,
                },
                chrono::Utc::now().timestamp(),
            )
            .unwrap();
        app.quota_status = app
            .store
            .record_quota_bytes_at(quota::MIN_LIMIT, chrono::Utc::now().timestamp())
            .unwrap();
        app.limits.global = Some(download_engine::bandwidth::MAX_RATE);
        let label = format!(
            "Data quota reached · Resets {}",
            reset_label(app.quota_status.resumes_at().unwrap())
        );
        let size = egui::vec2(860.0, 600.0);
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            ctx.set_theme(theme);
            frame(&mut app, &ctx, size, vec![]);
            let output = frame(&mut app, &ctx, size, vec![]);
            for text in [label.as_str(), "first.zip", "1024.0 MiB/s", "More"] {
                let rect = text_rect(&output, text);
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
                    "{text} outside window"
                );
                let clip = output.shapes.iter().find(|clipped| matches!(&clipped.shape, egui::epaint::Shape::Text(shape) if shape.galley.job.text.trim() == text)).unwrap().clip_rect;
                assert!(clip.expand(1.0).contains_rect(rect), "{text} clipped");
            }
        }
        click(&mut app, &ctx, &label);
        assert_eq!(app.dialog, Some(Dialog::DataQuotas));
    }
}

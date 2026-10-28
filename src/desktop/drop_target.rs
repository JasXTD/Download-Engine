//! The floating drop box's switch, menu item and dropped links. src/drop_target.rs
//! owns the native window; url_drop.rs validates links before they arrive here.
use super::*;
use crate::drop_target::Look;

#[derive(Default)]
pub(super) struct State {
    /// Set only for the real window. Tests and screenshots never open the box.
    native: Option<egui::Context>,
    #[cfg(windows)]
    window: Option<crate::drop_target::DropTarget>,
    /// Opening failed; retried after the switch is turned off and on again.
    failed: bool,
    look: Option<Look>,
    language: Option<download_engine::i18n::Language>,
    tray_checked: Option<bool>,
    /// Links dropped while another dialog was open, one per line.
    pending: Option<String>,
}

/// Box colors for the theme, including a custom theme's palette. The light palette's
/// border suits surfaces inside the window; the box sits on any desktop, so it gets
/// a stronger edge.
fn look(p: Palette, dark: bool) -> Look {
    let rgb = |color: Color32| [color.r(), color.g(), color.b()];
    Look {
        fill: rgb(if dark { p.background } else { p.surface }),
        border: if dark { rgb(p.border) } else { [201, 208, 215] },
        hover_fill: rgb(p.selected),
        hover_border: rgb(p.blue),
    }
}

impl Desktop {
    pub fn attach_drop_target(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "screenshot")]
        if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some()
            || std::env::var_os("EFRAME_SCREENSHOT_TO").is_some()
        {
            return;
        }
        self.drop_target.native = Some(ctx.clone());
        self.sync_drop_target(ctx);
    }

    /// Shows or hides the floating drop box and saves the choice.
    pub(super) fn set_drop_target(&mut self, ctx: &egui::Context, on: bool) {
        let mut value = self.interface;
        value.drop_target = on;
        self.save_interface(ctx, value);
        self.drop_target.failed = false;
        self.sync_drop_target(ctx);
    }

    /// The box exists only while the setting is on, in the app's theme and language.
    fn sync_drop_target(&mut self, ctx: &egui::Context) {
        let on = self.interface.drop_target;
        if let Some(tray) = self.tray.as_ref().filter(|tray| tray.available())
            && self.drop_target.tray_checked != Some(on)
        {
            self.drop_target.tray_checked = Some(on);
            if let Err(error) = tray.set_drop_target(on) {
                self.notice = Some(Notice::error(error.to_string()));
            }
        }
        if !on {
            #[cfg(windows)]
            {
                self.drop_target.window = None;
            }
            self.drop_target.look = None;
            self.drop_target.language = None;
            return;
        }
        let look = look(Palette::for_context(ctx), ctx.theme() == egui::Theme::Dark);
        let language = self.interface.language;
        #[cfg(windows)]
        {
            if self.drop_target.window.is_none()
                && !self.drop_target.failed
                && let Some(native) = &self.drop_target.native
            {
                match crate::drop_target::DropTarget::new(
                    native.clone(),
                    self.interface.drop_target_position,
                    look,
                    language,
                ) {
                    Ok(window) => {
                        self.drop_target.window = Some(window);
                        self.drop_target.look = Some(look);
                        self.drop_target.language = Some(language);
                    }
                    Err(error) => {
                        self.drop_target.failed = true;
                        self.notice = Some(Notice::error(tf!(
                            "Floating drop box unavailable: {error}",
                            error = format!("{error:#}")
                        )));
                    }
                }
            }
            if let Some(window) = &self.drop_target.window {
                let mut result = Ok(());
                if self.drop_target.look != Some(look) {
                    self.drop_target.look = Some(look);
                    result = window.set_look(look);
                }
                if self.drop_target.language != Some(language) {
                    self.drop_target.language = Some(language);
                    result = result.and(window.set_language(language));
                }
                if let Err(error) = result {
                    self.notice = Some(Notice::error(error.to_string()));
                }
            }
        }
    }

    pub(super) fn drop_target_background(&mut self, ctx: &egui::Context) {
        self.sync_drop_target(ctx);
        #[cfg(windows)]
        {
            use crate::drop_target::Event;
            let (events, drops): (Vec<_>, Vec<_>) = match &self.drop_target.window {
                Some(window) => (window.events(), window.drops.try_iter().collect()),
                None => Default::default(),
            };
            for event in events {
                match event {
                    Event::Moved(position) => {
                        let mut value = self.interface;
                        value.drop_target_position = Some(position);
                        self.save_interface(ctx, value);
                    }
                    Event::Open => self.show_window(ctx),
                    Event::Hide => self.set_drop_target(ctx, false),
                    Event::Failed(error) => {
                        self.drop_target.window = None;
                        self.drop_target.failed = true;
                        self.notice = Some(Notice::error(tf!(
                            "Floating drop box unavailable: {error}",
                            error = error
                        )));
                    }
                }
            }
            for text in drops {
                self.receive_dropped_links(ctx, &text);
            }
        }
        if self.dialog.is_none()
            && let Some(urls) = self.drop_target.pending.take()
        {
            self.open_dropped_links(urls);
        }
    }

    /// Links dropped on the box. The main window comes forward; a dialog that is
    /// already open is finished first, and the links wait for it.
    pub(super) fn receive_dropped_links(&mut self, ctx: &egui::Context, text: &str) {
        self.show_window(ctx);
        let combined = match &self.drop_target.pending {
            Some(pending) => format!("{pending}\n{text}"),
            None => text.to_owned(),
        };
        match crate::url_drop::urls(&combined) {
            Ok(urls) => self.drop_target.pending = Some(urls),
            Err(error) => {
                self.notice = Some(Notice::error(error.to_string()));
                return;
            }
        }
        if self.dialog.is_some() {
            self.notice = Some(Notice::info(
                tr("Finish the current dialog to review the dropped links.").into(),
            ));
        } else if let Some(urls) = self.drop_target.pending.take() {
            self.open_dropped_links(urls);
        }
    }

    /// One link opens Add download filled in; several open Add batch. Nothing
    /// starts until the user adds them.
    pub(super) fn open_dropped_links(&mut self, urls: String) {
        if urls.lines().count() > 1 {
            if let Err(error) = self.review_dropped_urls(&urls) {
                self.notice = Some(Notice::error(error.to_string()));
            }
            return;
        }
        self.url = urls;
        self.name.clear();
        self.expected_sha256.clear();
        self.add_ftp_security = None;
        self.add_authentication = false;
        self.add_login = add_login::Draft::default();
        self.open_add();
        // An open review blocks automatic exit or shutdown, as for batch drops.
        self.extras.exit_deadline = None;
        self.extras.shutdown_deadline = None;
    }

    pub(super) fn drop_target_menu_item(&mut self, ui: &mut egui::Ui) {
        let mut on = self.interface.drop_target;
        if ui
            .square_checkbox(&mut on, tr("Floating drop box"))
            .on_hover_text(tr(
                "A small box that stays on top of other windows. Drop links on it to download them.",
            ))
            .clicked()
        {
            self.set_drop_target(ui.ctx(), on);
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_tests::{click, fixture, frame};
    use super::*;

    fn brings_window_forward(output: &egui::LogicOutput) -> bool {
        let commands = &output.viewport_commands[&egui::ViewportId::ROOT];
        [
            egui::ViewportCommand::Visible(true),
            egui::ViewportCommand::Minimized(false),
            egui::ViewportCommand::Focus,
        ]
        .iter()
        .all(|command| commands.contains(command))
    }

    #[test]
    fn one_dropped_link_opens_add_download_filled_in_and_brings_the_window_forward() {
        let (_temp, mut app, ctx) = fixture();
        let jobs = app.store.list().unwrap().len();
        app.name = "stale name.bin".into();
        app.expected_sha256 = "ab".repeat(32);
        app.add_authentication = true;
        app.batch = "https://example.com/unfinished.zip".into();
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            app.receive_dropped_links(ctx, "https://example.com/setup.exe#fragment")
        });
        assert!(brings_window_forward(&output));
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert_eq!(app.url, "https://example.com/setup.exe");
        assert!(app.name.is_empty() && app.expected_sha256.is_empty());
        assert!(!app.add_authentication);
        assert_eq!(
            app.batch, "https://example.com/unfinished.zip",
            "Batch draft kept"
        );
        assert!(app.drop_target.pending.is_none());
        assert_eq!(app.store.list().unwrap().len(), jobs, "Nothing is queued");
    }

    #[test]
    fn several_dropped_links_open_add_batch_after_an_unfinished_batch_draft() {
        let (_temp, mut app, ctx) = fixture();
        app.batch = "https://example.com/earlier.zip".into();
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            app.receive_dropped_links(
                ctx,
                "https://example.com/a.zip\nhttps://example.com/b.iso\nhttps://example.com/a.zip",
            )
        });
        assert!(brings_window_forward(&output));
        assert_eq!(app.dialog, Some(Dialog::Batch));
        assert_eq!(
            app.batch,
            "https://example.com/earlier.zip\nhttps://example.com/a.zip\nhttps://example.com/b.iso"
        );
        assert_eq!(app.view, View::Downloads);
    }

    #[test]
    fn invalid_or_oversized_drops_open_nothing_and_explain_why() {
        let (_temp, mut app, ctx) = fixture();
        let many: String = (0..1001)
            .map(|i| format!("https://example.com/{i}.zip\n"))
            .collect();
        let oversized = format!("https://example.com/{}", "a".repeat(1_000_000));
        for text in [
            "file:///C:/secret.txt",
            "javascript:alert(1)",
            "https://user:pass@example.com/a.zip",
            "",
            &many,
            &oversized,
        ] {
            app.notice = None;
            let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
                app.receive_dropped_links(ctx, text)
            });
            // The window still comes forward so the reason can be read.
            assert!(brings_window_forward(&output));
            assert_eq!(app.dialog, None);
            assert!(app.drop_target.pending.is_none());
            assert!(app.url.is_empty() && app.batch.is_empty());
            assert!(app.notice.is_some());
        }
    }

    #[test]
    fn drops_during_a_dialog_wait_combine_and_open_when_it_closes() {
        let (_temp, mut app, ctx) = fixture();
        app.open_queue_dialog(Dialog::NewQueue);
        let first: String = (0..999)
            .map(|i| format!("https://example.com/{i}.zip\n"))
            .collect();
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            app.receive_dropped_links(ctx, &first);
            app.drop_target_background(ctx);
        });
        assert!(brings_window_forward(&output));
        assert_eq!(app.dialog, Some(Dialog::NewQueue));
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .contains("Finish the current dialog")
        );
        // A repeated link is merged; links beyond 1,000 are refused and the rest kept.
        let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            app.receive_dropped_links(
                ctx,
                "https://example.com/1.zip\nhttps://example.com/new.zip",
            );
            app.receive_dropped_links(ctx, "https://example.com/one-too-many.zip");
        });
        assert!(app.notice.as_ref().unwrap().contains("1,000"));
        app.dialog = None;
        let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            app.drop_target_background(ctx)
        });
        assert_eq!(app.dialog, Some(Dialog::Batch));
        assert_eq!(app.batch.lines().count(), 1000);
        assert!(app.batch.ends_with("https://example.com/new.zip"));
        assert!(app.drop_target.pending.is_none());
    }

    #[test]
    fn view_menu_and_tray_switch_the_drop_target_and_save_it() {
        let (_temp, mut app, ctx) = fixture();
        assert!(!app.interface.drop_target, "Off by default");
        for expected in [true, false] {
            click(&mut app, &ctx, "View");
            click(&mut app, &ctx, "Floating drop box");
            assert_eq!(app.interface.drop_target, expected);
            assert_eq!(
                app.store.interface_preferences().unwrap().drop_target,
                expected
            );
        }
        frame(&mut app, &ctx, egui::vec2(1080.0, 800.0), vec![]);
        app.tray_action(TrayAction::DropTarget, &ctx);
        assert!(app.store.interface_preferences().unwrap().drop_target);
        app.tray_action(TrayAction::DropTarget, &ctx);
        assert!(!app.store.interface_preferences().unwrap().drop_target);
        // Without attach_drop_target no native window is opened.
        app.interface.drop_target = true;
        app.drop_target_background(&ctx);
        #[cfg(windows)]
        assert!(app.drop_target.window.is_none());
    }

    #[test]
    fn box_colors_follow_the_theme() {
        let dark = look(Palette::new(true), true);
        let light = look(Palette::new(false), false);
        assert_eq!(dark.fill, [38, 42, 45]);
        assert_eq!(light.fill, [255, 255, 255]);
        assert_ne!(dark.hover_border, light.hover_border);
        let ctx = egui::Context::default();
        let custom = download_engine::preferences::CustomTheme::default();
        theme::apply_custom(&ctx, Some(custom));
        let themed = look(Palette::for_context(&ctx), true);
        assert_eq!(themed.fill, [17, 21, 32]);
        assert_ne!(themed.hover_fill, dark.hover_fill);
        for look in [dark, light, themed] {
            assert_ne!(look.fill, look.hover_fill);
            assert_ne!(look.border, look.hover_border);
        }
    }
}

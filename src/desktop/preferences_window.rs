//! A staged Preferences popup. Advanced editors retain their own explicit saves.
use super::*;
use egui::{Align, Layout, RichText, vec2};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Category {
    #[default]
    Appearance,
    Downloads,
    Connection,
    Browser,
    Antivirus,
    Startup,
}

impl Category {
    pub(super) const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Downloads,
        Self::Connection,
        Self::Browser,
        Self::Antivirus,
        Self::Startup,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Appearance => tr("Appearance"),
            Self::Downloads => tr("Downloads"),
            Self::Connection => tr("Connection"),
            Self::Browser => tr("Browser & AI"),
            Self::Antivirus => tr("Antivirus"),
            Self::Startup => tr("Startup & tray"),
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Appearance => "\u{e790}",
            Self::Downloads => "\u{e896}",
            Self::Connection => "\u{e71b}",
            Self::Browser => "\u{e774}",
            Self::Antivirus => "\u{e83d}",
            Self::Startup => "\u{e7ed}",
        }
    }
}

pub(super) struct State {
    pub(super) session: Uuid,
    pub(super) category: Category,
    pub(super) interface: InterfacePreferences,
    pub(super) behavior: Preferences,
    original_interface: InterfacePreferences,
    original_behavior: Preferences,
    pub(super) startup: bool,
    pub(super) start_hidden: bool,
    original_startup: bool,
    original_start_hidden: bool,
    pub(super) error: Option<String>,
    pub(super) advanced_saved: bool,
}

impl State {
    fn new(app: &Desktop) -> Self {
        Self {
            session: Uuid::new_v4(),
            category: Category::Appearance,
            interface: app.interface,
            original_interface: app.interface,
            behavior: app.preferences,
            original_behavior: app.preferences,
            startup: app.startup.enabled(),
            original_startup: app.startup.enabled(),
            start_hidden: app.startup.start_hidden(),
            original_start_hidden: app.startup.start_hidden(),
            error: None,
            advanced_saved: false,
        }
    }

    pub(super) fn dirty(&self) -> bool {
        self.interface != self.original_interface
            || self.behavior != self.original_behavior
            || self.startup != self.original_startup
            || self.start_hidden != self.original_start_hidden
    }

    fn reset_category(&mut self) {
        let defaults = InterfacePreferences::default();
        match self.category {
            Category::Appearance => {
                self.session = Uuid::new_v4();
                self.interface.language = defaults.language;
                self.interface.appearance = defaults.appearance;
                self.interface.custom_theme = defaults.custom_theme;
                self.interface.compact_rows = defaults.compact_rows;
                self.interface.show_details = defaults.show_details;
                self.interface.sidebar_labels = defaults.sidebar_labels;
                self.interface.columns = defaults.columns;
                self.interface.table_layout = defaults.table_layout;
            }
            Category::Downloads => {
                self.interface.auto_download_status = defaults.auto_download_status
            }
            Category::Browser => self.interface.clipboard_links = defaults.clipboard_links,
            Category::Startup => {
                self.behavior = Preferences::default();
                self.interface.completion_sound = defaults.completion_sound;
                self.startup = false;
                self.start_hidden =
                    download_engine::preferences::StartupPreferences::default().start_hidden;
            }
            Category::Connection | Category::Antivirus => {}
        }
        self.error = None;
    }

    /// Only replace fields edited here. A floating-window position or another
    /// independently saved setting may have changed while the popup was open.
    fn merged(&self, app: &Desktop) -> (InterfacePreferences, Preferences) {
        let mut interface = app.interface;
        let mut behavior = app.preferences;
        macro_rules! merge {
            ($target:ident, $draft:ident, $original:ident, $($field:ident),+ $(,)?) => {
                $(if self.$draft.$field != self.$original.$field {
                    $target.$field = self.$draft.$field;
                })+
            };
        }
        merge!(
            interface,
            interface,
            original_interface,
            language,
            appearance,
            custom_theme,
            compact_rows,
            show_details,
            sidebar_labels,
            columns,
            table_layout,
            auto_download_status,
            clipboard_links,
            completion_sound
        );
        merge!(
            behavior,
            behavior,
            original_behavior,
            close_to_tray,
            notify_complete,
            notify_failed,
            notify_scripts
        );
        (interface, behavior)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Editor {
    Columns,
    Connections,
    Folders,
    Speed,
    Quotas,
    Proxy,
    Browsers,
    Mcp,
    Antivirus,
}

impl Desktop {
    /// Called only after a separate settings editor confirms a successful save.
    pub(super) fn note_advanced_preferences_saved(&mut self) {
        if let Some(state) = &mut self.preferences_window {
            state.advanced_saved = true;
        }
    }
    pub(super) fn open_preferences_window(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        self.startup.refresh_if_stale();
        self.refresh_settings_summary();
        self.preferences_window = Some(State::new(self));
        self.dialog = Some(Dialog::PreferencesWindow);
    }

    pub(super) fn close_preferences_window(&mut self, ctx: &egui::Context) {
        self.theme_assets
            .discard_imports(self.interface.custom_theme.image);
        self.preferences_window = None;
        self.dialog = None;
        self.form_error = None;
        self.configure_appearance(ctx);
        ctx.request_repaint();
    }

    pub(super) fn save_preferences_window(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.preferences_window.take() else {
            return;
        };
        let (interface, behavior) = state.merged(self);
        // One transaction for appearance and notification preferences; a failed
        // write leaves both live values and the draft intact for a retry.
        let saved = if interface == self.interface {
            self.store.save_preferences(behavior)
        } else {
            self.store.save_desktop_preferences(behavior, interface)
        };
        if let Err(error) = saved {
            state.error = Some(tf!("Cannot save preferences: {error}", error = error));
            self.preferences_window = Some(state);
            return;
        }
        let language_changed = interface.language != self.interface.language;
        if (behavior.notify_complete != self.preferences.notify_complete
            || behavior.notify_failed != self.preferences.notify_failed
            || behavior.notify_scripts != self.preferences.notify_scripts)
            && let Some(tray) = &self.tray
            && let Err(error) = tray.clear_notifications()
        {
            self.notice = Some(Notice::error(error.to_string()));
        }
        self.theme_assets.replace_saved_image(
            &self.store.paths.data,
            self.interface.custom_theme.image,
            interface.custom_theme.image,
        );
        self.interface = interface;
        // The database is committed even if a later startup-setting write fails.
        self.theme_assets
            .discard_imports(interface.custom_theme.image);
        self.preferences = behavior;
        state.interface = interface;
        state.original_interface = interface;
        state.behavior = behavior;
        state.original_behavior = behavior;
        if language_changed {
            let _language = download_engine::i18n::scope(interface.language);
            self.refresh_settings_summary();
            if let Some(tray) = &self.tray
                && let Err(error) = tray.set_language(interface.language)
            {
                self.notice = Some(Notice::error(error.to_string()));
            }
        }
        // Windows startup is an external operation, not part of the database
        // transaction. Keep a failed change pending and explain any partial save.
        let startup_result = (|| -> anyhow::Result<()> {
            if state.startup != state.original_startup {
                self.startup.set_enabled(&self.store, state.startup)?;
                state.original_startup = state.startup;
            }
            if state.start_hidden != state.original_start_hidden {
                self.startup
                    .set_start_hidden(&self.store, state.start_hidden)?;
                state.original_start_hidden = state.start_hidden;
            }
            Ok(())
        })();
        if let Err(error) = startup_result {
            state.error = Some(tf!(
                "Preferences saved, but Windows startup could not be updated: {error}",
                error = error
            ));
            state.category = Category::Startup;
            self.preferences_window = Some(state);
            self.configure_appearance(ctx);
        } else {
            self.close_preferences_window(ctx);
        }
        ctx.request_repaint();
    }

    fn open_preferences_editor(&mut self, editor: Editor) {
        self.dialog = None;
        match editor {
            Editor::Columns => {
                self.open_columns();
            }
            Editor::Connections => self.open_connections(),
            Editor::Folders => self.open_download_folders(),
            Editor::Speed => self.open_speed_limits(),
            Editor::Quotas => self.open_data_quotas(),
            Editor::Proxy => self.open_proxy(),
            Editor::Browsers => self.open_browser_setup(),
            Editor::Mcp => self.open_mcp(),
            Editor::Antivirus => self.open_antivirus_settings(),
        }
        if self.dialog.is_none() {
            if let Some(state) = &mut self.preferences_window {
                state.error = self.form_error.take().or_else(|| {
                    self.notice
                        .as_ref()
                        .filter(|notice| matches!(notice.kind, interactions::NoticeKind::Error))
                        .map(|notice| notice.message.clone())
                });
            }
            self.dialog = Some(Dialog::PreferencesWindow);
        }
    }

    pub(super) fn preferences_window_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(mut state) = self.preferences_window.take() else {
            self.dialog = None;
            return;
        };
        let mut cancel = false;
        let mut save = false;
        let mut editor = None;
        let width = (ctx.content_rect().width() - 64.0).clamp(360.0, 780.0);
        let height = (ctx.content_rect().height()
            - 216.0
            - if state.error.is_some() { 64.0 } else { 0.0 }
            - if state.advanced_saved { 34.0 } else { 0.0 })
        .clamp(220.0, 430.0);
        let response = form_dialog::modal("preferences-window", p).show(ctx, |ui| {
            ui.set_width(width);
            cancel = form_dialog::title(
                ui,
                tr("Preferences"),
                form_dialog::Icon::Symbol("\u{e713}"),
                p,
                true,
            );
            if let Some(error) = &state.error {
                ui.label(RichText::new(error).color(p.warning));
            }
            if state.advanced_saved {
                ui.label(RichText::new(tr("Advanced settings saved. Cancel will not undo those changes.")).color(p.success));
            }
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(166.0, height),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_min_size(vec2(166.0, height));
                        ui.set_max_width(166.0);
                        ui.spacing_mut().item_spacing.y = 5.0;
                        for category in Category::ALL {
                            let selected = state.category == category;
                            let (rect, response) =
                                ui.allocate_exact_size(vec2(166.0, 40.0), egui::Sense::click());
                            let visuals = ui.style().interact(&response);
                            ui.painter().rect_filled(
                                rect,
                                6,
                                if selected {
                                    p.selected
                                } else {
                                    visuals.bg_fill
                                },
                            );
                            ui.painter().rect_stroke(
                                rect,
                                6,
                                if response.has_focus() {
                                    egui::Stroke::new(1.0, p.blue)
                                } else {
                                    visuals.bg_stroke
                                },
                                egui::StrokeKind::Inside,
                            );
                            // Reserve real icon space: leading spaces move to the wrong
                            // side of Arabic text and can paint the label over the icon.
                            let label_rect = egui::Rect::from_min_max(
                                rect.min + vec2(36.0, 0.0),
                                rect.max - vec2(8.0, 0.0),
                            );
                            ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(label_rect)
                                    .layout(Layout::left_to_right(Align::Center)),
                            )
                            .add(
                                egui::Label::new(RichText::new(category.label()).size(13.0))
                                    .truncate()
                                    .selectable(false)
                                    .sense(egui::Sense::hover()),
                            );
                            view::icon(
                                ui,
                                egui::pos2(response.rect.left() + 16.0, response.rect.center().y),
                                category.glyph(),
                                16.0,
                                if selected { p.blue } else { p.muted },
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    ui.is_enabled(),
                                    selected,
                                    category.label(),
                                )
                            });
                            if response.clicked() {
                                response.request_focus();
                                state.category = category;
                            }
                            response.on_hover_text(category.label());
                        }
                    },
                );
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_width((width - 190.0).max(170.0));
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    egui::ScrollArea::vertical()
                        .id_salt(("preferences-category", state.category as u8))
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height)
                        .show(ui, |ui| {
                            editor = self.preferences_category(ui, p, &mut state);
                        });
                });
            });
            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new(tr("Save and Cancel apply to edits in this window. Separate dialogs save independently.")).size(11.0).color(p.muted));
            // Explicit Save / Cancel / Reset order matches the approved mockup.
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if state.dirty() {
                        tr("Unsaved changes")
                    } else {
                        tr("No unsaved changes")
                    })
                    .size(11.0)
                    .color(p.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if form_dialog::secondary_enabled(
                        ui,
                        !matches!(state.category, Category::Connection | Category::Antivirus),
                        tr("Reset"),
                    )
                    .on_hover_text(tr("Reset this category to defaults. Save to apply."))
                    .clicked()
                    {
                        state.reset_category();
                    }
                    cancel |= form_dialog::secondary_button(ui, tr("Cancel")).clicked();
                    save = form_dialog::primary_enabled(ui, state.dirty() && !self.theme_assets.importing(), tr("Save")).clicked();
                });
            });
        });
        self.preferences_window = Some(state);
        if cancel || response.should_close() {
            self.close_preferences_window(ctx);
        } else if save {
            self.save_preferences_window(ctx);
        } else {
            if let Some(editor) = editor {
                self.open_preferences_editor(editor);
            }
            self.configure_appearance(ctx);
        }
    }
}

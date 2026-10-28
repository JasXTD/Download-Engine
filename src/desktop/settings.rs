//! Destination folder settings. Preferences live in preferences_window.
use super::*;

impl Desktop {
    pub(super) fn open_download_folders(&mut self) {
        if self.dialog.is_some() {
            self.notice = Some(Notice::info(
                tr("Finish the current dialog, then open Download folders in Preferences.").into(),
            ));
            return;
        }
        self.form_error = None;
        self.load_download_folders();
        if let Some(rules) = &self.download_folders {
            self.folders_draft = Some(FoldersDialog::new(rules));
            self.dialog = Some(Dialog::DownloadFolders);
        } else {
            self.notice = self.form_error.take().map(Notice::error);
        }
    }
    pub(super) fn download_folders_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(draft) = &mut self.folders_draft else {
            self.dialog = None;
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let response = form_dialog::modal("download-folders", p).show(ctx, |ui| {
            ui.set_width(530.0);
            cancel = form_dialog::title(
                ui,
                tr("Download folders"),
                form_dialog::Icon::Symbol("folder"),
                p,
                true,
            );
            form_dialog::body(ui, "folders-body", |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.square_checkbox(
                    &mut draft.organize,
                    tr("Automatically sort new downloads by category"),
                );
                ui.label(
                    RichText::new(
                        tr("Leave a category blank to use its default subfolder. Existing files stay where they are."),
                    )
                    .small()
                    .color(p.muted),
                );
                egui::Frame::new()
                    .fill(p.background)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add_enabled_ui(draft.organize, |ui| {
                            egui::Grid::new("category-folders")
                                .num_columns(2)
                                .spacing([12.0, 8.0])
                                .show(ui, |ui| {
                                    for (category, path) in
                                        Category::ALL.into_iter().zip(&mut draft.paths)
                                    {
                                        ui.label(tr(category.label()))
                                            .on_hover_text(category.examples());
                                        let default = self
                                            .store
                                            .paths
                                            .downloads
                                            .join(category.label())
                                            .to_string_lossy()
                                            .into_owned();
                                        ui.push_id(category.label(), |ui| {
                                            ui.set_width(400.0);
                                            self.folder_picker
                                                .field(
                                                    ui,
                                                    path,
                                                    std::path::Path::new(&default),
                                                    &mut self.form_error,
                                                )
                                                .on_hover_text(default);
                                        });
                                        ui.end_row();
                                    }
                                });
                        });
                    });
                let root = self.store.paths.downloads.to_string_lossy();
                ui.add(
                    egui::Label::new(
                        RichText::new(tf!("Default folder: {root}", root = root))
                            .small()
                            .color(p.muted),
                    )
                    .truncate(),
                )
                .on_hover_text(root.as_ref());
                if let Some(error) = &self.form_error {
                    ui.label(RichText::new(error).small().color(p.warning));
                }
            });
            save = form_dialog::footer(ui, tr("Save folder rules"), None, p);
        });
        if cancel || response.should_close() {
            self.folders_draft = None;
            self.dialog = None;
            self.form_error = None;
        } else if save {
            let rules = draft.rules();
            match self.store.save_download_folders(&rules) {
                Ok(()) => {
                    self.download_folders = Some(rules);
                    self.note_advanced_preferences_saved();
                    self.folders_draft = None;
                    self.dialog = None;
                    self.form_error = None;
                }
                Err(error) => self.form_error = Some(format!("{error:#}")),
            }
        }
    }
}

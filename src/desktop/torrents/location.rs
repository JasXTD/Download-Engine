//! Explicit desktop confirmation for relocating a completed torrent's saved file or folder.
use super::*;

pub(super) struct Draft {
    id: String,
    original: PathBuf,
    name: String,
    folder: Option<PathBuf>,
    rename: bool,
}

impl Desktop {
    pub(in crate::desktop) fn open_torrent_location(&mut self, job: &Job, rename: bool) {
        let Some(original) = job
            .saved_path
            .clone()
            .filter(|_| job.status == Status::Complete && job.torrent.is_some())
        else {
            return;
        };
        self.torrents.location = Some(Draft {
            id: job.id.clone(),
            name: original
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            original,
            folder: None,
            rename,
        });
        self.form_error = None;
        self.dialog = Some(Dialog::TorrentLocation);
    }

    pub(in crate::desktop) fn torrent_location_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(mut draft) = self.torrents.location.take() else {
            self.dialog = None;
            return;
        };
        let (mut cancel, mut submit) = (false, false);
        let response = form_dialog::modal("torrent-location", p).show(ctx, |ui| {
            ui.set_width(440.0_f32.min((ctx.content_rect().width() - 64.0).max(240.0)));
            cancel = form_dialog::title(
                ui,
                tr(if draft.rename {
                    "Rename torrent"
                } else {
                    "Move torrent"
                }),
                form_dialog::Icon::Download,
                p,
                true,
            );
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 190.0).max(100.0))
                .show(ui, |ui| {
                    ui.label(RichText::new(tr("Current location")).small().color(p.muted));
                    ui.add(egui::Label::new(draft.original.display().to_string()).wrap());
                    ui.add_space(8.0);
                    if draft.rename {
                        ui.label(tr("New name"));
                        ui.add(view::line_edit(&mut draft.name).desired_width(f32::INFINITY));
                    } else {
                        ui.label(tr("Destination folder"));
                        if let Some(folder) = &draft.folder {
                            ui.add(egui::Label::new(folder.display().to_string()).wrap());
                        }
                        if ui.button(tr("Browse…")).clicked() {
                            let current = draft
                                .folder
                                .as_ref()
                                .map(|folder| folder.display().to_string())
                                .unwrap_or_default();
                            match self.folder_picker.choose_titled(
                                &current,
                                draft.original.parent().unwrap_or(Path::new(".")),
                                tr("Destination folder"),
                            ) {
                                Ok(Some(folder)) => {
                                    draft.folder = Some(folder.into());
                                    self.form_error = None;
                                }
                                Ok(None) => {}
                                Err(error) => self.form_error = Some(format!("{error:#}")),
                            }
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(tr("Sharing stops before files change."))
                            .small()
                            .color(p.muted),
                    );
                    if let Some(error) = &self.form_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let enabled = if draft.rename {
                    !draft.name.is_empty()
                } else {
                    draft.folder.is_some()
                };
                submit = form_dialog::primary_enabled(
                    ui,
                    enabled,
                    tr(if draft.rename { "Save name" } else { "Move" }),
                )
                .clicked();
                cancel |= form_dialog::secondary_button(ui, tr("Cancel")).clicked();
            });
        });
        if submit {
            let result = (|| -> anyhow::Result<()> {
                let job = self
                    .store
                    .get(&draft.id)?
                    .ok_or_else(|| anyhow::anyhow!("This download was removed"))?;
                anyhow::ensure!(
                    job.status == Status::Complete
                        && job.saved_path.as_ref() == Some(&draft.original),
                    "The saved location changed. Reopen this dialog."
                );
                let command = if draft.rename {
                    anyhow::ensure!(
                        !draft.name.is_empty()
                            && download_engine::model::safe_filename(&draft.name) == draft.name,
                        "Choose a name without reserved characters or a trailing dot or space"
                    );
                    Command::RenameFile {
                        id: draft.id.clone(),
                        name: draft.name.clone(),
                    }
                } else {
                    Command::MoveFileToChosenFolder {
                        id: draft.id.clone(),
                        folder: draft
                            .folder
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("Choose a destination folder"))?,
                    }
                };
                self.scheduler.send(command)
            })();
            match result {
                Ok(()) => {
                    self.dialog = None;
                    self.force_refresh();
                    return;
                }
                Err(error) => self.form_error = Some(format!("{error:#}")),
            }
        }
        if cancel || response.should_close() {
            self.dialog = None;
            self.form_error = None;
        } else {
            self.torrents.location = Some(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
    #[test]
    fn torrent_location_forms_validate_names_and_keep_cancelled_files_in_place() {
        let (dir, mut app, ctx) = fixture();
        let root = dir.path().join("Sample collection");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("sample.bin"), b"harmless").unwrap();
        let mut job = Job::new_torrent(
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            "Sample collection",
            dir.path().into(),
            Uuid::new_v4().to_string(),
            Stage::Chosen,
        )
        .unwrap();
        job.status = Status::Complete;
        job.saved_path = Some(root.clone());
        app.store.enqueue(&job).unwrap();
        app.open_torrent_location(&job, true);
        for _ in 0..2 {
            frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
        }
        let output = frame(&mut app, &ctx, vec2(860.0, 600.0), vec![]);
        for label in [
            "Rename torrent",
            "Current location",
            "New name",
            "Save name",
            "Cancel",
        ] {
            assert!(ctx.content_rect().contains_rect(text_rect(&output, label)));
        }
        app.torrents.location.as_mut().unwrap().name = "bad:name".into();
        click(&mut app, &ctx, "Save name");
        assert_eq!(app.dialog, Some(Dialog::TorrentLocation));
        assert!(
            app.form_error
                .as_ref()
                .unwrap()
                .contains("reserved characters")
        );
        click(&mut app, &ctx, "Cancel");
        app.open_torrent_location(&job, false);
        app.folder_picker.response = Some(Ok(Some(dir.path().display().to_string())));
        click(&mut app, &ctx, "Browse…");
        assert_eq!(
            app.torrents.location.as_ref().unwrap().folder.as_deref(),
            Some(dir.path())
        );
        click(&mut app, &ctx, "Cancel");
        assert!(app.dialog.is_none());
        assert_eq!(std::fs::read(root.join("sample.bin")).unwrap(), b"harmless");
        assert_eq!(
            app.store.get(&job.id).unwrap().unwrap().saved_path,
            Some(root)
        );
        assert!(!app.store.torrents.is_running());
    }
}

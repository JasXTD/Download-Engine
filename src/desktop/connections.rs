use super::*;
use download_engine::connections::{Settings, Site};

pub(super) struct Form {
    original: Settings,
    draft: Settings,
    error: Option<String>,
}

fn limit_picker(ui: &mut egui::Ui, id: &'static str, value: &mut u8) {
    egui::ComboBox::from_id_salt(id)
        .width(85.0)
        .selected_text(value.to_string())
        .show_ui(ui, |ui| {
            for limit in [1, 2, 4, 8, 16, 32] {
                ui.selectable_value(value, limit, limit.to_string());
            }
        });
}
impl Desktop {
    pub(super) fn open_connections(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        match self.store.connection_settings() {
            Ok(settings) => {
                self.connections = Some(Form {
                    original: settings.clone(),
                    draft: settings,
                    error: None,
                });
                self.dialog = Some(Dialog::Connections);
            }
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }
    pub(super) fn connections_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(form) = &mut self.connections else {
            self.dialog = None;
            return;
        };
        let (mut save, mut close) = (false, false);
        let response=form_dialog::modal("download-connections",p).show(ctx,|ui| {
            ui.set_width(520.0);
            close=form_dialog::title(ui,tr("Download connections"),form_dialog::Icon::Symbol("\u{e968}"),p,true);
            ui.label(tr("Choose the maximum connections used by each HTTP/HTTPS download."));
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-210.0).max(180.0)).show(ui,|ui| {
                ui.horizontal(|ui| {ui.label(tr("Default per file")); limit_picker(ui,"default-connections",&mut form.draft.default_connections);});
                ui.label(RichText::new(tr("Small files and servers without range support use one connection.")).small().color(p.muted));
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new(tr("Website overrides")).strong());
                let mut remove=None;
                for (index,site) in form.draft.sites.iter_mut().enumerate() {
                    ui.push_id(index,|ui| {
                        egui::Frame::new().stroke((1.0,p.border)).corner_radius(6).inner_margin(8).show(ui,|ui| {
                            // Cards span the dialog; the host field takes what the picker and Remove leave.
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let host = ui.available_width() - 85.0 - view::button_width(ui, tr("Remove")) - 2.0 * ui.spacing().item_spacing.x;
                                ui.add(view::line_edit(&mut site.host).hint_text("example.com").desired_width(host.max(160.0)).char_limit(255));
                                limit_picker(ui,"site-limit",&mut site.connections);
                                if ui.button(tr("Remove")).clicked() {remove=Some(index);}
                            });
                            ui.square_checkbox(&mut site.subdomains,tr("Include subdomains"));
                        });
                    });
                }
                if let Some(index)=remove {form.draft.sites.remove(index);}
                if ui.add_enabled(form.draft.sites.len()<100,egui::Button::new(tr("Add website"))).clicked() {
                    form.draft.sites.push(Site {host:String::new(),subdomains:false,connections:1});
                }
                ui.label(RichText::new(tr("Use 1 for sites that reject parallel requests. More connections do not always improve speed.")).small().color(p.muted));
                ui.label(RichText::new(tr("Rules match the file's hostname. For redirects, the stricter matching rule wins. These limits are per file; queue limits control simultaneous files.")).small().color(p.muted));
                ui.label(RichText::new(tr("Pause and resume active downloads to apply changes. Video downloads use up to 4 connections and respect lower limits. FTP uses its own transfer rules.")).small().color(p.muted));
                if let Some(error)=&form.error {ui.colored_label(p.warning,error);}
            });
            save=form_dialog::footer(ui,tr("Save settings"),None,p);
        });
        if close || response.should_close() {
            self.connections = None;
            self.dialog = None;
        } else if save {
            match self
                .store
                .save_connection_settings(&form.original, form.draft.clone())
            {
                Ok(()) => {
                    self.connections = None;
                    self.note_advanced_preferences_saved();
                    self.dialog = None;
                    self.notice = Some(Notice::success(
                        tr("Connection settings saved. Pause and resume active files to apply them.")
                            .into(),
                    ));
                }
                Err(error) => form.error = Some(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
    #[test]
    fn connection_dialog_validates_saves_cancels_and_preserves_concurrent_edits() {
        let (_root, mut app, ctx) = fixture();
        app.open_connections();
        click(&mut app, &ctx, "Add website");
        click(&mut app, &ctx, "Save settings");
        assert!(app.connections.as_ref().unwrap().error.is_some());
        let form = app.connections.as_mut().unwrap();
        form.draft.sites[0].host = " EXAMPLE.com ".into();
        form.draft.sites[0].connections = 1;
        click(&mut app, &ctx, "Save settings");
        assert_eq!(app.dialog, None);
        let saved = app.store.connection_settings().unwrap();
        assert_eq!(saved.sites[0].host, "example.com");
        app.open_connections();
        click(&mut app, &ctx, "Remove");
        click(&mut app, &ctx, "×");
        assert_eq!(app.store.connection_settings().unwrap(), saved);
        app.open_connections();
        let mut changed = saved.clone();
        changed.default_connections = 8;
        app.store
            .save_connection_settings(&saved, changed.clone())
            .unwrap();
        click(&mut app, &ctx, "Save settings");
        assert!(
            app.connections
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("changed")
        );
        assert_eq!(app.store.connection_settings().unwrap(), changed);
        let size = egui::vec2(860.0, 600.0);
        frame(&mut app, &ctx, size, vec![]);
        let out = frame(&mut app, &ctx, size, vec![]);
        for label in ["Download connections", "Save settings", "×"] {
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                    .contains_rect(text_rect(&out, label)),
                "{label} clipped"
            );
        }
    }
}

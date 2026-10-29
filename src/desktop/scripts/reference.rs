//! One host API catalog powers completion and this searchable, contextual reference.
use super::*;
use download_engine::script_automation::api;

pub(super) fn show(ui: &mut egui::Ui, draft: &Script, search: &mut String, p: Palette) {
    let reader = matches!(draft.trigger, Trigger::SiteReader { .. });
    let button = ui.button(tr("Core API"));
    #[cfg(feature = "screenshot")]
    if std::env::var("IDM_EX_TEST_CORE_API").as_deref() == Ok("1") {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&button));
        if search.is_empty() {
            *search = "html".into();
        }
    }
    egui::Popup::from_toggle_button_response(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width((ui.ctx().content_rect().width() - 48.0).clamp(280.0, 520.0))
        .show(|ui| {
            ui.label(
                RichText::new(format!(
                    "Lua API {} · xd.api_version · xd.app_version · xd.script_kind",
                    api::VERSION
                ))
                .small()
                .color(p.muted),
            );
            ui.label(tr("Functions depend on the trigger and permissions."));
            ui.add(
                view::line_edit(search)
                    .hint_text(tr("Search"))
                    .desired_width(ui.available_width()),
            );
            let query = search.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("script-api-functions")
                .max_height(170.0)
                .show(ui, |ui| {
                    let functions: Vec<_> = if draft.trigger == Trigger::Provider {
                        download_engine::provider::functions().collect()
                    } else { api::FUNCTIONS.iter().filter(|f| f.available(reader)).collect() };
                    if draft.trigger == Trigger::Provider {
                        ui.label("Provider API 1 · xd.provider_api");
                        ui.label("resolve(request), refresh(request, previous), transform(plan, bytes, offset), finish(plan, result)");
                    }
                    for function in functions {
                        if !query.is_empty()
                            && !format!("{} {}", function.name, function.description)
                                .to_lowercase()
                                .contains(&query)
                        {
                            continue;
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!("{}({})", function.name, function.arguments))
                                    .monospace()
                                    .color(p.blue),
                            );
                            if ui.small_button(tr("Copy")).clicked() {
                                ui.ctx().copy_text(format!(
                                    "{}({})",
                                    function.name, function.arguments
                                ));
                            }
                        });
                        ui.label(RichText::new(function.description).small().color(p.muted));
                        ui.add_space(4.0);
                    }
                });
        });
}

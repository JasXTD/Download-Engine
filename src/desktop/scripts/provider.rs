use super::*;

pub(super) fn fields(ui: &mut egui::Ui, state: &mut State, p: Palette) {
    let changed = ui
        .horizontal(|ui| {
            ui.label(tr("Source URL"));
            ui.add(
                view::line_edit(&mut state.test_url)
                    .hint_text("https://example.com/encrypted.bin")
                    .desired_width(ui.available_width())
                    .char_limit(16384),
            )
            .changed()
        })
        .inner;
    let name_changed = ui
        .horizontal(|ui| {
            ui.label(tr("File name"));
            ui.add(
                view::line_edit(&mut state.provider_name)
                    .hint_text("output.bin")
                    .desired_width(ui.available_width())
                    .char_limit(240),
            )
            .changed()
        })
        .inner;
    if changed || name_changed {
        state.provider_request.clear();
        state.provider_job = None;
    }
    let text = if state.provider_job.is_some() {
        tr("Added to Main queue. Use the download list to pause, resume, stop or inspect errors.")
    } else {
        tr(
            "Uses Main queue and your download folders. The saved script revision stays with this download.",
        )
    };
    ui.label(
        RichText::new(text)
            .small()
            .color(if state.provider_job.is_some() {
                p.accent
            } else {
                p.muted
            }),
    );
}
pub(super) fn submit(store: &Store, state: &mut State, script: &Script) {
    if state.provider_request.is_empty() {
        state.provider_request = uuid::Uuid::new_v4().to_string();
    }
    let name = state.provider_name.trim();
    match store.add_provider_download(
        &script.id,
        script.revision,
        state.test_url.trim(),
        (!name.is_empty()).then_some(name),
        MAIN_QUEUE,
        &state.provider_request,
    ) {
        Ok(job) => {
            state.provider_job = Some(job.id);
            state.error = None;
        }
        Err(error) => state.error = Some(error.to_string()),
    }
}

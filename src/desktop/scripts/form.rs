//! Compact script settings with aligned, indivisible label/control pairs.
use super::*;

pub(super) const CONTROL_HEIGHT: f32 = view::CONTROL_HEIGHT;

fn field(
    ui: &mut egui::Ui,
    label: &str,
    label_width: f32,
    width: f32,
    p: Palette,
    content: impl FnOnce(&mut egui::Ui),
) {
    let height = ui.spacing().interact_size.y;
    // Wrap the entire pair, never a label separately from its control. A bounded
    // child also gives ComboBox::truncate an actual maximum, not just a minimum.
    ui.allocate_ui_with_layout(
        egui::vec2(label_width + ui.spacing().item_spacing.x + width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.allocate_ui(egui::vec2(label_width, height), |ui| {
                ui.set_min_width(label_width);
                ui.label(RichText::new(label).small().color(p.muted));
            });
            ui.allocate_ui(egui::vec2(width, height), content);
        },
    );
}

/// The page pattern being typed, kept outside the saved script.
#[derive(Default)]
pub(super) struct PageInput {
    pub(super) text: String,
    pub(super) error: Option<String>,
}

/// What the open script keeps between runs with xd.store.
#[derive(Default)]
pub(super) struct Values {
    pub(super) list: Vec<StoredValue>,
    pub(super) shown: bool,
    /// Clear was clicked; the page clears once drawing is done.
    pub(super) clear: bool,
}

/// The triggers in the order the list shows them.
fn triggers() -> [Trigger; 7] {
    [
        Trigger::Manual,
        Trigger::Startup,
        Trigger::Interval { minutes: 60 },
        Trigger::DownloadCompleted { queue: None },
        Trigger::DownloadFailed { queue: None },
        Trigger::QueueCompleted {
            queue: MAIN_QUEUE.into(),
        },
        Trigger::SiteReader { pages: Vec::new() },
    ]
}

/// A drop-down list just wide enough for its longest label, and never narrower than `min`.
fn combo_width<'a>(ui: &egui::Ui, labels: impl IntoIterator<Item = &'a str>, min: f32) -> f32 {
    let caret = ui.spacing().icon_width + ui.spacing().item_spacing.x;
    labels
        .into_iter()
        .map(|label| view::button_width(ui, label) + caret)
        .fold(min, f32::max)
        .ceil()
}

pub(super) fn show(
    ui: &mut egui::Ui,
    draft: &mut Script,
    pages: &mut PageInput,
    queues: &[DownloadQueue],
    values: &mut Values,
    p: Palette,
) {
    ui.scope(|ui| {
        // egui positions each widget immediately. Reserve the tallest control
        // before the first label so later dropdowns cannot move the row center.
        let height = CONTROL_HEIGHT;
        ui.spacing_mut().interact_size.y = height;
        ui.horizontal(|ui| {
            // Name fills what the Timeout pair leaves, so the row ends at the card edge.
            let gap = ui.spacing().item_spacing.x;
            let name_width = (ui.available_width() - (52.0 + gap) - gap - (46.0 + gap + 70.0)).max(120.0);
            field(ui, tr("Name"), 52.0, name_width, p, |ui| {
                ui.add(
                    view::line_edit(&mut draft.name)
                        .desired_width(ui.available_width())
                        .min_size(egui::vec2(0.0, height))
                        .margin(egui::Margin::symmetric(10, 4))
                        .vertical_align(egui::Align::Center)
                        .char_limit(64),
                );
            });
            field(ui, tr("Timeout"), 46.0, 70.0, p, |ui| {
                ui.add_sized(
                    [ui.available_width(), height],
                    egui::DragValue::new(&mut draft.timeout_seconds).range(1..=3600).suffix(" s"),
                );
            });
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            field(ui, tr("Language"), 52.0, 120.0, p, |ui| {
                let previous = draft.language;
                egui::ComboBox::from_id_salt("script-language")
                    .width(120.0)
                    .truncate()
                    .selected_text(draft.language.label())
                    .show_ui(ui, |ui| {
                        let reader = matches!(draft.trigger, Trigger::SiteReader { .. });
                        ui.selectable_value(&mut draft.language, Language::Lua, "Lua");
                        ui.add_enabled_ui(!reader, |ui| {
                            ui.selectable_value(&mut draft.language, Language::PowerShell, "PowerShell")
                                .on_disabled_hover_text(tr("Site readers are Lua only"));
                        });
                    })
                    .response
                    .on_hover_text(tr("Lua is built in. PowerShell provides full Windows file/program access. Changing language keeps custom code; only an untouched starter example is replaced."));
                if previous != draft.language
                    && (draft.body == previous.example() || draft.body == FAILED_EXAMPLE)
                {
                    draft.body = draft.language.example().into();
                }
            });
            // Wide enough for every trigger name, so none is cut off in any language.
            let trigger_width = combo_width(ui, triggers().iter().map(trigger_name), 165.0);
            field(ui, tr("Trigger"), 38.0, trigger_width, p, |ui| {
                egui::ComboBox::from_id_salt("script-trigger")
                    .width(trigger_width)
                    .truncate()
                    .selected_text(trigger_name(&draft.trigger))
                    .show_ui(ui, |ui| {
                        for trigger in triggers() {
                            let selected = std::mem::discriminant(&draft.trigger) == std::mem::discriminant(&trigger);
                            let reader = matches!(trigger, Trigger::SiteReader { .. });
                            if reader {
                                ui.separator();
                            }
                            let allowed = !reader || draft.language == Language::Lua;
                            let clicked = ui
                                .add_enabled(allowed, egui::Button::selectable(selected, trigger_name(&trigger)))
                                .on_disabled_hover_text(tr("Site readers are Lua only"))
                                .clicked();
                            if clicked && !selected {
                                // An untouched starter example follows the kind of script.
                                let was_reader = matches!(draft.trigger, Trigger::SiteReader { .. });
                                let was_failure = matches!(draft.trigger, Trigger::DownloadFailed { .. });
                                let failure = matches!(trigger, Trigger::DownloadFailed { .. });
                                if reader && draft.body == LUA_EXAMPLE {
                                    draft.body = READER_EXAMPLE.into();
                                } else if was_reader && draft.body == READER_EXAMPLE {
                                    draft.body = LUA_EXAMPLE.into();
                                } else if failure && draft.body == LUA_EXAMPLE {
                                    draft.body = FAILED_EXAMPLE.into();
                                } else if was_failure && draft.body == FAILED_EXAMPLE {
                                    draft.body = LUA_EXAMPLE.into();
                                }
                                if reader {
                                    draft.change_files = false;
                                }
                                draft.trigger = trigger;
                            }
                        }
                    });
            });
            // Fits "All queues" in every language; longer queue names are shortened.
            let queue_width = combo_width(ui, [tr("All queues")], 135.0);
            match &mut draft.trigger {
                Trigger::Interval { minutes } => {
                    field(ui, tr("Every"), 34.0, 88.0, p, |ui| {
                        ui.add(egui::DragValue::new(minutes).range(1..=10080).suffix(" min"));
                    });
                }
                Trigger::DownloadCompleted { queue } | Trigger::DownloadFailed { queue } => {
                    field(ui, tr("Queue"), 36.0, queue_width, p, |ui| {
                        let label = queue.as_deref().map_or_else(|| tr("All queues").into(), |id| automation::queue_label(queues, id));
                        egui::ComboBox::from_id_salt("script-queue")
                            .width(queue_width)
                            .truncate()
                            .selected_text(&label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(queue, None, tr("All queues"));
                                for q in queues {
                                    ui.selectable_value(queue, Some(q.id.clone()), display_queue_name(&q.id, &q.name));
                                }
                            }).response.on_hover_text(label);
                    });
                }
                Trigger::QueueCompleted { queue } => {
                    field(ui, tr("Queue"), 36.0, queue_width, p, |ui| {
                        let label = automation::queue_label(queues, queue);
                        egui::ComboBox::from_id_salt("script-queue")
                            .width(queue_width)
                            .truncate()
                            .selected_text(&label)
                            .show_ui(ui, |ui| {
                                for q in queues {
                                    ui.selectable_value(queue, q.id.clone(), display_queue_name(&q.id, &q.name));
                                }
                            }).response.on_hover_text(label);
                    });
                }
                Trigger::Manual | Trigger::Startup | Trigger::SiteReader { .. } => {}
            }
            if draft.trigger != Trigger::Manual {
                ui.square_checkbox(&mut draft.enabled, tr("Automatic trigger enabled"));
            }
        });
        if let Trigger::SiteReader { pages: list } = &mut draft.trigger {
            ui.add_space(6.0);
            page_list(ui, list, pages, p);
        } else {
            ui.add_space(6.0);
            access(ui, draft, values, p);
        }
    });
}

/// The files switch and what the script keeps between runs. Site readers have
/// neither: they cannot see downloads.
fn access(ui: &mut egui::Ui, draft: &mut Script, values: &mut Values, p: Palette) {
    let explanation = tr(
        "Lets this script rename, move and delete its downloads' files. Moves stay inside your download folders.",
    );
    ui.horizontal_wrapped(|ui| {
        ui.square_checkbox(&mut draft.change_files, tr("Can change downloaded files"))
            .on_hover_text(explanation);
        ui.add_space(14.0);
        ui.label(RichText::new(tr("Stored values")).small().color(p.muted));
        if values.list.is_empty() {
            ui.label(RichText::new(tr("none")).color(p.muted));
            return;
        }
        let bytes: usize = values
            .list
            .iter()
            .map(|v| v.key.len() + v.value.to_string().len())
            .sum();
        ui.label(if bytes < 1024 {
            format!("{} · {bytes} B", values.list.len())
        } else {
            format!("{} · {:.1} KiB", values.list.len(), bytes as f64 / 1024.0)
        });
        if ui
            .small_button(if values.shown { tr("Hide") } else { tr("Show") })
            .clicked()
        {
            values.shown = !values.shown;
        }
        if ui
            .small_button(tr("Clear"))
            .on_hover_text(tr("Forget every value this script stored"))
            .clicked()
        {
            values.clear = true;
        }
    });
    if draft.change_files {
        ui.label(RichText::new(explanation).small().color(p.muted));
    }
    if !values.shown || values.list.is_empty() {
        return;
    }
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(p.background)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("script-stored-values")
                .max_height(160.0)
                .show(ui, |ui| {
                    // Keys keep their own width (up to a limit); values take the rest.
                    let font = egui::FontId::monospace(12.0);
                    let key_width = values
                        .list
                        .iter()
                        .map(|v| {
                            ui.painter()
                                .layout_no_wrap(v.key.clone(), font.clone(), p.text)
                                .size()
                                .x
                        })
                        .fold(40.0_f32, f32::max)
                        .min(220.0)
                        .ceil();
                    let gap = 16.0;
                    let updated_width = 120.0;
                    let value_width =
                        (ui.available_width() - key_width - updated_width - 2.0 * gap).max(120.0);
                    let cell = |ui: &mut egui::Ui, width: f32, text: RichText| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 18.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(width);
                                ui.add(egui::Label::new(text).truncate())
                            },
                        )
                        .inner
                    };
                    egui::Grid::new("script-stored-values-grid")
                        .num_columns(3)
                        .spacing(egui::vec2(gap, 4.0))
                        .show(ui, |ui| {
                            for (header, width) in [
                                (tr("Key"), key_width),
                                (tr("Value"), value_width),
                                (tr("Updated"), updated_width),
                            ] {
                                cell(ui, width, RichText::new(header).small().color(p.muted));
                            }
                            ui.end_row();
                            for value in &values.list {
                                cell(ui, key_width, RichText::new(&value.key).font(font.clone()))
                                    .on_hover_text(&value.key);
                                let text = value.value.to_string();
                                cell(ui, value_width, RichText::new(&text).font(font.clone()))
                                    .on_hover_text(text.chars().take(2000).collect::<String>());
                                cell(
                                    ui,
                                    updated_width,
                                    RichText::new(time(value.updated)).small().color(p.muted),
                                );
                                ui.end_row();
                            }
                        });
                });
        });
    ui.label(
        RichText::new(tr("Read-only here. Lua scripts change these with xd.store.set; PowerShell uses Invoke-DownloadEngine state set."))
            .small()
            .color(p.muted),
    );
}

/// The reader's page patterns, how to add one, and what a reader may do.
fn page_list(ui: &mut egui::Ui, list: &mut Vec<String>, input: &mut PageInput, p: Palette) {
    use download_engine::script_automation::site;
    ui.horizontal_top(|ui| {
        ui.allocate_ui(egui::vec2(52.0, ui.spacing().interact_size.y), |ui| {
            ui.set_min_width(52.0);
            ui.label(RichText::new(tr("Pages")).small().color(p.muted));
        });
        ui.vertical(|ui| {
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for (index, page) in list.iter().enumerate() {
                    egui::Frame::new()
                        .fill(p.selected)
                        .corner_radius(5)
                        .inner_margin(egui::Margin {
                            left: 8,
                            right: 2,
                            top: 1,
                            bottom: 1,
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(page).monospace().size(12.0));
                                if ui
                                    .small_button("×")
                                    .on_hover_text(tf!("Remove {page}", page = page))
                                    .clicked()
                                {
                                    remove = Some(index);
                                }
                            });
                        });
                }
            });
            if let Some(index) = remove {
                list.remove(index);
            }
            ui.horizontal(|ui| {
                let response = ui.add(
                    view::line_edit(&mut input.text)
                        .id(egui::Id::new("site-reader-page"))
                        .hint_text(tr("Add a page, e.g. example.com/watch/*"))
                        .font(egui::TextStyle::Monospace)
                        .desired_width((ui.available_width() - 110.0).clamp(160.0, 420.0))
                        .char_limit(256),
                );
                let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button(tr("Add page")).clicked() || entered) && !input.text.trim().is_empty() {
                    match site::normalize_page(&input.text) {
                        Ok(page) if list.contains(&page) => input.text.clear(),
                        Ok(_) if list.len() >= site::MAX_PAGES => {
                            input.error = Some(format!(
                                "A site reader can list at most {} pages",
                                site::MAX_PAGES
                            ));
                        }
                        Ok(page) => {
                            list.push(page);
                            input.text.clear();
                            input.error = None;
                        }
                        Err(error) => input.error = Some(error.to_string()),
                    }
                }
            });
            if let Some(error) = &input.error {
                ui.colored_label(p.warning, error);
            }
            ui.label(
                RichText::new(tr("Matching pages open with this script from the Download video button, Find videos on this page, Add URL and Preview video. YouTube, Bilibili, TikTok and Twitch always use Download Engine's built-in readers."))
                    .small()
                    .color(p.muted),
            );
        });
    });
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(p.blue.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, p.blue.gamma_multiply(0.30)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(tr("Site readers can read web pages, but cannot see or change your downloads."))
                    .strong(),
            );
            ui.label(
                RichText::new(tr("xd.fetch reaches the public internet only: home-network and local addresses are blocked, and your browser cookies and logins are never sent."))
                    .small()
                    .color(p.muted),
            );
        });
}

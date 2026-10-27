use super::*;
use download_engine::link_fallback::{Selection as FileSelection, State as Discovery};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct State {
    seen: HashSet<Uuid>,
    review: Option<Review>,
}

struct Review {
    job: String,
    discovery: Discovery,
    selected: Option<usize>,
}

impl Desktop {
    #[cfg(feature = "screenshot")]
    pub(super) fn link_fallback_sample(&mut self, mode: &str) {
        let Ok(mut job) = Job::new(
            "https://files.example/share/design-kit",
            None,
            self.store.paths.downloads.clone(),
            "fallback-screenshot".into(),
        ) else {
            return;
        };
        job.status = Status::Failed;
        let names: Vec<String> = match mode {
            "link-fallback-empty" => vec![],
            "link-fallback-long" => (0..100)
                .map(|n| {
                    format!(
                        "Design resources {} {n}.zip",
                        "with a long filename ".repeat(8)
                    )
                })
                .collect(),
            _ => vec![
                "Design-kit.zip".into(),
                "Preview-images.zip".into(),
                "Readme.pdf".into(),
            ],
        };
        job.link_fallback = Some(Discovery {
            id: Uuid::new_v4(),
            page: job.url.clone(),
            truncated: mode == "link-fallback-long",
            selection: None,
            files: names
                .into_iter()
                .enumerate()
                .map(
                    |(index, name)| download_engine::website_grabber::Candidate {
                        url: format!(
                            "https://files.example/downloads/{index}/design-resources.zip"
                        ),
                        name,
                        filename: None,
                        page: job.url.clone(),
                    },
                )
                .collect(),
        });
        self.open_link_fallback(&job);
    }

    pub(super) fn poll_link_fallback(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        // Prune removed/restarted jobs so a long-running app has bounded history.
        let discoveries: HashSet<_> = self
            .jobs
            .iter()
            .filter_map(|job| job.link_fallback.as_ref().map(|state| state.id))
            .collect();
        self.link_fallback
            .seen
            .retain(|id| discoveries.contains(id));
        let pending = self
            .jobs
            .iter()
            .find(|job| {
                matches!(job.status, Status::Failed | Status::Paused)
                    && job.link_fallback.as_ref().is_some_and(|state| {
                        state.needs_review() && !self.link_fallback.seen.contains(&state.id)
                    })
            })
            .cloned();
        if let Some(job) = pending
            && self.open_link_fallback(&job)
        {
            self.show_window(ctx);
        }
    }

    pub(super) fn open_link_fallback(&mut self, job: &Job) -> bool {
        if !matches!(job.status, Status::Failed | Status::Paused) {
            return false;
        }
        let Some(discovery) = job
            .link_fallback
            .as_ref()
            .filter(|state| state.needs_review())
        else {
            return false;
        };
        if self.dialog.is_some() && self.dialog != Some(Dialog::LinkFallback) {
            self.notice = Some(Notice::info(
                tr("Close the current dialog to review this download's file links.").into(),
            ));
            return true;
        }
        self.link_fallback.seen.insert(discovery.id);
        self.link_fallback.review = Some(Review {
            job: job.id.clone(),
            selected: (discovery.files.len() == 1).then_some(0),
            discovery: discovery.clone(),
        });
        self.dialog = Some(Dialog::LinkFallback);
        true
    }

    pub(super) fn link_fallback_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(review) = &mut self.link_fallback.review else {
            self.dialog = None;
            return;
        };
        let mut close = false;
        let mut selection = None;
        let response = form_dialog::modal("link-fallback", p).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 80.0).clamp(280.0, 580.0));
            close = form_dialog::title(ui, tr("Choose a file"), form_dialog::Icon::Download, p, true);
            ui.label(tr("This link opened a webpage. The link grabber checked it for downloadable files."));
            ui.add(egui::Label::new(RichText::new(&review.discovery.page).small().color(p.muted)).truncate())
                .on_hover_text(&review.discovery.page);
            ui.add_space(6.0);
            let height = (ctx.content_rect().height() - 310.0).clamp(100.0, 320.0);
            egui::ScrollArea::vertical().id_salt("discovered-file-list").max_height(height).show(ui, |ui| {
                if review.discovery.files.is_empty() {
                    ui.label(tr("No direct file links were found. This site may require JavaScript, login, or a dedicated integration. Open the website to continue there."));
                }
                for (index, file) in review.discovery.files.iter().enumerate() {
                    ui.push_id(index, |ui| {
                        let selected = review.selected == Some(index);
                        ui.horizontal(|ui| {
                            if ui.radio(selected, "").clicked() { review.selected = Some(index); }
                            if ui.add(egui::Button::selectable(selected, &file.name).truncate()).clicked() {
                                review.selected = Some(index);
                            }
                        });
                        ui.add(egui::Label::new(RichText::new(&file.url).small().color(p.muted)).truncate())
                            .on_hover_text(&file.url);
                        ui.add_space(6.0);
                    });
                }
            });
            if review.discovery.truncated {
                ui.label(RichText::new(tr("Only part of this page's links could be listed. Use Website grabber for a broader scan.")).color(p.warning));
            }
            ui.hyperlink_to(tr("Open website"), &review.discovery.page);
            ui.label(RichText::new(tr("Your current queue, folder, and download settings will be kept.")).small().color(p.muted));
            form_dialog::controls(ui, |ui| {
                if form_dialog::primary_enabled(ui, review.selected.is_some(), tr("Download selected file")).clicked() {
                    selection = review.selected.map(FileSelection::File);
                }
                if form_dialog::secondary_button(ui, tr("Save HTML page")).clicked() {
                    selection = Some(FileSelection::Page);
                }
            });
        });
        if let Some(selection) = selection {
            let command = Command::SelectDiscoveredFile {
                id: review.job.clone(),
                discovery: review.discovery.id,
                selection,
            };
            match self.scheduler.send(command) {
                Ok(()) => close = true,
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
        }
        if close || response.should_close() {
            self.dialog = None;
            self.link_fallback.review = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
    fn pending(app: &Desktop, count: usize) -> Job {
        let mut job = Job::new(
            "https://files.example/share",
            None,
            app.store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = Status::Failed;
        job.link_fallback = Some(Discovery {
            id: Uuid::new_v4(),
            page: job.url.clone(),
            truncated: false,
            selection: None,
            files: (0..count)
                .map(|index| download_engine::website_grabber::Candidate {
                    url: format!("https://files.example/{index}.zip"),
                    name: format!("Archive {}.zip", index + 1),
                    filename: None,
                    page: job.url.clone(),
                })
                .collect(),
        });
        job
    }
    #[test]
    fn review_waits_for_other_dialogs_and_dismissal_does_not_reopen_it() {
        let (_temp, mut app, ctx) = fixture();
        let job = pending(&app, 2);
        app.store.enqueue(&job).unwrap();
        app.jobs = vec![job.clone()];
        app.dialog = Some(Dialog::About);
        app.poll_link_fallback(&ctx);
        assert_eq!(app.dialog, Some(Dialog::About));
        app.dialog = None;
        app.poll_link_fallback(&ctx);
        assert_eq!(app.dialog, Some(Dialog::LinkFallback));
        click(&mut app, &ctx, "×");
        app.poll_link_fallback(&ctx);
        assert!(app.dialog.is_none());
        assert_eq!(
            app.store.get(&job.id).unwrap().unwrap().status,
            Status::Failed
        );
        app.command(Command::Resume(job.id));
        assert_eq!(app.dialog, Some(Dialog::LinkFallback));
    }
    #[test]
    fn choosing_a_file_preserves_a_paused_queue_and_submits_once() {
        let (_temp, mut app, ctx) = fixture();
        let queue = app.store.create_queue("Review later").unwrap();
        let job = pending(&app, 2);
        app.store
            .enqueue_batch(std::slice::from_ref(&job), &queue.id)
            .unwrap();
        app.open_link_fallback(&job);
        click(&mut app, &ctx, "Download selected file");
        assert_eq!(
            app.dialog,
            Some(Dialog::LinkFallback),
            "no selection means disabled"
        );
        click(&mut app, &ctx, "Archive 2.zip");
        assert_eq!(app.link_fallback.review.as_ref().unwrap().selected, Some(1));
        click(&mut app, &ctx, "Download selected file");
        assert!(app.dialog.is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        let saved = loop {
            let saved = app.store.get(&job.id).unwrap().unwrap();
            if saved.status == Status::Queued {
                break saved;
            }
            assert!(Instant::now() < deadline, "scheduler did not apply review");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            saved.link_fallback.unwrap().selection,
            Some(FileSelection::File(1))
        );
        assert_eq!(app.store.job_queue(&job.id).unwrap(), Some(queue.id));
        assert_eq!(saved.directory, job.directory);
        assert_eq!(saved.url, job.url);
        assert!(app.store.list().unwrap().len() == 1);
    }
    #[test]
    fn empty_and_long_reviews_fit_small_and_large_windows_in_both_themes() {
        let (_temp, mut app, ctx) = fixture();
        for dark in [true, false] {
            app.interface.appearance = if dark {
                Appearance::Dark
            } else {
                Appearance::Light
            };
            app.configure_appearance(&ctx);
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                for count in [0, 3, 100] {
                    let mut job = pending(&app, count);
                    let state = job.link_fallback.as_mut().unwrap();
                    state.truncated = count == 100;
                    if let Some(file) = state.files.first_mut() {
                        file.name = format!("{}.zip", "Long archive name ".repeat(20));
                        file.url.push_str(&format!("?token={}", "a".repeat(2000)));
                    }
                    app.open_link_fallback(&job);
                    frame(&mut app, &ctx, size, vec![]);
                    let output = frame(&mut app, &ctx, size, vec![]);
                    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    for label in [
                        "Choose a file",
                        "Download selected file",
                        "Save HTML page",
                        "Open website",
                    ] {
                        assert!(
                            screen.contains_rect(text_rect(&output, label)),
                            "{size:?} {label}"
                        );
                    }
                    let download = text_rect(&output, "Download selected file");
                    let save = text_rect(&output, "Save HTML page");
                    assert!(!download.intersects(save));
                    app.dialog = None;
                }
            }
        }
    }
}

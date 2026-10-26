use super::*;
use download_engine::{
    browser_batch::{Destination, Options},
    browser_video::{Kind, Pending},
};

pub(super) struct State {
    pending: Pending,
    selected: Vec<bool>,
    search: String,
    queue: String,
    destination: usize,
    options: Options,
    error: Option<String>,
}

impl State {
    fn new(pending: Pending, queue: &str, options: Options) -> Self {
        let queue = options
            .queues
            .iter()
            .find(|q| q.id == queue)
            .or(options.queues.first())
            .map(|q| q.id.clone())
            .unwrap_or_default();
        Self {
            selected: vec![false; pending.review.items.len()],
            pending,
            search: String::new(),
            queue,
            destination: 0,
            options,
            error: None,
        }
    }
    fn destination(&self) -> Destination {
        self.options
            .destinations
            .get(self.destination)
            .map_or(Destination::Automatic, |v| v.id)
    }
}

impl Desktop {
    pub(super) fn poll_browser_video_requests(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some()
            || self.browser_video_request_check.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        self.browser_video_request_check = Instant::now();
        let result = self.store.next_browser_video_review().and_then(|pending| {
            pending
                .map(|pending| {
                    Ok(State::new(
                        pending,
                        &self.queue_id,
                        self.store.browser_batch_options()?,
                    ))
                })
                .transpose()
        });
        match result {
            Ok(Some(state)) => {
                let page = state.pending.review.page_url.clone();
                if download_engine::hls::is_site_video(&page) || self.has_site_reader(&page) {
                    // A built-in reader gives the best result; the browser's own
                    // findings stay pending in case the reader fails.
                    let queue = state.queue.clone();
                    if state.pending.review.items.is_empty() {
                        if let Err(error) =
                            self.store.dismiss_browser_video_review(&state.pending.id)
                        {
                            self.notice = Some(Notice::error(tf!(
                                "Cannot open the video preview: {error:#}",
                                error = error
                            )));
                            return;
                        }
                        self.open_browser_stream(&page, &queue, None);
                    } else {
                        self.open_site_video(state, &page, &queue);
                    }
                    self.show_window(ctx);
                    return;
                }
                self.browser_video = Some(state);
                self.dialog = Some(Dialog::BrowserVideo);
                self.show_window(ctx);
            }
            Ok(None) => {}
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot open video review: {error:#}",
                    error = error
                )))
            }
        }
    }

    /// Show the browser's findings after a site reader failed on the same page.
    pub(super) fn open_browser_video_fallback(&mut self, mut state: State, error: &str) {
        state.error = Some(tf!(
            "Showing the media your browser found because this page could not be read directly: {error}",
            error = error
        ));
        self.browser_video = Some(state);
        self.dialog = Some(Dialog::BrowserVideo);
    }

    /// Drop a fallback review that the site reader made unnecessary.
    pub(super) fn discard_browser_video(&mut self, state: State) {
        if let Err(error) = self.store.dismiss_browser_video_review(&state.pending.id) {
            self.notice = Some(Notice::error(tf!(
                "Cannot dismiss this video review: {error:#}",
                error = error
            )));
        }
    }

    /// Show what a site reader found in the same list the browser's findings use.
    pub(super) fn open_found_videos(
        &mut self,
        review: download_engine::browser_video::Review,
        queue: &str,
    ) {
        let id = format!("video:{}", Uuid::new_v4());
        let result = self
            .store
            .request_browser_video_review(&id, &review)
            .and_then(|_| self.store.browser_batch_options());
        match result {
            Ok(options) => {
                self.browser_video = Some(State::new(Pending { id, review }, queue, options));
                self.dialog = Some(Dialog::BrowserVideo);
            }
            Err(error) => {
                self.notice = Some(Notice::error(tf!(
                    "Cannot open video review: {error:#}",
                    error = error
                )))
            }
        }
    }

    fn close_browser_video(&mut self) -> bool {
        let Some(draft) = &mut self.browser_video else {
            return false;
        };
        if let Err(error) = self.store.dismiss_browser_video_review(&draft.pending.id) {
            draft.error = Some(tf!(
                "Cannot dismiss this video review: {error:#}",
                error = error
            ));
            return false;
        }
        self.browser_video = None;
        self.dialog = None;
        true
    }

    fn add_browser_video_files(&mut self) {
        let Some(draft) = &mut self.browser_video else {
            return;
        };
        let indices: Vec<_> = draft
            .selected
            .iter()
            .enumerate()
            .filter_map(|(i, selected)| selected.then_some(i))
            .collect();
        match self.store.add_browser_video_selection(
            &draft.pending.id,
            &indices,
            &draft.queue,
            draft.destination(),
        ) {
            Ok(ids) => {
                self.queue_id.clone_from(&draft.queue);
                if let [id] = ids.as_slice() {
                    self.expect_added_status(id);
                }
                self.set_selection(ids.first().cloned());
                self.view = View::Downloads;
                self.filter = Filter::All;
                self.search.clear();
                self.category = None;
                self.browser_video = None;
                self.dialog = None;
                self.notice = Some(Notice::success(tf!(
                    "Added {} selected video files.",
                    ids.len()
                )));
                self.force_refresh();
            }
            Err(error) => {
                draft.error = Some(tf!("Could not add video files: {error:#}", error = error))
            }
        }
    }

    pub(super) fn browser_video_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(draft) = &mut self.browser_video else {
            self.dialog = None;
            return;
        };
        let (mut close, mut add, mut reload) = (false, false, false);
        let mut preview = None;
        let empty = draft.pending.review.items.is_empty();
        let response = form_dialog::modal("browser-video-review", p).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width()-100.0).clamp(420.0,760.0));
                close = form_dialog::title(ui, tr("Video downloads"), form_dialog::Icon::Symbol("\u{e714}"), p, true);
                egui::ScrollArea::vertical().id_salt("browser-video-body").max_height((ctx.content_rect().height()-190.0).max(260.0)).show(ui, |ui| {
                    let review = &draft.pending.review;
                    ui.add(egui::Label::new(RichText::new(&review.page_title).strong()).truncate()).on_hover_text(&review.page_title);
                    ui.add(egui::Label::new(RichText::new(&review.page_url).small().color(p.muted)).truncate()).on_hover_text(&review.page_url);
                    ui.add_space(10.0);
                    if empty {
                        ui.strong(tr("No supported video found"));
                        ui.label(tr("The browser did not find a direct video file or an HLS / DASH playlist on this page."));
                        ui.label(tr("Turn on Detect media while browsing in the extension, play the video, then try Find videos again."));
                    } else {
                        ui.label(RichText::new(tr("Select files to download, or choose a stream quality.")).color(p.muted));
                        ui.add(view::line_edit(&mut draft.search).hint_text(tr("Search videos…")).desired_width(f32::INFINITY).char_limit(200));
                        let needle = draft.search.trim().to_lowercase();
                        let visible: Vec<_> = review.items.iter().enumerate().filter(|(_, item)| needle.is_empty() ||
                            [&item.filename,&item.label,&item.url].iter().any(|text| text.to_lowercase().contains(&needle))).map(|(i,_)| i).collect();
                        // Checkboxes and HLS/DASH tags share one lead column so every name starts at the same x.
                        let lead = ui.painter().layout_no_wrap("DASH".into(), egui::TextStyle::Body.resolve(ui.style()), p.accent)
                            .size().x.max(ui.spacing().icon_width);
                        egui::ScrollArea::vertical().id_salt("browser-video-list").max_height(160.0).show(ui, |ui| {
                            if visible.is_empty() { ui.label(tr("No videos match your search.")); }
                            for index in visible {
                                let item = &review.items[index];
                                ui.push_id(index, |ui| {
                                    ui.horizontal(|ui| {
                                        let selection = ui.horizontal(|ui| {
                                            ui.set_width(lead);
                                            if item.kind == Kind::File { Some(ui.square_checkbox(&mut draft.selected[index], "")) }
                                            else { ui.label(RichText::new(if item.kind == Kind::Hls { "HLS" } else { "DASH" }).color(p.accent)); None }
                                        }).inner;
                                        let label = ui.vertical(|ui| {
                                            ui.set_width((ui.available_width()-160.0).max(180.0));
                                            let label = ui.add(egui::Label::new(&item.filename).truncate().sense(egui::Sense::click())).on_hover_text(&item.url);
                                            if !item.label.is_empty() { ui.add(egui::Label::new(RichText::new(&item.label).small().color(p.muted)).truncate()); }
                                            label
                                        }).inner;
                                        if let Some(selection) = selection { selection.labelled_by(label.id); if label.clicked() { draft.selected[index] = !draft.selected[index]; } }
                                        if item.kind == Kind::File { ui.label(item.size.map(|size| format!("{:.1} MB",size as f64/1_048_576.0)).unwrap_or_else(|| tr("Size unknown").into())); }
                                        else if ui.button(tr("Choose quality")).clicked() { preview = Some(index); }
                                    });
                                    ui.separator();
                                });
                            }
                        });
                        if review.truncated { ui.colored_label(p.warning,tr("This page has more media than can be shown in one review.")); }
                        ui.horizontal(|ui| {
                            ui.label(tr("Queue"));
                            egui::ComboBox::from_id_salt("browser-video-queue").width(220.0)
                                .selected_text(draft.options.queues.iter().find(|q| q.id == draft.queue).map(|q| display_queue_name(&q.id, &q.name)).unwrap_or(tr("Choose queue")))
                                .show_ui(ui, |ui| { for queue in &draft.options.queues { ui.selectable_value(&mut draft.queue, queue.id.clone(), display_queue_name(&queue.id, &queue.name)); } });
                            ui.label(tr("Save to"));
                            egui::ComboBox::from_id_salt("browser-video-folder").width(200.0)
                                .selected_text(draft.options.destinations.get(draft.destination).map(|d| d.name.as_str()).unwrap_or(tr("Choose folder")))
                                .show_ui(ui, |ui| { for (i, folder) in draft.options.destinations.iter().enumerate() { ui.selectable_value(&mut draft.destination, i, &folder.name); } });
                        });
                        let folder = draft.options.destinations.get(draft.destination).and_then(|d| d.path.as_deref())
                            .unwrap_or(tr("Uses your download folder rules in Preferences."));
                        ui.add(egui::Label::new(RichText::new(folder).small().color(p.muted)).truncate()).on_hover_text(folder);
                        reload = ui.button(tr("Reload destinations")).clicked();
                    }
                    if let Some(error) = &draft.error { ui.colored_label(p.warning,error); }
                });
                form_dialog::controls(ui, |ui| {
                    if !empty {
                        let count = draft.selected.iter().filter(|selected| **selected).count();
                        add = form_dialog::primary_enabled(ui, count > 0, tr("Add selected files")).clicked();
                        ui.label(RichText::new(tf!("{count} selected", count = count)).color(p.muted));
                    }
                    if empty { ui.label(RichText::new(tr("Nothing has been downloaded.")).small().color(p.muted)); }
                });
            });
        if reload {
            match self.store.browser_batch_options() {
                Ok(options) => {
                    draft.options = options;
                    draft.error = None;
                }
                Err(error) => draft.error = Some(error.to_string()),
            }
        }
        if close || response.should_close() {
            self.close_browser_video();
        } else if let Some(index) = preview {
            let queue = draft.queue.clone();
            let folder = draft
                .options
                .destinations
                .get(draft.destination)
                .and_then(|d| d.path.clone());
            let page = draft.pending.review.page_url.clone();
            let public_only = draft.pending.review.public_only;
            match self
                .store
                .take_browser_video_stream(&draft.pending.id, index)
            {
                Ok(url) => {
                    self.browser_video = None;
                    self.dialog = None;
                    self.open_page_stream(&url, Some(&page), &queue, folder.as_deref());
                    if public_only {
                        self.restrict_stream_to_public();
                    }
                }
                Err(error) => draft.error = Some(error.to_string()),
            }
        } else if add {
            self.add_browser_video_files();
        }
    }

    #[cfg(any(test, feature = "screenshot"))]
    pub(super) fn browser_video_sample(&mut self, empty: bool) {
        use download_engine::browser_video::{Item, Review};
        let review = Review {
            public_only: false,
            page_url: if empty {
                "https://example.com/watch/no-media"
            } else {
                "https://example.com/watch/nature"
            }
            .into(),
            page_title: if empty {
                tr("Video from a web page")
            } else {
                tr("Nature documentary")
            }
            .into(),
            truncated: false,
            items: if empty {
                vec![]
            } else {
                vec![
                    Item {
                        url: "https://example.com/nature-1080p.mp4".into(),
                        filename: "Nature-documentary-1080p.mp4".into(),
                        label: "Video file".into(),
                        kind: Kind::File,
                        size: Some(84_600_000),
                    },
                    Item {
                        url: "https://example.com/master.m3u8".into(),
                        filename: "Nature documentary".into(),
                        label: "Quality and audio available in the next step".into(),
                        kind: Kind::Hls,
                        size: None,
                    },
                ]
            },
        };
        let id = format!("video:{}", Uuid::new_v4());
        self.store
            .request_browser_video_review(&id, &review)
            .unwrap();
        self.browser_video = Some(State::new(
            Pending { id, review },
            &self.queue_id,
            self.store.browser_batch_options().unwrap(),
        ));
        self.dialog = Some(Dialog::BrowserVideo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};

    #[test]
    fn browser_video_review_fits_both_themes_with_populated_and_empty_states() {
        for dark in [true, false] {
            for empty in [true, false] {
                let (_temp, mut app, ctx) = fixture();
                app.interface.appearance = if dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                app.configure_appearance(&ctx);
                app.browser_video_sample(empty);
                for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                    frame(&mut app, &ctx, size, vec![]);
                    let output = frame(&mut app, &ctx, size, vec![]);
                    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    let labels = if empty {
                        vec!["Video downloads", "No supported video found", "×"]
                    } else {
                        vec![
                            "Video downloads",
                            "Choose quality",
                            "Queue",
                            "Save to",
                            "Add selected files",
                            "×",
                        ]
                    };
                    for label in labels {
                        assert!(
                            screen.contains_rect(text_rect(&output, label)),
                            "{label} outside {size:?}"
                        );
                    }
                }
                assert!(app.store.list().unwrap().is_empty());
                click(&mut app, &ctx, "×");
                assert!(app.dialog.is_none());
                assert!(app.store.next_browser_video_review().unwrap().is_none());
                assert!(app.store.list().unwrap().is_empty());
            }
        }
    }

    #[test]
    fn browser_video_review_requires_selection_and_keeps_errors_for_retry() {
        let (_temp, mut app, ctx) = fixture();
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.browser_video_sample(false);
        click(&mut app, &ctx, "Add selected files");
        assert!(app.store.list().unwrap().is_empty());
        click(&mut app, &ctx, "Nature-documentary-1080p.mp4");
        let draft = app.browser_video.as_mut().unwrap();
        assert!(draft.selected[0]);
        draft.queue = "missing".into();
        click(&mut app, &ctx, "Add selected files");
        assert!(app.browser_video.as_ref().unwrap().error.is_some());
        assert!(app.store.list().unwrap().is_empty());
        app.browser_video.as_mut().unwrap().queue = MAIN_QUEUE.into();
        click(&mut app, &ctx, "Add selected files");
        assert!(app.dialog.is_none());
        let jobs = app.store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "Nature-documentary-1080p.mp4");
        assert!(jobs[0].hls.is_none());
        assert!(app.store.next_browser_video_review().unwrap().is_none());
    }

    #[test]
    fn incoming_browser_review_waits_for_existing_dialog_and_stays_pending_until_dismissed() {
        let (_temp, mut app, ctx) = fixture();
        app.browser_video_sample(false);
        app.browser_video = None;
        app.dialog = Some(Dialog::About);
        app.poll_browser_video_requests(&ctx);
        assert_eq!(app.dialog, Some(Dialog::About));
        app.dialog = None;
        app.poll_browser_video_requests(&ctx);
        assert_eq!(app.dialog, Some(Dialog::BrowserVideo));
        assert!(app.store.next_browser_video_review().unwrap().is_some());
        click(&mut app, &ctx, "×");
        assert!(app.store.next_browser_video_review().unwrap().is_none());
    }

    #[test]
    fn browser_stream_choice_opens_existing_quality_picker_without_a_download() {
        let (_temp, mut app, ctx) = fixture();
        app.browser_video_sample(false);
        let old = app.browser_video.take().unwrap();
        app.store
            .dismiss_browser_video_review(&old.pending.id)
            .unwrap();
        let mut pending = old.pending;
        pending.id = format!("video:{}", Uuid::new_v4());
        pending.review.items[1].url = "http://127.0.0.1:1/master.m3u8".into();
        app.store
            .request_browser_video_review(&pending.id, &pending.review)
            .unwrap();
        app.browser_video = Some(State::new(
            pending,
            MAIN_QUEUE,
            app.store.browser_batch_options().unwrap(),
        ));
        click(&mut app, &ctx, "Choose quality");
        assert_eq!(app.dialog, Some(Dialog::Hls));
        assert!(app.store.list().unwrap().is_empty());
        assert!(app.browser_video.is_none());
        click(&mut app, &ctx, "×");
        assert!(app.dialog.is_none());
    }
}

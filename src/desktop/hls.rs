use super::*;
use download_engine::hls::{self as service, Inspection, Playlist};
use std::sync::mpsc;
use tokio_util::sync::CancellationToken;

type Incoming = (String, String, bool);
#[derive(Default)]
pub(super) struct Requests {
    reading: Option<background_io::Task<Option<Incoming>>>,
    ready: Option<Incoming>,
    acknowledge: Option<background_io::Task<()>>,
}

enum Loaded {
    Inspection(Inspection, Option<String>),
    Preview(
        Playlist,
        Option<download_engine::youtube::Selection>,
        Option<download_engine::bilibili::Selection>,
        Option<download_engine::tiktok::Selection>,
    ),
    /// A site reader found several items, or files: the Video downloads list.
    Found(download_engine::browser_video::Review),
}
struct Task {
    inspecting: bool,
    cancel: CancellationToken,
    result: mpsc::Receiver<anyhow::Result<Loaded>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(super) struct State {
    store: Store,
    url: String,
    inspection: Option<Inspection>,
    selected: usize,
    audio_selected: usize,
    preview: Option<Playlist>,
    name: String,
    suggested_name: String,
    folder: String,
    queue: String,
    receipt: String,
    task: Option<Task>,
    saving: Option<background_io::Task<Job>>,
    error: Option<String>,
    title: Option<String>,
    youtube_selection: Option<download_engine::youtube::Selection>,
    bilibili_selection: Option<download_engine::bilibili::Selection>,
    tiktok_selection: Option<download_engine::tiktok::Selection>,
    start_on_poll: bool,
    /// The browser page a detected stream came from (sent as Referer).
    page: Option<String>,
    public_only: bool,
    /// The browser's own findings for a site page, kept until the site reader
    /// either works or fails; then they are offered instead.
    fallback: Option<super::browser_video::State>,
    reader_failed: Option<String>,
    found: Option<download_engine::browser_video::Review>,
}
impl State {
    #[cfg(test)]
    pub(super) fn saving(&self) -> bool {
        self.saving.is_some()
    }
    fn new(url: &str, queue: &str, store: Store) -> Self {
        Self {
            store,
            url: url.into(),
            inspection: None,
            selected: 0,
            audio_selected: 0,
            preview: None,
            name: String::new(),
            suggested_name: String::new(),
            folder: String::new(),
            queue: queue.into(),
            receipt: Uuid::new_v4().to_string(),
            task: None,
            saving: None,
            error: None,
            title: None,
            youtube_selection: None,
            bilibili_selection: None,
            tiktok_selection: None,
            start_on_poll: false,
            page: None,
            public_only: false,
            fallback: None,
            reader_failed: None,
            found: None,
        }
    }
    fn start(&mut self, inspect: bool) {
        // Dropping the old receiver and cancellation token prevents a superseded
        // quality/audio result from becoming the review the user can enqueue.
        self.invalidate_preview();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let (sender, result) = mpsc::channel();
        let url = self.url.clone();
        let page = self.page.clone();
        let public_only = self.public_only;
        let store = self.store.clone();
        let inspection = self.inspection.clone();
        let selected = self.selected;
        let audio_selected = self.audio_selected;
        if inspect {
            self.inspection = None;
            self.title = None;
            self.name.clear();
            self.suggested_name.clear();
        }
        self.task = Some(Task {
            inspecting: inspect,
            cancel: token,
            result,
        });
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| {
                    runtime.block_on(async {
                        let route = download_engine::proxy::Route::load(&store)?;
                        if inspect {
                            if public_only {
                                service::inspect_reader_via(
                                    &url,
                                    page.as_deref().unwrap_or(&url),
                                    &cancel,
                                    route,
                                )
                                .await
                                .map(|i| Loaded::Inspection(i, None))
                            } else if download_engine::youtube::is_url(&url) {
                                download_engine::youtube::inspect_via(&url, &cancel, route)
                                    .await
                                    .map(|video| {
                                        Loaded::Inspection(video.inspection, Some(video.title))
                                    })
                            } else if download_engine::bilibili::is_url(&url) {
                                download_engine::bilibili::inspect_via(&url, &cancel, route)
                                    .await
                                    .map(|video| {
                                        Loaded::Inspection(video.inspection, Some(video.title))
                                    })
                            } else if download_engine::tiktok::is_url(&url) {
                                download_engine::tiktok::inspect_via(&url, &cancel, route)
                                    .await
                                    .map(|video| {
                                        Loaded::Inspection(video.inspection, Some(video.title))
                                    })
                            } else if download_engine::twitch::is_url(&url) {
                                // Twitch qualities are ordinary HLS playlists from here on.
                                download_engine::twitch::inspect_via(&url, &cancel, route)
                                    .await
                                    .map(|video| {
                                        Loaded::Inspection(video.inspection, Some(video.title))
                                    })
                            } else if let Some(reader) = store.site_reader_for(&url)? {
                                // The user's own site reader; its page is the Referer.
                                let found = download_engine::script_automation::read_page(
                                    &store, &reader.id, &url, &cancel,
                                )
                                .await?;
                                if let Some(stream) = found.single_stream() {
                                    service::inspect_reader_via(&stream.url, &url, &cancel, route)
                                        .await
                                        .map(|i| Loaded::Inspection(i, found.title.clone()))
                                } else {
                                    anyhow::ensure!(
                                        !found.items.is_empty(),
                                        "{}",
                                        tf!(
                                            "{name} found no videos on this page",
                                            name = reader.name
                                        )
                                    );
                                    found.review(&url).map(Loaded::Found)
                                }
                            } else {
                                service::inspect_page_via(&url, page.as_deref(), &cancel, route)
                                    .await
                                    .map(|i| Loaded::Inspection(i, None))
                            }
                        } else {
                            let inspection = inspection.as_ref().ok_or_else(|| {
                                anyhow::anyhow!(tr("Load the video qualities first"))
                            })?;
                            if !inspection.public_only
                                && download_engine::youtube::is_url(&inspection.source)
                            {
                                download_engine::youtube::preview_via(
                                    inspection,
                                    selected,
                                    audio_selected,
                                    &cancel,
                                    route,
                                )
                                .await
                                .map(|p| Loaded::Preview(p.playlist, Some(p.selection), None, None))
                            } else if !inspection.public_only
                                && download_engine::bilibili::is_url(&inspection.source)
                            {
                                download_engine::bilibili::preview_via(
                                    inspection,
                                    selected,
                                    audio_selected,
                                    &cancel,
                                    route,
                                )
                                .await
                                .map(|p| Loaded::Preview(p.playlist, None, Some(p.selection), None))
                            } else if !inspection.public_only
                                && download_engine::tiktok::is_url(&inspection.source)
                            {
                                // One MP4 file per quality: the listing already has everything.
                                download_engine::tiktok::preview(inspection, selected).map(|p| {
                                    Loaded::Preview(p.playlist, None, None, Some(p.selection))
                                })
                            } else {
                                service::preview_with_audio_via(
                                    inspection,
                                    selected,
                                    audio_selected,
                                    &cancel,
                                    route,
                                )
                                .await
                                .map(|p| Loaded::Preview(p, None, None, None))
                            }
                        }
                    })
                });
            let _ = sender.send(result);
        });
    }
    fn invalidate_preview(&mut self) {
        self.task = None;
        self.preview = None;
        self.youtube_selection = None;
        self.bilibili_selection = None;
        self.tiktok_selection = None;
        self.error = None;
    }
    fn set_preview(&mut self, playlist: Playlist) {
        let stem = self
            .title
            .clone()
            .or_else(|| {
                validate_url(&self.url).ok().and_then(|url| {
                    url.path_segments()
                        .and_then(|mut parts| parts.next_back())
                        .map(str::to_owned)
                })
            })
            .unwrap_or_else(|| "video".into());
        let stem = if self.title.is_some() {
            stem.as_str()
        } else {
            std::path::Path::new(&stem)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("video")
        };
        // Leave room for the extension within the model's Windows UTF-16 limit.
        let mut units = 0;
        let stem: String = stem
            .chars()
            .take_while(|c| {
                units += c.len_utf16();
                units <= 150
            })
            .collect();
        let suggested_name = format!(
            "{}.{}",
            download_engine::model::safe_filename(&stem),
            playlist.output_extension()
        );
        if self.name.is_empty() || self.name == self.suggested_name {
            self.name.clone_from(&suggested_name);
        } else if let Some((stem, extension)) = self.name.rsplit_once('.')
            && std::path::Path::new(&self.suggested_name)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|old| old.eq_ignore_ascii_case(extension))
            && !extension.eq_ignore_ascii_case(playlist.output_extension())
        {
            // Keep a custom title when a new selection changes the output container.
            self.name = format!("{stem}.{}", playlist.output_extension());
        }
        self.suggested_name = suggested_name;
        self.receipt = Uuid::new_v4().to_string();
        self.preview = Some(playlist);
    }
    fn poll(&mut self) {
        if self.start_on_poll {
            self.start_on_poll = false;
            self.start(true);
        }
        let Some(task) = &self.task else {
            return;
        };
        let inspecting = task.inspecting;
        let result = match task.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!(tr(
                "Video preview worker stopped. Try again."
            ))),
        };
        self.task = None;
        match result {
            Ok(Loaded::Inspection(inspection, title)) => {
                self.title = title;
                self.selected = inspection
                    .variants
                    .iter()
                    .position(|v| v.unsupported.is_none())
                    .unwrap_or(0);
                self.audio_selected = inspection
                    .variants
                    .get(self.selected)
                    .map_or(0, service::Variant::default_audio);
                if let Some(playlist) = inspection.playlist.clone() {
                    self.set_preview(playlist);
                }
                self.inspection = Some(inspection);
                if self.preview.is_none()
                    && self.inspection.as_ref().is_some_and(|i| {
                        i.variants
                            .get(self.selected)
                            .is_some_and(|v| v.unsupported.is_none())
                    })
                {
                    self.start(false);
                }
            }
            Ok(Loaded::Preview(playlist, youtube, bilibili, tiktok)) => {
                self.set_preview(playlist);
                self.youtube_selection = youtube;
                self.bilibili_selection = bilibili;
                self.tiktok_selection = tiktok;
            }
            Ok(Loaded::Found(review)) => self.found = Some(review),
            Err(error) if inspecting && self.fallback.is_some() => {
                self.reader_failed = Some(format!("{error:#}"));
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
    }
}
impl Desktop {
    #[cfg(test)]
    pub(super) fn hls_requests_pending(&self) -> bool {
        self.hls_requests.reading.is_some() || self.hls_requests.acknowledge.is_some()
    }
    #[cfg(any(test, feature = "screenshot"))]
    pub(super) fn youtube_sample(&mut self, mode: &str) {
        self.hls_audio_sample(true);
        let draft = self.hls.as_mut().unwrap();
        draft.url = "https://www.youtube.com/watch?v=YE7VzlLtp-4".into();
        draft.title = Some("Big Buck Bunny".into());
        let media = |tag, suffix: &str| {
            format!(
                "https://manifest.googlevideo.com/api/manifest/hls_playlist/id/fixture/itag/{tag}/{suffix}"
            )
        };
        let inspection = draft.inspection.as_mut().unwrap();
        inspection.source.clone_from(&draft.url);
        for (i, variant) in inspection.variants.iter_mut().enumerate() {
            variant.url = media(95 - i, "file/index.m3u8");
            for (i, audio) in variant.audio.iter_mut().enumerate() {
                audio.url = Some(media(234 - i, "file/index.m3u8"));
            }
        }
        let mut preview = draft.preview.take().unwrap();
        preview.source.clone_from(&draft.url);
        preview.url.clone_from(&inspection.variants[0].url);
        for (i, part) in preview.parts.iter_mut().enumerate() {
            part.url = media(95, &format!("gosq/{i}/file/segment.ts"));
        }
        let audio = preview.audio.as_mut().unwrap();
        audio.url = media(234, "file/index.m3u8");
        audio.format = service::Format::PackedAac;
        for (i, part) in audio.parts.iter_mut().enumerate() {
            part.url = media(234, &format!("gosq/{i}/file/segment.aac"));
        }
        draft.youtube_selection = Some(download_engine::youtube::Selection {
            video_id: "YE7VzlLtp-4".into(),
            video_itag: 95,
            audio_itag: Some(234),
            audio_name: Some("English".into()),
            audio_language: Some("en".into()),
            resolution: inspection.variants[0].resolution,
            codecs: inspection.variants[0].codecs.clone(),
        });
        draft.set_preview(preview);
        if mode != "youtube-preview" {
            draft.preview = None;
            draft.youtube_selection = None;
        }
        if mode == "youtube-error" {
            draft.inspection = None;
            draft.title = None;
            draft.error=Some(tr("YouTube requires sign-in, an age check, or a browser verification for this video. This extractor supports public videos without sign-in.").into());
        }
    }
    #[cfg(any(test, feature = "screenshot"))]
    pub(super) fn hls_sample(&mut self, preview: bool) {
        self.open_hls("https://example.com/video/master.m3u8");
        let Some(draft) = &mut self.hls else {
            return;
        };
        let variants = [(1920, 1080, 4_500_000), (1280, 720, 2_000_000)]
            .into_iter()
            .map(|(w, h, bandwidth)| service::Variant {
                url: format!("https://example.com/video/{h}.m3u8"),
                bandwidth,
                resolution: Some((w, h)),
                codecs: "avc1.640028, mp4a.40.2".into(),
                unsupported: None,
                audio: Vec::new(),
            })
            .collect::<Vec<_>>();
        let playlist = Playlist {
            public_only: false,
            source: draft.url.clone(),
            url: variants[0].url.clone(),
            quality: variants[0].label(),
            bandwidth: Some(4_500_000),
            duration_ms: 180_000,
            sequence: 0,
            format: service::Format::TransportStream,
            parts: (0..30)
                .map(|i| service::Part {
                    url: format!("https://example.com/video/{i}.ts"),
                    range: None,
                    initialization: false,
                })
                .collect(),
            audio: None,
            dash: None,
        };
        draft.inspection = Some(Inspection {
            public_only: false,
            source: draft.url.clone(),
            variants,
            playlist: None,
            dash: None,
            tiktok: None,
            page: None,
        });
        if preview {
            draft.set_preview(playlist);
            draft.name = "Sample video.ts".into();
        }
    }
    #[cfg(any(test, feature = "screenshot"))]
    pub(super) fn hls_audio_sample(&mut self, preview: bool) {
        self.hls_sample(preview);
        let draft = self.hls.as_mut().unwrap();
        for variant in &mut draft.inspection.as_mut().unwrap().variants {
            variant.audio = [(tr("English"), "en"), (tr("Spanish"), "es")]
                .into_iter()
                .map(|(name, language)| service::AudioRendition {
                    name: name.into(),
                    language: Some(language.into()),
                    url: Some(format!("https://example.com/audio/{language}.m3u8")),
                    is_default: language == "en",
                })
                .collect();
        }
        if let Some(playlist) = &mut draft.preview {
            playlist.audio = Some(service::AudioTrack {
                name: tr("English").into(),
                language: Some("en".into()),
                url: "https://example.com/audio/en.m3u8".into(),
                duration_ms: playlist.duration_ms,
                sequence: 41,
                format: service::Format::TransportStream,
                parts: (0..30)
                    .map(|i| service::Part {
                        url: format!("https://example.com/audio/{i}.ts"),
                        range: None,
                        initialization: false,
                    })
                    .collect(),
            });
            draft.name = "Sample video.mp4".into();
        }
    }
    #[cfg(any(test, feature = "screenshot"))]
    pub(super) fn dash_sample(&mut self, preview: bool) {
        self.hls_audio_sample(preview);
        let draft = self.hls.as_mut().unwrap();
        draft.url = "https://example.com/video/main.mpd".into();
        draft
            .inspection
            .as_mut()
            .unwrap()
            .source
            .clone_from(&draft.url);
        if let Some(playlist) = &mut draft.preview {
            playlist.source.clone_from(&draft.url);
            playlist.url.clone_from(&draft.url);
            playlist.format = service::Format::FragmentedMp4;
            playlist.parts.insert(
                0,
                service::Part {
                    url: "https://example.com/video/init.mp4".into(),
                    range: None,
                    initialization: true,
                },
            );
            let audio = playlist.audio.as_mut().unwrap();
            audio.url.clone_from(&draft.url);
            audio.format = service::Format::FragmentedMp4;
            audio.parts.insert(
                0,
                service::Part {
                    url: "https://example.com/audio/init.mp4".into(),
                    range: None,
                    initialization: true,
                },
            );
            playlist.dash = Some(download_engine::dash::Selection {
                video: "0/1080p".into(),
                audio: Some("1/en".into()),
                video_offset_us: 0,
                audio_offset_us: 0,
                video_index: None,
                audio_index: None,
            });
        }
    }
    pub(super) fn open_hls(&mut self, url: &str) {
        if self.dialog.is_some() {
            return;
        }
        self.hls = Some(State::new(url, &self.queue_id, self.store.clone()));
        self.dialog = Some(Dialog::Hls);
    }
    pub(super) fn poll_hls_requests(&mut self, ctx: &egui::Context) {
        if let Some(result) = self
            .hls_requests
            .acknowledge
            .as_mut()
            .and_then(background_io::Task::poll)
        {
            self.hls_requests.acknowledge = None;
            if let Err(error) = result {
                self.notice = Some(Notice::error(tf!(
                    "Cannot acknowledge HLS preview: {error}",
                    error = error
                )));
            }
        }
        if let Some(result) = self
            .hls_requests
            .reading
            .as_mut()
            .and_then(background_io::Task::poll)
        {
            self.hls_requests.reading = None;
            match result {
                Ok(incoming) => self.hls_requests.ready = incoming,
                Err(error) => {
                    self.notice = Some(Notice::error(tf!(
                        "Cannot read HLS previews: {error}",
                        error = error
                    )))
                }
            }
        }
        if self.dialog.is_some() {
            return;
        }
        if let Some((id, url, reader)) = self.hls_requests.ready.take() {
            if reader {
                self.open_browser_stream(&url, &self.queue_id.clone(), None);
            } else {
                self.open_hls(&url);
            }
            let store = self.store.clone();
            match background_io::Task::spawn(Some(ctx.clone()), move || {
                store.acknowledge_hls_preview(&id)
            }) {
                Ok(task) => self.hls_requests.acknowledge = Some(task),
                Err(error) => self.notice = Some(Notice::error(error.to_string())),
            }
            self.show_window(ctx);
            return;
        }
        if self.hls_requests.reading.is_some()
            || self.hls_requests.acknowledge.is_some()
            || self.hls_request_check.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        self.hls_request_check = Instant::now();
        let store = self.store.clone();
        match background_io::Task::spawn(Some(ctx.clone()), move || {
            store
                .next_hls_preview()?
                .map(|(id, url)| {
                    let reader = download_engine::hls::is_site_video(&url)
                        || store.site_reader_for(&url)?.is_some();
                    Ok((id, url, reader))
                })
                .transpose()
        }) {
            Ok(task) => self.hls_requests.reading = Some(task),
            Err(error) => self.notice = Some(Notice::error(error.to_string())),
        }
    }
    pub(super) fn open_browser_stream(&mut self, url: &str, queue: &str, folder: Option<&str>) {
        self.open_page_stream(url, None, queue, folder);
    }
    /// A stream the browser found on `page`, whose origin is sent as the Referer.
    pub(super) fn open_page_stream(
        &mut self,
        url: &str,
        page: Option<&str>,
        queue: &str,
        folder: Option<&str>,
    ) {
        self.open_hls(url);
        if let Some(draft) = &mut self.hls {
            draft.page = page.map(str::to_owned);
            draft.queue = queue.into();
            draft.folder = folder.unwrap_or("").into();
            draft.start_on_poll = true;
        }
    }
    pub(super) fn restrict_stream_to_public(&mut self) {
        if let Some(draft) = &mut self.hls {
            draft.public_only = true;
        }
    }
    /// Read a site page with its built-in reader; if that fails, show what the
    /// browser itself detected on the page instead.
    pub(super) fn open_site_video(
        &mut self,
        fallback: super::browser_video::State,
        url: &str,
        queue: &str,
    ) {
        self.open_browser_stream(url, queue, None);
        if let Some(draft) = &mut self.hls {
            draft.fallback = Some(fallback);
        }
    }
    fn close_hls(&mut self) {
        if self
            .hls
            .as_ref()
            .is_some_and(|draft| draft.saving.is_some())
        {
            return;
        }
        if let Some(fallback) = self.hls.take().and_then(|draft| draft.fallback) {
            self.discard_browser_video(fallback);
        }
        self.dialog = None;
    }
    fn add_hls(&mut self) {
        let Some(draft) = &mut self.hls else {
            return;
        };
        if draft.task.is_some() || draft.saving.is_some() {
            return;
        }
        let Some(preview) = draft.preview.clone() else {
            return;
        };
        let folder = (!draft.folder.trim().is_empty()).then(|| PathBuf::from(draft.folder.trim()));
        let store = self.store.clone();
        let youtube = draft.youtube_selection.clone();
        let bilibili = draft.bilibili_selection.clone();
        let tiktok = draft.tiktok_selection.clone();
        let page = draft.inspection.as_ref().and_then(|i| i.page.clone());
        let name = draft.name.clone();
        let queue = draft.queue.clone();
        // The same receipt survives a failed attempt, including a lost reply.
        let receipt = draft.receipt.clone();
        draft.error = None;
        match background_io::Task::spawn(self.background_io.context.clone(), move || {
            if let Some(selection) = &youtube {
                store.enqueue_youtube(
                    &download_engine::youtube::Preview {
                        playlist: preview.clone(),
                        selection: selection.clone(),
                    },
                    &name,
                    folder.as_deref(),
                    &queue,
                    &receipt,
                )
            } else if let Some(selection) = &bilibili {
                store.enqueue_bilibili(
                    &download_engine::bilibili::Preview {
                        playlist: preview.clone(),
                        selection: selection.clone(),
                    },
                    &name,
                    folder.as_deref(),
                    &queue,
                    &receipt,
                )
            } else if let Some(selection) = &tiktok {
                store.enqueue_tiktok(
                    &download_engine::tiktok::Preview {
                        playlist: preview.clone(),
                        selection: selection.clone(),
                    },
                    &name,
                    folder.as_deref(),
                    &queue,
                    &receipt,
                )
            } else {
                store.enqueue_found_hls(
                    &preview,
                    page.as_deref(),
                    &name,
                    folder.as_deref(),
                    &queue,
                    &receipt,
                )
            }
        }) {
            Ok(task) => draft.saving = Some(task),
            Err(error) => draft.error = Some(format!("{error:#}")),
        }
    }

    pub(super) fn poll_hls_save(&mut self) {
        let Some(draft) = &mut self.hls else {
            return;
        };
        let Some(result) = draft.saving.as_mut().and_then(background_io::Task::poll) else {
            return;
        };
        draft.saving = None;
        match result {
            Ok(job) => {
                self.queue_id.clone_from(&draft.queue);
                self.expect_added_status(&job.id);
                self.set_selection(Some(job.id));
                self.view = View::Downloads;
                self.filter = Filter::All;
                self.search.clear();
                self.category = None;
                self.notice = None;
                self.close_hls();
                self.force_refresh();
            }
            Err(error) => draft.error = Some(format!("{error:#}")),
        }
    }
    pub(super) fn hls_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        self.poll_hls_save();
        let Some(draft) = &mut self.hls else {
            self.dialog = None;
            return;
        };
        draft.poll();
        if let Some(error) = draft.reader_failed.take()
            && let Some(fallback) = draft.fallback.take()
        {
            self.hls = None;
            self.open_browser_video_fallback(fallback, &error);
            return;
        }
        if let Some(review) = draft.found.take() {
            // The reader's list replaces the browser's own findings.
            let queue = draft.queue.clone();
            if let Some(fallback) = self.hls.take().and_then(|draft| draft.fallback) {
                self.discard_browser_video(fallback);
            }
            self.dialog = None;
            self.open_found_videos(review, &queue);
            return;
        }
        let saving = draft.saving.is_some();
        let inspecting = draft.task.as_ref().is_some_and(|task| task.inspecting);
        let (mut load, mut preview, mut add, mut close, mut edit) =
            (false, false, false, false, false);
        let mut retarget = false;
        let response = form_dialog::modal("hls-preview", p).show(ctx, |ui| {
            if saving { ui.disable(); }
            ui.set_width((ctx.content_rect().width() - 100.0).clamp(440.0, 680.0));
            close = form_dialog::title(
                ui,
                tr("Preview video"),
                form_dialog::Icon::Symbol("\u{e714}"),
                p,
                true,
            );
            ui.label(
                RichText::new(tr("Choose quality and audio. The preview updates automatically."))
                    .small()
                    .color(p.muted),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("hls-preview-body")
                .max_height((ctx.content_rect().height() - 165.0).max(260.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    egui::Frame::new()
                        .fill(p.background)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 6.0;
                            if draft.inspection.is_none() {
                                ui.label(tr("YouTube video or HLS / DASH URL"));
                                ui.add_enabled_ui(!inspecting, |ui| {
                                    ui.horizontal(|ui| {
                                        let width = (ui.available_width() - 148.0).max(180.0);
                                        if ui
                                            .add(
                                                view::line_edit(&mut draft.url)
                                                    .desired_width(width)
                                                    .char_limit(16384)
                                                    .hint_text(
                                                        "https://example.com/video/master.m3u8",
                                                    ),
                                            )
                                            .changed()
                                        {
                                            draft.invalidate_preview();
                                            draft.title = None;
                                            retarget = true;
                                        }
                                        load = ui.button(tr("Load video qualities")).clicked();
                                    });
                                });
                            } else {
                                ui.horizontal(|ui| {
                                    edit = ui.button(tr("Change URL")).clicked();
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(tf!("Source: {}", draft.url))
                                                .small()
                                                .color(p.muted),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&draft.url);
                                });
                            }
                            if let Some(title) = &draft.title {
                                ui.add(egui::Label::new(RichText::new(title).strong()).wrap());
                            }
                            if let Some(inspection) = &draft.inspection
                                && !inspection.variants.is_empty()
                            {
                                let mut requested = false;
                                let mut changed = false;
                                let picker_width = (ui.available_width() - 95.0).max(180.0);
                                ui.add_enabled_ui(!inspecting, |ui| {
                                    egui::Grid::new("hls-choices")
                                        .num_columns(2)
                                        .spacing([12.0, 6.0])
                                        .show(ui, |ui| {
                                            ui.label(tr("Quality"));
                                            let before = draft.selected;
                                            egui::ComboBox::from_id_salt("hls-quality")
                                                .width(picker_width)
                                                .truncate()
                                                .selected_text(
                                                    inspection.variants[draft.selected].label(),
                                                )
                                                .show_ui(ui, |ui| {
                                                    for (index, variant) in
                                                        inspection.variants.iter().enumerate()
                                                    {
                                                        requested |= ui
                                                            .selectable_value(
                                                                &mut draft.selected,
                                                                index,
                                                                variant.label(),
                                                            )
                                                            .clicked();
                                                    }
                                                });
                                            ui.end_row();
                                            changed = before != draft.selected;
                                            if changed {
                                                draft.audio_selected = inspection.variants
                                                    [draft.selected]
                                                    .default_audio();
                                            }
                                            let variant = &inspection.variants[draft.selected];
                                            if !variant.audio.is_empty() {
                                                ui.label(tr("Audio track"));
                                                let before = draft.audio_selected;
                                                egui::ComboBox::from_id_salt("hls-audio")
                                                    .width(picker_width)
                                                    .truncate()
                                                    .selected_text(
                                                        variant.audio[draft.audio_selected].label(),
                                                    )
                                                    .show_ui(ui, |ui| {
                                                        for (index, track) in
                                                            variant.audio.iter().enumerate()
                                                        {
                                                            requested |= ui
                                                                .selectable_value(
                                                                    &mut draft.audio_selected,
                                                                    index,
                                                                    track.label(),
                                                                )
                                                                .clicked();
                                                        }
                                                    });
                                                ui.end_row();
                                                changed |= before != draft.audio_selected;
                                            }
                                        });
                                });
                                let variant = &inspection.variants[draft.selected];
                                if let Some(reason) = &variant.unsupported {
                                    ui.colored_label(p.warning, reason);
                                }
                                let supported = variant.unsupported.is_none();
                                if changed {
                                    draft.invalidate_preview();
                                }
                                preview = supported
                                    && (changed
                                        || requested
                                            && draft.preview.is_none()
                                            && draft.task.is_none());
                                if supported
                                    && draft.preview.is_none()
                                    && draft.error.is_some()
                                    && draft.task.is_none()
                                {
                                    preview |= ui.button(tr("Retry preview")).clicked();
                                }
                            }
                            if draft.task.is_some() || preview {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(if inspecting {
                                        tr("Loading video qualities…")
                                    } else {
                                        tr("Updating preview…")
                                    });
                                });
                            }
                            if let Some(playlist) = &draft.preview {
                                if draft.inspection.as_ref().is_none_or(|i| i.variants.is_empty()) {
                                    ui.label(&playlist.quality);
                                }
                                let seconds = playlist.duration_ms / 1000;
                                let size_summary = playlist
                                    .estimated_size()
                                    .map(|bytes| format!(" · ~{:.1} MiB", bytes as f64 / 1048576.0))
                                    .unwrap_or_default();
                                ui.label(
                                    RichText::new(tf!(
                                        "{}:{:02}:{:02} · {} segments{size_summary} · {}",
                                        seconds / 3600,
                                        (seconds / 60) % 60,
                                        seconds % 60,
                                        playlist.media_count(),
                                        playlist.output_label()
                                    , size_summary = size_summary))
                                    .small()
                                    .color(p.muted),
                                );
                                let audio_picker = draft
                                    .inspection
                                    .as_ref()
                                    .and_then(|i| i.variants.get(draft.selected))
                                    .is_some_and(|v| !v.audio.is_empty());
                                if let Some(audio) = &playlist.audio
                                    && !audio_picker
                                {
                                    ui.label(
                                        RichText::new(tf!(
                                            "Audio: {}{}",
                                            audio.name,
                                            audio
                                                .language
                                                .as_ref()
                                                .map(|v| format!(" · {v}"))
                                                .unwrap_or_default()
                                        ))
                                        .small()
                                        .color(p.muted),
                                    );
                                } else if playlist.audio.is_none() && playlist.dash.is_some() {
                                    ui.label(
                                        RichText::new(tr("Audio: none in manifest"))
                                            .small()
                                            .color(p.muted),
                                    );
                                }
                            }
                        });
                    if draft.preview.is_some() {
                        egui::Frame::new()
                            .fill(p.background)
                            .stroke(egui::Stroke::new(1.0, p.border))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                let field_width = (ui.available_width() - 80.0).max(180.0);
                                egui::Grid::new("hls-destination")
                                    .num_columns(2)
                                    .spacing([12.0, 8.0])
                                    .show(ui, |ui| {
                                        ui.label(tr("Filename"));
                                        ui.add(
                                            view::line_edit(&mut draft.name)
                                                .desired_width(field_width)
                                                .char_limit(160),
                                        );
                                        ui.end_row();
                                        ui.label(tr("Queue"));
                                        egui::ComboBox::from_id_salt("hls-queue")
                                            .width(field_width)
                                            .selected_text(
                                                self.queues
                                                    .iter()
                                                    .find(|q| q.id == draft.queue)
                                                    .map(|q| display_queue_name(&q.id, &q.name))
                                                    .unwrap_or(tr("Choose queue")),
                                            )
                                            .show_ui(ui, |ui| {
                                                for queue in &self.queues {
                                                    ui.selectable_value(
                                                        &mut draft.queue,
                                                        queue.id.clone(),
                                                        format!(
                                                            "{} · {}",
                                                            display_queue_name(&queue.id, &queue.name),
                                                            if queue.running {
                                                                "running"
                                                            } else {
                                                                tr("paused")
                                                            }
                                                        ),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                        ui.label(tr("Save to"))
                                            .on_hover_text(tr("Leave blank to use your video folder rule"));
                                        ui.push_id("hls-save-to", |ui| {
                                            ui.set_width(field_width);
                                            self.folder_picker.field(
                                                ui,
                                                &mut draft.folder,
                                                &self.store.paths.downloads,
                                                &mut draft.error,
                                            );
                                        });
                                        ui.end_row();
                                    });
                            });
                    }
                    if let Some(error) = &draft.error {
                        ui.colored_label(p.warning, error);
                    }
                    ui.label(
                        RichText::new(
                            tr("Public finished streams only · Separate audio merges without re-encoding."),
                        )
                        .small()
                        .color(p.muted),
                    );
                });
            form_dialog::controls(ui, |ui| {
                add = form_dialog::primary_enabled(
                    ui,
                    draft.preview.is_some() && draft.task.is_none() && !saving && !preview && !edit,
                    if saving { tr("Adding…") } else { tr("Add video to queue") },
                )
                .clicked();
                if draft.task.is_some()
                    && form_dialog::secondary_button(ui, tr("Cancel reading")).clicked()
                {
                    draft.invalidate_preview();
                    draft.error =
                        Some(tr("Preview cancelled. Choose a quality or retry when ready.").into());
                    preview = false;
                }
            });
        });
        if load {
            draft.start(true);
        }
        if preview {
            draft.start(false);
        }
        // The browser page and its findings belong to the address the dialog
        // opened with; a typed address is a manual entry.
        let stale = (edit || retarget).then(|| draft.fallback.take()).flatten();
        if edit || retarget {
            draft.page = None;
            draft.public_only = false;
        }
        if edit {
            draft.invalidate_preview();
            draft.inspection = None;
            draft.title = None;
        }
        if draft.task.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(stale) = stale {
            self.discard_browser_video(stale);
        }
        if saving {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if close || response.should_close() {
            self.close_hls();
        } else if add {
            self.add_hls();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::interaction_tests::{click, fixture, frame, text_rect};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct PreviewServer {
        base: String,
        fail: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<String>>>,
        stop: CancellationToken,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl PreviewServer {
        fn new() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let stop = CancellationToken::new();
            let fail = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let (cancel, failing, seen) = (stop.clone(), fail.clone(), requests.clone());
            let worker = std::thread::spawn(move || {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                    loop {
                        let (mut socket, _) = tokio::select! {
                            _ = cancel.cancelled() => break,
                            incoming = listener.accept() => incoming.unwrap(),
                        };
                        let (cancel, failing, seen) = (cancel.clone(), failing.clone(), seen.clone());
                        tokio::spawn(async move {
                            let mut request = Vec::new();
                            let mut bytes = [0; 1024];
                            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                                let n = socket.read(&mut bytes).await.unwrap();
                                if n == 0 { return; }
                                request.extend_from_slice(&bytes[..n]);
                                assert!(request.len() <= 8192);
                            }
                            let request = String::from_utf8(request).unwrap();
                            let path = request.split_whitespace().nth(1).unwrap();
                            seen.lock().unwrap().push(path.to_owned());
                            if path == "/slow.m3u8" {
                                cancel.cancelled().await;
                                return;
                            }
                            let master = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"en.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Spanish\",LANGUAGE=\"es\",DEFAULT=NO,AUTOSELECT=YES,URI=\"es.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=4500000,RESOLUTION=1920x1080,AUDIO=\"a\"\nhigh.m3u8\n";
                            let body = match path {
                                "/master.m3u8" => format!("{master}#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720,AUDIO=\"a\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,AUDIO=\"a\"\nslow.m3u8\n"),
                                "/one.m3u8" => master.into(),
                                "/high.m3u8" | "/low.m3u8" | "/en.m3u8" | "/es.m3u8" | "/direct.m3u8" => "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\npart.ts\n#EXT-X-ENDLIST\n".into(),
                                _ => panic!("Unexpected preview request: {path}"),
                            };
                            let status = if failing.load(Ordering::SeqCst) { "503 Service Unavailable" } else { "200 OK" };
                            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            // A replacement selection may have cancelled this connection.
                            let _ = socket.write_all(response.as_bytes()).await;
                        });
                    }
                });
            });
            Self {
                base,
                fail,
                requests,
                stop,
                worker: Some(worker),
            }
        }
        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }
    }
    impl Drop for PreviewServer {
        fn drop(&mut self) {
            self.stop.cancel();
            self.worker.take().unwrap().join().unwrap();
        }
    }
    fn finish_preview(app: &mut Desktop) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let draft = app.hls.as_mut().unwrap();
            draft.poll();
            if draft.task.is_none() {
                break;
            }
            assert!(Instant::now() < deadline, "Local preview did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn assert_visible(output: &egui::FullOutput, size: egui::Vec2, label: &str) {
        let rect = text_rect(output, label);
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
            "{label} outside {size:?}"
        );
        assert!(
            output.shapes.iter().any(
                |shape| matches!(&shape.shape, egui::epaint::Shape::Text(text)
            if text.galley.job.text.trim() == label && shape.clip_rect.contains_rect(rect))
            ),
            "{label} is clipped: {rect:?}; clips: {:?}",
            output.shapes.iter().filter_map(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.job.text.trim() == label).then_some(shape.clip_rect)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn selecting_quality_starts_preview_without_a_second_click() {
        let (_temp, mut app, ctx) = fixture();
        app.hls_sample(false);
        let draft = app.hls.as_mut().unwrap();
        let inspection = draft.inspection.as_mut().unwrap();
        inspection.source = "http://127.0.0.1:9/master.m3u8".into();
        for variant in &mut inspection.variants {
            variant.url = "http://127.0.0.1:9/video.m3u8".into();
        }
        let labels: Vec<_> = inspection.variants.iter().map(|v| v.label()).collect();
        click(&mut app, &ctx, &labels[0]);
        click(&mut app, &ctx, &labels[1]);
        assert!(
            app.hls.as_ref().unwrap().task.is_some(),
            "Selecting quality must start its preview"
        );
        assert!(app.store.list().unwrap().is_empty());
    }

    #[test]
    fn initial_preview_handles_single_quality_and_direct_playlists_without_downloading() {
        let server = PreviewServer::new();
        for path in ["/one.m3u8", "/direct.m3u8"] {
            let (_temp, mut app, _ctx) = fixture();
            app.open_browser_stream(&server.url(path), MAIN_QUEUE, None);
            finish_preview(&mut app);
            let draft = app.hls.as_ref().unwrap();
            assert!(draft.preview.is_some(), "{:?}", draft.error);
            assert_eq!(draft.preview.as_ref().unwrap().duration_ms, 2000);
            assert!(app.store.list().unwrap().is_empty());
        }
        let paths = server.requests.lock().unwrap();
        assert!(paths.iter().any(|p| p == "/high.m3u8"));
        assert!(paths.iter().any(|p| p == "/en.m3u8"));
        assert!(paths.iter().all(|p| p.ends_with(".m3u8")));
    }

    #[test]
    fn changing_output_container_preserves_a_custom_filename_stem() {
        let (_temp, mut app, _ctx) = fixture();
        app.hls_audio_sample(true);
        let draft = app.hls.as_mut().unwrap();
        let merged = draft.preview.clone().unwrap();
        draft.name.clear();
        draft.set_preview(merged.clone());
        draft.name = "My custom video.mp4".into();
        let mut video_only = merged.clone();
        video_only.audio = None;
        draft.set_preview(video_only);
        assert_eq!(draft.name, "My custom video.ts");
        draft.set_preview(merged);
        assert_eq!(draft.name, "My custom video.mp4");
    }

    #[test]
    fn automatic_preview_can_retry_after_failure_or_cancel_and_skips_unsupported_quality() {
        let server = PreviewServer::new();
        let (_temp, mut app, ctx) = fixture();
        app.open_browser_stream(&server.url("/master.m3u8"), MAIN_QUEUE, None);
        finish_preview(&mut app);
        let labels: Vec<_> = app
            .hls
            .as_ref()
            .unwrap()
            .inspection
            .as_ref()
            .unwrap()
            .variants
            .iter()
            .map(|v| v.label())
            .collect();
        server.fail.store(true, Ordering::SeqCst);
        click(&mut app, &ctx, &labels[0]);
        click(&mut app, &ctx, &labels[1]);
        finish_preview(&mut app);
        let draft = app.hls.as_ref().unwrap();
        assert!(draft.preview.is_none());
        assert!(draft.error.as_ref().unwrap().contains("503"));
        click(&mut app, &ctx, "Add video to queue");
        assert!(app.store.list().unwrap().is_empty());
        server.fail.store(false, Ordering::SeqCst);
        click(&mut app, &ctx, "Retry preview");
        finish_preview(&mut app);
        assert_eq!(
            app.hls.as_ref().unwrap().preview.as_ref().unwrap().quality,
            labels[1]
        );
        click(&mut app, &ctx, &labels[1]);
        click(&mut app, &ctx, &labels[2]);
        let old = app
            .hls
            .as_ref()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .cancel
            .clone();
        click(&mut app, &ctx, "Cancel reading");
        assert!(old.is_cancelled());
        assert!(app.hls.as_ref().unwrap().preview.is_none());
        click(&mut app, &ctx, "Retry preview");
        assert!(app.hls.as_ref().unwrap().task.is_some());
        click(&mut app, &ctx, &labels[2]);
        click(&mut app, &ctx, &labels[0]);
        finish_preview(&mut app);
        assert!(app.hls.as_ref().unwrap().error.is_none());
        let variant = &mut app
            .hls
            .as_mut()
            .unwrap()
            .inspection
            .as_mut()
            .unwrap()
            .variants[1];
        variant.unsupported = Some("Unsupported fixture codec".into());
        let unsupported = variant.label();
        click(&mut app, &ctx, &labels[0]);
        click(&mut app, &ctx, &unsupported);
        let draft = app.hls.as_ref().unwrap();
        assert!(draft.task.is_none());
        assert!(draft.preview.is_none());
        assert!(app.store.list().unwrap().is_empty());
    }

    #[test]
    fn youtube_review_uses_the_title_and_saves_only_the_explicitly_reviewed_choice() {
        let (_temp, mut app, ctx) = fixture();
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.refresh_and_wait();
        app.youtube_sample("youtube-preview");
        let draft = app.hls.as_mut().unwrap();
        draft.title = Some(format!("Title. With dots {}", "界".repeat(170)));
        draft.name.clear();
        draft.set_preview(draft.preview.clone().unwrap());
        assert!(draft.name.starts_with("Title. With dots"));
        assert!(draft.name.ends_with(".mp4"));
        assert!(draft.name.encode_utf16().count() <= 160);
        click(&mut app, &ctx, "Add video to queue");
        let jobs = app.store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        let saved = jobs[0].hls.as_ref().unwrap();
        assert_eq!(app.selected.as_deref(), Some(jobs[0].id.as_str()));
        let output = frame(&mut app, &ctx, egui::vec2(860., 600.), vec![]);
        assert!(app.dialog.is_none());
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == "Dismiss"
                || text.galley.job.text.contains("Added the reviewed video"))));
        assert_eq!(saved.version, 5);
        assert_eq!(saved.youtube.as_ref().unwrap().video_id, "YE7VzlLtp-4");
        assert_eq!(
            saved.playlist.audio.as_ref().unwrap().format,
            service::Format::PackedAac
        );
        app.duplicate_download(&ctx, &jobs[0]);
        let draft = app.hls.as_ref().unwrap();
        assert!(draft.preview.is_none());
        assert_eq!(draft.url, "https://www.youtube.com/watch?v=YE7VzlLtp-4");
    }

    #[test]
    fn youtube_states_fit_and_cancel_discards_stale_worker_results() {
        for dark in [true, false] {
            for mode in ["youtube-qualities", "youtube-preview", "youtube-error"] {
                let (_temp, mut app, ctx) = fixture();
                app.interface.appearance = if dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                app.configure_appearance(&ctx);
                app.youtube_sample(mode);
                for size in [egui::vec2(860., 600.), egui::vec2(1280., 860.)] {
                    frame(&mut app, &ctx, size, vec![]);
                    let output = frame(&mut app, &ctx, size, vec![]);
                    for label in ["Preview video", "Add video to queue", "×"] {
                        assert_visible(&output, size, label);
                    }
                    if mode == "youtube-preview" {
                        for label in [
                            "Quality",
                            "Audio track",
                            "Filename",
                            "Queue",
                            "Save to",
                            "Browse…",
                        ] {
                            assert_visible(&output, size, label);
                        }
                        assert_visible(
                            &output,
                            size,
                            "Public finished streams only · Separate audio merges without re-encoding.",
                        );
                    }
                }
                assert!(app.store.list().unwrap().is_empty());
                let (sender, result) = mpsc::channel();
                let token = CancellationToken::new();
                app.hls.as_mut().unwrap().task = Some(Task {
                    inspecting: false,
                    cancel: token.clone(),
                    result,
                });
                click(&mut app, &ctx, "Cancel reading");
                assert!(token.is_cancelled());
                assert!(sender.send(Err(anyhow::anyhow!("obsolete error"))).is_err());
                assert!(app.hls.as_ref().unwrap().error.as_deref() != Some("obsolete error"));
                click(&mut app, &ctx, "×");
                assert!(app.hls.is_none());
            }
        }
    }

    #[test]
    fn site_video_handoff_and_pasted_url_open_quality_selection_without_queuing_html() {
        for url in [
            "https://www.youtube.com/watch?v=YE7VzlLtp-4",
            "https://www.bilibili.com/video/BV1hCnw65EaE/",
            "https://www.tiktok.com/@jannanickofficial/video/7696495296011930887",
            "https://www.bilibili.com/bangumi/play/ep733316",
            "https://www.twitch.tv/videos/2902385151",
        ] {
            let (_temp, mut app, ctx) = fixture();
            let id = format!("video:{}", Uuid::new_v4());
            app.store
                .request_browser_video_review(
                    &id,
                    &download_engine::browser_video::Review {
                        public_only: false,
                        page_url: url.into(),
                        page_title: "Video page".into(),
                        items: vec![],
                        truncated: false,
                    },
                )
                .unwrap();
            app.poll_browser_video_requests(&ctx);
            app.wait_for_io();
            assert_eq!(app.dialog, Some(Dialog::Hls), "{url}");
            assert!(app.hls.as_ref().unwrap().start_on_poll);
            assert_eq!(app.hls.as_ref().unwrap().url, url);
            assert!(app.store.list().unwrap().is_empty());
            assert!(app.store.next_browser_video_review().unwrap().is_none());
            app.hls = None;
            app.dialog = None;
            app.url = url.into();
            app.open_add();
            app.add();
            app.wait_for_io();
            assert_eq!(app.dialog, Some(Dialog::Hls), "{url}");
            assert_eq!(app.hls.as_ref().unwrap().url, url);
            assert!(app.store.list().unwrap().is_empty());
            // The extension's link handoff opens the same quality choice.
            app.hls = None;
            app.dialog = None;
            app.store
                .request_hls_preview(&format!("hls:{}", Uuid::new_v4()), url, None)
                .unwrap();
            app.hls_request_check = Instant::now() - Duration::from_secs(2);
            app.poll_hls_requests(&ctx);
            app.wait_for_io();
            assert_eq!(app.dialog, Some(Dialog::Hls), "{url}");
            assert!(app.hls.as_ref().unwrap().start_on_poll);
            assert!(app.store.list().unwrap().is_empty());
            // No frame is run: the deferred worker must not perform a live request in this test.
        }
    }

    /// A site page's review whose browser findings back up the site reader.
    fn site_review(app: &mut Desktop, ctx: &egui::Context, page: &str) {
        app.store
            .request_browser_video_review(
                &format!("video:{}", Uuid::new_v4()),
                &download_engine::browser_video::Review {
                    public_only: false,
                    page_url: page.into(),
                    page_title: "Episode".into(),
                    items: vec![download_engine::browser_video::Item {
                        url: "https://cdn.example.test/episode.mp4".into(),
                        filename: "episode.mp4".into(),
                        label: "Video".into(),
                        kind: download_engine::browser_video::Kind::File,
                        size: Some(42),
                    }],
                    truncated: false,
                },
            )
            .unwrap();
        app.poll_browser_video_requests(ctx);
        app.wait_for_io();
        assert_eq!(app.dialog, Some(Dialog::Hls));
        let draft = app.hls.as_mut().unwrap();
        assert!(draft.fallback.is_some() && draft.start_on_poll);
        // No live lookup in tests: results are injected below.
        draft.start_on_poll = false;
    }
    fn reply(app: &mut Desktop, inspecting: bool, result: anyhow::Result<Loaded>) {
        let (sender, receiver) = mpsc::channel();
        app.hls.as_mut().unwrap().task = Some(Task {
            inspecting,
            cancel: CancellationToken::new(),
            result: receiver,
        });
        sender.send(result).unwrap();
    }

    #[test]
    fn your_site_reader_opens_its_pages_and_its_list_replaces_the_browser_findings() {
        use download_engine::script_automation::{Language, READER_EXAMPLE, Script, Trigger};
        let page = "https://www.example.com/watch/a1";
        let (_temp, mut app, ctx) = fixture();
        app.store
            .save_script(&Script {
                id: Uuid::new_v4().to_string(),
                revision: 0,
                name: "Example reader".into(),
                language: Language::Lua,
                body: READER_EXAMPLE.into(),
                enabled: true,
                trigger: Trigger::SiteReader {
                    pages: vec!["www.example.com/watch/*".into()],
                },
                timeout_seconds: 60,
                change_files: false,
                extension: None,
            })
            .unwrap();
        // A page the reader covers opens like a built-in site, browser findings kept.
        site_review(&mut app, &ctx, page);
        let browser = app.store.next_browser_video_review().unwrap().unwrap().id;
        let found = download_engine::script_automation::Reading::from_value(serde_json::json!({
            "title": "Nature documentary",
            "items": [
                {"kind": "hls", "url": "https://cdn.example.com/a1/master.m3u8", "label": "All qualities"},
                {"kind": "file", "url": "https://cdn.example.com/a1/720.mp4", "label": "MP4 720p"},
            ],
        }))
        .unwrap();
        reply(
            &mut app,
            true,
            Ok(Loaded::Found(found.review(page).unwrap())),
        );
        frame(&mut app, &ctx, egui::vec2(1280., 860.), vec![]);
        assert!(app.hls.is_none());
        assert_eq!(app.dialog, Some(Dialog::BrowserVideo));
        let pending = app.store.next_browser_video_review().unwrap().unwrap();
        assert_ne!(
            pending.id, browser,
            "the browser's own findings were dismissed"
        );
        assert_eq!(pending.review.page_title, "Nature documentary");
        assert_eq!(pending.review.items.len(), 2);
        let output = frame(&mut app, &ctx, egui::vec2(1280., 860.), vec![]);
        text_rect(&output, "720.mp4");
        // The same page with the reader turned off is an ordinary page again.
        let mut reader = app.store.scripts().unwrap().remove(0);
        reader.enabled = false;
        app.store.save_script(&reader).unwrap();
        assert!(app.store.site_reader_for(page).unwrap().is_none());
    }

    #[test]
    fn reader_security_video_list_keeps_public_only_when_choosing_a_stream() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let (_temp, mut app, ctx) = fixture();
        app.store
            .save_proxy_settings(
                &app.store.proxy_settings().unwrap(),
                download_engine::proxy::Settings {
                    mode: download_engine::proxy::Mode::Direct,
                    ..Default::default()
                },
                download_engine::proxy::LoginChange::Remove,
            )
            .unwrap();
        let found = download_engine::script_automation::Reading::from_value(serde_json::json!({
            "items": [
                {"kind": "hls", "url": format!("http://{}/master.m3u8", listener.local_addr().unwrap())},
                {"kind": "file", "url": "https://example.com/video.mp4"}
            ]
        })).unwrap();
        app.open_found_videos(
            found.review("https://example.com/watch").unwrap(),
            MAIN_QUEUE,
        );
        click(&mut app, &ctx, "Choose quality");
        finish_preview(&mut app);
        let draft = app.hls.as_ref().unwrap();
        assert!(draft.public_only);
        assert!(draft.error.as_ref().unwrap().contains("public internet"));
        assert!(draft.preview.is_none() && app.store.list().unwrap().is_empty());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn failed_site_reader_falls_back_to_what_the_browser_found() {
        let page = "https://www.bilibili.com/bangumi/play/ep733316";
        let size = egui::vec2(1280., 860.);
        let (_temp, mut app, ctx) = fixture();
        site_review(&mut app, &ctx, page);
        // The browser's findings stay pending while the reader works.
        assert!(app.store.next_browser_video_review().unwrap().is_some());
        reply(
            &mut app,
            true,
            Err(anyhow::anyhow!(
                "This Bilibili video is not available in your region"
            )),
        );
        frame(&mut app, &ctx, size, vec![]);
        assert!(app.hls.is_none());
        assert_eq!(app.dialog, Some(Dialog::BrowserVideo));
        let output = frame(&mut app, &ctx, size, vec![]);
        text_rect(
            &output,
            "Showing the media your browser found because this page could not be read directly: This Bilibili video is not available in your region",
        );
        click(&mut app, &ctx, "episode.mp4");
        click(&mut app, &ctx, "Add selected files");
        let jobs = app.store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].url, "https://cdn.example.test/episode.mp4");
        assert_eq!(jobs[0].source_page.as_deref(), Some(page));
        assert!(app.store.next_browser_video_review().unwrap().is_none());

        // A later quality error stays in the reader; closing it drops the findings.
        let (_temp, mut app, ctx) = fixture();
        site_review(&mut app, &ctx, page);
        reply(&mut app, false, Err(anyhow::anyhow!("quality failed")));
        frame(&mut app, &ctx, size, vec![]);
        assert_eq!(app.dialog, Some(Dialog::Hls));
        assert_eq!(
            app.hls.as_ref().unwrap().error.as_deref(),
            Some("quality failed")
        );
        click(&mut app, &ctx, "×");
        assert!(app.hls.is_none() && app.dialog.is_none());
        assert!(app.store.next_browser_video_review().unwrap().is_none());
        assert!(app.store.list().unwrap().is_empty());

        // A typed address is a manual entry: no findings, no Referer page.
        let (_temp, mut app, ctx) = fixture();
        site_review(&mut app, &ctx, page);
        click(&mut app, &ctx, page);
        frame(&mut app, &ctx, size, vec![egui::Event::Text("x".into())]);
        app.wait_for_io();
        assert!(app.hls.as_ref().unwrap().fallback.is_none());
        assert!(app.store.next_browser_video_review().unwrap().is_none());
        let stream = "https://cdn.example.test/master.m3u8";
        app.hls = None;
        app.dialog = None;
        app.open_page_stream(
            stream,
            Some("https://videos.example.test/watch"),
            &app.queue_id.clone(),
            None,
        );
        app.hls.as_mut().unwrap().start_on_poll = false;
        assert!(app.hls.as_ref().unwrap().page.is_some());
        click(&mut app, &ctx, stream);
        frame(&mut app, &ctx, size, vec![egui::Event::Text("x".into())]);
        assert!(app.hls.as_ref().unwrap().page.is_none());
    }

    #[test]
    fn hls_preview_actions_fit_both_themes_at_the_minimum_window_size() {
        for dark in [false, true] {
            let (_temp, mut app, ctx) = fixture();
            app.interface.appearance = if dark {
                Appearance::Dark
            } else {
                Appearance::Light
            };
            app.configure_appearance(&ctx);
            app.hls_sample(true);
            for size in [egui::vec2(860.0, 600.0), egui::vec2(1280.0, 860.0)] {
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                for label in [
                    "Preview video",
                    "Change URL",
                    "Filename",
                    "Queue",
                    "Save to",
                    "Add video to queue",
                    "×",
                ] {
                    assert_visible(&output, size, label);
                }
            }
            assert!(app.store.list().unwrap().is_empty());
        }
    }
    #[test]
    fn choosing_another_quality_invalidates_preview_and_queueing_requires_review() {
        let server = PreviewServer::new();
        let (_temp, mut app, ctx) = fixture();
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.refresh_and_wait();
        app.open_browser_stream(&server.url("/master.m3u8"), MAIN_QUEUE, None);
        finish_preview(&mut app);
        let draft = app.hls.as_ref().unwrap();
        assert!(draft.preview.is_some(), "{:?}", draft.error);
        let labels = draft
            .inspection
            .as_ref()
            .unwrap()
            .variants
            .iter()
            .map(|v| v.label())
            .collect::<Vec<_>>();
        click(&mut app, &ctx, &labels[0]);
        click(&mut app, &ctx, &labels[2]);
        let old = app
            .hls
            .as_ref()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .cancel
            .clone();
        assert!(app.hls.as_ref().unwrap().preview.is_none());
        click(&mut app, &ctx, "Add video to queue");
        assert!(app.store.list().unwrap().is_empty());
        click(&mut app, &ctx, &labels[2]);
        click(&mut app, &ctx, &labels[1]);
        assert!(old.is_cancelled());
        finish_preview(&mut app);
        assert_eq!(
            app.hls.as_ref().unwrap().preview.as_ref().unwrap().quality,
            labels[1]
        );
        assert!(app.store.list().unwrap().is_empty());
        click(&mut app, &ctx, "Add video to queue");
        assert_eq!(app.dialog, None);
        let jobs = app.store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].hls.is_some());
        assert_eq!(jobs[0].status, Status::Queued);
        app.duplicate_download(&ctx, &jobs[0]);
        assert_eq!(app.dialog, Some(Dialog::Hls));
        assert!(app.hls.as_ref().unwrap().preview.is_none());
        assert_eq!(app.store.list().unwrap().len(), 1);
    }
    #[test]
    fn browser_preview_waits_for_existing_dialog_and_never_adds_a_job() {
        let (_temp, mut app, ctx) = fixture();
        let id = format!("hls:{}", Uuid::new_v4());
        app.store
            .request_hls_preview(&id, "https://example.com/master.m3u8", None)
            .unwrap();
        app.dialog = Some(Dialog::Add);
        app.url = "https://example.com/unsaved.zip".into();
        app.poll_hls_requests(&ctx);
        app.wait_for_io();
        assert_eq!(app.dialog, Some(Dialog::Add));
        assert!(app.store.next_hls_preview().unwrap().is_some());
        app.dialog = None;
        app.poll_hls_requests(&ctx);
        app.wait_for_io();
        assert_eq!(app.dialog, Some(Dialog::Hls));
        assert!(app.hls.as_ref().unwrap().preview.is_none());
        assert!(app.store.list().unwrap().is_empty());
        assert!(app.store.next_hls_preview().unwrap().is_none());
    }
    #[test]
    fn audio_selection_clears_review_and_new_quality_restores_its_default() {
        let server = PreviewServer::new();
        let (temp, mut app, ctx) = fixture();
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.refresh_and_wait();
        let folder = temp.path().join("chosen-videos");
        app.open_browser_stream(&server.url("/master.m3u8"), MAIN_QUEUE, folder.to_str());
        finish_preview(&mut app);
        assert!(app.hls.as_ref().unwrap().preview.is_some());
        app.hls.as_mut().unwrap().name = "My video.mp4".into();
        click(&mut app, &ctx, "English · en");
        click(&mut app, &ctx, "Spanish · es");
        assert_eq!(app.hls.as_ref().unwrap().audio_selected, 1);
        assert!(app.hls.as_ref().unwrap().preview.is_none());
        assert!(app.hls.as_ref().unwrap().task.is_some());
        finish_preview(&mut app);
        assert_eq!(
            app.hls
                .as_ref()
                .unwrap()
                .preview
                .as_ref()
                .unwrap()
                .audio
                .as_ref()
                .unwrap()
                .language
                .as_deref(),
            Some("es")
        );
        assert!(app.store.list().unwrap().is_empty());
        let labels: Vec<_> = app
            .hls
            .as_ref()
            .unwrap()
            .inspection
            .as_ref()
            .unwrap()
            .variants
            .iter()
            .map(|v| v.label())
            .collect();
        click(&mut app, &ctx, &labels[0]);
        click(&mut app, &ctx, &labels[1]);
        assert_eq!(app.hls.as_ref().unwrap().audio_selected, 0);
        assert!(app.hls.as_ref().unwrap().preview.is_none());
        finish_preview(&mut app);
        let draft = app.hls.as_ref().unwrap();
        let reviewed = draft.preview.clone().unwrap();
        assert_eq!(reviewed.quality, labels[1]);
        assert_eq!(
            reviewed.audio.as_ref().unwrap().language.as_deref(),
            Some("en")
        );
        assert_eq!(draft.name, "My video.mp4");
        assert_eq!(draft.folder, folder.to_str().unwrap());
        assert!(app.store.list().unwrap().is_empty());
        click(&mut app, &ctx, "Add video to queue");
        let jobs = app.store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].hls.as_ref().unwrap().playlist, reviewed);
        assert!(jobs[0].name.ends_with(".mp4"));
    }
    #[test]
    fn audio_picker_and_preview_fit_small_windows_in_both_themes() {
        for dark in [false, true] {
            for preview in [false, true] {
                let (_temp, mut app, ctx) = fixture();
                app.interface.appearance = if dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                app.configure_appearance(&ctx);
                app.hls_audio_sample(preview);
                let size = egui::vec2(860.0, 600.0);
                frame(&mut app, &ctx, size, vec![]);
                let output = frame(&mut app, &ctx, size, vec![]);
                let labels = if preview {
                    vec![
                        "English · en",
                        "Change URL",
                        "Filename",
                        "Queue",
                        "Save to",
                        "Browse…",
                    ]
                } else {
                    vec![
                        "Quality",
                        "Audio track",
                        "English · en",
                        "Choose quality and audio. The preview updates automatically.",
                    ]
                };
                for label in labels.into_iter().chain(["Add video to queue", "×"]) {
                    assert_visible(&output, size, label);
                }
                assert!(app.store.list().unwrap().is_empty());
            }
        }
    }
    #[test]
    fn dash_review_retains_selected_audio_and_requires_explicit_queue_add() {
        let (_temp, mut app, ctx) = fixture();
        let now = chrono::Utc::now().timestamp();
        app.store
            .schedule_queue(MAIN_QUEUE, now + 3600, now)
            .unwrap();
        app.refresh_and_wait();
        app.dash_sample(true);
        let reviewed = app.hls.as_ref().unwrap().preview.clone().unwrap();
        frame(&mut app, &ctx, egui::vec2(860., 600.), vec![]);
        assert!(app.store.list().unwrap().is_empty());
        click(&mut app, &ctx, "Add video to queue");
        let jobs = app.store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].hls.as_ref().unwrap().playlist, reviewed);
        assert_eq!(jobs[0].hls.as_ref().unwrap().version, 3);
        assert_eq!(jobs[0].status, Status::Queued);
        app.duplicate_download(&ctx, &jobs[0]);
        assert!(app.hls.as_ref().unwrap().preview.is_none());
        assert!(app.hls.as_ref().unwrap().url.ends_with("main.mpd"));
    }
}

use super::*;
use crate::{
    hls::{AudioRendition, AudioTrack, CompletedPart, Download, Format, Part, Variant},
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
};

fn url(tag: u32, segment: bool) -> String {
    format!(
        "https://manifest.googlevideo.com/api/manifest/hls_playlist/id/content123/itag/{tag}/expire/100/sgovp/clen%3D1000%3Blmt%3D5678/gosq/0/file/{}",
        if segment { "segment.ts" } else { "index.m3u8" }
    )
}

fn fixture() -> Preview {
    let part = |tag| Part {
        url: url(tag, true),
        range: None,
        initialization: false,
    };
    Preview {
        playlist: Playlist {
            public_only: false,
            source: watch_url("YE7VzlLtp-4").unwrap(),
            url: url(95, false),
            quality: "720p".into(),
            bandwidth: Some(2_000_000),
            duration_ms: 5000,
            sequence: 0,
            format: Format::TransportStream,
            parts: vec![part(95)],
            audio: Some(AudioTrack {
                name: "English".into(),
                language: Some("en".into()),
                url: url(234, false),
                duration_ms: 5000,
                sequence: 0,
                format: Format::TransportStream,
                parts: vec![part(234)],
            }),
            dash: None,
        },
        selection: Selection {
            video_id: "YE7VzlLtp-4".into(),
            video_itag: 95,
            audio_itag: Some(234),
            audio_name: Some("English".into()),
            audio_language: Some("en".into()),
            resolution: Some((1280, 720)),
            codecs: "avc1.4D401F,mp4a.40.2".into(),
        },
    }
}

#[test]
fn youtube_urls_extract_one_video_without_fetching_or_accepting_lookalike_hosts() {
    for value in [
        "https://www.youtube.com/watch?v=YE7VzlLtp-4&list=playlist&t=10",
        "https://youtu.be/YE7VzlLtp-4?si=tracking",
        "https://m.youtube.com/shorts/YE7VzlLtp-4",
        "https://www.youtube-nocookie.com/embed/YE7VzlLtp-4",
        "http://music.youtube.com/watch?v=YE7VzlLtp-4&v=abcdefghijk",
    ] {
        assert_eq!(video_id(value).unwrap(), "YE7VzlLtp-4");
    }
    for value in [
        "https://youtube.com.evil.test/watch?v=YE7VzlLtp-4",
        "https://youtube.com@127.0.0.1/watch?v=YE7VzlLtp-4",
        "https://youtube.com:123/watch?v=YE7VzlLtp-4",
        "https://youtube.com/playlist?list=YE7VzlLtp-4",
        "https://youtu.be/YE7VzlLtp-4/extra",
        "https://youtube.com/watch?v=short",
        "https://youtube.com/watch?v=YE7VzlLtp-4extra",
        "https://youtube.com/watch?v=bad&v=YE7VzlLtp-4",
        "file:///YE7VzlLtp-4",
    ] {
        assert!(video_id(value).is_err(), "{value}");
    }
}

#[test]
fn media_addresses_require_public_https_googlevideo_and_resolved_challenges() {
    assert!(media_url(&url(95, false)).is_ok());
    assert!(itag(&format!("{}/unpaired", url(95, false))).is_err());
    for value in [
        "http://manifest.googlevideo.com/index.m3u8",
        "https://googlevideo.com.evil.test/index.m3u8",
        "https://127.0.0.1/index.m3u8",
        "https://manifest.googlevideo.com:8443/index.m3u8",
        "https://manifest.googlevideo.com/n/challenge/index.m3u8",
        "https://manifest.googlevideo.com/index.m3u8?n=challenge",
    ] {
        assert!(media_url(value).is_err(), "{value}");
    }
}

#[test]
fn expired_links_refresh_only_with_the_same_encoding_and_segment_identity() {
    let preview = fixture();
    let old = preview.playlist;
    let mut new = old.clone();
    for part in new
        .parts
        .iter_mut()
        .chain(new.audio.iter_mut().flat_map(|a| &mut a.parts))
    {
        part.url = part
            .url
            .replace("expire/100", "expire/999")
            .replace("manifest.googlevideo.com", "rr1.googlevideo.com");
    }
    check_refresh(&old, &new).unwrap();
    for mutation in 0..7 {
        let mut changed = new.clone();
        match mutation {
            0 => changed.parts[0].url = changed.parts[0].url.replace("lmt%3D5678", "lmt%3D9999"),
            1 => changed.parts[0].url = changed.parts[0].url.replace("content123", "other-content"),
            2 => changed.parts[0].url = changed.parts[0].url.replace("gosq/0", "gosq/1"),
            3 => changed.duration_ms += 1,
            4 => changed.audio.as_mut().unwrap().duration_ms += 1,
            5 => changed.parts[0].url = changed.parts[0].url.replace("lmt%3D5678", "no-version"),
            _ => changed.parts.push(changed.parts[0].clone()),
        }
        assert!(
            check_refresh(&old, &changed).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn refresh_matches_quality_and_audio_identity_instead_of_list_position() {
    let preview = fixture();
    let selected = Variant {
        url: url(95, false),
        bandwidth: 2_000_000,
        resolution: Some((1280, 720)),
        codecs: preview.selection.codecs.clone(),
        unsupported: None,
        audio: vec![AudioRendition {
            name: "English".into(),
            language: Some("en".into()),
            url: Some(url(234, false)),
            is_default: true,
        }],
    };
    let mut other = selected.clone();
    other.url = url(93, false);
    let mut inspection = Inspection {
        public_only: false,
        source: preview.playlist.source,
        variants: vec![other, selected.clone()],
        playlist: None,
        dash: None,
        tiktok: None,
        page: None,
    };
    assert_eq!(preview.selection.indices(&inspection).unwrap(), (1, 0));
    inspection.variants.push(selected);
    assert!(preview.selection.indices(&inspection).is_err());
    inspection.variants.remove(2);
    inspection.variants[1].audio[0].language = Some("es".into());
    assert!(preview.selection.indices(&inspection).is_err());
}

#[test]
fn youtube_selection_is_persisted_atomically_and_old_hls_versions_still_load() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running(MAIN_QUEUE, false).unwrap();
    let preview = fixture();
    let job = store
        .enqueue_youtube(&preview, "Video.mp4", None, MAIN_QUEUE, "youtube-fixture")
        .unwrap();
    assert_eq!(job.url, watch_url("YE7VzlLtp-4").unwrap());
    let download = job.hls.as_ref().unwrap();
    assert_eq!(download.version, 5);
    assert_eq!(download.youtube.as_ref().unwrap(), &preview.selection);
    assert_eq!(
        store
            .enqueue_youtube(&preview, "Video.mp4", None, MAIN_QUEUE, "youtube-fixture")
            .unwrap()
            .id,
        job.id
    );
    assert_eq!(store.list().unwrap().len(), 1);
    let mut changed = fixture();
    changed.selection.codecs = "avc1.64001f,mp4a.40.2".into();
    assert!(
        store
            .enqueue_youtube(&changed, "Video.mp4", None, MAIN_QUEUE, "youtube-fixture")
            .unwrap_err()
            .to_string()
            .contains("another YouTube selection")
    );
    assert!(
        store
            .enqueue_hls(
                &preview.playlist,
                "Video.mp4",
                None,
                MAIN_QUEUE,
                "youtube-fixture",
            )
            .unwrap_err()
            .to_string()
            .contains("another YouTube selection")
    );
    assert_eq!(store.list().unwrap().len(), 1);
    drop(store);
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store
        .get(&job.id)
        .unwrap()
        .unwrap()
        .validate_persisted()
        .unwrap();
    let legacy = Download::new(preview.playlist.clone());
    let json = serde_json::to_string(&legacy).unwrap();
    assert!(!json.contains("youtube\""));
    serde_json::from_str::<Download>(&json)
        .unwrap()
        .validate()
        .unwrap();
    let mut invalid = download.clone();
    invalid.version = 2;
    assert!(invalid.validate().is_err());
    invalid = download.clone();
    invalid.youtube.as_mut().unwrap().video_id = "abcdefghijk".into();
    assert!(invalid.validate().is_err());
    invalid = download.clone();
    invalid.youtube = None;
    assert!(invalid.validate().is_err());
    invalid = download.clone();
    invalid.completed = vec![
        CompletedPart {
            size: 188,
            sha256: "ab".repeat(32)
        };
        3
    ];
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn cancelled_youtube_lookup_stops_before_any_network_request() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = inspect("https://youtu.be/YE7VzlLtp-4", &cancel)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("cancelled"));
}

#[tokio::test]
async fn direct_download_of_a_youtube_page_fails_before_saving_html() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let job = store
        .new_download(
            &watch_url("YE7VzlLtp-4").unwrap(),
            Some("video.mp4"),
            None,
            "direct-youtube-fixture".into(),
        )
        .unwrap();
    let job = store.enqueue(&job).unwrap();
    let id = job.id.clone();
    crate::engine::run(
        store.clone(),
        job,
        CancellationToken::new(),
        crate::bandwidth::Limiter::new(&crate::bandwidth::Limits::default()).unwrap(),
    )
    .await
    .unwrap();
    let saved = store.get(&id).unwrap().unwrap();
    assert!(saved.error.unwrap().contains("Preview video"));
    assert!(saved.saved_path.is_none());
    assert_eq!(saved.downloaded, 0);
}

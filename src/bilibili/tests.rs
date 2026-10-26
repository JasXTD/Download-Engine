use super::*;
use crate::{
    hls::{Download, Format, Part, Range},
    paths::AppPaths,
    queue::MAIN_QUEUE,
    store::Store,
};
use serde_json::json;

const BVID: &str = "BV1hCnw65EaE";

#[test]
fn bilibili_links_identify_one_video_part_without_accepting_lookalikes() {
    for (value, page) in [
        ("https://www.bilibili.com/video/BV1hCnw65EaE/", 1),
        (
            "https://www.bilibili.com/video/BV1hCnw65EaE?spm_id_from=333.1007&vd_source=x",
            1,
        ),
        ("https://bilibili.com/video/BV1hCnw65EaE/?p=3", 3),
        ("http://m.bilibili.com/video/av170001?p=10", 10),
    ] {
        let reference = reference(value).unwrap();
        assert_eq!(reference.page, page, "{value}");
        assert!(is_url(value));
    }
    assert!(matches!(
        reference("https://www.bilibili.com/video/av170001")
            .unwrap()
            .id,
        Id::Av(170001)
    ));
    for value in [
        "https://bilibili.com.evil.test/video/BV1hCnw65EaE",
        "https://www.bilibili.com:8443/video/BV1hCnw65EaE",
        "https://www.bilibili.com/",
        "https://www.bilibili.com/video/BV1hCnw65EaE/extra",
        "https://www.bilibili.com/video/BVshort",
        "https://www.bilibili.com/video/av0",
        "https://www.bilibili.com/video/av12x",
        "https://www.bilibili.com/video/BV1hCnw65EaE?p=0",
        "https://www.bilibili.com/video/BV1hCnw65EaE?p=1001",
        "https://b23.tv/BV1hCnw65EaE",
        "file:///video/BV1hCnw65EaE",
    ] {
        assert!(!is_url(value), "{value}");
    }
    assert_eq!(
        source_url(BVID, 1).unwrap(),
        "https://www.bilibili.com/video/BV1hCnw65EaE/"
    );
    assert_eq!(
        source_url(BVID, 2).unwrap(),
        "https://www.bilibili.com/video/BV1hCnw65EaE/?p=2"
    );
    assert!(source_url("BV1hCnw65EaE/../x", 1).is_err());
    assert!(crate::hls::is_site_video(&source_url(BVID, 2).unwrap()));
}

#[test]
fn bangumi_links_identify_one_episode_or_season() {
    for (value, ep) in [
        ("https://www.bilibili.com/bangumi/play/ep733316", 733316),
        (
            "https://www.bilibili.com/bangumi/play/ep733316/?from_spmid=666.25&p=9",
            733316,
        ),
        ("https://m.bilibili.com/bangumi/play/ep1", 1),
        (
            "http://bilibili.com/bangumi/play/ep999999999999",
            999_999_999_999,
        ),
    ] {
        let reference = reference(value).unwrap();
        assert!(matches!(reference.id, Id::Episode(n) if n == ep), "{value}");
        assert_eq!(reference.page, 1, "{value}");
        assert!(is_url(value) && crate::hls::is_site_video(value));
    }
    assert!(matches!(
        reference("https://www.bilibili.com/bangumi/play/ss28747")
            .unwrap()
            .id,
        Id::Season(28747)
    ));
    for value in [
        "https://www.bilibili.com/bangumi/play/ep0",
        "https://www.bilibili.com/bangumi/play/ep",
        "https://www.bilibili.com/bangumi/play/ep12x",
        "https://www.bilibili.com/bangumi/play/ep1234567890123",
        "https://www.bilibili.com/bangumi/play/md28747",
        "https://www.bilibili.com/bangumi/media/md28747",
        "https://www.bilibili.com/bangumi/play/ep733316/extra",
        "https://bilibili.com.evil.test/bangumi/play/ep733316",
    ] {
        assert!(!is_url(value), "{value}");
    }
    assert_eq!(
        episode_url(733316).unwrap(),
        "https://www.bilibili.com/bangumi/play/ep733316"
    );
    assert!(episode_url(0).is_err());
}

#[test]
fn md5_matches_rfc_1321_test_suite() {
    for (input, digest) in [
        ("", "d41d8cd98f00b204e9800998ecf8427e"),
        ("a", "0cc175b9c0f1b6a831c399e269772661"),
        ("abc", "900150983cd24fb0d6963f7d28e17f72"),
        ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        (
            "abcdefghijklmnopqrstuvwxyz",
            "c3fcd3d76192e4007dfb496cca67e13b",
        ),
        (
            "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
            "57edf4a22be3c955ac49da2e2107b67a",
        ),
    ] {
        assert_eq!(client::md5_hex(input.as_bytes()), digest, "{input}");
    }
}

#[test]
fn wbi_signature_matches_the_documented_example_and_encoding() {
    let key = client::mixin("7cd084941338484aae1ad9425b84077c4932caff0ff746eab6f01bf08b70ac45");
    assert_eq!(key, "ea1db124af3c7062474693fa704f4ff8");
    let signed = client::sign(
        &[
            ("foo", "114".into()),
            ("bar", "514".into()),
            ("zab", "1919810".into()),
        ],
        &key,
        1702204169,
    );
    let pairs: Vec<_> = signed.iter().map(|(k, v)| format!("{k}={v}")).collect();
    assert_eq!(
        pairs,
        [
            "bar=514",
            "foo=114",
            "wts=1702204169",
            "zab=1919810",
            "w_rid=8f6f2b5b3d485fe1886cec6a0be8c5d4"
        ]
    );
    // Values lose !'()* and are percent-encoded like Python's quote(safe="").
    let signed = client::sign(&[("q", "a b/c!'()*~".into())], &key, 1);
    assert_eq!(signed.last().unwrap().1, "ca6da6b9f7879d7876f8b06c62de0477");
}

/// A playurl reply as Bilibili sends it: every field in two spellings.
fn play_json(video_host: &str) -> serde_json::Value {
    let stream = |id: u32,
                  codecid: u32,
                  codecs: &str,
                  bandwidth: u64,
                  kind: &str,
                  size: (u32, u32)| {
        let url = format!(
            "https://{video_host}/upgcxcode/75/48/1/1-1-{id}.m4s?e=x&deadline=1&upsig=a&os=akam"
        );
        let backup =
            format!("https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/48/1/1-1-{id}.m4s?e=y");
        let segment = json!({"initialization": "0-914", "index_range": "915-1158"});
        json!({
            "id": id, "baseUrl": url, "base_url": url, "backupUrl": [backup], "backup_url": [backup],
            "bandwidth": bandwidth, "mimeType": format!("{kind}/mp4"), "mime_type": format!("{kind}/mp4"),
            "codecs": codecs, "codecid": codecid, "width": size.0, "height": size.1,
            "frameRate": "30.006", "frame_rate": "30.006",
            "SegmentBase": {"Initialization": "0-914", "indexRange": "915-1158"}, "segment_base": segment,
        })
    };
    json!({
        "quality": 32, "timelength": 80155, "accept_quality": [80, 64, 32, 16],
        "dash": {
            "duration": 81, "minBufferTime": 1.5, "min_buffer_time": 1.5,
            "video": [
                stream(32, 7, "avc1.64001F", 1_000_000, "video", (852, 480)),
                stream(32, 13, "av01.0.08M.08.0.110.01.01.01.0", 700_000, "video", (852, 480)),
                stream(16, 7, "avc1.64001E", 400_000, "video", (640, 360)),
            ],
            "audio": [
                stream(30280, 0, "mp4a.40.2", 192_000, "audio", (0, 0)),
                stream(30216, 0, "mp4a.40.2", 64_000, "audio", (0, 0)),
            ],
            "dolby": {"type": 0, "audio": null}, "flac": null,
        },
    })
}

#[test]
fn playurl_reply_becomes_an_indexed_dash_choice_list() {
    let play: client::Play =
        serde_json::from_value(play_json("upos-hz-mirrorakam.akamaized.net")).unwrap();
    let source = source_url(BVID, 1).unwrap();
    let text = manifest(&play).unwrap();
    let inspection = inspection(source.clone(), &play).unwrap();
    assert_eq!(inspection.source, source);
    // Bilibili's video servers get its own page as the Referer.
    assert_eq!(inspection.page.as_deref(), Some(source.as_str()));
    let labels: Vec<_> = inspection
        .variants
        .iter()
        .map(|v| (v.resolution, v.codecs.as_str(), v.bandwidth))
        .collect();
    // Highest bandwidth first; both codecs of the same resolution stay choices.
    assert_eq!(
        labels,
        [
            (Some((852, 480)), "avc1.64001F", 1_000_000),
            (Some((852, 480)), "av01.0.08M.08.0.110.01.01.01.0", 700_000),
            (Some((640, 360)), "avc1.64001E", 400_000),
        ]
    );
    let audio: Vec<_> = inspection.variants[0]
        .audio
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(audio, ["AAC · 192 kb/s", "AAC · 64 kb/s"]);
    // Saved keys find the same choices again after a refresh.
    let dash = inspection.dash.as_ref().unwrap();
    assert_eq!(dash.position("0/16-7", Some("1/30216")), Some((2, 1)));
    assert_eq!(dash.position("0/80-7", Some("1/30216")), None);
    assert_eq!(dash.position("0/16-7", None), None);
    // Signed query strings survive XML escaping exactly.
    assert!(text.contains("e=x&amp;deadline=1&amp;upsig=a"));
}

#[test]
fn bangumi_replies_use_result_and_flag_member_previews() {
    // Bangumi (PGC) endpoints put the payload in `result`, others in `data`.
    let season: client::Season = client::payload(
        &json!({"code": 0, "message": "success", "result": {"title": "Series", "episodes": []}})
            .to_string(),
    )
    .unwrap();
    assert_eq!(season.title, "Series");
    let view: serde_json::Value =
        client::payload(&json!({"code": 0, "data": {"bvid": BVID}}).to_string()).unwrap();
    assert_eq!(view["bvid"], BVID);
    let missing = client::payload::<client::Season>(
        &json!({"code": -404, "message": "啥都木有", "result": null}).to_string(),
    )
    .err()
    .unwrap();
    assert!(missing.to_string().contains("does not exist"));
    let region = client::payload::<client::Season>(
        &json!({"code": -10403, "message": "抱歉您所在地区不可观看！"}).to_string(),
    )
    .err()
    .unwrap();
    assert!(region.to_string().contains("region"));
    // The play reply flags arrive as numbers or booleans.
    let mut reply = play_json("upos-hz-mirrorakam.akamaized.net");
    reply["is_preview"] = json!(1);
    reply["is_drm"] = json!(false);
    let play: client::Play = serde_json::from_value(reply).unwrap();
    assert!(client::flag(play.is_preview.as_ref()));
    assert!(!client::flag(play.is_drm.as_ref()));
    assert!(client::flag(Some(&json!(true))));
    assert!(!client::flag(Some(&json!(0))) && !client::flag(None));
}

#[test]
fn season_lists_extra_episodes_and_names_them_like_the_site() {
    let season: client::Season = serde_json::from_value(json!({
        "title": "凡人修仙传",
        "episodes": [
            {"id": 733316, "ep_id": 733316, "cid": 1022370693, "title": "1",
             "long_title": "凡人风起天南1重制版", "show_title": "第1话 凡人风起天南1重制版"},
            {"id": 733317, "ep_id": 733317, "cid": 2, "title": "2", "long_title": "Second",
             "show_title": null},
        ],
        "section": [
            {"title": "预告", "episodes": [{"id": 900, "cid": 3, "title": "PV", "long_title": null}]},
            {"title": "empty", "episodes": null},
        ],
    }))
    .unwrap();
    let ids: Vec<_> = season.all_episodes().map(|e| e.id).collect();
    assert_eq!(ids, [733316, 733317, 900]);
    let named = |id| {
        episode_title(
            &season.title,
            season.all_episodes().find(|e| e.id == id).unwrap(),
        )
    };
    assert_eq!(named(733316), "凡人修仙传 - 第1话 凡人风起天南1重制版");
    assert_eq!(named(733317), "凡人修仙传 - 2 Second");
    assert_eq!(named(900), "凡人修仙传 - PV");
    let bare: client::Season =
        serde_json::from_value(json!({"title": "Series", "episodes": [{"id": 1, "cid": 1}]}))
            .unwrap();
    assert_eq!(
        episode_title(&bare.title, bare.all_episodes().next().unwrap()),
        "Series"
    );
}

#[test]
fn peer_mirrors_are_avoided_and_old_single_file_replies_are_refused() {
    let play: client::Play =
        serde_json::from_value(play_json("xy1x2x3x4xy.mcdn.bilivideo.cn:4483")).unwrap();
    let text = manifest(&play).unwrap();
    assert!(!text.contains("mcdn.bilivideo.cn"));
    assert!(text.contains("upos-sz-mirrorcos.bilivideo.com"));
    let legacy: client::Play = serde_json::from_value(
        json!({"timelength": 1000, "durl": [{"url": "https://x.test/a.flv"}]}),
    )
    .unwrap();
    assert!(
        manifest(&legacy)
            .unwrap_err()
            .to_string()
            .contains("old single-file")
    );
}

fn preview(host: &str) -> Preview {
    let video = format!("https://{host}/1-1-32.m4s?deadline=1");
    let audio = format!("https://{host}/1-1-30280.m4s?deadline=1");
    let index = |url: &str| {
        serde_json::from_value::<crate::dash::IndexIdentity>(json!({
            "url": url, "range": {"start": 915, "length": 244}, "sha256": "a".repeat(64),
            "total": 100_000, "etag": null, "last_modified": null,
        }))
        .unwrap()
    };
    let parts = |url: &str| {
        vec![
            Part {
                url: url.into(),
                range: Some(Range {
                    start: 0,
                    length: 915,
                }),
                initialization: true,
            },
            Part {
                url: url.into(),
                range: Some(Range {
                    start: 1159,
                    length: 50_000,
                }),
                initialization: false,
            },
        ]
    };
    let playlist = Playlist {
        public_only: false,
        source: source_url(BVID, 1).unwrap(),
        url: source_url(BVID, 1).unwrap(),
        quality: "852×480".into(),
        bandwidth: Some(1_192_000),
        duration_ms: 80_155,
        sequence: 0,
        format: Format::FragmentedMp4,
        parts: parts(&video),
        audio: Some(crate::hls::AudioTrack {
            name: "AAC · 192 kb/s".into(),
            language: None,
            url: source_url(BVID, 1).unwrap(),
            duration_ms: 80_155,
            sequence: 0,
            format: Format::FragmentedMp4,
            parts: parts(&audio),
        }),
        dash: Some(crate::dash::Selection {
            video: "0/32-7".into(),
            audio: Some("1/30280".into()),
            video_offset_us: 0,
            audio_offset_us: 0,
            video_index: Some(index(&video)),
            audio_index: Some(index(&audio)),
        }),
    };
    Preview {
        playlist,
        selection: Selection {
            bvid: BVID.into(),
            page: 1,
            episode: None,
            video: "0/32-7".into(),
            audio: Some("1/30280".into()),
        },
    }
}

#[test]
fn reviewed_bilibili_choice_is_saved_as_a_version_6_download() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let preview = preview("upos-hz-mirrorakam.akamaized.net");
    let job = store
        .enqueue_bilibili(&preview, "Video.mp4", None, MAIN_QUEUE, "bilibili-request")
        .unwrap();
    let saved = store.get(&job.id).unwrap().unwrap();
    let download = saved.hls.unwrap();
    assert_eq!(download.version, 6);
    assert_eq!(download.bilibili.as_ref(), Some(&preview.selection));
    assert!(download.youtube.is_none());
    download.validate().unwrap();
    // A saved choice must match its playlist; a mismatched one is refused.
    let mut mismatched = Download::new(preview.playlist.clone());
    mismatched.version = 6;
    mismatched.bilibili = Some(Selection {
        video: "0/16-7".into(),
        ..preview.selection.clone()
    });
    assert!(mismatched.validate().is_err());
    let mut unmarked = Download::new(preview.playlist);
    unmarked.bilibili = Some(preview.selection);
    assert!(unmarked.validate().is_err());
}

#[test]
fn bangumi_choices_save_the_episode_and_older_choices_still_load() {
    // A 1.18/1.19 saved choice has no episode field.
    let saved: Selection = serde_json::from_value(
        json!({"bvid": BVID, "page": 2, "video": "0/32-7", "audio": "1/30280"}),
    )
    .unwrap();
    assert_eq!(saved.episode, None);
    assert_eq!(
        saved.source().unwrap(),
        "https://www.bilibili.com/video/BV1hCnw65EaE/?p=2"
    );
    assert!(!serde_json::to_string(&saved).unwrap().contains("episode"));
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let mut preview = preview("upos-hz-mirrorakam.akamaized.net");
    preview.playlist.source = episode_url(733316).unwrap();
    preview.selection = Selection {
        bvid: String::new(),
        page: 1,
        episode: Some(733316),
        ..preview.selection
    };
    let job = store
        .enqueue_bilibili(&preview, "Episode.mp4", None, MAIN_QUEUE, "bangumi-request")
        .unwrap();
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.url, "https://www.bilibili.com/bangumi/play/ep733316");
    let download = saved.hls.unwrap();
    assert_eq!(download.version, 6);
    assert_eq!(download.bilibili.as_ref(), Some(&preview.selection));
    download.validate().unwrap();
    // An episode choice is tied to its own page and carries no video ID.
    for wrong in [
        Selection {
            episode: Some(733317),
            ..preview.selection.clone()
        },
        Selection {
            bvid: BVID.into(),
            ..preview.selection.clone()
        },
        Selection {
            page: 2,
            ..preview.selection.clone()
        },
    ] {
        assert!(wrong.validate(&preview.playlist).is_err());
    }
}

#[test]
fn expired_links_refresh_only_while_the_media_is_unchanged() {
    let old = preview("upos-hz-mirrorakam.akamaized.net").playlist;
    let new = preview("upos-sz-mirrorcos.bilivideo.com").playlist;
    check_refresh(&old, &new).unwrap();
    let mut longer = new.clone();
    longer.duration_ms += 1;
    assert!(check_refresh(&old, &longer).is_err());
    let mut moved = new.clone();
    moved.parts[1].range = Some(Range {
        start: 1159,
        length: 50_001,
    });
    assert!(check_refresh(&old, &moved).is_err());
    let mut reindexed = new;
    let dash = reindexed.dash.as_mut().unwrap();
    dash.audio_index = serde_json::from_value(json!({
        "url": reindexed.audio.as_ref().unwrap().parts[0].url, "range": {"start": 915, "length": 244},
        "sha256": "b".repeat(64), "total": 100_000, "etag": null, "last_modified": null,
    }))
    .unwrap();
    assert!(check_refresh(&old, &reindexed).is_err());
}

#[tokio::test]
async fn cancelled_bilibili_lookup_stops_before_any_network_request() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = inspect_via(
        "https://www.bilibili.com/video/BV1hCnw65EaE",
        &cancel,
        crate::proxy::Route::direct(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("cancelled"));
}

#[tokio::test]
async fn direct_download_of_a_bilibili_page_fails_before_saving_html() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let job = store
        .new_download(
            &source_url(BVID, 1).unwrap(),
            Some("video.mp4"),
            None,
            "direct-bilibili-fixture".into(),
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
}

/// Live check against Bilibili: cargo test --lib bilibili::tests::live -- --ignored
#[tokio::test]
#[ignore = "contacts Bilibili"]
async fn live_lookup_and_preview_resolve_signed_dash_streams() {
    let cancel = CancellationToken::new();
    let route = crate::proxy::Route::direct();
    let video = inspect_via(
        "https://www.bilibili.com/video/BV1hCnw65EaE/",
        &cancel,
        route.clone(),
    )
    .await
    .unwrap();
    assert!(!video.title.is_empty());
    assert!(!video.inspection.variants.is_empty());
    let preview = preview_via(&video.inspection, 0, 0, &cancel, route)
        .await
        .unwrap();
    assert!(preview.playlist.dash.as_ref().unwrap().has_index());
    assert!(preview.playlist.parts.len() > 1);
    println!(
        "{} | {} | {} parts + {} audio parts",
        video.title,
        preview.playlist.quality,
        preview.playlist.parts.len(),
        preview.playlist.audio.as_ref().map_or(0, |a| a.parts.len())
    );
}

/// Live check of a free Bangumi episode: cargo test --lib bilibili::tests::live -- --ignored
#[tokio::test]
#[ignore = "contacts Bilibili"]
async fn live_bangumi_episode_resolves_its_dash_streams() {
    let cancel = CancellationToken::new();
    let route = crate::proxy::Route::direct();
    let video = inspect_via(
        "https://www.bilibili.com/bangumi/play/ep733316",
        &cancel,
        route.clone(),
    )
    .await
    .unwrap();
    assert!(video.title.starts_with("凡人修仙传 - "), "{}", video.title);
    assert_eq!(
        video.inspection.source,
        "https://www.bilibili.com/bangumi/play/ep733316"
    );
    let smallest = video.inspection.variants.len() - 1;
    let preview = preview_via(&video.inspection, smallest, 0, &cancel, route)
        .await
        .unwrap();
    assert_eq!(preview.selection.episode, Some(733316));
    assert!(preview.playlist.dash.as_ref().unwrap().has_index());
    println!(
        "{} | {} | {} parts + {} audio parts",
        video.title,
        preview.playlist.quality,
        preview.playlist.parts.len(),
        preview.playlist.audio.as_ref().map_or(0, |a| a.parts.len())
    );
}

/// Live end to end: cargo test --lib bilibili::tests::live -- --ignored
#[tokio::test]
#[ignore = "contacts Bilibili and downloads about 1 MB"]
async fn live_download_fetches_and_merges_video_and_audio() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let route = crate::proxy::Route::direct();
    let video = inspect_via(
        "https://www.bilibili.com/video/BV1hCnw65EaE/",
        &cancel,
        route.clone(),
    )
    .await
    .unwrap();
    // The smallest offered choice keeps the download short.
    let smallest = video.inspection.variants.len() - 1;
    let preview = preview_via(&video.inspection, smallest, 0, &cancel, route)
        .await
        .unwrap();
    let job = store
        .enqueue_bilibili(
            &preview,
            "bilibili-live.mp4",
            None,
            MAIN_QUEUE,
            "bilibili-live",
        )
        .unwrap();
    let id = job.id.clone();
    crate::engine::run(
        store.clone(),
        job,
        cancel,
        crate::bandwidth::Limiter::new(&crate::bandwidth::Limits::default()).unwrap(),
    )
    .await
    .unwrap();
    let saved = store.get(&id).unwrap().unwrap();
    assert_eq!(
        saved.status,
        crate::model::Status::Complete,
        "{:?}",
        saved.error
    );
    let path = saved.saved_path.unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() > 100_000);
    // An MP4 file with both a video and a sound track.
    assert_eq!(&bytes[4..8], b"ftyp");
    assert!(bytes.windows(4).any(|w| w == b"vide") && bytes.windows(4).any(|w| w == b"soun"));
    println!("{} bytes at {}", bytes.len(), path.display());
}

/// Live end to end with a 12-second Bangumi extra, which the download re-reads
/// by its episode page: cargo test --lib bilibili::tests::live -- --ignored
#[tokio::test]
#[ignore = "contacts Bilibili and downloads under 1 MB"]
async fn live_bangumi_extra_downloads_by_its_episode_page() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let route = crate::proxy::Route::direct();
    let video = inspect_via(&episode_url(334568).unwrap(), &cancel, route.clone())
        .await
        .unwrap();
    let smallest = video.inspection.variants.len() - 1;
    let preview = preview_via(&video.inspection, smallest, 0, &cancel, route)
        .await
        .unwrap();
    let job = store
        .enqueue_bilibili(
            &preview,
            "bangumi-live.mp4",
            None,
            MAIN_QUEUE,
            "bangumi-live",
        )
        .unwrap();
    let id = job.id.clone();
    crate::engine::run(
        store.clone(),
        job,
        cancel,
        crate::bandwidth::Limiter::new(&crate::bandwidth::Limits::default()).unwrap(),
    )
    .await
    .unwrap();
    let saved = store.get(&id).unwrap().unwrap();
    assert_eq!(
        saved.status,
        crate::model::Status::Complete,
        "{:?}",
        saved.error
    );
    let bytes = std::fs::read(saved.saved_path.unwrap()).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");
    assert!(bytes.windows(4).any(|w| w == b"vide") && bytes.windows(4).any(|w| w == b"soun"));
    println!("{} | {} bytes", video.title, bytes.len());
}

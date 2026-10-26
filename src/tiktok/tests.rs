use super::*;
use crate::{hls::Download, paths::AppPaths, queue::MAIN_QUEUE, store::Store};
use serde_json::json;

const ID: &str = "7696495296011930887";
const MIB: u64 = 1024 * 1024;

#[test]
fn tiktok_links_identify_one_video_without_accepting_lookalikes() {
    for (value, author) in [
        (
            "https://www.tiktok.com/@jannanickofficial/video/7696495296011930887",
            "jannanickofficial",
        ),
        (
            "https://www.tiktok.com/@jannanickofficial/video/7696495296011930887/?is_from_webapp=1&sender_device=pc",
            "jannanickofficial",
        ),
        (
            "https://m.tiktok.com/@user.name_1/video/7696495296011930887",
            "user.name_1",
        ),
        ("https://tiktok.com/@/video/7696495296011930887", ""),
    ] {
        let reference = reference(value).unwrap();
        assert_eq!(
            (reference.author.as_str(), reference.video_id.as_str()),
            (author, ID),
            "{value}"
        );
        assert!(is_url(value) && crate::hls::is_site_video(value));
    }
    for value in [
        "https://tiktok.com.evil.test/@user/video/7696495296011930887",
        "https://www.tiktok.com:8443/@user/video/7696495296011930887",
        "https://www.tiktok.com/@user/photo/7696495296011930887",
        "https://www.tiktok.com/@user",
        "https://www.tiktok.com/video/7696495296011930887",
        "https://www.tiktok.com/@user/video/76964952960119x",
        "https://www.tiktok.com/@us er/video/7696495296011930887",
        "https://vm.tiktok.com/ZMabcdef/",
        "file:///@user/video/7696495296011930887",
    ] {
        assert!(!is_url(value), "{value}");
    }
    assert_eq!(
        source_url("jannanickofficial", ID).unwrap(),
        "https://www.tiktok.com/@jannanickofficial/video/7696495296011930887"
    );
    assert!(source_url("bad/name", ID).is_err());
}

#[test]
fn visitor_cookies_keep_only_well_formed_pairs_for_the_video_servers() {
    let headers = [
        "ttwid=1%7Cabc; Domain=.tiktok.com; Path=/; Expires=Thu, 21 Oct 2027 00:00:00 GMT; HttpOnly",
        "tt_chain_token=xyz==; Max-Age=15552000; Secure",
        "msToken=a-b_c; Path=/",
        "bad name=value",
        "comma=a,b",
        "=empty",
    ];
    assert_eq!(
        client::cookies(headers.into_iter()),
        "ttwid=1%7Cabc; tt_chain_token=xyz==; msToken=a-b_c"
    );
}

fn page(detail: serde_json::Value) -> String {
    let data =
        json!({"__DEFAULT_SCOPE__": {"webapp.app-context": {}, "webapp.video-detail": detail}});
    format!(
        r#"<html><head><script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{data}</script></head></html>"#
    )
}

fn item() -> serde_json::Value {
    let play = |id: &str, size: u64, w: u64, h: u64, hosts: &[&str]| {
        // As on the real page, the size arrives as a string.
        json!({
            "DataSize": size.to_string(), "FileHash": format!("hash-{id}"), "FileCs": "c:0-1", "Width": w, "Height": h,
            "Uri": "v0f044", "UrlKey": id,
            "UrlList": hosts.iter().map(|h| format!("https://{h}/video/tos/alisg/{id}/?mime_type=video_mp4&expire=1&signature=s")).collect::<Vec<_>>(),
        })
    };
    let tiktok = [
        "v16-webapp-prime.tiktok.com",
        "v19-webapp-prime.tiktok.com",
        "www.tiktok.com",
    ];
    json!({
        "id": ID, "desc": "Sunset walk #fyp", "author": {"uniqueId": "jannanickofficial"}, "imagePost": null,
        "video": {"duration": 42, "format": "mp4", "playAddr": "https://v16-webapp-prime.tiktok.com/x", "bitrateInfo": [
            {"GearName": "adapt_540_1", "Bitrate": 624333, "CodecType": "h265_hvc1", "PlayAddr": play("c", 40 * MIB, 576, 1024, &tiktok)},
            {"GearName": "lower_540_0", "Bitrate": 833535, "CodecType": "h264", "PlayAddr": play("a", 4_429_719, 576, 1024, &tiktok)},
            {"GearName": "adapt_lower_720_1", "Bitrate": 763790, "CodecType": "h265_hvc1", "PlayAddr": play("b", 4_059_167, 720, 1280, &tiktok)},
            // Skipped: only a foreign server, no size, or a repeated quality.
            {"GearName": "foreign", "Bitrate": 1, "CodecType": "h264", "PlayAddr": play("d", 100, 1, 1, &["cdn.example.test"])},
            {"GearName": "empty", "Bitrate": 1, "CodecType": "h264", "PlayAddr": play("e", 0, 1, 1, &tiktok)},
            {"GearName": "lower_540_0", "Bitrate": 1, "CodecType": "h264", "PlayAddr": play("f", 5, 1, 1, &tiktok)},
        ]},
    })
}

#[test]
fn page_data_is_read_without_scripts_and_status_codes_become_clear_errors() {
    let html = page(json!({"statusCode": 0, "statusMsg": "", "itemInfo": {"itemStruct": item()}}));
    assert_eq!(client::item(&html).unwrap()["id"], ID);
    for (code, message) in [
        (10204, "removed or does not exist"),
        (10222, "private"),
        (12345, "code 12345: blocked"),
    ] {
        let html = page(json!({"statusCode": code, "statusMsg": "blocked"}));
        assert!(
            client::item(&html)
                .unwrap_err()
                .to_string()
                .contains(message),
            "{code}"
        );
    }
    let unexpected = "<html><body>Something else</body></html>";
    assert!(
        client::item(unexpected)
            .unwrap_err()
            .to_string()
            .contains("did not return the video data")
    );
    // TikTok's JavaScript browser check (as served to this PC) is reported, never passed.
    let challenge = r#"<html><head><script id="slardar-config" type="application/json">{"slardarClient": "SlardarWAF"}</script></head><body>Please wait... <p id="wci" class="_wafchallengeid"></p></body></html>"#;
    assert!(
        client::item(challenge)
            .unwrap_err()
            .to_string()
            .contains("browser check")
    );
}

#[test]
fn qualities_are_listed_largest_first_with_h264_before_h265_at_equal_size() {
    let (title, list) = listing(&item(), ID).unwrap();
    assert_eq!(title, "Sunset walk #fyp");
    assert_eq!(list.duration_ms, 42_000);
    let order: Vec<_> = list
        .qualities
        .iter()
        .map(|q| (q.gear.as_str(), q.codec.as_str(), q.resolution))
        .collect();
    assert_eq!(
        order,
        [
            ("adapt_lower_720_1", "hvc1", Some((720, 1280))),
            ("lower_540_0", "avc1", Some((576, 1024))),
            ("adapt_540_1", "hvc1", Some((576, 1024))),
        ]
    );
    assert!(
        list.qualities
            .iter()
            .all(|q| q.url.starts_with("https://v16-webapp-prime.tiktok.com/"))
    );
    let mut photo = item();
    photo["imagePost"] = json!({"images": []});
    assert!(
        listing(&photo, ID)
            .unwrap_err()
            .to_string()
            .contains("photo")
    );
    assert!(
        listing(&item(), "1")
            .unwrap_err()
            .to_string()
            .contains("different video")
    );
    let mut silent = item();
    silent["desc"] = json!("  ");
    assert_eq!(
        listing(&silent, ID).unwrap().0,
        format!("TikTok @jannanickofficial {ID}")
    );
}

fn preview_of(gear: &str) -> Preview {
    let (_, listing) = listing(&item(), ID).unwrap();
    let index = listing
        .qualities
        .iter()
        .position(|q| q.gear == gear)
        .unwrap();
    preview(&inspection(listing).unwrap(), index).unwrap()
}

#[test]
fn a_quality_becomes_one_mp4_file_in_consecutive_resumable_parts() {
    let preview = preview_of("adapt_540_1");
    let ranges: Vec<_> = preview
        .playlist
        .parts
        .iter()
        .map(|p| p.range.clone().map(|r| (r.start, r.length)))
        .collect();
    assert_eq!(
        ranges,
        [
            Some((0, 16 * MIB)),
            Some((16 * MIB, 16 * MIB)),
            Some((32 * MIB, 8 * MIB))
        ]
    );
    assert_eq!(preview.playlist.format, Format::Progressive);
    assert_eq!(preview.playlist.output_extension(), "mp4");
    assert!(!preview.playlist.needs_merge());
    assert_eq!(
        preview.selection,
        Selection {
            video_id: ID.into(),
            author: "jannanickofficial".into(),
            gear: "adapt_540_1".into(),
            size: 40 * MIB,
            hash: "hash-c".into(),
        }
    );
    assert_eq!(preview_of("lower_540_0").playlist.parts.len(), 1);
    // Single files must be consecutive ranges of one address, with no other tracks.
    let mut gap = preview.playlist.clone();
    gap.parts[1].range.as_mut().unwrap().start += 1;
    assert!(gap.validate().is_err());
    let mut mixed = preview.playlist.clone();
    mixed.parts[2].url = mixed.parts[2].url.replace("v16-", "v19-");
    assert!(mixed.validate().is_err());
    let mut ranged = preview.playlist;
    ranged.parts[0].range = None;
    assert!(ranged.validate().is_err());
}

#[test]
fn reviewed_tiktok_choice_is_saved_as_a_version_7_download() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let preview = preview_of("lower_540_0");
    let job = store
        .enqueue_tiktok(
            &preview,
            "Sunset walk.mp4",
            None,
            MAIN_QUEUE,
            "tiktok-request",
        )
        .unwrap();
    let download = store.get(&job.id).unwrap().unwrap().hls.unwrap();
    assert_eq!(download.version, 7);
    assert_eq!(download.tiktok.as_ref(), Some(&preview.selection));
    download.validate().unwrap();
    let mut resized = Download::new(preview.playlist.clone());
    resized.version = 7;
    resized.tiktok = Some(Selection {
        size: 1,
        ..preview.selection.clone()
    });
    assert!(resized.validate().is_err());
    // A single MP4 file is only accepted from a reviewed TikTok choice.
    assert!(Download::new(preview.playlist).validate().is_err());
}

#[test]
fn page_cookies_and_referer_go_only_to_tiktok_servers() {
    let site = site_headers("ttwid=a; tt_chain_token=b").unwrap();
    assert_eq!(site.headers[header::REFERER], "https://www.tiktok.com/");
    assert_eq!(site.headers[header::COOKIE], "ttwid=a; tt_chain_token=b");
    for (url, sent) in [
        ("https://v16-webapp-prime.tiktok.com/video/tos/x", true),
        ("https://tiktok.com/x", true),
        ("http://v16-webapp-prime.tiktok.com/video/tos/x", false),
        ("https://eviltiktok.com/x", false),
        ("https://tiktok.com.evil.test/x", false),
        ("https://cdn.example.test/x", false),
    ] {
        assert_eq!(site.applies(&Url::parse(url).unwrap()), sent, "{url}");
    }
    assert!(
        !site_headers("")
            .unwrap()
            .headers
            .contains_key(header::COOKIE)
    );
}

#[tokio::test]
async fn cancelled_tiktok_lookup_stops_before_any_network_request() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = inspect_via(
        "https://www.tiktok.com/@jannanickofficial/video/7696495296011930887",
        &cancel,
        crate::proxy::Route::direct(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("cancelled"));
}

#[tokio::test]
async fn direct_download_of_a_tiktok_page_fails_before_saving_html() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let job = store
        .new_download(
            &source_url("jannanickofficial", ID).unwrap(),
            Some("video.mp4"),
            None,
            "direct-tiktok-fixture".into(),
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

const LIVE: &str = "https://www.tiktok.com/@jannanickofficial/video/7696495296011930887";

/// Live check against TikTok: cargo test --lib tiktok::tests::live -- --ignored
#[tokio::test]
#[ignore = "contacts TikTok"]
async fn live_lookup_lists_single_file_qualities() {
    let video = inspect_via(
        LIVE,
        &CancellationToken::new(),
        crate::proxy::Route::direct(),
    )
    .await
    .unwrap();
    assert!(!video.inspection.variants.is_empty());
    let preview = preview(&video.inspection, 0).unwrap();
    println!(
        "{} | {} | {} bytes in {} parts",
        video.title,
        preview.playlist.quality,
        preview.selection.size,
        preview.playlist.parts.len()
    );
}

/// Live end to end, with the fresh cookies and Referer the video servers require.
#[tokio::test]
#[ignore = "contacts TikTok and downloads a few MB"]
async fn live_download_saves_the_exact_mp4_file() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let video = inspect_via(
        LIVE,
        &CancellationToken::new(),
        crate::proxy::Route::direct(),
    )
    .await
    .unwrap();
    let smallest = (0..video.inspection.variants.len())
        .min_by_key(|&i| preview(&video.inspection, i).unwrap().selection.size)
        .unwrap();
    let preview = preview(&video.inspection, smallest).unwrap();
    let job = store
        .enqueue_tiktok(&preview, "tiktok-live.mp4", None, MAIN_QUEUE, "tiktok-live")
        .unwrap();
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
    assert_eq!(
        saved.status,
        crate::model::Status::Complete,
        "{:?}",
        saved.error
    );
    let bytes = std::fs::read(saved.saved_path.as_ref().unwrap()).unwrap();
    assert_eq!(bytes.len() as u64, preview.selection.size);
    assert_eq!(&bytes[4..8], b"ftyp");
    assert!(bytes.windows(4).any(|w| w == b"vide") && bytes.windows(4).any(|w| w == b"soun"));
    println!(
        "{} bytes at {}",
        bytes.len(),
        saved.saved_path.unwrap().display()
    );
}

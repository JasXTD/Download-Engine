use super::*;
use crate::{paths::AppPaths, queue::MAIN_QUEUE, store::Store};
use serde_json::json;

#[test]
fn twitch_links_identify_one_video_without_accepting_lookalikes() {
    for value in [
        "https://www.twitch.tv/videos/2902385151",
        "https://twitch.tv/videos/2902385151/",
        "https://m.twitch.tv/videos/2902385151?t=1h2m3s",
        "http://www.twitch.tv/videos/1",
    ] {
        assert!(is_url(value) && crate::hls::is_site_video(value), "{value}");
    }
    assert_eq!(
        video_id("https://m.twitch.tv/videos/2902385151?t=1h2m3s").unwrap(),
        "2902385151"
    );
    for value in [
        "https://www.twitch.tv/twitch",
        "https://www.twitch.tv/videos/",
        "https://www.twitch.tv/videos/abc",
        "https://www.twitch.tv/videos/0123",
        "https://www.twitch.tv/videos/123456789012345678901",
        "https://www.twitch.tv/videos/2902385151/extra",
        "https://www.twitch.tv/twitch/clip/SomeClip",
        "https://clips.twitch.tv/SomeClip",
        "https://www.twitch.tv:8443/videos/2902385151",
        "https://twitch.tv.evil.test/videos/2902385151",
        "file:///videos/2902385151",
    ] {
        assert!(!is_url(value), "{value}");
    }
    assert_eq!(
        source_url("2902385151").unwrap(),
        "https://www.twitch.tv/videos/2902385151"
    );
    assert!(source_url("1/../2").is_err());
}

fn reply(video: serde_json::Value) -> serde_json::Value {
    json!({"data": {
        "video": video,
        "videoPlaybackAccessToken": {
            "value": r#"{"authorization":{"forbidden":false},"vod_id":2902385151,"expires":1}"#,
            "signature": "0123456789abcdef0123456789abcdef01234567",
        },
    }})
}

#[test]
fn graphql_reply_gives_the_title_and_token_or_a_clear_error() {
    let id = "2902385151";
    let found = access(
        &reply(
            json!({"id": id, "title": " Stream\u{7}night ", "owner": {"displayName": "Twitch"}}),
        ),
        id,
    )
    .unwrap();
    assert_eq!(found.title, "Twitch - Streamnight");
    assert_eq!(found.signature.len(), 40);
    // The token is a JSON string; it must survive URL encoding exactly.
    let usher = usher_url(id, &found).unwrap();
    assert_eq!(usher.host_str(), Some("usher.ttvnw.net"));
    assert_eq!(usher.path(), "/vod/2902385151.m3u8");
    let token = usher
        .query_pairs()
        .find(|(k, _)| k == "token")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(token, found.token);
    assert!(!usher.as_str().contains('{') && !usher.as_str().contains('"'));
    let untitled = access(&reply(json!({"id": id, "title": "", "owner": null})), id).unwrap();
    assert_eq!(untitled.title, "Twitch video 2902385151");

    let missing = access(&reply(serde_json::Value::Null), id).err().unwrap();
    assert!(missing.to_string().contains("does not exist"));
    let other = access(&reply(json!({"id": "1", "title": "x"})), id)
        .err()
        .unwrap();
    assert!(other.to_string().contains("different video"));
    let refused = access(&json!({"errors": [{"message": "service timeout"}]}), id)
        .err()
        .unwrap();
    assert!(refused.to_string().contains("service timeout"));
    let mut forged = reply(json!({"id": id, "title": "x"}));
    forged["data"]["videoPlaybackAccessToken"]["signature"] = json!("not hex&x=1");
    assert!(access(&forged, id).is_err());
    let mut tokenless = reply(json!({"id": id, "title": "x"}));
    tokenless["data"]["videoPlaybackAccessToken"] = serde_json::Value::Null;
    assert!(access(&tokenless, id).is_err());
}

/// The shape of a real usher reply (hosts and the viewer's details replaced).
const MASTER: &str = r#"#EXTM3U
#EXT-X-TWITCH-INFO:ORIGIN="s3",B="false",REGION="AS",USER-IP="192.0.2.1",SERVING-ID="x",CLUSTER="cloudfront_vod",USER-COUNTRY="MY",MANIFEST-CLUSTER="cloudfront_vod"
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="chunked",NAME="1080p60",AUTOSELECT=NO,DEFAULT=NO
#EXT-X-STREAM-INF:BANDWIDTH=7707400,CODECS="avc1.4D402A,mp4a.40.2",RESOLUTION=1920x1080,VIDEO="chunked",FRAME-RATE=60.000
https://vod.example.test/abc_twitch_1/chunked/index-dvr.m3u8
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="720p60",NAME="720p60",AUTOSELECT=YES,DEFAULT=YES
#EXT-X-STREAM-INF:BANDWIDTH=3258523,CODECS="avc1.4D4020,mp4a.40.2",RESOLUTION=1280x720,VIDEO="720p60",FRAME-RATE=60.000
https://vod.example.test/abc_twitch_1/720p60/index-dvr.m3u8
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="160p30",NAME="160p",AUTOSELECT=YES,DEFAULT=YES
#EXT-X-STREAM-INF:BANDWIDTH=254653,CODECS="avc1.4D400C,mp4a.40.2",RESOLUTION=284x160,VIDEO="160p30",FRAME-RATE=30.000
https://vod.example.test/abc_twitch_1/160p30/index-dvr.m3u8
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="audio_only",NAME="Audio Only",AUTOSELECT=NO,DEFAULT=NO
#EXT-X-STREAM-INF:BANDWIDTH=204913,CODECS="mp4a.40.2",VIDEO="audio_only"
https://vod.example.test/abc_twitch_1/audio_only/index-dvr.m3u8
"#;

#[test]
fn usher_master_playlist_lists_every_quality_under_the_page_address() {
    let source = source_url("2902385151").unwrap();
    let inspection = crate::hls::inspection_from(
        source.clone(),
        "https://usher.ttvnw.net/vod/2902385151.m3u8?sig=a&token=b",
        MASTER,
    )
    .unwrap();
    // The page, not the token-bearing usher address, is what a download keeps.
    assert_eq!(inspection.source, source);
    assert!(inspection.playlist.is_none() && inspection.dash.is_none());
    let urls: Vec<_> = inspection
        .variants
        .iter()
        .map(|v| v.url.rsplit('/').nth(1).unwrap())
        .collect();
    assert_eq!(urls.len(), 4);
    for quality in ["chunked", "720p60", "160p30", "audio_only"] {
        assert!(urls.contains(&quality), "{quality}");
    }
    assert!(
        inspection
            .variants
            .iter()
            .any(|v| v.resolution == Some((1920, 1080)) && v.unsupported.is_none())
    );
}

#[tokio::test]
async fn cancelled_twitch_lookup_stops_before_any_network_request() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = inspect_via(
        "https://www.twitch.tv/videos/2902385151",
        &cancel,
        crate::proxy::Route::direct(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("cancelled"));
}

#[tokio::test]
async fn direct_download_of_a_twitch_page_fails_before_saving_html() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let job = store
        .new_download(
            &source_url("2902385151").unwrap(),
            Some("video.mp4"),
            None,
            "direct-twitch-fixture".into(),
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

/// A public video of Twitch's own channel, so the live tests need no fixed ID:
/// the newest past broadcast (fMP4) or the shortest upload (MPEG-TS).
async fn channel_video(kind: &str, shortest: bool) -> String {
    let query = format!(
        r#"query {{ user(login: "twitch") {{ videos(first: 30, type: {kind}) {{ edges {{ node {{ id lengthSeconds }} }} }} }} }}"#
    );
    let reply: serde_json::Value = serde_json::from_str(
        &reqwest::Client::new()
            .post("https://gql.twitch.tv/gql")
            .header("Client-ID", "kimne78kx3ncx6brgo4mv6wki5h1ko")
            .body(json!({ "query": query }).to_string())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    let nodes = reply
        .pointer("/data/user/videos/edges")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    let node = if shortest {
        nodes
            .iter()
            .min_by_key(|e| e["node"]["lengthSeconds"].as_u64().unwrap_or(u64::MAX))
            .unwrap()
    } else {
        &nodes[0]
    };
    source_url(node["node"]["id"].as_str().unwrap()).unwrap()
}

fn smallest_video_quality(inspection: &Inspection) -> usize {
    inspection
        .variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.resolution.is_some())
        .min_by_key(|(_, v)| v.bandwidth)
        .unwrap()
        .0
}

/// Live check against Twitch: cargo test --lib twitch::tests::live -- --ignored
#[tokio::test]
#[ignore = "contacts Twitch"]
async fn live_broadcast_lists_finished_fmp4_qualities() {
    let cancel = CancellationToken::new();
    let route = crate::proxy::Route::direct();
    let page = channel_video("ARCHIVE", false).await;
    let video = inspect_via(&page, &cancel, route.clone()).await.unwrap();
    assert!(!video.title.is_empty());
    assert_eq!(video.inspection.source, page);
    assert!(video.inspection.variants.len() > 1);
    let quality = smallest_video_quality(&video.inspection);
    let preview = crate::hls::preview_with_audio_via(&video.inspection, quality, 0, &cancel, route)
        .await
        .unwrap();
    assert_eq!(preview.source, page);
    assert!(preview.parts[0].initialization && preview.parts.len() > 2);
    // The quality's own files need no token: the initialization segment opens.
    let init = reqwest::get(&preview.parts[0].url)
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&init[4..8], b"ftyp");
    println!(
        "{} | {} | {} parts | .{}",
        video.title,
        preview.quality,
        preview.parts.len(),
        preview.output_extension()
    );
}

/// Live end to end with the channel's shortest upload (a few seconds):
/// cargo test --lib twitch::tests::live -- --ignored
#[tokio::test]
#[ignore = "contacts Twitch and downloads under 10 MB"]
async fn live_download_of_a_short_upload_completes() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let cancel = CancellationToken::new();
    let route = crate::proxy::Route::direct();
    let video = inspect_via(&channel_video("UPLOAD", true).await, &cancel, route.clone())
        .await
        .unwrap();
    let quality = smallest_video_quality(&video.inspection);
    let preview = crate::hls::preview_with_audio_via(&video.inspection, quality, 0, &cancel, route)
        .await
        .unwrap();
    assert!(preview.duration_ms <= 120_000, "{}", preview.duration_ms);
    let name = format!("twitch-live.{}", preview.output_extension());
    let job = store
        .enqueue_hls(&preview, &name, None, MAIN_QUEUE, "twitch-live")
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
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size > 10_000);
    println!("{} | {size} bytes at {}", video.title, path.display());
}

use super::*;
use parser::{Parsed, parse};

const URL: &str = "https://example.com/video/index.m3u8";
fn media(body: &str) -> String {
    format!("#EXTM3U\n#EXT-X-TARGETDURATION:10\n{body}\n#EXT-X-ENDLIST\n")
}

#[test]
fn qualities_handle_quoted_codecs_resolution_and_external_audio() {
    let text = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"sound\",NAME=\"English\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,AUDIO=\"sound\"\nhigh.m3u8\n";
    let Parsed::Master(variants) = parse(URL, text).unwrap() else {
        panic!()
    };
    assert_eq!(variants.len(), 2);
    assert!(variants[0].unsupported.is_none());
    assert_eq!(variants[0].audio.len(), 1);
    assert_eq!(variants[0].audio[0].name, "English");
    assert_eq!(
        variants[0].audio[0].url.as_deref(),
        Some("https://example.com/video/audio.m3u8")
    );
    assert_eq!(variants[1].url, "https://example.com/video/low.m3u8");
    assert!(variants[1].label().contains("640 × 360"));
    assert!(variants[1].label().contains("avc1.4d401e,mp4a.40.2"));
}

#[test]
fn audio_defaults_languages_and_group_errors_are_explicit() {
    let prefix = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"English\",LANGUAGE=\"en\",URI=\"en.m3u8\"\n";
    let suffix = "\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,AUDIO=\"a\"\nv.m3u8\n";
    let second = "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Spanish\",LANGUAGE=\"es\",DEFAULT=YES,URI=\"es.m3u8\"";
    let Parsed::Master(choices) = parse(URL, &format!("{prefix}{second}{suffix}")).unwrap() else {
        panic!()
    };
    assert_eq!(choices[0].default_audio(), 1);
    assert_eq!(choices[0].audio[1].label(), "Spanish · es");
    for invalid in [
        second.replace("Spanish", "English"),
        second.replace("DEFAULT=YES", "DEFAULT=MAYBE"),
        second.replace("es.m3u8", "file:///C:/secret"),
        second.replace("es.m3u8", "http://example.com/audio"),
        second.replace(
            "LANGUAGE=\"es\"",
            &format!("LANGUAGE=\"{}\"", "x".repeat(65)),
        ),
    ] {
        assert!(parse(URL, &format!("{prefix}{invalid}{suffix}")).is_err());
    }
    assert!(
        parse(
            URL,
            &format!(
                "{}{second}{suffix}",
                prefix.replace("LANGUAGE=\"en\"", "DEFAULT=YES")
            )
        )
        .is_err()
    );
    assert!(
        parse(
            URL,
            &format!(
                "{prefix}{}",
                suffix.replace("AUDIO=\"a\"", "AUDIO=\"missing\"")
            )
        )
        .is_err()
    );
}

#[test]
fn old_hls_jobs_remain_compatible_and_audio_progress_is_bounded_across_both_tracks() {
    let Parsed::Media(playlist) = parse(URL, &media("#EXTINF:5,\na.ts")).unwrap() else {
        panic!()
    };
    let mut playlist = *playlist;
    let old = Download::new(playlist.clone());
    let json = serde_json::to_string(&old).unwrap();
    assert!(!json.contains("audio"));
    serde_json::from_str::<Download>(&json)
        .unwrap()
        .validate()
        .unwrap();
    playlist.audio = Some(AudioTrack {
        name: "English".into(),
        language: Some("en".into()),
        url: URL.into(),
        duration_ms: 5000,
        sequence: 41,
        format: Format::TransportStream,
        parts: playlist.parts.clone(),
    });
    let mut download = Download::new(playlist);
    assert_eq!(download.version, 2);
    assert_eq!(download.playlist.part_count(), 2);
    assert_eq!(download.playlist.media_count(), 2);
    download.completed = vec![
        CompletedPart {
            size: 188,
            sha256: "ab".repeat(32)
        };
        2
    ];
    download.validate().unwrap();
    let restored: Download =
        serde_json::from_str(&serde_json::to_string(&download).unwrap()).unwrap();
    assert_eq!(restored.playlist, download.playlist);
    download.version = 1;
    assert!(download.validate().is_err());
    download.version = 2;
    download.completed.push(download.completed[0].clone());
    assert!(download.validate().is_err());
    download.completed.clear();
    download.playlist.audio.as_mut().unwrap().parts[0].url = "file:///C:/private".into();
    assert!(download.validate().is_err());
}

#[test]
fn all_encryption_is_rejected_even_after_clear_segments_or_in_session_keys() {
    for key in [
        "#EXT-X-KEY:METHOD=AES-128,URI=\"key\"",
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\"",
        "#EXT-X-SESSION-KEY:METHOD=AES-128,URI=\"key\"",
        "#EXT-X-KEY:METHOD=NONE,URI=\"key\"",
    ] {
        assert!(
            parse(
                URL,
                &media(&format!("#EXTINF:5,\na.ts\n{key}\n#EXTINF:5,\nb.ts"))
            )
            .is_err(),
            "{key}"
        );
    }
    assert!(parse(URL, &media("#EXT-X-KEY:METHOD=NONE\n#EXTINF:5,\na.ts")).is_ok());
}

#[test]
fn fmp4_init_and_implicit_ranges_have_exact_order_and_bounds() {
    let Parsed::Media(playlist) = parse(URL,&media("#EXT-X-MAP:URI=\"all.mp4\",BYTERANGE=\"100@0\"\n#EXTINF:4.5,\n#EXT-X-BYTERANGE:200@100\nall.mp4\n#EXTINF:5.5,\n#EXT-X-BYTERANGE:300\nall.mp4")).unwrap() else { panic!() };
    assert_eq!(playlist.format, Format::FragmentedMp4);
    assert_eq!(playlist.duration_ms, 10_000);
    assert_eq!(playlist.media_count(), 2);
    assert!(playlist.parts[0].initialization);
    assert_eq!(
        playlist.parts[2].range,
        Some(Range {
            start: 300,
            length: 300
        })
    );
}

#[test]
fn invalid_live_unsafe_and_unsupported_playlists_fail_before_any_media_fetch() {
    for body in [
        "#EXTINF:5,\nfile:///etc/passwd",
        "#EXTINF:5,\nhttp://example.com/a.ts",
        "#EXTINF:5,\nhttps://user:password@example.com/a.ts",
        "#EXTINF:NaN,\na.ts",
        "#EXTINF:-1,\na.ts",
        "#EXTINF:5,\n#EXT-X-BYTERANGE:10\na.ts",
        "#EXTINF:5,\n#EXT-X-BYTERANGE:10@18446744073709551615\na.ts",
        "#EXTINF:5,\na.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:5,\nb.ts",
        "#EXTINF:5,\na.ts\n#EXT-X-GAP",
        "#EXTINF:5,\n{$token}/a.ts",
        "#EXT-X-MAP:URI=\"one.mp4\"\n#EXTINF:5,\na.m4s\n#EXT-X-MAP:URI=\"two.mp4\"\n#EXTINF:5,\nb.m4s",
    ] {
        assert!(parse(URL, &media(body)).is_err(), "{body}");
    }
    assert!(parse(URL, "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:5,\na.ts\n").is_err());
    assert!(parse(URL, "<html>Sign in</html>").is_err());
    assert!(parse(URL, &format!("#EXTM3U\n{}", "a".repeat(MAX_PLAYLIST_BYTES))).is_err());
}

#[test]
fn browser_request_is_a_durable_preview_only_and_has_a_bounded_inbox() {
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::paths::AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    };
    let store = crate::store::Store::open(paths).unwrap();
    let id = format!("hls:{}", uuid::Uuid::new_v4());
    store.request_hls_preview(&id, URL, None).unwrap();
    store.request_hls_preview(&id, URL, None).unwrap();
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        store.next_hls_preview().unwrap(),
        Some((id.clone(), URL.into()))
    );
    assert!(
        store
            .request_hls_preview(&id, "https://example.com/other", None)
            .is_err()
    );
    store.acknowledge_hls_preview(&id).unwrap();
    store.request_hls_preview(&id, URL, None).unwrap();
    assert!(store.next_hls_preview().unwrap().is_none());
    for _ in 0..10 {
        store
            .request_hls_preview(&format!("hls:{}", uuid::Uuid::new_v4()), URL, None)
            .unwrap();
    }
    assert!(
        store
            .request_hls_preview(&format!("hls:{}", uuid::Uuid::new_v4()), URL, None)
            .is_err()
    );
    assert!(
        store
            .request_hls_preview("hls:../escape", URL, None)
            .is_err()
    );
}

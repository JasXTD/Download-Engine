use super::*;

fn mpd(body: &str) -> String {
    format!(
        r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT6S"><Period>{body}</Period></MPD>"#
    )
}
fn video(template: &str) -> String {
    format!(
        r#"<AdaptationSet mimeType="video/mp4" codecs="avc1.64001e">{template}<Representation id="v" bandwidth="1000000" width="640" height="360"/></AdaptationSet>"#
    )
}
fn parse_ok(text: &str) -> Manifest {
    parse(
        "https://test.example/main.mpd",
        "https://test.example/main.mpd",
        text,
    )
    .unwrap()
    .dash
    .unwrap()
}

#[tokio::test]
async fn reader_security_public_dash_selection_can_be_rechecked_after_serialization() {
    let url = "https://test.example/main.mpd";
    let text = mpd(&video(
        r#"<SegmentTemplate duration="2" initialization="init.mp4" media="part-$Number$.m4s"/>"#,
    ));
    let mut inspection = parse(url, url, &text).unwrap();
    inspection.public_only = true;
    let cancel = CancellationToken::new();
    let preview = crate::hls::preview(&inspection, 0, &cancel).await.unwrap();
    assert!(preview.public_only);
    let restored: Playlist =
        serde_json::from_str(&serde_json::to_string(&preview).unwrap()).unwrap();
    let mut network = Network::new(url)
        .await
        .unwrap()
        .with_public_only(true)
        .unwrap();
    inspection
        .dash
        .unwrap()
        .recheck(&restored, &mut network, &cancel)
        .await
        .unwrap();
}

#[tokio::test]
async fn numbered_templates_inherit_and_preview_without_network() {
    let text = mpd(&format!(
        r#"{}<AdaptationSet mimeType="audio/mp4" lang="en"><Role value="main"/><SegmentTemplate timescale="10" duration="20" initialization="audio-$RepresentationID$.mp4" media="a-$Number$.m4s"/><Representation id="en" bandwidth="128000"/></AdaptationSet>"#,
        video(
            r#"<BaseURL>media/</BaseURL><SegmentTemplate timescale="10" duration="20" startNumber="7" initialization="init-$RepresentationID$.mp4" media="$Bandwidth$/v-$$-$Number%05d$.m4s"/>"#
        )
    ));
    let manifest = parse_ok(&text);
    let preview = manifest
        .preview(
            "https://test.example/main.mpd",
            None,
            0,
            0,
            &CancellationToken::new(),
            crate::proxy::Route::direct(),
        )
        .await
        .unwrap();
    assert_eq!(
        preview.parts[1].url,
        "https://test.example/media/1000000/v-$-00007.m4s"
    );
    assert_eq!(
        preview.parts[3].url,
        "https://test.example/media/1000000/v-$-00009.m4s"
    );
    assert_eq!(
        preview.audio.as_ref().unwrap().language.as_deref(),
        Some("en")
    );
    assert_eq!(preview.media_count(), 6);
    assert_eq!(crate::hls::Download::new(preview.clone()).version, 3);
    manifest
        .recheck(
            &preview,
            &mut Network::new(&preview.source).await.unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn time_templates_bound_negative_repeats_and_preserve_offsets() {
    let text = mpd(&video(
        r#"<SegmentTemplate timescale="1000" presentationTimeOffset="10000" initialization="init.mp4" media="$Time$.m4s"><SegmentTimeline><S t="10000" d="2000" r="-1"/></SegmentTimeline></SegmentTemplate>"#,
    ));
    let preview = parse_ok(&text)
        .preview(
            "https://test.example/main.mpd",
            None,
            0,
            0,
            &CancellationToken::new(),
            crate::proxy::Route::direct(),
        )
        .await
        .unwrap();
    assert_eq!(preview.parts.len(), 4);
    assert!(preview.parts[3].url.ends_with("14000.m4s"));
    assert_eq!(preview.dash.unwrap().video_offset_us, 10_000_000);
}
#[tokio::test]
async fn segment_lists_support_ranges_and_escaped_urls() {
    let text = mpd(&video(
        r#"<BaseURL>media.mp4?a=1&amp;b=2</BaseURL><SegmentList duration="2"><Initialization range="0-999"/><SegmentURL mediaRange="1000-1999"/><SegmentURL mediaRange="2000-2999"/><SegmentURL mediaRange="3000-3999"/></SegmentList>"#,
    ));
    let preview = parse_ok(&text)
        .preview(
            "https://test.example/main.mpd",
            None,
            0,
            0,
            &CancellationToken::new(),
            crate::proxy::Route::direct(),
        )
        .await
        .unwrap();
    assert!(preview.parts[1].url.ends_with("media.mp4?a=1&b=2"));
    assert_eq!(preview.parts[1].range.as_ref().unwrap().start, 1000);
}

#[test]
fn segment_base_inherits_ranges_and_rejects_unbounded_or_external_indexes() {
    let valid = mpd(&video(
        r#"<BaseURL>single.mp4</BaseURL><SegmentBase timescale="1000" presentationTimeOffset="5000" indexRange="100-155" indexRangeExact="true"><Initialization range="0-99"/></SegmentBase>"#,
    ));
    let manifest = parse_ok(&valid);
    let index = manifest.videos[0].index.as_ref().unwrap();
    assert_eq!(index.url, "https://test.example/single.mp4");
    assert_eq!(
        index.range,
        crate::hls::Range {
            start: 100,
            length: 56
        }
    );
    assert_eq!(index.offset_us, 5_000_000);
    for invalid in [
        valid.replace("100-155", "100-2000000"),
        valid.replace("0-99", "0-100"),
        valid.replace("indexRange=\"100-155\"", ""),
        valid.replace(
            "<Initialization",
            "<RepresentationIndex sourceURL=\"other.mp4\"/><Initialization",
        ),
        valid.replace("range=\"0-99\"", "range=\"0-99\" sourceURL=\"other.mp4\""),
        valid.replace("indexRangeExact=\"true\"", "indexRangeExact=\"maybe\""),
    ] {
        assert!(
            parse(
                "https://test.example/main.mpd",
                "https://test.example/main.mpd",
                &invalid
            )
            .is_err()
        );
    }
}
#[test]
fn unsafe_dynamic_encrypted_and_unbounded_manifests_are_rejected() {
    let valid = mpd(&video(
        r#"<SegmentTemplate duration="2" initialization="init.mp4" media="$Number$.m4s"/>"#,
    ));
    for bad in [valid.replace("static","dynamic"), valid.replace("<Representation", "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\"/><Representation"),
        format!("<!DOCTYPE MPD [<!ENTITY x SYSTEM 'file:///secret'>]>{valid}"), valid.replace("duration=\"2\"","duration=\"0\""),
        valid.replace("init.mp4","file:///C:/secret"), valid.replace("init.mp4","http://test.example/init.mp4"),
        valid.replace("$Number$", "$Unknown$"), valid.replace("<Period>","<Period xmlns:xlink=\"http://www.w3.org/1999/xlink\" xlink:href=\"https://example.com\">"),
        valid.replace("<Period>","<Period/><Period>"), valid.replace("PT6S","PT999999999999999S"), valid.replace("PT6S","PTNaNS"),
        valid.replace("duration=\"2\"","timescale=\"1000000000\" duration=\"1\"") ] {
        assert!(parse("https://test.example/main.mpd","https://test.example/main.mpd",&bad).is_err(),"accepted {bad}");
    }
}

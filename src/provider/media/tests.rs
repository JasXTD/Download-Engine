use super::*;
use crate::provider::runtime::{Capabilities, Runtime};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;

pub(super) fn plan() -> Plan {
    serde_json::from_value(json!({"kind":"media","media_api":1,"content_id":"fixture-v1","container":"mkv","tracks":[
        {"id":"video","kind":"video","format":"mp4","segments":[{"id":"init","url":"https://example.invalid/init?secret=one","size":16,"processing":"lua"}]},
        {"id":"english","kind":"audio","format":"aac","language":"en","default":true,"segments":[{"id":"one","url":"https://example.invalid/audio","size":32}]}
    ]})).unwrap()
}
#[test]
fn saved_identity_excludes_urls_but_binds_segment_order_format_and_processing() {
    let original = plan();
    original.validate(true).unwrap();
    let content = original.content();
    content.validate(false).unwrap();
    let saved = serde_json::to_string(&content).unwrap();
    assert!(
        !saved.contains("https:") && !saved.contains("secret=one") && !saved.contains("\"url\"")
    );
    let mut renewed = original.clone();
    renewed.tracks[0].segments[0].url.push_str("renewed");
    assert_eq!(renewed.content(), content);
    renewed.tracks.swap(0, 1);
    assert_ne!(renewed.content(), content);
    assert!(original.validate(false).is_err());
    assert!(content.validate(true).is_err());
}
#[test]
fn media_plans_reject_unbounded_ambiguous_or_unsafe_inputs() {
    let value = serde_json::to_value(plan()).unwrap();
    for (pointer, bad) in [
        ("/media_api", json!(2)),
        ("/tracks/0/id", json!("../escape")),
        ("/tracks/0/segments/0/size", json!(0)),
        (
            "/tracks/0/segments/0/size",
            json!(crate::hls::MAX_PART_BYTES + 1),
        ),
        ("/tracks/0/segments/0/url", json!("file:///private")),
        (
            "/tracks/0/segments/0/url",
            json!("https://user:secret@example.invalid/"),
        ),
        ("/tracks/0/kind", json!("subtitle")),
        ("/tracks/1/language", json!("en\n-file")),
    ] {
        let mut changed = value.clone();
        *changed.pointer_mut(pointer).unwrap() = bad;
        assert!(
            serde_json::from_value::<Plan>(changed)
                .unwrap()
                .validate(true)
                .is_err(),
            "{pointer}"
        );
    }
    let mut extra = value;
    extra["ffmpeg_args"] = json!(["-i", "https://example.invalid/"]);
    assert!(serde_json::from_value::<Plan>(extra).is_err());
    let mut large = plan();
    large.tracks[0].segments = vec![large.tracks[0].segments[0].clone(); MAX_SEGMENTS + 1];
    assert!(large.validate(true).is_err());
    let mut duplicate = plan();
    duplicate.tracks.push(duplicate.tracks[0].clone());
    assert!(duplicate.validate(true).is_err());
}
#[test]
fn media_processor_uses_offsets_and_cannot_fetch_or_change_chunk_length() {
    let source = r#"
        assert(xd.media_api == 1 and xd.supports('xd.media_plan'))
        assert(xd.fetch == nil and not xd.supports('xd.fetch'))
        return {
            resolve=function() end, refresh=function() end,
            transform=function(context, bytes, offset)
                assert(context.track_id == 'video' and context.segment_id == 'part2')
                assert(context.url == nil and context.path == nil)
                return xd.aes_ctr('603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4', '000102030405060708090a0b0c0d0e0f', bytes, context.track_offset + offset)
            end,
            finish=function(plan,result) assert(result.stage=='before_mux' and plan.tracks[1].segments[1].url==nil); return true end
        }
    "#;
    let runtime =
        Runtime::new(source, Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
    let context = Chunk {
        content_id: "fixture".into(),
        track_id: "video".into(),
        kind: TrackKind::Video,
        segment_id: "part2".into(),
        segment_index: 1,
        track_offset: 17,
        size: 40,
    };
    let input = vec![42; 23];
    let request = json!({"op":"media_transform","context":context,"offset":3,"bytes":STANDARD.encode(&input)});
    let encrypted = runtime.handle(request.clone()).unwrap();
    let mut decrypt = request;
    decrypt["bytes"] = encrypted;
    let recovered = runtime.handle(decrypt).unwrap();
    assert_eq!(STANDARD.decode(recovered.as_str().unwrap()).unwrap(), input);
    assert_eq!(
        runtime
            .handle(json!({"op":"media_finish","plan":plan().content()}))
            .unwrap(),
        true
    );
    assert!(runtime.handle(json!({"op":"media_transform","context":context,"offset":39,"bytes":STANDARD.encode([1,2])})).is_err());
    let bad = Runtime::new("return {resolve=function() end,refresh=function() end,transform=function() return 'extra' end,finish=function() return true end}", Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
    assert!(bad.handle(json!({"op":"media_transform","context":context,"offset":0,"bytes":STANDARD.encode([1])})).is_err());
}

#[test]
fn media_checkpoint_upgrades_the_profile_and_blocks_older_open_writers() {
    use crate::{
        paths::AppPaths,
        script_automation::{Language, Script, Trigger},
        store::Store,
    };
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: root.path().join("data"),
        downloads: root.path().join("downloads"),
    })
    .unwrap();
    let old = store.connect().unwrap();
    for name in ["xdownloader_writer_version", "xdownloader_data_version"] {
        old.create_scalar_function(
            name,
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
            |_| Ok(6_i64),
        )
        .unwrap();
    }
    let script = store
        .save_script(&Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: "Media".into(),
            language: Language::Lua,
            trigger: Trigger::Provider,
            body: crate::provider::EXAMPLE.into(),
            enabled: false,
            timeout_seconds: 5,
            change_files: false,
            extension: None,
        })
        .unwrap();
    let mut job = store
        .add_provider_download(
            &script.id,
            script.revision,
            "https://example.invalid/",
            Some("sample.mkv"),
            "main",
            "media-profile",
        )
        .unwrap();
    job.provider.as_mut().unwrap().media = Some(plan().content());
    store.save(&job).unwrap();
    assert_eq!(
        store
            .connect()
            .unwrap()
            .query_row("SELECT data_version FROM profile_format", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert!(
        old.execute(
            "UPDATE jobs SET data=json_remove(data,'$.provider.media') WHERE id=?1",
            [&job.id]
        )
        .is_err()
    );
    let saved = store.get(&job.id).unwrap().unwrap();
    assert_eq!(saved.provider.unwrap().media.unwrap(), plan().content());
}

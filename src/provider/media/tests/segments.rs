use super::*;

const SOURCE: &str = r#"
    assert(xd.segment_api==1 and xd.supports('media.segment_processing'))
    assert(xd.http_session==nil and not xd.supports('xd.http_session'))
    local prepared={}
    return {
        resolve=function()end,refresh=function()end,transform=function(_,b)return b end,
        prepare_track=function(c,b) assert(c.url==nil and b=='init-data');prepared[c.track_id]=true;return true end,
        transform_segment=function(c,b) assert(prepared[c.track_id]);return b:sub(5)..'rewritten' end,
        finish=function(_,r) assert(r.stage=='before_mux');return true end,
    }
"#;
fn context(size: usize) -> Chunk {
    Chunk {
        content_id: "fixture".into(),
        track_id: "video".into(),
        kind: TrackKind::Video,
        segment_id: "part".into(),
        segment_index: 0,
        track_offset: 0,
        output_offset: 0,
        size: size as u64,
    }
}
fn run(runtime: &Runtime, bytes: &[u8], prepare: bool) -> Vec<u8> {
    runtime
        .handle(json!({"op":"segment_begin","context":context(bytes.len()),"prepare":prepare}))
        .unwrap();
    for bytes in bytes.chunks(crate::provider::CHUNK_BYTES) {
        runtime
            .handle(json!({"op":"segment_feed","bytes":STANDARD.encode(bytes)}))
            .unwrap();
    }
    let length = runtime
        .handle(json!({"op":"segment_run"}))
        .unwrap()
        .as_u64()
        .unwrap() as usize;
    let mut output = Vec::new();
    while output.len() < length {
        let value = runtime
            .handle(json!({"op":"segment_read","offset":output.len()}))
            .unwrap();
        output.extend(STANDARD.decode(value.as_str().unwrap()).unwrap());
    }
    output
}
#[test]
fn segment_callbacks_replay_initialization_and_rewrite_lengths_with_chunked_ipc() {
    let input = vec![b'a'; crate::provider::CHUNK_BYTES * 2 + 3];
    for _restart in 0..2 {
        let runtime =
            Runtime::new(SOURCE, Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
        assert!(run(&runtime, b"init-data", true).is_empty());
        let output = run(&runtime, &input, false);
        assert_eq!(output.len(), input.len() - 4 + 9);
        assert_eq!(&output[..input.len() - 4], &input[4..]);
        assert!(output.ends_with(b"rewritten"));
    }
}
#[test]
fn malformed_segment_messages_reset_buffers_and_reject_wrong_phases() {
    let runtime =
        Runtime::new(SOURCE, Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
    assert!(runtime.handle(json!({"op":"segment_run"})).is_err());
    assert!(
        runtime
            .handle(
                json!({"op":"segment_begin","context":context(MAX_SEGMENT_INPUT+1),"prepare":false})
            )
            .is_err()
    );
    runtime
        .handle(json!({"op":"segment_begin","context":context(2),"prepare":false}))
        .unwrap();
    assert!(
        runtime
            .handle(json!({"op":"segment_feed","bytes":STANDARD.encode(b"too long")}))
            .is_err()
    );
    run(&runtime, b"init-data", true);
    runtime
        .handle(json!({"op":"segment_begin","context":context(10),"prepare":false}))
        .unwrap();
    assert!(runtime.handle(json!({"op":"segment_run"})).is_err());
    assert!(run(&runtime, b"init-data", true).is_empty());
    let resolver=Runtime::new("return {resolve=function()end,refresh=function()end,transform=function()end,finish=function()end}",Capabilities::Resolver(Box::new(crate::script_automation::Fetcher::new(crate::proxy::Route::direct()).unwrap())),None).unwrap();
    assert!(
        resolver
            .handle(json!({"op":"segment_begin","context":context(1),"prepare":false}))
            .is_err()
    );
}
#[test]
fn plan_binds_initialization_and_limits_whole_segment_memory() {
    let mut source = plan();
    source.tracks[0].init_segment = Some("init".into());
    source.tracks[0].segments[0].processing = Processing::LuaSegment;
    assert!(source.uses_segment_callbacks());
    source.validate(true).unwrap();
    let saved = source.content();
    source.tracks[0].init_segment = Some("missing".into());
    assert!(source.validate(true).is_err());
    source.tracks[0].init_segment = None;
    source.tracks[0].segments[0].size = MAX_SEGMENT_INPUT as u64 + 1;
    assert!(source.validate(true).is_err());
    assert_ne!(saved, plan().content());
    assert!(!plan().uses_segment_callbacks());
}
#[test]
fn segment_callback_output_limits_and_initialization_acknowledgement_are_enforced() {
    for body in ["return ''", "return {}", "return string.rep('a',16777217)"] {
        let source = format!(
            "return {{resolve=function()end,refresh=function()end,transform=function()end,finish=function()return true end,transform_segment=function() {body} end}}"
        );
        let runtime =
            Runtime::new(&source, Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
        runtime
            .handle(json!({"op":"segment_begin","context":context(1),"prepare":false}))
            .unwrap();
        runtime
            .handle(json!({"op":"segment_feed","bytes":STANDARD.encode(b"a")}))
            .unwrap();
        assert!(runtime.handle(json!({"op":"segment_run"})).is_err());
    }
}

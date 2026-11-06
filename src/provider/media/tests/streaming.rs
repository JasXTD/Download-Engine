use super::*;
use crate::provider::CHUNK_BYTES;

#[test]
fn streaming_callback_cannot_exceed_the_remaining_job_allowance() {
    let source = r#"
        return {resolve=function()end,refresh=function()end,transform=function(_,b)return b end,finish=function()return true end,
        transform_stream=function(_,_,writer) writer:write('sixteen bytes!!!'); return true end}
    "#;
    let runtime =
        Runtime::new(source, Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
    let context = json!({"content_id":"budget","track_id":"v","kind":"video",
        "segment_id":"one","segment_index":0,"track_offset":0,"output_offset":0,"size":1});
    runtime
        .handle(json!({"op":"segment_begin","context":context,"prepare":false,"stream":true,"output_limit":5}))
        .unwrap();
    runtime
        .handle(json!({"op":"segment_feed","bytes":STANDARD.encode(b"a")}))
        .unwrap();
    let result = runtime.handle(json!({"op":"segment_run"}));
    assert!(
        result.is_err(),
        "stream writer ignored the remaining job allowance"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Stream output exceeds its limit")
    );
    // A failed callback expires both scratch files and returns to the idle state.
    runtime.handle(json!({"op":"segment_begin","context":context,"prepare":false,"stream":true,"output_limit":16})).unwrap();
    runtime
        .handle(json!({"op":"segment_feed","bytes":STANDARD.encode(b"a")}))
        .unwrap();
    assert_eq!(runtime.handle(json!({"op":"segment_run"})).unwrap(), 16);
    let output = runtime
        .handle(json!({"op":"segment_read","offset":0}))
        .unwrap();
    assert_eq!(
        STANDARD.decode(output.as_str().unwrap()).unwrap(),
        b"sixteen bytes!!!"
    );
}

#[test]
fn segment_output_allowances_are_required_bounded_and_applied_to_whole_segments() {
    let source = "return {resolve=function()end,refresh=function()end,transform=function(_,b)return b end,finish=function()return true end,transform_segment=function()return '12345' end}";
    let runtime =
        Runtime::new(source, Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
    let context = json!({"content_id":"budget","track_id":"v","kind":"video",
        "segment_id":"one","segment_index":0,"track_offset":0,"output_offset":0,"size":1});
    for limit in [json!(null), json!(0), json!(-1), json!(u64::MAX)] {
        assert!(runtime.handle(json!({"op":"segment_begin","context":context,"prepare":false,"stream":true,"output_limit":limit})).is_err());
    }
    runtime
        .handle(json!({"op":"segment_begin","context":context,"prepare":false,"output_limit":4}))
        .unwrap();
    runtime
        .handle(json!({"op":"segment_feed","bytes":STANDARD.encode(b"a")}))
        .unwrap();
    assert!(
        runtime
            .handle(json!({"op":"segment_run"}))
            .unwrap_err()
            .to_string()
            .contains("output exceeds its allowance")
    );
    let mut content = plan().content();
    for segment in content.tracks.iter_mut().flat_map(|t| &mut t.segments) {
        segment.size = 1;
        segment.processing = Processing::LuaStream;
    }
    assert!(
        runtime
            .handle(json!({"op":"media_finish","plan":content,"size":16 * 1024 * 1024 + 5}))
            .is_err()
    );
}

#[test]
fn unknown_sizes_ass_and_large_plans_keep_validation_and_saved_identity() {
    let mut value = plan();
    value.tracks[0].segments[0].size = 0;
    assert!(value.has_unknown_sizes() && value.needs_current_writer());
    value.validate(true).unwrap();
    value.tracks[0].segments[0].range_start = Some(0);
    assert!(value.validate(true).is_err());
    value.tracks[0].segments[0].range_start = None;
    value.tracks[1].kind = TrackKind::Subtitle;
    value.tracks[1].format = Format::Ass;
    value.validate(true).unwrap();
    let source = r#"
        local function resolve()
            local segments={}
            for i=1,16384 do segments[i]={id='part'..i,url='https://example.invalid/part'..i} end
            return xd.media_plan({content_id='large',container='mkv',tracks={{id='video',kind='video',format='mp4',segments=segments}}})
        end
        return {resolve=resolve,refresh=resolve,transform=function(_,b)return b end,finish=function()return true end}
    "#;
    let runtime = Runtime::new(
        source,
        Capabilities::Resolver(Box::new(
            crate::script_automation::Fetcher::new(crate::proxy::Route::direct()).unwrap(),
        )),
        None,
    )
    .unwrap();
    let reply = runtime
        .handle(json!({"op":"resolve","url":"https://example.invalid/","previous":null}))
        .unwrap();
    let plan: Plan = serde_json::from_value(reply["plan"].clone()).unwrap();
    assert_eq!(plan.tracks[0].segments.len(), MAX_SEGMENTS);
    assert!(serde_json::to_vec(&plan).unwrap().len() > 1024 * 1024);
    let mut wire = Vec::new();
    crate::provider::process::write_frame(&mut wire, &reply).unwrap();
    assert_eq!(
        crate::provider::process::read_frame(&mut wire.as_slice()).unwrap(),
        reply
    );
}

#[test]
fn streaming_callbacks_rewrite_more_than_eight_mib_and_expire_retained_handles() {
    let source = r#"
        local previous_reader,previous_writer
        return {resolve=function()end,refresh=function()end,transform=function(_,b)return b end,finish=function()return true end,
        transform_stream=function(context,reader,writer)
            if previous_reader then
                assert(not pcall(previous_reader.size,previous_reader))
                assert(not pcall(previous_writer.write,previous_writer,'late'))
            end
            assert(reader:size()==context.size and context.path==nil and xd.fetch==nil)
            assert(not pcall(reader.read,reader,0,262145))
            assert(not pcall(reader.read,reader,-1,1))
            local offset=4
            while offset<reader:size() do
                local bytes=reader:read(offset,262144)
                writer:write(bytes)
                offset=offset+#bytes
            end
            writer:write('tail')
            previous_reader,previous_writer=reader,writer
            return true
        end}
    "#;
    let runtime =
        Runtime::new(source, Capabilities::Processor(uuid::Uuid::new_v4()), None).unwrap();
    for size in [9 * 1024 * 1024 + 13, 37] {
        let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let context = Chunk {
            content_id: "test".into(),
            track_id: "v".into(),
            kind: TrackKind::Video,
            segment_id: "one".into(),
            segment_index: 0,
            track_offset: 0,
            output_offset: 0,
            size: size as u64,
        };
        runtime
            .handle(json!({"op":"segment_begin","context":context,"prepare":false,"stream":true,"output_limit":size}))
            .unwrap();
        for chunk in bytes.chunks(CHUNK_BYTES) {
            runtime
                .handle(json!({"op":"segment_feed","bytes":STANDARD.encode(chunk)}))
                .unwrap();
        }
        let length = runtime
            .handle(json!({"op":"segment_run"}))
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(length, size as u64);
        let mut actual = Vec::new();
        while actual.len() < size {
            let chunk = runtime
                .handle(json!({"op":"segment_read","offset":actual.len()}))
                .unwrap();
            actual.extend(STANDARD.decode(chunk.as_str().unwrap()).unwrap());
        }
        assert_eq!(&actual[..size - 4], &bytes[4..]);
        assert_eq!(&actual[size - 4..], b"tail");
    }
}

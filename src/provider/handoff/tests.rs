use super::*;
use crate::{
    provider::{
        crypto,
        runtime::{Capabilities, Runtime},
    },
    script_automation::Fetcher,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;

fn plan() -> serde_json::Value {
    json!({"url":"https://example.invalid/file","size":16,"input_sha256":"0".repeat(64),
        "output_sha256":"1".repeat(64),"transform_id":"fixture"})
}
fn source(resolve: &str) -> String {
    format!(
        r#"
        assert(xd.supports('xd.handoff_key') == (xd.phase == 'resolver'))
        assert(xd.supports('xd.session_key') == (xd.phase == 'processor'))
        assert(xd.supports('xd.fetch_bytes') == (xd.phase == 'resolver'))
        local function resolve(request)
            {resolve}
            return xd.json([==[{}]==])
        end
        return {{resolve=resolve, refresh=resolve,
            transform=function(plan, bytes, offset)
                assert(xd.fetch == nil and xd.fetch_bytes == nil and xd.handoff_key == nil)
                local key = xd.session_key('video')
                assert(not pcall(xd.to_json, key))
                return xd.aes_ctr(key, 'f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff', bytes, offset)
            end, finish=function() return true end}}
    "#,
        plan()
    )
}
fn resolver(source: &str, scope: Uuid, owner: Uuid, fetch: Fetcher) -> Runtime {
    Runtime::with_handoff(
        source,
        Capabilities::Resolver(Box::new(fetch)),
        None,
        scope,
        owner,
        None,
    )
    .unwrap()
}
fn resolve(runtime: &Runtime) -> serde_json::Value {
    runtime
        .handle(json!({"op":"resolve","url":"https://example.invalid/sample","previous":null}))
        .unwrap()
}
fn processor(
    source: &str,
    scope: Uuid,
    owner: Uuid,
    keys: serde_json::Value,
) -> anyhow::Result<Runtime> {
    Runtime::with_handoff(
        source,
        Capabilities::Processor(owner),
        None,
        scope,
        owner,
        Some(serde_json::from_value(keys)?),
    )
}
fn direct() -> Fetcher {
    Fetcher::new(crate::proxy::Route::direct()).unwrap()
}
const KEY: &str = "2b7e151628aed2a6abf7158809cf4f3c";
fn give_key() -> &'static str {
    "xd.handoff_key('video', string.char(43,126,21,22,40,174,210,166,171,247,21,136,9,207,79,60))"
}
fn transform(runtime: &Runtime) -> anyhow::Result<serde_json::Value> {
    let mut content = plan();
    content.as_object_mut().unwrap().remove("url");
    runtime.handle(json!({"op":"transform","plan":content,"offset":0,
        "bytes":STANDARD.encode(crypto::unhex("874d6191b620e3261bef6864990db6ce").unwrap())}))
}

#[test]
fn keys_cross_the_private_channel_as_ciphertext_and_remain_opaque() {
    let (scope, owner) = (Uuid::new_v4(), Uuid::new_v4());
    let source = source(give_key());
    let reply = resolve(&resolver(&source, scope, owner, direct()));
    let serialized = reply.to_string();
    assert!(!serialized.contains(KEY));
    assert!(!serialized.contains(&STANDARD.encode(crypto::unhex(KEY).unwrap())));
    assert!(reply["plan"].get("keys").is_none());
    let worker = processor(&source, scope, owner, reply["handoff"].clone()).unwrap();
    let result = transform(&worker).unwrap();
    assert_eq!(
        STANDARD.decode(result.as_str().unwrap()).unwrap(),
        crypto::unhex("6bc1bee22e409f96e93d7e117393172a").unwrap()
    );
    assert!(processor(&source, Uuid::new_v4(), owner, reply["handoff"].clone()).is_err());
    assert!(processor(&source, scope, Uuid::new_v4(), reply["handoff"].clone()).is_err());
    // Rewriting the envelope labels cannot rebind the ciphertext either.
    let other_run = Uuid::new_v4();
    let mut rebound = reply["handoff"].clone();
    rebound["scope"] = json!(other_run);
    assert!(processor(&source, other_run, owner, rebound).is_err());
    let other_provider = Uuid::new_v4();
    let mut rebound = reply["handoff"].clone();
    rebound["owner"] = json!(other_provider);
    assert!(processor(&source, scope, other_provider, rebound).is_err());
    let mut tampered = reply["handoff"].clone();
    tampered["values"]["video"] = json!("provider-key-v1:AAAA");
    assert!(processor(&source, scope, owner, tampered).is_err());
    let mut renamed = reply["handoff"].clone();
    let token = renamed["values"]
        .as_object_mut()
        .unwrap()
        .remove("video")
        .unwrap();
    renamed["values"]["audio"] = token;
    assert!(processor(&source, scope, owner, renamed).is_err());
}

#[test]
fn failed_refresh_and_empty_refresh_never_reuse_prior_keys() {
    let (scope, owner) = (Uuid::new_v4(), Uuid::new_v4());
    let text = format!(
        "local attempts=0\n{}",
        source(&format!(
            "attempts=attempts+1; if attempts < 3 then {} end; if attempts == 2 then error('fixture failure') end",
            give_key()
        ))
    );
    let runtime = resolver(&text, scope, owner, direct());
    let first = resolve(&runtime);
    assert_eq!(first["handoff"]["values"].as_object().unwrap().len(), 1);
    assert!(
        runtime
            .handle(
                json!({"op":"resolve","url":"https://example.invalid/sample","previous":plan()})
            )
            .is_err()
    );
    let last = resolve(&runtime);
    assert!(last["handoff"]["values"].as_object().unwrap().is_empty());
    let worker = processor(&text, scope, owner, last["handoff"].clone()).unwrap();
    assert!(transform(&worker).is_err());
}

#[test]
fn keys_have_bounded_names_sizes_and_lifetimes() {
    for expression in [
        "xd.handoff_key('', string.rep('k',16))",
        "xd.handoff_key('../x', string.rep('k',16))",
        "xd.handoff_key(string.rep('x',65), string.rep('k',16))",
        "xd.handoff_key('video', string.rep('k',15))",
        "xd.handoff_key('video', string.rep('k',33))",
        "xd.handoff_key('video', string.rep('k',16)); xd.handoff_key('video', string.rep('j',16))",
        "for i=1,65 do xd.handoff_key('k'..i, string.rep('k',16)) end",
    ] {
        let runtime = resolver(
            &source(expression),
            Uuid::new_v4(),
            Uuid::new_v4(),
            direct(),
        );
        assert!(
            runtime
                .handle(
                    json!({"op":"resolve","url":"https://example.invalid/sample","previous":null})
                )
                .is_err(),
            "{expression}"
        );
    }
    let runtime = resolver(
        &source(
            "for i=1,64 do xd.handoff_key('key-'..i, string.rep('k', ({16,24,32})[i%3+1])) end",
        ),
        Uuid::new_v4(),
        Uuid::new_v4(),
        direct(),
    );
    let keys: Keys = serde_json::from_value(resolve(&runtime)["handoff"].clone()).unwrap();
    assert_eq!(keys.values.len(), 64);
    let scope = keys.scope;
    let owner = keys.owner;
    Runtime::with_handoff(
        &source(""),
        Capabilities::Processor(owner),
        None,
        scope,
        owner,
        Some(keys),
    )
    .unwrap();
    let text = format!("{}; {}", give_key(), source(""));
    assert!(
        Runtime::with_handoff(
            &text,
            Capabilities::Resolver(Box::new(direct())),
            None,
            Uuid::new_v4(),
            Uuid::new_v4(),
            None
        )
        .is_err()
    );
}

#[test]
fn binary_http_and_key_handoff_work_together_against_an_offline_server() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("Fixture server did not receive a request: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut received = Vec::new();
        let mut block = [0; 1024];
        let end = loop {
            if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            let count = stream.read(&mut block).unwrap();
            assert!(count > 0);
            received.extend_from_slice(&block[..count]);
        };
        while received.len() < end + 256 {
            let count = stream.read(&mut block).unwrap();
            assert!(count > 0);
            received.extend_from_slice(&block[..count]);
        }
        assert_eq!(&received[end..], (0..=255u8).collect::<Vec<_>>());
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 16\r\nConnection: close\r\n\r\n").unwrap();
        stream.write_all(&crypto::unhex(KEY).unwrap()).unwrap();
    });
    let (scope, owner) = (Uuid::new_v4(), Uuid::new_v4());
    let text = source(
        "local bytes={} for i=0,255 do bytes[#bytes+1]=string.char(i) end; local reply=xd.fetch_bytes(request.url,{method='POST',body=table.concat(bytes),headers={['Content-Type']='application/octet-stream'}}); assert(reply.status==200 and #reply.body==16); xd.handoff_key('video',reply.body)",
    );
    let runtime = resolver(&text, scope, owner, direct().allowing_local());
    let reply = runtime
        .handle(json!({"op":"resolve","url":url,"previous":null}))
        .unwrap();
    server.join().unwrap();
    let worker = processor(&text, scope, owner, reply["handoff"].clone()).unwrap();
    let transformed = transform(&worker).unwrap();
    assert_eq!(
        STANDARD.decode(transformed.as_str().unwrap()).unwrap(),
        crypto::unhex("6bc1bee22e409f96e93d7e117393172a").unwrap()
    );
}

#[test]
fn binary_fetch_retains_network_permissions() {
    let text = source("xd.fetch_bytes(request.url, {method='POST',body=string.char(0,255)})");
    for fetch in [
        direct(),
        direct()
            .allowing_local()
            .limit_hosts(vec!["example.invalid".into()]),
    ] {
        let runtime = resolver(&text, Uuid::new_v4(), Uuid::new_v4(), fetch);
        let error = runtime
            .handle(json!({"op":"resolve","url":"http://127.0.0.1:1/","previous":null}))
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("public internet") || text.contains("did not declare"),
            "{text}"
        );
    }
}

#[test]
fn http_handoff_is_bound_to_the_run_name_and_media_plan_not_the_processor() {
    let (scope, owner) = (Uuid::new_v4(), Uuid::new_v4());
    let script = source(
        "local session=xd.http_session('https://example.invalid', {cookies={login='private-cookie'}, headers={Authorization='Bearer private-token'}}); xd.handoff_session('media',session)",
    );
    let reply = resolve(&resolver(&script, scope, owner, direct()));
    assert!(!reply.to_string().contains("private-cookie"));
    assert!(!reply.to_string().contains("private-token"));
    assert!(processor(&script, scope, owner, reply["handoff"].clone()).is_err());
    let mut keys: Keys = serde_json::from_value(reply["handoff"].clone()).unwrap();
    let media = super::super::Resolution::parse(json!({
        "kind":"media", "media_api":1, "content_id":"session-test", "container":"mkv",
        "tracks":[{"id":"video","kind":"video","format":"mp4","segments":[
            {"id":"one","url":"https://example.invalid/file","session":"media"}
        ]}]
    }))
    .unwrap();
    keys.validate_plan(&media).unwrap();
    assert!(
        keys.validate_plan(&super::super::Resolution::parse(plan()).unwrap())
            .is_err()
    );
    let sessions = keys.take_sessions().unwrap();
    let url = reqwest::Url::parse("https://example.invalid/file").unwrap();
    assert_eq!(
        sessions["media"].cookie_header(&url).unwrap().as_deref(),
        Some("login=private-cookie")
    );
    assert!(keys.validate_plan(&media).is_err());
    processor(&script, scope, owner, serde_json::to_value(&keys).unwrap()).unwrap();
    for field in ["scope", "owner"] {
        let mut changed = reply["handoff"].clone();
        changed[field] = json!(Uuid::new_v4());
        let mut keys: Keys = serde_json::from_value(changed).unwrap();
        assert!(keys.take_sessions().is_err());
    }
    let mut keys: Keys = serde_json::from_value(reply["handoff"].clone()).unwrap();
    let token = keys.sessions.remove("media").unwrap();
    keys.sessions.insert("renamed".into(), token);
    assert!(keys.take_sessions().is_err());
}

#[test]
fn http_handoff_is_bounded_and_failed_refresh_clears_sessions() {
    for action in [
        "xd.handoff_session('',s)",
        "xd.handoff_session('same',s);xd.handoff_session('same',s)",
        "for i=1,9 do xd.handoff_session('s'..i,s) end",
    ] {
        let script = source(&format!(
            "local s=xd.http_session('https://example.invalid');{action}"
        ));
        let runtime = resolver(&script, Uuid::new_v4(), Uuid::new_v4(), direct());
        assert!(
            runtime
                .handle(json!({"op":"resolve","url":"https://example.invalid/","previous":null}))
                .is_err()
        );
    }
    let script = format!(
        "local attempt=0\n{}",
        source(
            "attempt=attempt+1;if attempt<3 then xd.handoff_session('s',xd.http_session('https://example.invalid')) end;if attempt==2 then error('fixture') end"
        )
    );
    let runtime = resolver(&script, Uuid::new_v4(), Uuid::new_v4(), direct());
    assert_eq!(
        resolve(&runtime)["handoff"]["sessions"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert!(
        runtime
            .handle(json!({"op":"resolve","url":"https://example.invalid/","previous":plan()}))
            .is_err()
    );
    assert!(resolve(&runtime)["handoff"].get("sessions").is_none());
}

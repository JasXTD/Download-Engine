use super::*;
use crate::{control::Action, scheduler::Scheduler};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub(super) fn fixture() -> (tempfile::TempDir, Context) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running("main", false).unwrap();
    (
        temp,
        Context {
            store,
            read_only: false,
            permissions: Default::default(),
            no_start: true,
            start: |_| panic!("Must not launch the desktop in isolated store tests"),
        },
    )
}
fn modern(method: &str, mut params: Value) -> Value {
    params["_meta"] = json!({VERSION_META:MODERN,CAPABILITIES_META:{}});
    json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
}
fn reply(dispatch: Dispatch) -> Value {
    match dispatch {
        Dispatch::Reply(value) => value,
        _ => panic!("Expected a reply"),
    }
}
fn call(ctx: &Context, name: &str, args: Value) -> Result<Value> {
    let schema = tools::catalog(false)
        .into_iter()
        .find(|tool| tool["name"] == name)
        .unwrap()["inputSchema"]
        .clone();
    tools::validate(&schema, &args)?;
    tools::call(ctx, name, &args, &AtomicBool::new(false))
}

fn saved_script(
    store: &Store,
    language: crate::script_automation::Language,
) -> crate::script_automation::Script {
    use crate::script_automation::{Script, Trigger};
    store
        .save_script(&Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            name: "Reviewed script".into(),
            language,
            body: "print('reviewed')".into(),
            enabled: false,
            trigger: Trigger::Manual,
            timeout_seconds: 30,
            change_files: false,
        })
        .unwrap()
}

#[test]
fn server_capabilities_hide_and_reject_disallowed_tools_at_both_layers() {
    let (_temp, mut ctx) = fixture();
    ctx.permissions = Permissions {
        downloads: false,
        queues: true,
        lua: false,
        powershell: false,
    };
    let listing = reply(Protocol::default().accept(modern("tools/list", json!({})), ctx.access()));
    let tools = listing["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "create_queue"));
    for name in ["pause_all", "add_downloads", "run_script", "stop_script"] {
        assert!(!tools.iter().any(|t| t["name"] == name));
        let result = reply(Protocol::default().accept(
            modern("tools/call", json!({"name":name,"arguments":{}})),
            ctx.access(),
        ));
        assert_eq!(result["result"]["isError"], true);
        assert!(
            call(
                &ctx,
                name,
                if name == "run_script" {
                    json!({"id":uuid::Uuid::new_v4().to_string()})
                } else {
                    json!({})
                }
            )
            .is_err()
        );
    }
    let key = uuid::Uuid::new_v4().to_string();
    ctx.store.submit_control(&key, &Action::PauseAll).unwrap();
    assert!(call(&ctx, "cancel_request", json!({"request_id":key})).is_err());
    assert_eq!(ctx.store.control_receipt(&key).unwrap().state, "pending");
    assert!(ctx.store.script_runs(None).unwrap().is_empty());
}

#[test]
fn scripts_require_language_capability_and_local_revision_approval() {
    use crate::script_automation::Language;
    let (_temp, mut ctx) = fixture();
    let script = saved_script(&ctx.store, Language::PowerShell);
    assert!(call(&ctx, "run_script", json!({"id":script.id})).is_err());
    ctx.permissions.lua = true;
    assert!(call(&ctx, "run_script", json!({"id":script.id})).is_err());
    ctx.store
        .approve_mcp_script(&script.id, script.revision)
        .unwrap();
    assert!(call(&ctx, "run_script", json!({"id":script.id})).is_err());
    ctx.permissions.powershell = true;
    ctx.store.revoke_mcp_script(&script.id).unwrap();
    assert!(call(&ctx, "run_script", json!({"id":script.id})).is_err());
    assert!(ctx.store.script_runs(None).unwrap().is_empty());
    assert!(ctx.store.claim_control().unwrap().is_none());
}

#[test]
fn queued_mcp_script_rechecks_revision_and_revocation_before_snapshotting() {
    use crate::script_automation::Language;
    let (_temp, ctx) = fixture();
    let script = saved_script(&ctx.store, Language::Lua);
    ctx.store
        .approve_mcp_script(&script.id, script.revision)
        .unwrap();
    let action = Action::RunMcpScript {
        id: script.id.clone(),
        revision: script.revision,
    };
    let request = uuid::Uuid::new_v4().to_string();
    ctx.store.submit_control(&request, &action).unwrap();
    let (key, accepted) = ctx.store.claim_control().unwrap().unwrap();
    ctx.store.revoke_mcp_script(&script.id).unwrap();
    assert!(crate::control::execute_stored(&ctx.store, &key, accepted).is_err());
    ctx.store
        .approve_mcp_script(&script.id, script.revision)
        .unwrap();
    let mut changed = script.clone();
    changed.language = Language::PowerShell;
    changed.body = "Write-Output 'changed'".into();
    let changed = ctx.store.save_script(&changed).unwrap();
    assert!(
        ctx.store
            .approve_mcp_script(&changed.id, script.revision)
            .is_err()
    );
    assert!(crate::control::execute_stored(&ctx.store, &key, action.clone()).is_err());
    assert!(ctx.store.script_runs(None).unwrap().is_empty());
    ctx.store
        .approve_mcp_script(&changed.id, changed.revision)
        .unwrap();
    assert!(crate::control::execute_stored(&ctx.store, &key, action).is_err());
    let run = ctx
        .store
        .run_mcp_script(
            &changed.id,
            changed.revision,
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(run.revision, changed.revision);
    assert_eq!(run.language, Language::PowerShell);
    assert_eq!(run.state, "queued");
}

#[test]
fn approval_fingerprint_covers_source_permissions_and_language_even_if_revision_is_reused() {
    use crate::script_automation::Language;
    let (_temp, ctx) = fixture();
    let script = saved_script(&ctx.store, Language::Lua);
    ctx.store
        .approve_mcp_script(&script.id, script.revision)
        .unwrap();
    ctx.store.connect().unwrap().execute("UPDATE automation_scripts SET data=json_set(data,'$.change_files',json('true')) WHERE id=?1",[&script.id]).unwrap();
    assert!(
        ctx.store
            .run_mcp_script(
                &script.id,
                script.revision,
                &uuid::Uuid::new_v4().to_string()
            )
            .is_err()
    );
    assert!(ctx.store.script_runs(None).unwrap().is_empty());
}

#[test]
fn modern_requests_are_stateless_and_legacy_clients_negotiate() {
    let mut p = Protocol::default();
    let discovered = reply(p.accept(modern("server/discover", json!({})), false));
    assert_eq!(discovered["result"]["supportedVersions"], supported());
    assert_eq!(discovered["result"]["resultType"], "complete");
    assert!(
        reply(p.accept(modern("tools/list", json!({})), false))["result"]["tools"]
            .as_array()
            .unwrap()
            .len()
            > 10
    );
    let no_meta = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
    assert_eq!(
        reply(p.accept(no_meta.clone(), false))["error"]["code"],
        -32602
    );
    for version in LEGACY {
        let mut p = Protocol::default();
        let initialize = json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
        let out = reply(p.accept(initialize, false));
        assert_eq!(out["result"]["protocolVersion"], version);
        assert!(out["result"].get("resultType").is_none());
        assert_eq!(
            reply(p.accept(no_meta.clone(), false))["error"]["code"],
            -32602
        );
        assert!(matches!(
            p.accept(
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                false
            ),
            Dispatch::Ignore
        ));
        assert!(reply(p.accept(no_meta.clone(), false))["result"]["tools"].is_array());
    }
}

#[test]
fn malformed_versions_ids_notifications_and_unknown_methods_have_explicit_outcomes() {
    let mut p = Protocol::default();
    let mut request = modern("ping", json!({}));
    request["params"]["_meta"][VERSION_META] = json!("2099-01-01");
    let out = reply(p.accept(request, false));
    assert_eq!(out["error"]["code"], -32022);
    assert_eq!(out["error"]["data"]["supported"], supported());
    let mut request = modern("ping", json!({}));
    request["params"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove(CAPABILITIES_META);
    assert_eq!(reply(p.accept(request, false))["error"]["code"], -32602);
    for id in [Value::Null, json!(true), json!([]), json!(1.2)] {
        let mut request = modern("ping", json!({}));
        request["id"] = id;
        assert_eq!(reply(p.accept(request, false))["error"]["code"], -32600);
    }
    assert_eq!(reply(p.accept(json!([]), false))["error"]["code"], -32600);
    assert_eq!(
        reply(p.accept(modern("unknown", json!({})), false))["error"]["code"],
        -32601
    );
    assert!(matches!(
        p.accept(
            json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"pause_all"}}),
            false
        ),
        Dispatch::Ignore
    ));
    assert!(
        matches!(p.accept(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}),false),Dispatch::Cancel(id) if id=="3")
    );
}

#[test]
fn schemas_reject_invalid_fields_and_read_only_is_enforced_at_both_layers() {
    let mut p = Protocol::default();
    let listing = reply(p.accept(modern("tools/list", json!({})), true));
    assert!(
        listing["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["annotations"]["readOnlyHint"] == true)
    );
    let out = reply(p.accept(
        modern("tools/call", json!({"name":"pause_all","arguments":{}})),
        true,
    ));
    assert_eq!(out["result"]["isError"], true);
    for (name, args) in [
        ("list_downloads", json!({"limit":0})),
        ("list_downloads", json!({"limit":101})),
        ("list_downloads", json!({"status":"finished"})),
        ("list_queues", json!({"offset":-1})),
        ("add_downloads", json!({"urls":[]})),
        ("add_downloads", json!({"urls":[4]})),
        ("create_queue", json!({"name":""})),
        ("create_queue", json!({"name":"ok","execute":"bad"})),
        ("get_download", json!({"id":"../file"})),
        ("set_queue_limit", json!({"queue":"main","limit":33})),
        ("pause_all", json!({"request_id":null})),
        ("run_script", json!({})),
    ] {
        let out = reply(p.accept(
            modern("tools/call", json!({"name":name,"arguments":args})),
            false,
        ));
        assert_eq!(out["result"]["isError"], true, "{name}: {out}");
    }
    let (_temp, mut ctx) = fixture();
    ctx.read_only = true;
    assert!(
        call(&ctx, "pause_all", json!({}))
            .unwrap_err()
            .to_string()
            .contains("read-only")
    );
}

#[test]
fn bounded_tool_output_and_embedded_guide_are_well_formed() {
    let output = tool_result(Ok(json!({"text":"x".repeat(MAX_OUTPUT)})));
    assert_eq!(output["isError"], true);
    let output = tool_result(Ok(json!({"text":"line1\nline2"})));
    let bytes = serde_json::to_vec(&response(json!(1), output, false)).unwrap();
    let parsed: Value = serde_json::from_slice(&bytes).unwrap();
    let text: Value =
        serde_json::from_str(parsed["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, parsed["result"]["structuredContent"]);
    let (_temp, ctx) = fixture();
    let guide = call(&ctx, "get_usage_guide", json!({})).unwrap();
    assert!(guide["guide"].as_str().unwrap().contains("request_id"));
    assert_eq!(guide["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn scheduler_mutations_retry_receipts_filtering_and_validation_share_app_behavior() {
    let (_temp, ctx) = fixture();
    let scheduler = Scheduler::start(ctx.store.clone()).unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    let args = json!({"name":"MCP queue","request_id":request});
    let created = call(&ctx, "create_queue", args.clone()).unwrap();
    assert_eq!(created["ok"], true);
    assert_eq!(created, call(&ctx, "create_queue", args).unwrap());
    assert!(
        call(
            &ctx,
            "create_queue",
            json!({"name":"Other","request_id":request})
        )
        .is_err()
    );
    let added = call(
        &ctx,
        "add_downloads",
        json!({"urls":["https://example.invalid/a.bin"],"queue":"MCP queue"}),
    )
    .unwrap();
    assert_eq!(added["ok"], true);
    let list = call(
        &ctx,
        "list_downloads",
        json!({"queue":"MCP queue","status":"queued","limit":1}),
    )
    .unwrap();
    assert_eq!(list["total"], 1);
    assert!(list["items"][0].get("url").is_none());
    assert_eq!(
        call(&ctx, "list_downloads", json!({"offset":1})).unwrap()["items"],
        json!([])
    );
    for args in [
        json!({"urls":["file:///secret"]}),
        json!({"urls":["https://example.invalid/a"],"folder":"relative"}),
        json!({"urls":["https://example.invalid/a","https://example.invalid/b"],"name":"one"}),
    ] {
        assert!(call(&ctx, "add_downloads", args).is_err());
    }
    assert_eq!(ctx.store.list().unwrap().len(), 1);
    assert_eq!(
        call(&ctx, "get_request", json!({"request_id":request})).unwrap()["receipt"]["state"],
        "done"
    );
    drop(scheduler);
}

#[test]
fn cancellation_and_deadlines_cancel_only_unclaimed_commands() {
    let (_temp, ctx) = fixture();
    let id = uuid::Uuid::new_v4().to_string();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (worker_ctx, token, worker_id) = (ctx.clone(), cancelled.clone(), id.clone());
    let worker = thread::spawn(move || {
        tools::call(
            &worker_ctx,
            "pause_all",
            &json!({"request_id":worker_id}),
            &token,
        )
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while ctx.store.control_receipt(&id).is_err() {
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        worker.join().unwrap().unwrap()["receipt"]["state"],
        "cancelled"
    );
    let timeout = call(&ctx, "pause_all", json!({"wait_seconds":1})).unwrap();
    assert_eq!(timeout["ok"], false);
    assert_eq!(timeout["receipt"]["state"], "cancelled");
    let id = uuid::Uuid::new_v4().to_string();
    ctx.store.submit_control(&id, &Action::PauseAll).unwrap();
    assert_eq!(ctx.store.claim_control().unwrap().unwrap().0, id);
    assert_eq!(
        call(&ctx, "cancel_request", json!({"request_id":id})).unwrap()["receipt"]["state"],
        "running"
    );
}

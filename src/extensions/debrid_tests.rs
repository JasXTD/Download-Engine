//! Execute the shipped Lua, using synthetic API replies, never customer accounts.
use super::*;
use crate::{
    paths::AppPaths,
    script_automation::{self, FetchReply, FetchRequest, Reading},
    store::Store,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};

fn fixture(
    which: usize,
    job: &str,
) -> (tempfile::TempDir, Store, script_automation::Script, Context) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    store.set_queue_running("main", false).unwrap();
    let installed = store
        .install_extension(examples::PLUGINS[which], &Grants::default())
        .unwrap();
    assert!(installed.enabled);
    let script = store.extension_job(&installed.package.id, job).unwrap();
    let context = store
        .extension_context(&script, Reach::Web)
        .unwrap()
        .unwrap();
    (temp, store, script, context)
}
struct Step {
    path: &'static str,
    status: u16,
    body: Value,
    token: Option<&'static str>,
}
fn step(path: &'static str, body: Value) -> Step {
    Step {
        path,
        status: 200,
        body,
        token: None,
    }
}
fn success(data: Value) -> Value {
    json!({"status":"success","data":data})
}
fn form(bytes: &[u8]) -> BTreeMap<String, String> {
    reqwest::Url::parse(&format!(
        "https://fixture.test/?{}",
        std::str::from_utf8(bytes).unwrap()
    ))
    .unwrap()
    .query_pairs()
    .into_owned()
    .collect()
}
fn run(
    which: usize,
    action: &str,
    account: Option<Value>,
    steps: Vec<Step>,
) -> (
    anyhow::Result<Reading>,
    Vec<Option<String>>,
    Vec<FetchRequest>,
) {
    let (_temp, _store, script, mut context) = fixture(which, action);
    if let Some(account) = account {
        context.settings.insert(
            "account".into(),
            if account.is_string() {
                account
            } else {
                account.to_string().into()
            },
        );
    }
    let replies = Rc::new(RefCell::new(VecDeque::from(steps)));
    let requests = Rc::new(RefCell::new(Vec::new()));
    let saved = Rc::new(RefCell::new(Vec::new()));
    let (responses, seen, writes) = (replies.clone(), requests.clone(), saved.clone());
    let hosts = context.hosts.clone();
    let result = script_automation::execute_reader_action(
        &script.body,
        &json!({"url":"https://files.example.test/file?id=1&name=a%20b", "trigger":"site_reader"}),
        Some(&context),
        move |request| {
            let response = responses
                .borrow_mut()
                .pop_front()
                .expect("unexpected API call");
            assert!(url_allowed(&hosts, &request.url));
            assert!(
                request.url.ends_with(response.path),
                "unexpected path, no credentials printed"
            );
            let url = reqwest::Url::parse(&request.url).unwrap();
            let session = request
                .session
                .as_ref()
                .expect("origin-bound session required");
            session.check_origin(&url).unwrap();
            assert!(
                session
                    .check_origin(&reqwest::Url::parse("https://untrusted.example/").unwrap())
                    .is_err()
            );
            let mut headers = reqwest::header::HeaderMap::new();
            session.apply_headers(&mut headers).unwrap();
            if let Some(token) = response.token {
                assert_eq!(headers["authorization"], format!("Bearer {token}"));
            }
            if response.path.ends_with("/link/unlock") || response.path.ends_with("unrestrict/link")
            {
                assert_eq!(request.method.as_deref(), Some("POST"));
                let form = form(request.body.as_ref().unwrap());
                assert_eq!(
                    form["link"],
                    "https://files.example.test/file?id=1&name=a%20b"
                );
            }
            seen.borrow_mut().push(request.clone());
            Ok(FetchReply {
                status: response.status,
                url: request.url,
                headers: BTreeMap::new(),
                body: response.body.to_string().into_bytes(),
            })
        },
        move |key, value| {
            assert_eq!(key, "account");
            writes.borrow_mut().push(value.map(str::to_owned));
            Ok(())
        },
    )
    .and_then(Reading::from_value);
    assert!(
        replies.borrow().is_empty(),
        "fixture replies were not consumed"
    );
    let saves = saved.borrow().clone();
    let seen = requests.borrow().clone();
    (result, saves, seen)
}
fn ad() -> Value {
    json!({"apikey":"fixture-ad-token"})
}
fn rd() -> Value {
    json!({"client_id":"user-client","client_secret":"fixture-secret","access_token":"fixture-old","refresh_token":"fixture-refresh"})
}

#[test]
fn debrid_reviewed_installs_are_enabled_without_starting_any_actions() {
    for bytes in examples::PLUGINS {
        let temp = tempfile::tempdir().unwrap();
        let paths = || AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        };
        let store = Store::open(paths()).unwrap();
        let review = store.review_extension(bytes).unwrap();
        assert!(
            store.extensions().unwrap().is_empty(),
            "review saves nothing"
        );
        let installed = store
            .install_reviewed(bytes, &Grants::default(), &review)
            .unwrap();
        assert!(installed.enabled);
        for job in &installed.package.jobs {
            assert!(
                store
                    .extension_job(&installed.package.id, &job.id)
                    .unwrap()
                    .enabled
            );
        }
        assert!(store.script_runs(None).unwrap().is_empty());
        assert!(store.site_readers().unwrap().is_empty());
        store
            .set_extension_enabled(&installed.package.id, false)
            .unwrap();
        drop(store);
        let store = Store::open(paths()).unwrap();
        assert!(!store.extension(&installed.package.id).unwrap().enabled);
        for job in &installed.package.jobs {
            assert!(
                !store
                    .extension_job(&installed.package.id, &job.id)
                    .unwrap()
                    .enabled
            );
        }
        assert!(store.script_runs(None).unwrap().is_empty());
    }
}

#[test]
fn debrid_custom_copies_still_install_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(AppPaths {
        data: temp.path().join("data"),
        downloads: temp.path().join("downloads"),
    })
    .unwrap();
    let mut custom: Value = serde_json::from_slice(examples::PLUGINS[0]).unwrap();
    custom["id"] = json!("custom.account-plugin");
    let bytes = serde_json::to_vec(&custom).unwrap();
    let review = store.review_extension(&bytes).unwrap();
    let installed = store
        .install_reviewed(&bytes, &Grants::default(), &review)
        .unwrap();
    assert!(!installed.enabled);
    assert!(store.script_runs(None).unwrap().is_empty());
}

#[test]
fn debrid_plugins_are_separate_reviewed_and_not_automatic_page_readers() {
    for index in 0..3 {
        let (_temp, store, script, _) = fixture(index, "connect");
        assert!(
            store
                .site_reader_for("https://api.alldebrid.com/v4/user")
                .unwrap()
                .is_none()
        );
        assert!(store.site_readers().unwrap().is_empty());
        let link = script.extension.unwrap();
        assert!(store.extension_conflict(&link.id).unwrap().is_none());
        let package = store.extension(&link.id).unwrap().package;
        assert_eq!(
            package.name,
            ["All Debrid", "Real Debrid", "Premiumize.me"][index]
        );
        let mut modified: Value = serde_json::from_slice(examples::PLUGINS[index]).unwrap();
        modified["files"]["main.lua"] = json!("return {items={}}");
        assert!(
            store
                .review_extension(&serde_json::to_vec(&modified).unwrap())
                .is_err()
        );
    }
}
#[test]
fn debrid_all_debrid_pin_authorization_saves_only_the_key() {
    let (result, saved, _) = run(
        0,
        "connect",
        None,
        vec![
            step(
                "/v4.1/pin/get",
                success(json!({"pin":"A1B2","check":"fixture-check","expires_in":600})),
            ),
            step("/v4/pin/check", success(json!({"activated":false}))),
            step(
                "/v4/pin/check",
                success(json!({"activated":true,"apikey":"fixture-ad-token"})),
            ),
            Step {
                token: Some("fixture-ad-token"),
                ..step(
                    "/v4/user",
                    success(
                        json!({"user":{"isPremium":true,"email":"not-for-storage@example.test"}}),
                    ),
                )
            },
        ],
    );
    assert!(result.unwrap().title.unwrap().contains("premium"));
    assert_eq!(saved, vec![Some(ad().to_string())]);
}
#[test]
fn debrid_real_debrid_device_authorization_uses_public_client_and_protected_tokens() {
    let (result, saved, requests) = run(
        1,
        "connect",
        None,
        vec![
            step(
                "/oauth/v2/device/code?client_id=X245A4XAIBGVM&new_credentials=yes",
                json!({"device_code":"fixture-device","user_code":"ABCD","interval":5,"expires_in":1800}),
            ),
            Step {
                status: 403,
                ..step(
                    "/oauth/v2/device/credentials?client_id=X245A4XAIBGVM&code=fixture-device",
                    json!({"error":null,"error_code":null}),
                )
            },
            Step {
                status: 403,
                ..step(
                    "/oauth/v2/device/credentials?client_id=X245A4XAIBGVM&code=fixture-device",
                    json!({"error":"authorization_pending"}),
                )
            },
            step(
                "/oauth/v2/device/credentials?client_id=X245A4XAIBGVM&code=fixture-device",
                json!({"client_id":"user-client","client_secret":"fixture-secret"}),
            ),
            step(
                "/oauth/v2/token",
                json!({"access_token":"fixture-new","refresh_token":"fixture-refresh","expires_in":3600}),
            ),
        ],
    );
    assert!(result.unwrap().items.is_empty());
    assert_eq!(saved.len(), 1);
    assert!(saved[0].as_ref().unwrap().contains("fixture-new"));
    let form = form(requests.last().unwrap().body.as_ref().unwrap());
    assert_eq!(form["grant_type"], "http://oauth.net/grant_type/device/1.0");
    assert_eq!(form["code"], "fixture-device");
}
#[test]
fn debrid_device_refusal_is_not_treated_as_pending() {
    let (result, saved, _) = run(
        1,
        "connect",
        None,
        vec![
            step(
                "/oauth/v2/device/code?client_id=X245A4XAIBGVM&new_credentials=yes",
                json!({"device_code":"fixture-device","user_code":"ABCD","interval":5,"expires_in":1800}),
            ),
            Step {
                status: 403,
                ..step(
                    "/oauth/v2/device/credentials?client_id=X245A4XAIBGVM&code=fixture-device",
                    json!({"error":"access_denied","error_code":9}),
                )
            },
        ],
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("refused or expired")
    );
    assert!(saved.is_empty());
}
#[test]
fn debrid_generated_links_keep_names_sizes_and_single_connection_limit() {
    let (result, saved, _) = run(
        0,
        "generate",
        Some(ad()),
        vec![Step {
            token: Some("fixture-ad-token"),
            ..step(
                "/v4/link/unlock",
                success(
                    json!({"link":"https://cdn.debrid.it/file.bin","filename":"Harmless.bin","filesize":4096}),
                ),
            )
        }],
    );
    let reading = result.unwrap();
    assert!(reading.single_connection);
    assert_eq!(reading.items[0].filename.as_deref(), Some("Harmless.bin"));
    assert_eq!(reading.items[0].size, Some(4096));
    assert!(saved.is_empty());
    let (result, _, _) = run(
        1,
        "generate",
        Some(rd()),
        vec![Step {
            token: Some("fixture-old"),
            ..step(
                "/rest/1.0/unrestrict/link",
                json!([
            {"download":"https://cdn.real-debrid.com/a.bin","filename":"A.bin","filesize":12,"chunks":1},
            {"download":"https://cdn.real-debrid.com/b.bin","filename":"B.bin","filesize":22,"chunks":4}]),
            )
        }],
    );
    let reading = result.unwrap();
    assert_eq!(reading.items.len(), 2);
    assert!(
        reading
            .review("https://real-debrid.com/")
            .unwrap()
            .single_connection
    );
}
#[test]
fn debrid_refresh_rotates_tokens_once_and_retries_with_new_token() {
    let (result, saved, requests) = run(
        1,
        "generate",
        Some(rd()),
        vec![
            Step {
                status: 401,
                token: Some("fixture-old"),
                ..step(
                    "/rest/1.0/unrestrict/link",
                    json!({"error":"bad_token","error_code":8}),
                )
            },
            step(
                "/oauth/v2/token",
                json!({"access_token":"fixture-rotated","refresh_token":"fixture-next"}),
            ),
            Step {
                token: Some("fixture-rotated"),
                ..step(
                    "/rest/1.0/unrestrict/link",
                    json!({"download":"https://cdn.real-debrid.com/safe.bin","filename":"safe.bin","filesize":4,"chunks":1}),
                )
            },
        ],
    );
    assert!(result.is_ok());
    assert_eq!(saved.len(), 1);
    assert!(saved[0].as_ref().unwrap().contains("fixture-next"));
    let form = form(requests[1].body.as_ref().unwrap());
    assert_eq!(form["code"], "fixture-refresh");
}
#[test]
fn debrid_failure_messages_do_not_echo_response_secrets_or_private_links() {
    for (code, expected) in [
        (20, "premium"),
        (16, "support"),
        (22, "IP"),
        (23, "allowance"),
    ] {
        let (result, saved, _) = run(
            1,
            "generate",
            Some(rd()),
            vec![Step {
                status: 403,
                ..step(
                    "/rest/1.0/unrestrict/link",
                    json!({"error":"PRIVATE-TOKEN-DO-NOT-PRINT","error_code":code}),
                )
            }],
        );
        let message = result.unwrap_err().to_string();
        assert!(message.contains(expected), "{message}");
        assert!(!message.contains("PRIVATE-TOKEN"));
        assert!(saved.is_empty());
    }
    let (result, _, _) = run(
        0,
        "generate",
        Some(ad()),
        vec![Step {
            status: 429,
            ..step("/v4/link/unlock", json!({"secret":"PRIVATE-TOKEN"}))
        }],
    );
    assert!(result.unwrap_err().to_string().contains("limiting"));
}
#[test]
fn debrid_delayed_file_waits_then_returns_the_generated_link() {
    let (result, _, _) = run(
        0,
        "generate",
        Some(ad()),
        vec![
            step(
                "/v4/link/unlock",
                success(json!({"delayed":7,"filename":"safe.bin"})),
            ),
            step(
                "/v4/link/delayed",
                success(json!({"status":2,"link":"https://cdn.debrid.it/safe.bin"})),
            ),
        ],
    );
    assert_eq!(result.unwrap().items.len(), 1);
}
#[test]
fn debrid_rejects_unsafe_or_unapproved_generated_links() {
    for url in [
        "file:///C:/private",
        "https://127.0.0.1/private",
        "https://evil.test/x",
        "https://real-debrid.com.evil.test/x",
        "https://user@cdn.real-debrid.com/x",
    ] {
        let (result, _, _) = run(
            1,
            "generate",
            Some(rd()),
            vec![step("/rest/1.0/unrestrict/link", json!({"download":url}))],
        );
        assert!(result.is_err());
    }
}
#[test]
fn debrid_disconnect_is_local_and_missing_account_never_contacts_the_service() {
    for index in 0..3 {
        let (result, saved, requests) = run(index, "disconnect", None, vec![]);
        assert!(result.is_ok());
        assert_eq!(saved, vec![None]);
        assert!(requests.is_empty());
        let (result, saved, requests) = run(index, "generate", None, vec![]);
        let expected = if index == 2 {
            "API key"
        } else {
            "Connect your account"
        };
        assert!(result.unwrap_err().to_string().contains(expected));
        assert!(saved.is_empty() && requests.is_empty());
    }
}
#[test]
fn debrid_secret_writes_require_live_scope_and_reject_stopped_stale_or_parallel_actions() {
    let (_temp, store, script, context) = fixture(0, "connect");
    let minimum: i64 = store
        .connect()
        .unwrap()
        .query_row(
            "SELECT min_writer_version FROM profile_format WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        minimum, 12,
        "old binaries must not discard action permissions"
    );
    let run = store
        .run_site_reader(
            &script.id,
            "https://api.alldebrid.com/",
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert!(
        store
            .save_action_secret(&run.id, &context, "account", None, Some("fixture-first"))
            .is_err()
    );
    store.claim_script(true).unwrap().unwrap();
    let other = store.extension_job(&context.id, "disconnect").unwrap();
    assert!(
        store
            .run_site_reader(
                &other.id,
                "https://api.alldebrid.com/",
                &uuid::Uuid::new_v4().to_string()
            )
            .is_err()
    );
    store
        .save_action_secret(&run.id, &context, "account", None, Some("fixture-first"))
        .unwrap();
    assert_eq!(
        store.extension_settings(&context.id).unwrap()["account"],
        Value::Null
    );
    assert!(
        store
            .save_action_secret(&run.id, &context, "account", None, Some("stale"))
            .is_err()
    );
    assert!(
        store
            .save_action_secret(&run.id, &context, "other", None, Some("bad"))
            .is_err()
    );
    store
        .set_extension_secret(&context.id, "account", "fixture-user-replacement")
        .unwrap();
    assert!(
        store
            .save_action_secret(
                &run.id,
                &context,
                "account",
                Some("fixture-first"),
                Some("stale")
            )
            .is_err()
    );
    store.stop_script(&run.id).unwrap();
    assert!(
        store
            .save_action_secret(
                &run.id,
                &context,
                "account",
                Some("fixture-user-replacement"),
                Some("stopped")
            )
            .is_err()
    );
    let restored = store
        .extension_context(&script, Reach::Web)
        .unwrap()
        .unwrap();
    assert_eq!(restored.settings["account"], "fixture-user-replacement");
}

#[test]
fn debrid_premiumize_uses_bearer_key_and_maps_multiple_file_paths() {
    let (result, saved, requests) = run(
        2,
        "connect",
        Some(json!("fixture-pm-key")),
        vec![Step {
            token: Some("fixture-pm-key"),
            ..step(
                "/api/account/info",
                json!({"status":"success","customer_id":"fixture","limit_used":0.42}),
            )
        }],
    );
    assert!(result.unwrap().title.unwrap().contains("42%"));
    assert!(saved.is_empty());
    assert!(!requests[0].url.contains("fixture-pm-key"));
    let (result, _, requests) = run(
        2,
        "generate",
        Some(json!("fixture-pm-key")),
        vec![Step {
            token: Some("fixture-pm-key"),
            ..step(
                "/api/transfer/directdl",
                json!({"status":"success","content":[
                    {"path":"folder/a.bin","size":12,"link":"https://cdn.energycdn.com/a"},
                    {"path":"other/a.bin","size":30,"link":"https://cdn.energycdn.com/b"}
                ]}),
            )
        }],
    );
    let reading = result.unwrap();
    assert_eq!(reading.items[0].filename.as_deref(), Some("a.bin"));
    assert_eq!(reading.items[1].filename.as_deref(), Some("a (2).bin"));
    assert!(reading.single_connection);
    let data = form(requests[0].body.as_ref().unwrap());
    assert_eq!(
        data["src"],
        "https://files.example.test/file?id=1&name=a%20b"
    );
    assert!(!data.contains_key("apikey"));
}

#[test]
fn debrid_premiumize_errors_and_invalid_empty_responses_are_explicit() {
    for body in [
        json!({"status":"error","code":"authentication_failed","message":"PRIVATE-DO-NOT-ECHO"}),
        json!({"status":"error","code":"account_limit_reached","message":"PRIVATE-DO-NOT-ECHO"}),
        json!({"status":"success","content":[]}),
        json!({"status":"success","content":[{"path":"..","link":"https://cdn.energycdn.com/a"}]}),
    ] {
        let (result, saved, _) = run(
            2,
            "generate",
            Some(json!("fixture-pm-key")),
            vec![step("/api/transfer/directdl", body)],
        );
        assert!(
            !result
                .unwrap_err()
                .to_string()
                .contains("PRIVATE-DO-NOT-ECHO")
        );
        assert!(saved.is_empty());
    }
}

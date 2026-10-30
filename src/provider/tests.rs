use super::*;
use crate::{
    paths::AppPaths,
    script_automation::{Fetcher, package},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;

fn plan() -> Plan {
    Plan {
        url: "https://example.invalid/file.bin".into(),
        size: 16,
        input_sha256: "a".repeat(64),
        output_sha256: "b".repeat(64),
        transform_id: "test-v1".into(),
    }
}
fn script() -> Script {
    Script {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 0,
        name: "Provider test".into(),
        language: Language::Lua,
        trigger: Trigger::Provider,
        body: EXAMPLE.into(),
        enabled: false,
        timeout_seconds: 5,
        change_files: false,
        extension: None,
    }
}
fn store(root: &std::path::Path) -> Store {
    Store::open(AppPaths {
        data: root.join("data"),
        downloads: root.join("downloads"),
    })
    .unwrap()
}
#[test]
fn plans_bind_content_and_reject_invalid_fields() {
    let a = plan();
    a.validate().unwrap();
    let mut b = a.clone();
    b.url.push_str("?renewed=yes");
    assert!(a.same_content(&b));
    for field in ["size", "input_sha256", "output_sha256", "transform_id"] {
        let mut changed = serde_json::to_value(&a).unwrap();
        changed[field] = if field == "size" {
            json!(17)
        } else {
            json!("c".repeat(64))
        };
        assert!(!a.same_content(&serde_json::from_value(changed).unwrap()));
    }
    for (field, value) in [
        ("size", json!(0)),
        ("url", json!("file:///C:/private")),
        ("url", json!("https://user:password@example.com")),
        ("input_sha256", json!("A".repeat(64))),
        ("transform_id", json!("")),
    ] {
        let mut bad = serde_json::to_value(&a).unwrap();
        bad[field] = value;
        assert!(
            serde_json::from_value::<Plan>(bad)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}
#[test]
fn queue_freezes_revision_and_import_does_not_execute_or_grant_access() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let saved = store.save_script(&script()).unwrap();
    let job = store
        .add_provider_download(
            &saved.id,
            saved.revision,
            "https://example.invalid/file.bin",
            Some("plain.bin"),
            "main",
            "one",
        )
        .unwrap();
    let repeated = store
        .add_provider_download(
            &saved.id,
            saved.revision,
            &job.url,
            Some("plain.bin"),
            "main",
            "one",
        )
        .unwrap();
    assert_eq!(job.id, repeated.id);
    let error = store
        .begin_browser_recovery(&job.id, "https://example.invalid/page")
        .err()
        .unwrap();
    assert!(error.to_string().contains("direct HTTP/HTTPS"));
    let exported = store
        .export_list(&crate::list_transfer::Scope::All, false)
        .unwrap();
    assert!(exported.entries.is_empty());
    assert_eq!(exported.streams, 1);
    let mut next = saved.clone();
    next.body.push_str("\n-- edited");
    store.save_script(&next).unwrap();
    assert!(
        store
            .add_provider_download(&saved.id, saved.revision, &job.url, None, "main", "two")
            .is_err()
    );
    store.remove_script(&saved.id).unwrap();
    assert_eq!(
        store
            .get(&job.id)
            .unwrap()
            .unwrap()
            .provider
            .unwrap()
            .script
            .body,
        saved.body
    );
    assert_eq!(
        store
            .connect()
            .unwrap()
            .query_row("SELECT data_version FROM profile_format", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    let imported = package::decode(&package::encode(&saved).unwrap())
        .unwrap()
        .script;
    assert_eq!(imported.trigger, Trigger::Provider);
    assert!(!imported.enabled && !imported.change_files && imported.revision == 0);
    let mut bad: serde_json::Value =
        serde_json::from_slice(&package::encode(&saved).unwrap()).unwrap();
    bad["provider_api"] = json!(2);
    assert!(package::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
}
#[test]
fn providers_cannot_be_run_as_automation_or_request_existing_file_access() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let saved = store.save_script(&script()).unwrap();
    assert!(
        store
            .run_script(&saved.id, &uuid::Uuid::new_v4().to_string())
            .is_err()
    );
    let mut bad = saved.clone();
    bad.change_files = true;
    assert!(bad.validate().is_err());
    bad.change_files = false;
    bad.language = Language::PowerShell;
    assert!(bad.validate().is_err());
}

#[test]
fn older_connected_writers_cannot_discard_provider_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let old = store.connect().unwrap();
    for name in ["xdownloader_writer_version", "xdownloader_data_version"] {
        old.create_scalar_function(
            name,
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
            |_| Ok(1_i64),
        )
        .unwrap();
    }
    let saved = store.save_script(&script()).unwrap();
    let job = store
        .add_provider_download(
            &saved.id,
            saved.revision,
            "https://example.invalid/file.bin",
            None,
            "main",
            "old-writer",
        )
        .unwrap();
    assert!(
        old.execute(
            "UPDATE jobs SET data=json_remove(data,'$.provider') WHERE id=?1",
            [&job.id]
        )
        .is_err()
    );
    assert!(store.get(&job.id).unwrap().unwrap().provider.is_some());
}
fn runtime(body: &str) -> runtime::Runtime {
    runtime::Runtime::new(
        body,
        runtime::Capabilities::Processor(uuid::Uuid::new_v4()),
        None,
    )
    .unwrap()
}
fn resolver(body: &str) -> runtime::Runtime {
    runtime::Runtime::new(
        body,
        runtime::Capabilities::Resolver(Box::new(
            Fetcher::new(crate::proxy::Route::direct()).unwrap(),
        )),
        None,
    )
    .unwrap()
}
#[test]
fn processor_has_no_fetch_to_cache_during_source_evaluation() {
    runtime(
        "assert(xd.fetch == nil and not xd.supports('xd.fetch')); return {resolve=function() end, refresh=function() end, transform=function(p,b) return b end, finish=function() return true end}",
    );
}

#[cfg(windows)]
#[test]
fn saved_provider_job_contains_neither_resolved_url_nor_source_literals() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let mut definition = script();
    definition
        .body
        .push_str("\n-- PRIVATE_SOURCE_SENTINEL_98234");
    let saved = store.save_script(&definition).unwrap();
    let mut job = store
        .add_provider_download(
            &saved.id,
            saved.revision,
            "https://example.invalid/input",
            None,
            "main",
            "private-state",
        )
        .unwrap();
    let mut resolved = plan();
    resolved.url.push_str("?token=SIGNED_URL_SENTINEL_27845");
    job.provider.as_mut().unwrap().plan = Some(resolved.content());
    store.save(&job).unwrap();
    let raw: String = store
        .connect()
        .unwrap()
        .query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r.get(0))
        .unwrap();
    assert!(
        !raw.contains("SIGNED_URL_SENTINEL_27845"),
        "Resolved credentials were persisted"
    );
    assert!(
        !raw.contains("PRIVATE_SOURCE_SENTINEL_98234"),
        "Frozen source was persisted as plaintext"
    );
    let restored = store.get(&job.id).unwrap().unwrap();
    assert_eq!(restored.provider.as_ref().unwrap().script.body, saved.body);
    assert_eq!(
        restored.provider.as_ref().unwrap().plan,
        job.provider.as_ref().unwrap().plan
    );
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(value["provider"]["plan"].get("url").is_none());
    assert!(!format!("{:?}", job.provider).contains("PRIVATE_SOURCE_SENTINEL_98234"));
}
#[cfg(windows)]
#[test]
fn production_lua_crypto_matches_nist_and_has_only_documented_capabilities() {
    let mut source = String::from(
        "assert(xd.provider_api == 1 and xd.script_kind == 'provider'); assert(io == nil and os == nil and package == nil and load == nil and print == nil and xd.command == nil and xd.add == nil and xd.store == nil)\n",
    );
    for f in functions() {
        if f.name == "xd.fetch" {
            continue;
        }
        source.push_str(&format!(
            "assert(type({}) == 'function' and xd.supports('{}'))\n",
            f.name, f.name
        ));
    }
    source.push_str("assert(not xd.supports('xd.log')); return {resolve=function() end, refresh=function() end, finish=function() return true end, transform=function(p,b,o) return xd.aes_ctr('2b7e151628aed2a6abf7158809cf4f3c','f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff',b,o) end}");
    let runtime = runtime(&source);
    let cipher = crypto::unhex("874d6191b620e3261bef6864990db6ce").unwrap();
    let plain = runtime
        .handle(json!({"op":"transform","plan":plan(),"offset":0,"bytes":STANDARD.encode(cipher)}))
        .unwrap();
    assert_eq!(
        STANDARD.decode(plain.as_str().unwrap()).unwrap(),
        crypto::unhex("6bc1bee22e409f96e93d7e117393172a").unwrap()
    );
}
#[test]
fn malformed_output_and_recursive_metadata_are_rejected() {
    for result in [
        "return string.rep('x',300000)",
        "return {}",
        "return nil",
        "return 1234567890123456",
    ] {
        let body = format!(
            "return {{resolve=function() end,refresh=function() end,finish=function() return true end,transform=function() {result} end}}"
        );
        assert!(runtime(&body).handle(json!({"op":"transform","plan":plan(),"offset":0,"bytes":STANDARD.encode([0u8;16])})).is_err());
    }
    let body = "return {resolve=function() local t={} t.self=t return t end, refresh=function() end, transform=function() end, finish=function() end}";
    assert!(
        resolver(body)
            .handle(json!({"op":"resolve","url":"https://example.invalid/","previous":null}))
            .is_err()
    );
    for result in ["return 1", "return 'ok'", "return {}", "return nil"] {
        let body = format!(
            "return {{resolve=function() end, refresh=function() end, transform=function() end, finish=function() {result} end}}"
        );
        assert!(
            runtime(&body)
                .handle(json!({"op":"finish","plan":plan()}))
                .is_err()
        );
    }
}

#[test]
fn worker_roles_reject_cross_phase_calls_and_processor_cannot_cache_network() {
    let body = r#"
        local cached_fetch = xd.fetch
        local network = xd.phase == 'resolver'
        assert(xd.supports('xd.fetch') == network)
        assert(network == (type(cached_fetch) == 'function'))
        if network then assert(xd.key == nil and not xd.supports('xd.key')) end
        return {
            resolve=function() error('resolver callback was invoked') end,
            refresh=function() error('refresh callback was invoked') end,
            transform=function(p,b)
                assert(not network and cached_fetch == nil and xd.fetch == nil)
                assert(not pcall(function() return cached_fetch('https://example.invalid/', {method='POST', body=b}) end))
                assert(p.url == nil)
                return b
            end,
            finish=function(p,r) assert(not network and cached_fetch == nil and p.url == nil); return true end,
        }
    "#;
    let processor = runtime(body);
    for previous in [json!(null), json!(plan().content())] {
        let error = processor
            .handle(json!({"op":"resolve","url":"https://example.invalid/","previous":previous}))
            .unwrap_err();
        assert!(error.to_string().contains("cannot resolve or refresh"));
    }
    let message = json!({"op":"transform","plan":plan().content(),"offset":0,"bytes":STANDARD.encode([1u8;16])});
    assert_eq!(
        processor.handle(message.clone()).unwrap(),
        json!(STANDARD.encode([1u8; 16]))
    );
    assert_eq!(
        processor
            .handle(json!({"op":"finish","plan":plan().content()}))
            .unwrap(),
        true
    );
    let resolver = resolver(body);
    assert!(
        resolver
            .handle(message)
            .unwrap_err()
            .to_string()
            .contains("cannot process")
    );
    assert!(
        resolver
            .handle(json!({"op":"finish","plan":plan().content()}))
            .unwrap_err()
            .to_string()
            .contains("cannot finish")
    );
}

#[cfg(windows)]
#[test]
fn opaque_keys_are_bound_to_provider_and_never_expose_bytes_to_lua() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let saved = store.save_script(&script()).unwrap();
    let hex = "2b7e151628aed2a6abf7158809cf4f3c";
    let token = store.protect_provider_key(&saved.id, hex).unwrap();
    assert!(!token.contains(hex));
    let source = format!(
        r#"
        local key
        if xd.phase == 'processor' then
            key = xd.key({})
            assert(type(key) == 'userdata')
            assert(not pcall(xd.to_json, key))
            assert(not tostring(key):find('{}', 1, true))
        end
        return {{resolve=function() end, refresh=function() end,
            transform=function(p,b,o) return xd.aes_ctr(key,'f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff',b,o) end,
            finish=function() return true end}}
    "#,
        serde_json::to_string(&token).unwrap(),
        hex
    );
    let processor = runtime::Runtime::new(
        &source,
        runtime::Capabilities::Processor(uuid::Uuid::parse_str(&saved.id).unwrap()),
        None,
    )
    .unwrap();
    let cipher = crypto::unhex("874d6191b620e3261bef6864990db6ce").unwrap();
    let output = processor.handle(json!({"op":"transform","plan":plan().content(),"bytes":STANDARD.encode(cipher),"offset":0})).unwrap();
    assert_eq!(
        STANDARD.decode(output.as_str().unwrap()).unwrap(),
        crypto::unhex("6bc1bee22e409f96e93d7e117393172a").unwrap()
    );
    assert!(
        runtime::Runtime::new(
            &source,
            runtime::Capabilities::Processor(uuid::Uuid::new_v4()),
            None
        )
        .is_err()
    );
    let mut bytes = STANDARD
        .decode(token.strip_prefix("provider-key-v1:").unwrap())
        .unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let tampered = format!("provider-key-v1:{}", STANDARD.encode(bytes));
    assert!(super::keys::unlock(uuid::Uuid::parse_str(&saved.id).unwrap(), &tampered).is_err());
    assert!(store.protect_provider_key(&saved.id, "0011").is_err());
    assert!(
        store
            .protect_provider_key(&saved.id, "zz".repeat(16).as_str())
            .is_err()
    );
}

#[cfg(windows)]
#[test]
fn frozen_source_rejects_tampering_and_keeps_checkpoint_identity() {
    let mut script = script();
    script.revision = 1;
    let frozen = FrozenScript::new(script.clone()).unwrap();
    let sealed = serde_json::to_value(&frozen).unwrap();
    assert_eq!(
        serde_json::from_value::<FrozenScript>(sealed.clone()).unwrap(),
        script
    );
    let mut wrong_id = sealed.clone();
    wrong_id["id"] = json!(uuid::Uuid::new_v4());
    assert!(serde_json::from_value::<FrozenScript>(wrong_id).is_err());
    let mut changed = sealed;
    changed["sealed"] = json!("invalid ciphertext");
    assert!(serde_json::from_value::<FrozenScript>(changed).is_err());
    let state = State {
        api_version: VERSION,
        script: frozen,
        plan: Some(plan().content()),
    };
    let mut old_plan = plan();
    old_plan.url.clear();
    let expected = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                VERSION,
                &script.body,
                script.revision,
                old_plan,
                CHUNK_BYTES
            ))
            .unwrap()
        )
    );
    assert_eq!(state.binding().unwrap(), expected);
}

#[cfg(windows)]
#[test]
fn legacy_provider_upgrade_preserves_jobs_and_blocks_version_two_writers() {
    let temp = tempfile::tempdir().unwrap();
    let original = store(temp.path());
    let paths = original.paths.clone();
    let mut script = script();
    script.revision = 1;
    script.body.push_str("\n-- LEGACY_PRIVATE_LITERAL");
    let job = original
        .new_download("https://example.invalid/input", None, None, "legacy".into())
        .unwrap();
    let job = original.enqueue_batch(&[job], "main").unwrap().remove(0);
    let mut old = serde_json::to_value(&job).unwrap();
    let mut resolved = plan();
    resolved.url.push_str("?token=LEGACY_SIGNED_TOKEN");
    old["provider"] = json!({"api_version":1,"script":script,"plan":resolved});
    let old_writer = original.connect().unwrap();
    old_writer
        .execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            rusqlite::params![old.to_string(), job.id],
        )
        .unwrap();
    old_writer
        .execute(
            "UPDATE profile_format SET data_version=2,min_writer_version=2",
            [],
        )
        .unwrap();
    for name in ["xdownloader_writer_version", "xdownloader_data_version"] {
        old_writer
            .create_scalar_function(
                name,
                0,
                rusqlite::functions::FunctionFlags::SQLITE_UTF8
                    | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
                |_| Ok(2_i64),
            )
            .unwrap();
    }
    drop(original);
    let upgraded = Store::open(paths).unwrap();
    let restored = upgraded.get(&job.id).unwrap().unwrap();
    assert_eq!(restored.provider.as_ref().unwrap().script, script);
    assert_eq!(
        restored.provider.as_ref().unwrap().plan.as_ref().unwrap(),
        &resolved.content()
    );
    let raw: String = old_writer
        .query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r.get(0))
        .unwrap();
    assert!(!raw.contains("LEGACY_SIGNED_TOKEN") && !raw.contains("LEGACY_PRIVATE_LITERAL"));
    assert!(
        old_writer
            .execute(
                "UPDATE jobs SET data=?1 WHERE id=?2",
                rusqlite::params![old.to_string(), job.id]
            )
            .is_err()
    );
    let before = raw;
    drop(upgraded);
    let reopened = store(temp.path());
    let raw: String = reopened
        .connect()
        .unwrap()
        .query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r.get(0))
        .unwrap();
    assert_eq!(raw, before);
}

#[cfg(windows)]
#[test]
fn failed_provider_upgrade_rolls_back_rows_and_profile_version() {
    let temp = tempfile::tempdir().unwrap();
    let original = store(temp.path());
    let paths = original.paths.clone();
    let conn = original.connect().unwrap();
    let mut definition = script();
    definition.revision = 1;
    let mut before = Vec::new();
    for index in 0..2 {
        let job = original
            .new_download(
                "https://example.invalid/input",
                None,
                None,
                format!("rollback-{index}"),
            )
            .unwrap();
        let job = original.enqueue_batch(&[job], "main").unwrap().remove(0);
        let mut value = serde_json::to_value(&job).unwrap();
        value["provider"] = json!({"api_version":1,"script":definition,"plan":plan()});
        if index == 1 {
            value["provider"]["script"]["revision"] = json!(0);
        }
        let data = value.to_string();
        conn.execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            rusqlite::params![data, job.id],
        )
        .unwrap();
        before.push((job.id, data));
    }
    conn.execute(
        "UPDATE profile_format SET data_version=2,min_writer_version=2",
        [],
    )
    .unwrap();
    drop(original);
    assert!(Store::open(paths).is_err());
    for (id, data) in before {
        assert_eq!(
            conn.query_row("SELECT data FROM jobs WHERE id=?1", [id], |r| r
                .get::<_, String>(0))
                .unwrap(),
            data
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT data_version,min_writer_version FROM profile_format",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        (2, 2)
    );
}

#[cfg(windows)]
#[test]
fn an_extension_processor_unlocks_only_keys_saved_for_its_extension() {
    let scope = uuid::Uuid::new_v4();
    let token = seal_key(scope, "2b7e151628aed2a6abf7158809cf4f3c").unwrap();
    let context = crate::extensions::Context {
        keys: [("content_key".to_owned(), token)].into(),
        key_scope: Some(scope),
        ..Default::default()
    };
    let source = r#"
        local key = xd.key_setting("content_key")
        return {resolve=function() end, refresh=function() end,
            transform=function(_, bytes, offset)
                return xd.aes_ctr(key, "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff", bytes, offset)
            end,
            finish=function() return true end}
    "#;
    let processor = runtime::Runtime::new(
        source,
        runtime::Capabilities::Processor(uuid::Uuid::new_v4()),
        Some(&context),
    )
    .unwrap();
    let cipher = crypto::unhex("874d6191b620e3261bef6864990db6ce").unwrap();
    let output = processor
        .handle(json!({"op":"transform","plan":plan().content(),"bytes":STANDARD.encode(cipher),"offset":0}))
        .unwrap();
    assert_eq!(
        STANDARD.decode(output.as_str().unwrap()).unwrap(),
        crypto::unhex("6bc1bee22e409f96e93d7e117393172a").unwrap()
    );

    let wrong = crate::extensions::Context {
        key_scope: Some(uuid::Uuid::new_v4()),
        ..context.clone()
    };
    assert!(
        runtime::Runtime::new(
            source,
            runtime::Capabilities::Processor(uuid::Uuid::new_v4()),
            Some(&wrong)
        )
        .is_err()
    );
    runtime::Runtime::new(
        "assert(xd.key_setting == nil); return {resolve=function() end, refresh=function() end, transform=function(p,b) return b end, finish=function() return true end}",
        runtime::Capabilities::Resolver(Box::new(
            Fetcher::new(crate::proxy::Route::direct()).unwrap(),
        )),
        Some(&context),
    )
    .unwrap();
}

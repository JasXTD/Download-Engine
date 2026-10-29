use super::*;
use anyhow::Result;
use std::{fs, io::Write, time::Duration};
use tokio_util::sync::CancellationToken;

fn options() -> Options {
    Options {
        chunk_bytes: 4099,
        ..Options::default()
    }
}
fn execute(f: &Fixture, script: &str, opt: &Options) -> Result<Report> {
    run(script, &f.grants, &f.root(), opt, CancellationToken::new())
}
fn pause(f: &Fixture) -> Report {
    execute(
        f,
        SAMPLE,
        &Options {
            pause_after_chunks: Some(2),
            ..options()
        },
    )
    .unwrap()
}
fn replace_transform(body: &str) -> String {
    SAMPLE.replace(
        "return xd.aes_ctr(plan.key_id, plan.counter, bytes, offset)",
        body,
    )
}
fn error(result: Result<Report>, expected: &str) {
    let actual = format!("{:#}", result.unwrap_err());
    assert!(
        actual.contains(expected),
        "Expected {expected:?}, received {actual:?}"
    );
}

#[test]
fn aes_matches_nist_vectors_and_unaligned_ranges() {
    // Independent known-answer tests, NIST SP 800-38A F.5.1 and F.5.5.
    // https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38a.pdf
    let plain = crypto::unhex(concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710"
    ))
    .unwrap();
    let counter = crypto::unhex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff")
        .unwrap()
        .try_into()
        .unwrap();
    for (key, expected) in [
        (
            "2b7e151628aed2a6abf7158809cf4f3c",
            concat!(
                "874d6191b620e3261bef6864990db6ce",
                "9806f66b7970fdff8617187bb9fffdff",
                "5ae4df3edbd5d35e5b4f09020db03eab",
                "1e031dda2fbe03d1792170a0f3009cee"
            ),
        ),
        (
            "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
            concat!(
                "601ec313775789a5b7a7f504bbf3d228",
                "f443e3ca4d62b59aca84e990cacaf5c5",
                "2b0930daa23de94ce87017ba2d84988d",
                "dfc9c58db67aada613c2dd08457941a6"
            ),
        ),
    ] {
        let key = crypto::unhex(key).unwrap();
        let cipher = crypto::unhex(expected).unwrap();
        assert_eq!(crypto::ctr(&key, &counter, &plain, 0).unwrap(), cipher);
        assert_eq!(crypto::ctr(&key, &counter, &cipher, 0).unwrap(), plain);
        for (offset, length) in [(1, 3), (15, 33), (16, 16), (31, 32), (63, 1)] {
            assert_eq!(
                crypto::ctr(
                    &key,
                    &counter,
                    &cipher[offset..offset + length],
                    offset as u64
                )
                .unwrap(),
                plain[offset..offset + length]
            );
        }
    }
}

#[test]
fn crypto_rejects_invalid_sizes_and_counter_wrap() {
    assert!(crypto::ctr(&[0; 15], &[0; 16], b"a", 0).is_err());
    assert!(crypto::ctr(&[0; 16], &[255; 16], &[0; 17], 0).is_err());
    assert!(crypto::ctr(&[0; 16], &[0; 16], &[0; 17], u64::MAX).is_err());
    assert!(crypto::ctr(&[0; 16], &[0; 16], &vec![0; crypto::MAX_CHUNK + 1], 0).is_err());
    assert_eq!(
        crypto::ctr(&[0; 24], &[0; 16], &[], 0).unwrap(),
        Vec::<u8>::new()
    );
    assert!(crypto::unhex("zz").is_err());
    assert!(crypto::unhex("0").is_err());
}

#[test]
fn full_binary_file_decrypts_and_repeat_does_not_duplicate_or_reprocess() {
    let f = Fixture::new(192_031).unwrap();
    let ciphertext = fs::read(f.input()).unwrap();
    let report = execute(&f, SAMPLE, &options()).unwrap();
    assert_eq!(report.state, State::Complete);
    assert_eq!(fs::read(f.root().join("output.bin")).unwrap(), f.plaintext);
    assert_eq!(fs::read(f.input()).unwrap(), ciphertext);
    let report = execute(&f, SAMPLE, &options()).unwrap();
    assert_eq!(report.transformed_chunks, 0);
    assert_eq!(report.resumed_from, f.plaintext.len() as u64);
    let path = f.temp.path().to_owned();
    f.temp.close().unwrap();
    assert!(!path.exists());
}

#[test]
fn pause_then_fresh_runtime_refreshes_reference_and_retains_progress() {
    let mut f = Fixture::new(70_031).unwrap();
    let paused = pause(&f);
    assert_eq!(paused.state, State::Paused);
    assert_eq!(paused.bytes, 8198);
    assert!(!f.root().join("output.bin").exists());
    let prefix = fs::read(f.root().join("output.partial")).unwrap();
    assert_eq!(prefix, f.plaintext[..8198]);
    f.grants.metadata.source = "sample-v2".into();
    f.grants.sources.remove("sample-v1"); // The old reference really is unavailable.
    let report = execute(&f, SAMPLE, &options()).unwrap();
    assert_eq!(report.resumed_from, paused.bytes);
    assert_eq!(
        report.transformed_chunks,
        (f.plaintext.len() - 8198).div_ceil(4099)
    );
    assert_eq!(fs::read(f.root().join("output.bin")).unwrap(), f.plaintext);
}

#[test]
fn interrupted_write_tail_is_discarded_only_after_prefix_verification() {
    let f = Fixture::new(50_003).unwrap();
    pause(&f);
    let partial = f.root().join("output.partial");
    let mut out = fs::OpenOptions::new().append(true).open(&partial).unwrap();
    out.write_all(b"an uncheckpointed tail from a simulated interrupted write")
        .unwrap();
    drop(out);
    execute(&f, SAMPLE, &options()).unwrap();
    assert_eq!(fs::read(f.root().join("output.bin")).unwrap(), f.plaintext);
}

#[test]
fn changed_script_cannot_reuse_saved_progress() {
    let f = Fixture::new(50_003).unwrap();
    pause(&f);
    let before = fs::read(f.root().join("output.partial")).unwrap();
    error(
        execute(&f, &(SAMPLE.to_owned() + "\n-- edited"), &options()),
        "Script changed",
    );
    assert_eq!(fs::read(f.root().join("output.partial")).unwrap(), before);
}

#[test]
fn changed_key_and_processing_settings_cannot_resume() {
    let mut f = Fixture::new(50_003).unwrap();
    pause(&f);
    f.grants.keys.get_mut("sample-key").unwrap()[0] ^= 1;
    error(
        execute(&f, SAMPLE, &options()),
        "Content, key or processing settings changed",
    );
    f.grants.keys.get_mut("sample-key").unwrap()[0] ^= 1;
    error(
        execute(
            &f,
            SAMPLE,
            &Options {
                chunk_bytes: 4096,
                ..options()
            },
        ),
        "processing settings changed",
    );
}

#[test]
fn changed_content_from_refresh_cannot_resume() {
    let mut f = Fixture::new(50_003).unwrap();
    pause(&f);
    f.grants.metadata.output_sha256 = sha256(b"different episode");
    error(
        execute(&f, SAMPLE, &options()),
        "Content, key or processing settings changed",
    );
}

#[test]
fn corrupted_source_and_partial_are_rejected() {
    let f = Fixture::new(50_003).unwrap();
    let original = fs::read(f.input()).unwrap();
    let mut bad = original.clone();
    bad[77] ^= 1;
    fs::write(f.input(), &bad).unwrap();
    error(
        execute(&f, SAMPLE, &options()),
        "Encrypted source hash mismatch",
    );
    fs::write(f.input(), original).unwrap();
    pause(&f);
    let partial = f.root().join("output.partial");
    let mut bad = fs::read(&partial).unwrap();
    bad[41] ^= 1;
    fs::write(&partial, bad).unwrap();
    error(
        execute(&f, SAMPLE, &options()),
        "Checkpoint output hash mismatch",
    );
    assert!(!f.root().join("output.bin").exists());
}

#[test]
fn wrong_key_never_publishes_a_finished_file() {
    let mut f = Fixture::new(8003).unwrap();
    f.grants.keys.get_mut("sample-key").unwrap()[0] ^= 1;
    error(execute(&f, SAMPLE, &options()), "Plaintext hash mismatch");
    assert!(!f.root().join("output.bin").exists());
}

#[test]
fn host_verifies_plaintext_even_when_script_claims_success() {
    let f = Fixture::new(8003).unwrap();
    let script = replace_transform("return bytes");
    error(execute(&f, &script, &options()), "Plaintext hash mismatch");
    assert!(!f.root().join("output.bin").exists());
}

#[test]
fn custom_lua_byte_processing_works_without_native_crypto_calls() {
    let mut f = Fixture::new(17_019).unwrap();
    // Deliberately simple XOR obfuscation, NOT a second cryptographic scheme.
    let encoded: Vec<u8> = f
        .plaintext
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ ((i * 7 + 19) % 256) as u8)
        .collect();
    fs::write(f.input(), &encoded).unwrap();
    f.grants.metadata.input_sha256 = sha256(&encoded);
    f.grants.metadata.transform_id = "custom-lua-xor-demo".into();
    let script = replace_transform(
        r#"
        local out = {}
        for i = 1, #bytes do
            out[i] = string.char(string.byte(bytes, i) ~ (((offset+i-1)*7+19) % 256))
        end
        return table.concat(out)
    "#,
    );
    execute(
        &f,
        &script,
        &Options {
            pause_after_chunks: Some(2),
            ..options()
        },
    )
    .unwrap();
    execute(&f, &script, &options()).unwrap();
    assert_eq!(fs::read(f.root().join("output.bin")).unwrap(), f.plaintext);
}

#[test]
fn lua_implements_counter_mode_and_resumes_without_the_host_ctr_decoder() {
    let f = Fixture::new(196_033).unwrap();
    let source = include_str!("../lua-provider-custom-ctr.lua");
    assert!(!source.contains("xd.aes_ctr"));
    let paused = execute(
        &f,
        source,
        &Options {
            pause_after_chunks: Some(2),
            ..options()
        },
    )
    .unwrap();
    let resumed = execute(&f, source, &options()).unwrap();
    assert_eq!(resumed.resumed_from, paused.bytes);
    assert_eq!(fs::read(f.root().join("output.bin")).unwrap(), f.plaintext);
}

#[test]
fn lua_counter_mode_matches_independent_nist_ciphertext() {
    let mut f = Fixture::new(64).unwrap();
    let plaintext = crypto::unhex(concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710"
    ))
    .unwrap();
    let ciphertext = crypto::unhex(concat!(
        "601ec313775789a5b7a7f504bbf3d228",
        "f443e3ca4d62b59aca84e990cacaf5c5",
        "2b0930daa23de94ce87017ba2d84988d",
        "dfc9c58db67aada613c2dd08457941a6"
    ))
    .unwrap();
    fs::write(f.input(), &ciphertext).unwrap();
    f.grants.metadata.counter = "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff".into();
    f.grants.metadata.input_sha256 = sha256(&ciphertext);
    f.grants.metadata.output_sha256 = sha256(&plaintext);
    execute(
        &f,
        include_str!("../lua-provider-custom-ctr.lua"),
        &Options {
            chunk_bytes: 17,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(fs::read(f.root().join("output.bin")).unwrap(), plaintext);
}

#[test]
fn cancelling_busy_lua_keeps_checkpoint_and_does_not_publish() {
    let f = Fixture::new(8003).unwrap();
    let token = CancellationToken::new();
    let stop = token.clone();
    let result = runtime::observe_hooks(
        move |stage| {
            if stage == "transform" {
                stop.cancel();
            }
        },
        || {
            run(
                &replace_transform("while true do end"),
                &f.grants,
                &f.root(),
                &options(),
                token,
            )
        },
    );
    let result = result.unwrap();
    assert_eq!(result.state, State::Paused);
    assert_eq!(result.bytes, 0);
    assert!(!f.root().join("output.bin").exists());
}

#[test]
fn callback_timeout_and_memory_limit_are_enforced() {
    for (script, expected) in [
        (replace_transform("while true do end"), "timed out"),
        (
            replace_transform("return string.rep('x', 32 * 1024 * 1024)"),
            "memory",
        ),
    ] {
        let f = Fixture::new(1003).unwrap();
        error(
            execute(
                &f,
                &script,
                &Options {
                    callback_budget: Duration::from_millis(50),
                    ..options()
                },
            ),
            expected,
        );
        assert!(!f.root().join("output.bin").exists());
    }
}

#[test]
fn cancellation_during_final_callback_preserves_verified_checkpoint() {
    let f = Fixture::new(1003).unwrap();
    let source = SAMPLE.replace("return true", "while true do end");
    let token = CancellationToken::new();
    let stop = token.clone();
    let result = runtime::observe_hooks(
        move |stage| {
            if stage == "finish" {
                stop.cancel();
            }
        },
        || run(&source, &f.grants, &f.root(), &options(), token),
    );
    let result = result.unwrap();
    assert_eq!(result.state, State::Paused);
    assert_eq!(result.bytes, 1003);
    assert!(!f.root().join("output.bin").exists());
    assert_eq!(
        fs::read(f.root().join("output.partial")).unwrap(),
        f.plaintext
    );
}

#[test]
fn malformed_transforms_and_finish_errors_do_not_publish() {
    for (body, message) in [
        ("return {}", "binary string"),
        ("return bytes .. 'extra'", "preserve chunk length"),
    ] {
        let f = Fixture::new(1003).unwrap();
        error(execute(&f, &replace_transform(body), &options()), message);
    }
    let f = Fixture::new(1003).unwrap();
    error(
        execute(
            &f,
            &SAMPLE.replace("return true", "return false"),
            &options(),
        ),
        "finish did not accept",
    );
    assert!(!f.root().join("output.bin").exists());
}

#[test]
fn runtime_has_no_system_network_or_implicit_key_access() {
    let f = Fixture::new(1003).unwrap();
    let checked = format!(
        "assert(io == nil and os == nil and package == nil and debug == nil)\nassert(load == nil and dofile == nil and pcall == nil and coroutine == nil)\nassert(xd.fetch == nil and xd.command == nil)\n{SAMPLE}"
    );
    execute(&f, &checked, &options()).unwrap();
    let f = Fixture::new(1003).unwrap();
    error(
        execute(
            &f,
            &SAMPLE.replace("xd.aes_ctr(plan.key_id", "xd.aes_ctr('other-key'"),
            &options(),
        ),
        "Key is not granted",
    );
    let f = Fixture::new(1003).unwrap();
    error(
        execute(
            &f,
            &SAMPLE.replace("xd.metadata(context.asset)", "xd.metadata('private')"),
            &options(),
        ),
        "Asset is not granted",
    );
}

#[test]
fn source_plan_cannot_name_an_arbitrary_file() {
    let f = Fixture::new(1003).unwrap();
    let script = SAMPLE.replace(
        "return xd.metadata(context.asset)",
        "local p = xd.metadata(context.asset); p.source = 'not-granted'; return p",
    );
    error(execute(&f, &script, &options()), "Source is not granted");
    let script = script.replace("not-granted", "../../outside");
    error(
        execute(&f, &script, &options()),
        "Invalid provider identifier",
    );
}

#[test]
fn unrelated_output_and_linked_source_are_preserved() {
    let f = Fixture::new(1003).unwrap();
    fs::write(f.root().join("output.bin"), b"someone else's file").unwrap();
    error(
        execute(&f, SAMPLE, &options()),
        "Unowned output already exists",
    );
    assert_eq!(
        fs::read(f.root().join("output.bin")).unwrap(),
        b"someone else's file"
    );
    let f = Fixture::new(1003).unwrap();
    fs::hard_link(f.input(), f.temp.path().join("linked.bin")).unwrap();
    error(execute(&f, SAMPLE, &options()), "Unexpected lab hard link");
}

#[test]
fn malformed_or_missing_checkpoint_data_is_not_silently_restarted() {
    let f = Fixture::new(50_003).unwrap();
    pause(&f);
    fs::remove_file(f.root().join("output.partial")).unwrap();
    error(
        execute(&f, SAMPLE, &options()),
        "Checkpoint output is missing",
    );
    let f = Fixture::new(50_003).unwrap();
    pause(&f);
    let file = f.root().join("checkpoint.json");
    let mut cp: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    cp["offset"] = 12.into();
    fs::write(&file, serde_json::to_vec(&cp).unwrap()).unwrap();
    error(execute(&f, SAMPLE, &options()), "completed chunk boundary");
}

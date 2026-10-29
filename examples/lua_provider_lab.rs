//! Run with `cargo run --offline --locked --example lua_provider_lab`.
//! Everything happens in a disposable local folder; nothing is downloaded.
#[cfg(windows)]
mod provider_lab;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::ensure;
    use provider_lab::{Fixture, Options, State, run};
    use std::{io::Read, path::PathBuf};
    use tokio_util::sync::CancellationToken;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let path = match args.as_slice() {
        [] => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/lua-provider.lua"),
        [flag, path] if flag == "--script" => PathBuf::from(path),
        _ => anyhow::bail!("Usage: lua_provider_lab [--script path-to-provider.lua]"),
    };
    let mut source = String::new();
    std::fs::File::open(path)?
        .take(65_537)
        .read_to_string(&mut source)?;
    ensure!(source.len() <= 65_536, "Provider script exceeds 64 KiB");
    let mut sample = Fixture::new(2 * 1024 * 1024 + 37)?;
    let root = sample.root();
    let result = (|| -> anyhow::Result<_> {
        let before = std::fs::read(sample.input())?;
        let paused = run(
            &source,
            &sample.grants,
            &root,
            &Options {
                pause_after_chunks: Some(3),
                ..Options::default()
            },
            CancellationToken::new(),
        )?;
        ensure!(
            paused.state == State::Paused && !root.join("output.bin").exists(),
            "Pause did not preserve a partial run"
        );
        // Models an expired source reference, not an HTTP or live-service test.
        sample.grants.metadata.source = "sample-v2".into();
        sample.grants.sources.remove("sample-v1");
        let resumed = run(
            &source,
            &sample.grants,
            &root,
            &Options::default(),
            CancellationToken::new(),
        )?;
        ensure!(
            resumed.state == State::Complete && resumed.resumed_from == paused.bytes,
            "Resume did not use the checkpoint"
        );
        ensure!(
            std::fs::read(root.join("output.bin"))? == sample.plaintext,
            "Plaintext differs"
        );
        ensure!(
            std::fs::read(sample.input())? == before,
            "Encrypted input changed"
        );
        let repeated = run(
            &source,
            &sample.grants,
            &root,
            &Options::default(),
            CancellationToken::new(),
        )?;
        ensure!(
            repeated.transformed_chunks == 0,
            "Completed run was processed again"
        );
        Ok(serde_json::json!({
            "experiment": "offline Lua provider", "network_requests": 0,
            "script_sha256": provider_lab::sha256(source.as_bytes()),
            "encryption": "AES-256-CTR via Windows CNG", "paused": paused,
            "resumed": resumed, "repeat": repeated,
            "plaintext_matches": true, "encrypted_source_unchanged": true
        }))
    })();
    let mut report = match (result, sample.temp.close()) {
        (Ok(report), Ok(())) => report,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(cleanup)) => return Err(cleanup.into()),
        (Err(error), Err(cleanup)) => {
            anyhow::bail!("Demo failed: {error:#}; cleanup also failed: {cleanup}")
        }
    };
    report["sample_files_cleaned"] = true.into();
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("The offline provider lab currently requires Windows CNG")
}

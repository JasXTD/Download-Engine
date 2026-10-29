//! Console entry point. stdout is one JSON document per invocation (except help/version).
#[path = "../desktop_process.rs"]
mod desktop_process;
use anyhow::{Context, Result, bail, ensure};
use download_engine::{
    automation::{Rule, Workflow},
    control::{self, Action},
    paths::AppPaths,
    store::Store,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    time::{Duration, Instant},
};

const HELP: &str = concat!(
    "Download Engine CLI ",
    env!("CARGO_PKG_VERSION"),
    "
Usage: de-cli COMMAND [arguments] [options]
  mcp [--port N] [--read-only] [--no-start] Serve MCP at http://127.0.0.1:17842/mcp
  mcp-config [--port N] Print HTTP client settings, including the private token
  mcp-approve-script ID REVISION  Approve the reviewed script revision for MCP
  mcp-revoke-script ID          Revoke its MCP execution approval
MCP controls: --no-download-control, --no-queue-control, --allow-lua,
              --allow-powershell (scripts also need local revision approval)
  list                         List downloads
  status ID                    Read one download
  add URL... [--queue NAME|ID] [--folder ABSOLUTE_PATH] [--name FILE]
  batch FILE                   Add URLs from a UTF-8 file (one per line)
  export FILE [--queue NAME|ID] Save downloads to a new .json list or .txt links file
  pause ID | resume ID | pause-all
  retry ID                     Try again a download that needs attention
  restart ID                   Start a download over
  rename ID NAME | move ID FOLDER  Rename or move a finished download's file
  remove ID [--delete-file]    Remove a download from the list (and its finished file)
  queues                       List queues
  queue create NAME | start NAME|ID | pause NAME|ID | limit NAME|ID 1..32
  automation list | history
  automation route NAME --queue NAME|ID [--host example.com] [--extension zip] [--speed-kib 1024]
  automation chain NAME --source NAME|ID --queue NAME|ID
  automation enable ID | disable ID | remove ID
  scripts list | show ID | save JSON_FILE | remove ID
  scripts download ID URL [--name FILE] [--queue ID]  Add using a saved Lua provider
  scripts protect-key ID        Read an AES hex key from redirected stdin; return a protected token for xd.key
  scripts enable ID | disable ID | run ID | stop RUN_ID
  scripts sample ID [DOWNLOAD_ID...]  Run an event script with chosen downloads; changes are simulated
  scripts runs [SCRIPT_ID] | logs RUN_ID
  scripts state SCRIPT_ID [clear]  Show or clear what a script keeps between runs
Inside a running script only:
  notify TITLE [TEXT]          Show a Windows notification (at most 5 per run)
  state get KEY | set KEY JSON | delete KEY | keys
  request ID                   Read a command receipt
  cancel ID                    Cancel a command that has not started
Options: --json (default), --wait SECONDS (1..120, default 15),
         --request-id UUID (safe retry of the same command), --no-start
Mutations launch the desktop when needed. Reads do not. Rules run while the app
is running, including in the tray. Pause acknowledges the request; poll status
until paused before acting on a file. See docs/automation.md for examples."
);

struct Args {
    words: Vec<String>,
    options: BTreeMap<String, String>,
    no_start: bool,
    delete_file: bool,
}
impl Args {
    fn parse(raw: Vec<String>) -> Result<Self> {
        let mut a = Self {
            words: Vec::new(),
            options: BTreeMap::new(),
            no_start: false,
            delete_file: false,
        };
        let mut iter = raw.into_iter();
        while let Some(v) = iter.next() {
            match v.as_str() {
                "--json" => {}
                "--no-start" => a.no_start = true,
                "--delete-file" => a.delete_file = true,
                "--queue" | "--folder" | "--name" | "--host" | "--extension" | "--source"
                | "--speed-kib" | "--wait" | "--request-id" => {
                    let value = iter
                        .next()
                        .with_context(|| format!("Missing value for {v}"))?;
                    ensure!(!value.starts_with("--"), "Missing value for {v}");
                    ensure!(
                        a.options.insert(v.clone(), value).is_none(),
                        "Duplicate option {v}"
                    );
                }
                _ if v.starts_with("--") => bail!("Unknown option {v}"),
                _ => a.words.push(v),
            }
        }
        Ok(a)
    }
    fn take(&mut self, key: &str) -> Option<String> {
        self.options.remove(key)
    }
    fn required(&mut self, key: &str) -> Result<String> {
        self.take(key).with_context(|| format!("Missing {key}"))
    }
    fn finish(&self) -> Result<()> {
        ensure!(
            self.options.is_empty(),
            "Unexpected option(s): {}",
            self.options.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--provider-worker"] {
        if download_engine::provider::worker_main().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if matches!(args.first().map(String::as_str), Some("mcp" | "mcp-config")) {
        if let Err(error) = mcp(&args) {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }
    if args == ["--lua-worker"] {
        if let Err(error) = lua_worker() {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.is_empty() || args == ["--help"] || args == ["help"] {
        println!("{HELP}");
        return;
    }
    if args == ["--version"] {
        println!("Download Engine {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    match run(args) {
        Ok((value, code)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("JSON value")
            );
            std::process::exit(code);
        }
        Err(error) => {
            eprintln!("{}", json!({"ok":false,"error":format!("{error:#}")}));
            std::process::exit(1);
        }
    }
}

fn mcp(args: &[String]) -> Result<()> {
    let mut port = download_engine::mcp::DEFAULT_PORT;
    let (mut read_only, mut no_start) = (false, false);
    let mut permissions = download_engine::mcp::Permissions::default();
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--port" => {
                port = options
                    .next()
                    .context("--port needs a number")?
                    .parse::<u16>()?;
                ensure!(port != 0, "MCP port must be 1..65535");
            }
            "--read-only" if args[0] == "mcp" => read_only = true,
            "--no-start" if args[0] == "mcp" => no_start = true,
            "--no-download-control" if args[0] == "mcp" => permissions.downloads = false,
            "--no-queue-control" if args[0] == "mcp" => permissions.queues = false,
            "--allow-lua" if args[0] == "mcp" => permissions.lua = true,
            "--allow-powershell" if args[0] == "mcp" => permissions.powershell = true,
            _ => anyhow::bail!(
                "MCP options: --port NUMBER; mcp also accepts --read-only, --no-start, --no-download-control, --no-queue-control, --allow-lua and --allow-powershell. See docs/mcp.md."
            ),
        }
    }
    ensure!(
        std::env::var_os("IDM_EX_SCRIPT_RUN").is_none(),
        "MCP cannot run inside an automation worker"
    );
    if args[0] == "mcp-config" {
        println!(
            "{}",
            serde_json::to_string_pretty(&download_engine::mcp::client_config(
                &AppPaths::discover()?,
                port
            )?)?
        );
        return Ok(());
    }
    let _installation = download_engine::paths::installation_guard()?;
    download_engine::mcp::run(
        download_engine::mcp::Context {
            store: Store::open(AppPaths::discover()?)?,
            read_only,
            permissions,
            no_start,
            start: desktop_process::ensure_running,
        },
        port,
    )
}

fn lua_worker() -> Result<()> {
    download_engine::model::validate_job_id(&std::env::var("IDM_EX_SCRIPT_RUN")?)?;
    let read = |name: &str, max: usize| -> Result<String> {
        let mut source = String::new();
        std::fs::File::open(std::env::var(name)?)?
            .take(max as u64 + 1)
            .read_to_string(&mut source)?;
        ensure!(source.len() <= max, "Script input exceeds its size limit");
        Ok(source)
    };
    let source = read(
        "XDOWNLOADER_SCRIPT_FILE",
        download_engine::script_automation::MAX_SCRIPT_BYTES,
    )?;
    let event: Value = serde_json::from_str(&read("XDOWNLOADER_EVENT_FILE", 2 * 1024 * 1024)?)?;
    if event["trigger"] == "site_reader" {
        return site_reader(&source, &event);
    }
    download_engine::script_automation::execute_lua(&source, &event, |args| {
        let (response, code) = run(args)?;
        ensure!(code == 0, "Download Engine command failed: {response}");
        // A sample run's "Would …" line, or the notification just shown.
        if let Some(note) = response.get("note").and_then(Value::as_str) {
            println!("{note}");
        }
        Ok(response
            .get("result")
            .or_else(|| response.get("receipt").and_then(|r| r.get("result")))
            .cloned()
            .unwrap_or(Value::Null))
    })
}

/// A site reader gets xd.fetch (through the profile's proxy settings) instead of
/// app commands, and leaves what it found where the scheduler collects it.
fn site_reader(source: &str, event: &Value) -> Result<()> {
    use download_engine::script_automation::{Fetcher, Reading, execute_reader};
    let paths: AppPaths = serde_json::from_str(&std::env::var("IDM_EX_SCRIPT_PROFILE")?)?;
    ensure!(
        paths.data.is_absolute() && paths.downloads.is_absolute(),
        "Script profile paths must be absolute"
    );
    let route = download_engine::proxy::Route::load(&Store::open(paths)?)?;
    let mut fetcher = Fetcher::new(route)?;
    let value = execute_reader(source, event, move |request| fetcher.fetch(request))?;
    let reading = Reading::from_value(value)?;
    std::fs::write(
        std::env::var("XDOWNLOADER_RESULT_FILE")?,
        serde_json::to_vec(&reading)?,
    )
    .context("Cannot save the site reader's result")
}

fn run(raw: Vec<String>) -> Result<(Value, i32)> {
    let mut args = Args::parse(raw)?;
    let wait = args
        .take("--wait")
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(15);
    ensure!((1..=120).contains(&wait), "--wait must be 1..120 seconds");
    let request = args.take("--request-id");
    if let Some(id) = &request {
        download_engine::model::validate_job_id(id)?;
    }
    let _installation = download_engine::paths::installation_guard()?;
    // A script inherits the desktop's exact profile, including portable custom
    // folders. Its CLI children must never launch a second desktop outside the job.
    let script_run = std::env::var("IDM_EX_SCRIPT_RUN").ok();
    let paths = if let Some(run) = &script_run {
        download_engine::model::validate_job_id(run)?;
        args.no_start = true;
        let paths: AppPaths = serde_json::from_str(&std::env::var("IDM_EX_SCRIPT_PROFILE")?)?;
        ensure!(
            paths.data.is_absolute() && paths.downloads.is_absolute(),
            "Script profile paths must be absolute"
        );
        paths
    } else {
        AppPaths::discover()?
    };
    let store = Store::open(paths.clone())?;
    let words = args.words.clone();
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    if let ["scripts", "protect-key", id] = w.as_slice() {
        use std::io::IsTerminal;
        args.finish()?;
        ensure!(
            !args.delete_file && request.is_none(),
            "Key protection does not accept queued-command options"
        );
        ensure!(
            script_run.is_none(),
            "Automation workers cannot provision provider keys"
        );
        ensure!(
            !std::io::stdin().is_terminal(),
            "Supply the AES hex key through redirected stdin, never a command argument"
        );
        let mut key = zeroize::Zeroizing::new(String::new());
        std::io::stdin().lock().take(67).read_to_string(&mut key)?;
        ensure!(key.len() <= 66, "AES key input is too long");
        let token = store.protect_provider_key(id, key.trim())?;
        return Ok((json!({"ok":true,"result":{"token":token}}), 0));
    }
    ensure!(
        !args.delete_file || matches!(w.as_slice(), ["remove", _]),
        "--delete-file is for remove"
    );
    if matches!(
        w.first(),
        Some(&"mcp-approve-script" | &"mcp-revoke-script")
    ) {
        args.finish()?;
        ensure!(
            script_run.is_none(),
            "Automation workers cannot grant or revoke MCP script approvals"
        );
        ensure!(
            request.is_none(),
            "MCP approvals are local settings, not queued app commands"
        );
        match w.as_slice() {
            ["mcp-approve-script", id, revision] => {
                store.approve_mcp_script(id, revision.parse()?)?
            }
            ["mcp-revoke-script", id] => store.revoke_mcp_script(id)?,
            _ => bail!("Use mcp-approve-script ID REVISION or mcp-revoke-script ID"),
        }
        return Ok((json!({"ok":true}), 0));
    }
    // What a script keeps between runs. Only a running script has a store.
    if let ["state", rest @ ..] = w.as_slice() {
        args.finish()?;
        ensure!(request.is_none(), "--request-id is for app commands");
        let run = script_run.as_deref().context(
            "state is available inside a running script only; use scripts state SCRIPT_ID",
        )?;
        return script_state(&store, run, rest);
    }
    // Export writes a file, so every option is checked before anything is written.
    if let ["export", file] = w.as_slice() {
        let queue = args.take("--queue");
        args.finish()?;
        ensure!(request.is_none(), "--request-id is for mutations");
        return Ok((json!({"ok":true,"result":export(&store, queue, file)?}), 0));
    }
    let read = match w.as_slice() {
        ["list"] => Some(Value::Array(
            store
                .list()?
                .iter()
                .map(|j| control::job_summary(&store, j))
                .collect::<Result<_>>()?,
        )),
        ["status", id] => Some(control::job_summary(
            &store,
            &store.get(id)?.context("Download not found")?,
        )?),
        ["queues"] => Some(control::queue_summaries(&store)?),
        ["automation", "list"] => Some(serde_json::to_value(store.automation_rules()?)?),
        ["automation", "history"] => Some(serde_json::to_value(store.automation_history()?)?),
        ["scripts", "list"] => Some(serde_json::to_value(store.scripts()?)?),
        ["scripts", "show", id] => Some(serde_json::to_value(store.script(id)?)?),
        ["scripts", "runs"] => Some(serde_json::to_value(store.script_runs(None)?)?),
        ["scripts", "runs", id] => Some(serde_json::to_value(store.script_runs(Some(id))?)?),
        ["scripts", "logs", id] => Some(serde_json::to_value(store.script_logs(id)?)?),
        ["scripts", "state", id] => {
            store.script(id)?;
            Some(serde_json::to_value(store.script_values(id)?)?)
        }
        ["request", id] => Some(serde_json::to_value(store.control_receipt(id)?)?),
        ["cancel", id] => {
            args.finish()?;
            ensure!(request.is_none(), "--request-id is for mutations");
            if let Some(run) = &script_run {
                ensure!(
                    !store.script_for_run(run)?.1,
                    "A sample run cannot cancel real pending commands"
                );
            }
            Some(serde_json::to_value(store.cancel_control(id)?)?)
        }
        _ => None,
    };
    if let Some(value) = read {
        args.finish()?;
        ensure!(request.is_none(), "--request-id is for mutations");
        return Ok((json!({"ok":true,"result":value}), 0));
    }
    let action = match w.as_slice() {
        ["add", urls @ ..] if !urls.is_empty() => add(
            &mut args,
            urls.iter().map(|s| s.to_string()).collect(),
            script_run.clone(),
        )?,
        ["batch", file] => {
            let mut text = String::new();
            std::fs::File::open(file)?
                .take(1_000_001)
                .read_to_string(&mut text)?;
            ensure!(text.len() <= 1_000_000, "Batch file is too large");
            add(
                &mut args,
                text.trim_start_matches('\u{feff}')
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                script_run.clone(),
            )?
        }
        ["pause", id] => Action::Pause { id: (*id).into() },
        ["resume", id] => Action::Resume { id: (*id).into() },
        ["pause-all"] => Action::PauseAll,
        ["retry", id] => Action::Retry {
            id: (*id).into(),
            script_run: script_run.clone(),
        },
        ["restart", id] => Action::Restart {
            id: (*id).into(),
            script_run: script_run.clone(),
        },
        ["rename", id, name] => Action::Rename {
            id: (*id).into(),
            name: (*name).into(),
            script_run: script_run.clone(),
        },
        ["move", id, folder] => Action::MoveFile {
            id: (*id).into(),
            folder: PathBuf::from(folder),
            script_run: script_run.clone(),
        },
        ["remove", id] => Action::Remove {
            id: (*id).into(),
            delete_file: args.delete_file,
            script_run: script_run.clone(),
        },
        ["notify", title, text @ ..] if text.len() <= 1 => Action::Notify {
            title: control::clip(title, control::NOTICE_TITLE_CHARS),
            text: control::clip(
                text.first().copied().unwrap_or(""),
                control::NOTICE_TEXT_CHARS,
            ),
            script_run: script_run
                .clone()
                .context("notify is available inside a running script only")?,
        },
        ["scripts", "state", id, "clear"] => Action::ClearScriptValues { id: (*id).into() },
        ["queue", "create", name] => Action::CreateQueue {
            name: (*name).into(),
        },
        ["queue", "start", queue] => Action::StartQueue {
            queue: (*queue).into(),
        },
        ["queue", "pause", queue] => Action::PauseQueue {
            queue: (*queue).into(),
        },
        ["queue", "limit", queue, limit] => Action::QueueLimit {
            queue: (*queue).into(),
            limit: limit.parse()?,
        },
        ["automation", kind @ ("route" | "chain"), name] => {
            let queue = control::queue_id(&store, &args.required("--queue")?)?;
            let workflow = if *kind == "route" {
                let speed = args
                    .take("--speed-kib")
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .map(|v| v.checked_mul(1024).context("Speed is too large"))
                    .transpose()?;
                Workflow::DownloadAdded {
                    host: args
                        .take("--host")
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_lowercase(),
                    extension: args
                        .take("--extension")
                        .unwrap_or_default()
                        .trim_start_matches('.')
                        .to_ascii_lowercase(),
                    queue,
                    bytes_per_second: speed,
                }
            } else {
                Workflow::QueueCompleted {
                    source: control::queue_id(&store, &args.required("--source")?)?,
                    start: queue,
                }
            };
            Action::SaveRule {
                rule: Rule {
                    id: request
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    name: (*name).into(),
                    enabled: true,
                    workflow,
                },
            }
        }
        ["automation", mode @ ("enable" | "disable"), id] => {
            let mut rule = store
                .automation_rules()?
                .into_iter()
                .find(|r| r.id == *id)
                .context("Rule not found")?;
            rule.enabled = *mode == "enable";
            Action::SaveRule { rule }
        }
        ["automation", "remove", id] => Action::RemoveRule { id: (*id).into() },
        ["scripts", "save", file] => {
            let mut text = String::new();
            std::fs::File::open(file)?
                .take(400_001)
                .read_to_string(&mut text)?;
            ensure!(text.len() <= 400_000, "Script definition is too large");
            Action::SaveScript {
                script: serde_json::from_str(text.trim_start_matches('\u{feff}'))?,
            }
        }
        ["scripts", mode @ ("enable" | "disable"), id] => {
            let mut script = store.script(id)?;
            script.enabled = *mode == "enable";
            Action::SaveScript { script }
        }
        ["scripts", "remove", id] => Action::RemoveScript { id: (*id).into() },
        ["scripts", "download", id, url] => {
            ensure!(
                script_run.is_none(),
                "Automation workers cannot start provider downloads"
            );
            let script = store.script(id)?;
            Action::AddProvider {
                script: (*id).into(),
                revision: script.revision,
                url: (*url).into(),
                name: args.take("--name"),
                queue: args.take("--queue").unwrap_or_else(|| "main".into()),
            }
        }
        ["scripts", "run", id] => Action::RunScript { id: (*id).into() },
        ["scripts", "sample", id, downloads @ ..] => Action::RunScriptSample {
            id: (*id).into(),
            downloads: downloads.iter().map(|s| s.to_string()).collect(),
        },
        ["scripts", "stop", run_id] => Action::StopScript {
            run_id: (*run_id).into(),
        },
        _ => bail!("Unknown command or wrong argument count. Use --help."),
    };
    args.finish()?;
    // A sample run reports what it would do instead of changing anything.
    if let Some(run) = &script_run
        && store.script_for_run(run)?.1
    {
        control::check_script_files(&store, &action)?;
        if let Action::Retry { id, .. }
        | Action::Restart { id, .. }
        | Action::Rename { id, .. }
        | Action::MoveFile { id, .. }
        | Action::Remove { id, .. }
        | Action::Pause { id }
        | Action::Resume { id } = &action
        {
            store.get(id)?.context("Download not found")?;
        }
        let note = control::simulated(&store, &action);
        return Ok((
            json!({"ok":true,"simulated":true,"note":note,"result":null}),
            0,
        ));
    }
    if !args.no_start {
        desktop_process::ensure_running(&paths)?;
    }
    let id = request.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut receipt = store.submit_control(&id, &action)?;
    let deadline = Instant::now() + Duration::from_secs(wait);
    while matches!(receipt.state.as_str(), "pending" | "running") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        receipt = store.control_receipt(&id)?;
    }
    if receipt.state == "pending" {
        receipt = store.cancel_control(&id)?;
    }
    let ok = receipt.state == "done";
    let code = if ok {
        0
    } else if matches!(
        receipt.state.as_str(),
        "running" | "interrupted" | "retired"
    ) {
        2
    } else {
        1
    };
    let mut response = json!({"ok":ok,"receipt":receipt});
    // The script's log shows each notification where it was sent.
    if ok && let Action::Notify { title, text, .. } = &action {
        response["note"] = if text.is_empty() {
            json!(format!("Notification: {title}"))
        } else {
            json!(format!("Notification: {title} · {text}"))
        };
    }
    Ok((response, code))
}

/// `state get KEY | set KEY JSON | delete KEY | keys` for the running script.
/// Sample runs read stored values but do not change them.
fn script_state(store: &Store, run: &str, words: &[&str]) -> Result<(Value, i32)> {
    let (script, sample) = store.script_for_run(run)?;
    let result = match words {
        ["get", key] => store.script_value(&script.id, key)?.unwrap_or(Value::Null),
        ["keys"] => Value::Array(
            store
                .script_values(&script.id)?
                .into_iter()
                .map(|v| Value::String(v.key))
                .collect(),
        ),
        [verb @ ("set" | "delete"), key, rest @ ..] => {
            let value = match (*verb, rest) {
                ("set", [json]) => serde_json::from_str(json.trim_start_matches('\u{feff}'))
                    .context("state set expects a JSON value, such as 42, \"text\" or {\"a\":1}")?,
                ("delete", []) => Value::Null,
                _ => bail!("Use state set KEY JSON or state delete KEY"),
            };
            if sample {
                let note = format!("Would store {key} (sample run, nothing changed)");
                return Ok((
                    json!({"ok":true,"simulated":true,"note":note,"result":null}),
                    0,
                ));
            }
            store.set_script_value(&script.id, key, &value)?;
            Value::Null
        }
        _ => bail!("Use state get KEY, state set KEY JSON, state delete KEY or state keys"),
    };
    Ok((json!({"ok":true,"result":result}), 0))
}

/// Export reads the store and writes a new file; it never replaces an existing one.
fn export(store: &Store, queue: Option<String>, file: &str) -> Result<Value> {
    use download_engine::list_transfer::{self as lists, Format, Scope};
    let path = std::path::absolute(file)?;
    let format = Format::from_path(&path).context("Use a file name ending in .json or .txt")?;
    let scope = match queue {
        Some(queue) => Scope::Queue(control::queue_id(store, &queue)?),
        None => Scope::All,
    };
    let collected = store.export_list(&scope, false)?;
    lists::write_file(&path, &lists::render(&collected.entries, format)?, false)?;
    Ok(json!({
        "path": path,
        "exported": collected.entries.len(),
        "streams_left_out": collected.streams,
    }))
}

fn add(args: &mut Args, urls: Vec<String>, script_run: Option<String>) -> Result<Action> {
    ensure!(
        !urls.is_empty() && urls.len() <= 1000,
        "Add between 1 and 1,000 URLs"
    );
    Ok(Action::Add {
        urls,
        queue: args.take("--queue").unwrap_or_else(|| "main".into()),
        folder: args.take("--folder").map(PathBuf::from),
        name: args.take("--name"),
        script_run,
    })
}

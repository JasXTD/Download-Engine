//! The engine as a program of its own, driven by JSON lines on stdin and stdout. Each
//! command line gets exactly one reply line.

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use url::form_urlencoded;

use crate::meta::TorrentMeta;
use crate::session::{AddOptions, Session, SessionOptions};

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    Files {
        torrent: Option<String>,
        name: Option<String>,
    },
    Add {
        name: String,
        torrent: Option<String>,
        magnet: Option<String>,
        output: PathBuf,
        only: Option<Vec<usize>>,
        #[serde(default)]
        peers: Vec<SocketAddr>,
        #[serde(default)]
        choose_later: bool,
        #[serde(default)]
        seed_only: bool,
        file_name: Option<String>,
        #[serde(default)]
        trackers: Vec<String>,
        /// The person's yes to looking up this magnet on the DHT. See `session::magnet_discovery`.
        #[serde(default)]
        dht_search: bool,
    },
    Select {
        name: String,
        only: Option<Vec<usize>>,
    },
    Details {
        name: String,
    },
    Recheck {
        name: String,
    },
    AddTracker {
        name: String,
        url: String,
    },
    RemoveTracker {
        name: String,
        url: String,
    },
    Import {
        torrent: String,
    },
    Remove {
        name: String,
    },
    Configure {
        upload_limit: Option<u32>,
        #[serde(default)]
        no_upload: bool,
        /// Turns router setup on or off while running. Absent leaves it as it is.
        #[serde(default)]
        upnp: Option<bool>,
    },
    Allow {
        name: String,
        until: Option<u64>,
    },
    Stats {
        name: Option<String>,
    },
    Pause {
        name: String,
    },
    Resume {
        name: String,
    },
    StopAll,
    ResumeAll,
    #[cfg(feature = "lab")]
    Snapshot {
        output: PathBuf,
    },
    #[cfg(feature = "lab")]
    Verify {
        output: PathBuf,
    },
    Exit,
}

/// Far above any real torrent. This keeps a huge file from filling the memory.
const MAX_TORRENT_FILE: u64 = 64 * 1024 * 1024;

async fn read_torrent_file(path: &str) -> Result<Vec<u8>> {
    let size = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("Could not read {path}"))?
        .len();
    ensure!(size <= MAX_TORRENT_FILE, "This torrent file is too large");
    tokio::fs::read(path)
        .await
        .with_context(|| format!("Could not read {path}"))
}

fn file_list(meta: &TorrentMeta) -> Value {
    let files: Vec<Value> = meta
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            json!({
                "index": index,
                "path": file.display,
                "saved_as": file.path,
                "len": file.len,
                "virtual": file.virtual_file,
            })
        })
        .collect();
    let encode = |text: &str| form_urlencoded::byte_serialize(text.as_bytes()).collect::<String>();
    let mut magnet = format!("{}&dn={}", meta.hashes.magnet(), encode(&meta.name));
    for tracker in &meta.trackers {
        magnet.push_str("&tr=");
        magnet.push_str(&encode(tracker.as_str()));
    }
    json!({
        "files": files,
        "info_hash": meta.hashes.key(),
        "torrent_version": if meta.hashes.v2.is_none(){"v1"}else if meta.hashes.v1.is_some(){"hybrid"}else{"v2"},
        "name": meta.name,
        "magnet": magnet,
    })
}

async fn handle(session: &Arc<Session>, command: Command) -> Result<Value> {
    match command {
        Command::Files { torrent, name } => match (torrent, name) {
            (Some(path), _) => {
                let bytes = read_torrent_file(&path).await?;
                Ok(file_list(&TorrentMeta::from_torrent_file(&bytes)?))
            }
            (None, Some(name)) => {
                let meta = session
                    .torrent(&name)?
                    .meta()
                    .context("The torrent's files aren't known yet")?;
                Ok(file_list(&meta))
            }
            (None, None) => bail!("say which torrent"),
        },
        Command::Add {
            name,
            torrent,
            magnet,
            output,
            only,
            peers,
            choose_later,
            seed_only,
            file_name,
            trackers,
            dht_search,
        } => {
            let add = AddOptions {
                output,
                only,
                peers,
                choose_later,
                seed_only,
                file_name,
                trackers,
                dht_search,
            };
            let info_hash = match (torrent, magnet) {
                (Some(path), None) => {
                    let bytes = read_torrent_file(&path).await?;
                    session.add_torrent_file(name, &bytes, add).await?
                }
                (None, Some(link)) => session.add_magnet(name, &link, add).await?,
                _ => bail!("give either a torrent file or a magnet link"),
            };
            Ok(json!({ "info_hash": info_hash }))
        }
        Command::Select { name, only } => {
            session.torrent(&name)?.select(only)?;
            Ok(json!({}))
        }
        Command::Details { name } => Ok(serde_json::to_value(session.torrent(&name)?.details())?),
        Command::Recheck { name } => {
            session.torrent(&name)?.recheck().await?;
            Ok(json!({}))
        }
        Command::AddTracker { name, url } => {
            session.torrent(&name)?.add_tracker(&url)?;
            Ok(json!({}))
        }
        Command::RemoveTracker { name, url } => {
            session.torrent(&name)?.remove_tracker(&url)?;
            Ok(json!({}))
        }
        Command::Import { torrent } => {
            let bytes = read_torrent_file(&torrent).await?;
            Ok(file_list(&session.import(&bytes)?))
        }
        Command::Remove { name } => {
            session.remove(&name).await?;
            Ok(json!({}))
        }
        Command::Configure {
            upload_limit,
            no_upload,
            upnp,
        } => {
            session.configure(upload_limit, no_upload);
            if let Some(on) = upnp {
                session.set_upnp(on).await;
            }
            Ok(json!({}))
        }
        Command::Allow { name, until } => Ok(json!({ "received": session.allow(&name, until)? })),
        Command::Stats { name: None } => Ok(serde_json::to_value(session.stats())?),
        Command::Stats { name: Some(name) } => {
            let mut stats = serde_json::to_value(session.torrent(&name)?.stats())?;
            // Every worker asks for its torrent often, so router setup rides along.
            stats["router"] = serde_json::to_value(session.router_status())?;
            Ok(stats)
        }
        Command::Pause { name } => {
            session.pause(&name).await?;
            Ok(json!({}))
        }
        Command::Resume { name } => {
            session.resume(&name).await?;
            Ok(json!({}))
        }
        Command::StopAll => Ok(json!({ "stopped": session.stop_all().await })),
        Command::ResumeAll => Ok(json!({ "resumed": session.resume_all().await })),
        #[cfg(feature = "lab")]
        Command::Snapshot { output } => {
            tokio::task::spawn_blocking(move || lab::snapshot(&output)).await?
        }
        #[cfg(feature = "lab")]
        Command::Verify { output } => {
            tokio::task::spawn_blocking(move || lab::verify(&output)).await?
        }
        Command::Exit => Ok(json!({})),
    }
}

async fn serve(options: SessionOptions) -> Result<()> {
    let session = Session::new(options).await?;
    let result = serve_commands(&session).await;
    session.shutdown().await;
    result
}

async fn serve_commands(session: &Arc<Session>) -> Result<()> {
    let mut lines = command_lines()?;
    let monitor = session.wait_for_interface_loss();
    tokio::pin!(monitor);
    while let Some(line) = tokio::select! {
        line = lines.recv() => line.transpose()?,
        _ = &mut monitor => {
            bail!("The selected torrent interface/address is unavailable. Progress was saved; reconnect it before retrying.");
        }
    } {
        let command: Result<Command> = serde_json::from_str(&line).context("bad command");
        let done = matches!(command, Ok(Command::Exit));
        if done {
            session.shutdown().await;
        }
        let reply = match command {
            Ok(command) => handle(session, command).await,
            Err(error) => Err(error),
        };
        let reply = match reply {
            Ok(result) => json!({ "ok": true, "result": result }),
            Err(error) => json!({
                "ok": false,
                "error": format!("{error:#}"),
                "code": crate::reason::code_of(&error),
            }),
        };
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
        if done {
            return Ok(());
        }
    }
    Ok(())
}

/// Tokio's stdin uses an uncancellable blocking-pool read. Runtime shutdown waits
/// for that read, so interface loss could save progress but leave the helper alive
/// until its parent sent another line. A dedicated bounded input thread is not
/// owned by the runtime; process exit can release a still-idle pipe reader safely.
fn command_lines() -> Result<tokio::sync::mpsc::Receiver<std::io::Result<String>>> {
    let (send, receive) = tokio::sync::mpsc::channel(1);
    std::thread::Builder::new()
        .name("torrent-commands".into())
        .spawn(move || {
            use std::io::{BufRead, Read};
            const LIMIT: u64 = 2 * 1024 * 1024;
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            loop {
                let mut bytes = Vec::new();
                let count = (&mut input).take(LIMIT + 1).read_until(b'\n', &mut bytes);
                if matches!(count, Ok(0)) {
                    break;
                }
                let line = count.and_then(|count| {
                    if count as u64 > LIMIT {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Torrent command is too large",
                        ));
                    }
                    String::from_utf8(bytes).map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, error)
                    })
                });
                let failed = line.is_err();
                if send.blocking_send(line).is_err() || failed {
                    break;
                }
            }
        })
        .context("Cannot start torrent command reader")?;
    Ok(receive)
}

fn options_from(arguments: &[String]) -> Result<SessionOptions> {
    let mut options = SessionOptions {
        seed: true,
        ..Default::default()
    };
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().context("a value is missing");
        match argument.as_str() {
            "--port" => options.listen_port = Some(value()?.parse()?).filter(|port| *port != 0),
            "--upload-limit" => {
                options.upload_limit = Some(value()?.parse()?).filter(|rate| *rate != 0);
            }
            "--download-limit" => {
                options.download_limit = Some(value()?.parse()?).filter(|rate| *rate != 0);
            }
            "--state-dir" => options.state_dir = Some(Path::new(value()?).to_path_buf()),
            "--no-seed" => options.seed = false,
            "--no-upload" => options.no_upload = true,
            "--trackers" => options.use_trackers = true,
            "--dht" => options.use_dht = true,
            "--upnp" => options.upnp = true,
            "--utp" => options.use_utp = true,
            "--web-seeds" => options.web_seeds = true,
            "--bind-network" => {
                options.binding = Some(
                    serde_json::from_str(value()?).context("Invalid torrent interface settings")?,
                )
            }
            "--peer-encryption" => options.encryption = crate::mse::Encryption::parse(value()?)?,
            // Only the offline lab can accept loopback peers from another local client.
            // This switch is absent from normal helper/release builds.
            #[cfg(feature = "lab")]
            "--lab-allow-loopback-peers" => options.allow_loopback_peers = true,
            other => bail!("unknown option {other}"),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod option_tests {
    use super::options_from;

    #[test]
    fn local_peer_override_is_restricted_to_lab_builds() {
        assert!(!options_from(&[]).unwrap().allow_loopback_peers);
        let result = options_from(&["--lab-allow-loopback-peers".into()]);
        #[cfg(feature = "lab")]
        assert!(result.unwrap().allow_loopback_peers);
        #[cfg(not(feature = "lab"))]
        assert!(result.is_err());
    }
}

#[tokio::main]
pub async fn run() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        // The form the lab uses for every engine it tests.
        #[cfg(feature = "lab")]
        ["engine", port, upload] => {
            let arguments = [
                "--port".into(),
                port.into(),
                "--upload-limit".into(),
                upload.into(),
                "--state-dir".into(),
                "/data/.de-torrent".into(),
            ];
            serve(options_from(&arguments)?).await
        }
        _ => serve(options_from(&arguments)?).await,
    }
}

#[cfg(feature = "lab")]
mod lab {
    use std::path::{Path, PathBuf};
    use std::time::UNIX_EPOCH;

    use anyhow::Result;
    use serde_json::{Map, Value, json};
    use sha1w::{ISha256, Sha256};

    fn walk(root: &Path, relative: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        let folder = root.join(relative);
        if !folder.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let next = relative.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                walk(root, &next, found)?;
            } else {
                found.push(next);
            }
        }
        Ok(())
    }

    fn files_under(root: &Path) -> Result<Vec<PathBuf>> {
        let mut found = Vec::new();
        walk(root, Path::new(""), &mut found)?;
        found.sort();
        Ok(found)
    }

    pub fn snapshot(root: &Path) -> Result<Value> {
        let mut out = Map::new();
        for relative in files_under(root)? {
            let info = std::fs::metadata(root.join(&relative))?;
            let modified = info.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
            out.insert(
                relative.to_string_lossy().into_owned(),
                json!({ "len": info.len(), "modified": modified.to_string() }),
            );
        }
        Ok(Value::Object(out))
    }

    fn sha256_file(path: &Path) -> Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        Ok(hash
            .finish()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    pub fn verify(root: &Path) -> Result<Value> {
        let mut out = Map::new();
        for relative in files_under(root)? {
            let path = root.join(&relative);
            out.insert(
                relative.to_string_lossy().into_owned(),
                json!({ "len": std::fs::metadata(&path)?.len(), "sha256": sha256_file(&path)? }),
            );
        }
        Ok(Value::Object(out))
    }
}

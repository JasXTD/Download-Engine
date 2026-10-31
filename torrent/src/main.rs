//! The torrent engine as a program of its own, driven by JSON lines on stdin and stdout.
//! Each command line gets exactly one reply line.

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use de_torrent::meta::TorrentMeta;
use de_torrent::session::{AddOptions, Session, SessionOptions};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

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
    },
    Select {
        name: String,
        only: Option<Vec<usize>>,
    },
    Stats,
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

fn file_list(meta: &TorrentMeta) -> Value {
    let files: Vec<Value> = meta
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| json!({ "index": index, "path": file.display, "len": file.len }))
        .collect();
    json!({ "files": files, "info_hash": meta.info_hash.as_string(), "name": meta.name })
}

async fn handle(session: &Arc<Session>, command: Command) -> Result<Value> {
    match command {
        Command::Files { torrent, name } => match (torrent, name) {
            (Some(path), _) => {
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("Could not read {path}"))?;
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
        } => {
            let add = AddOptions {
                output,
                only,
                peers,
                choose_later,
            };
            let info_hash = match (torrent, magnet) {
                (Some(path), None) => {
                    let bytes = tokio::fs::read(&path)
                        .await
                        .with_context(|| format!("Could not read {path}"))?;
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
        Command::Stats => Ok(serde_json::to_value(session.stats())?),
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
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let command: Result<Command> = serde_json::from_str(&line).context("bad command");
        let done = matches!(command, Ok(Command::Exit));
        if done {
            session.shutdown().await;
        }
        let reply = match command {
            Ok(command) => handle(&session, command).await,
            Err(error) => Err(error),
        };
        let reply = match reply {
            Ok(result) => json!({ "ok": true, "result": result }),
            Err(error) => json!({ "ok": false, "error": format!("{error:#}") }),
        };
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
        if done {
            return Ok(());
        }
    }
    session.shutdown().await;
    Ok(())
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
            "--trackers" => options.use_trackers = true,
            "--dht" => options.use_dht = true,
            other => bail!("unknown option {other}"),
        }
    }
    Ok(options)
}

#[tokio::main]
async fn main() -> Result<()> {
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

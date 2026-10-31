//! Torrent downloads. The engine is a separate program, `de-torrent.exe`, because everything
//! it reads comes from strangers. Nothing here starts it unless the user turned torrents on,
//! and it only runs while a torrent needs it. `torrent/README.md` describes its side.
mod helper;
mod publish;
mod settings;

pub use helper::Host;
pub use settings::{OFF_MESSAGE, PROXY_MESSAGE, Settings};

pub(crate) use publish::{delete_saved, discard};
pub(crate) use settings::initialize;

use crate::{
    bandwidth::Limiter,
    model::{Job, Status},
    store::Store,
    synchronization::FinishedFile,
    transfer_activity::{Phase, TorrentSnapshot},
};
use anyhow::{Context, Result, bail, ensure};
use helper::Lease;
use reqwest::Url;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Component, Path, PathBuf},
    time::Duration,
};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

const MAX_LINK: usize = 16384;
const HASH_LEN: usize = 40;
const INVALID_LINK: &str = "That magnet link isn't valid. Copy the whole link and try again.";
const STATS_EVERY: Duration = Duration::from_millis(500);
const SAVE_EVERY: Duration = Duration::from_secs(1);
/// How long a speed-limited torrent waits before asking whether more bytes arrived.
const LIMIT_POLL: Duration = Duration::from_millis(50);
/// How far ahead of what it has paid for a limited torrent may run. Enough to keep data
/// flowing between payments, and little enough that the limit stays honest.
const MIN_WINDOW: u64 = 64 * 1024;
const MAX_WINDOW: u64 = 16 * 1024 * 1024;
const MAX_GRANT: u64 = 64 * 1024;

pub fn is_magnet(value: &str) -> bool {
    value
        .trim()
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("magnet:"))
}

fn is_hash(value: &str) -> bool {
    value.len() == HASH_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn base32_to_hex(text: &str) -> Option<String> {
    let mut hex = String::with_capacity(HASH_LEN);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte.to_ascii_uppercase() {
            letter @ b'A'..=b'Z' => letter - b'A',
            digit @ b'2'..=b'7' => digit - b'2' + 26,
            _ => return None,
        };
        buffer = buffer << 5 | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            hex.push_str(&format!("{:02x}", (buffer >> bits) & 0xff));
            buffer &= (1 << bits) - 1;
        }
    }
    Some(hex)
}

/// The info hash of a magnet link, as lowercase hex. The engine reads the rest of the link.
pub fn info_hash(link: &str) -> Result<String> {
    ensure!(link.len() <= MAX_LINK, INVALID_LINK);
    let url = Url::parse(link.trim()).context(INVALID_LINK)?;
    ensure!(url.scheme() == "magnet", INVALID_LINK);
    let hash = url
        .query_pairs()
        .find_map(|(key, value)| {
            (key == "xt")
                .then(|| value.strip_prefix("urn:btih:").map(str::to_owned))
                .flatten()
        })
        .context(INVALID_LINK)?;
    let hash = match hash.len() {
        32 => base32_to_hex(&hash).context(INVALID_LINK)?,
        _ => hash.to_ascii_lowercase(),
    };
    ensure!(is_hash(&hash), INVALID_LINK);
    Ok(hash)
}

fn magnet_name(link: &str) -> Option<String> {
    Url::parse(link.trim())
        .ok()?
        .query_pairs()
        .find_map(|(key, value)| (key == "dn").then(|| value.into_owned()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Added with "choose later": the file list still has to come from other people.
    Details,
    /// The file list is here, and nothing downloads until someone chooses from it.
    Choosing,
    Chosen,
}

/// The part of a torrent that lives in the job. The file list has its own table, because
/// it can be huge and jobs are saved every second.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub info_hash: String,
    pub stage: Stage,
}
impl State {
    pub(crate) fn validate(&self, url: &str) -> Result<()> {
        ensure!(
            is_hash(&self.info_hash) && info_hash(url)? == self.info_hash,
            "Invalid saved torrent download state"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    /// The file's number in the torrent.
    pub index: usize,
    /// The name as the torrent has it. For showing, never for a path.
    pub path: String,
    /// Where it goes inside the torrent's folder.
    pub saved_as: PathBuf,
    pub len: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Files {
    pub list: Vec<File>,
    pub chosen: Vec<usize>,
}
impl Files {
    fn chosen_files(&self) -> impl Iterator<Item = &File> {
        self.list
            .iter()
            .filter(|file| self.chosen.contains(&file.index))
    }
    pub fn chosen_size(&self) -> u64 {
        self.chosen_files().map(|file| file.len).sum()
    }
    fn validate(&self) -> Result<()> {
        for file in &self.list {
            check_relative(&file.saved_as)?;
        }
        ensure!(
            !self.chosen.is_empty(),
            "Select at least one file to download"
        );
        ensure!(
            self.chosen
                .iter()
                .all(|index| self.list.iter().any(|file| file.index == *index)),
            "This torrent has no such file"
        );
        Ok(())
    }
}

/// The helper reads names from strangers. Its answers are checked here too, because these
/// paths are later used to move and delete files.
fn check_relative(path: &Path) -> Result<()> {
    let valid = path.components().count() > 0
        && path.components().all(|part| match part {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                !name.contains([':', '*', '?', '"', '<', '>', '|', '\0', '\r', '\n'])
                    && !name.ends_with(['.', ' '])
            }
            _ => false,
        });
    ensure!(valid, "This torrent has an unsafe file path");
    Ok(())
}

/// What a torrent holds, before it becomes a download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    pub info_hash: String,
    pub name: String,
    pub magnet: String,
    pub files: Vec<File>,
}
impl Details {
    fn parse(value: Value) -> Result<Self> {
        #[derive(Deserialize)]
        struct Entry {
            index: usize,
            path: String,
            saved_as: PathBuf,
            len: u64,
            #[serde(rename = "virtual")]
            hidden: bool,
        }
        #[derive(Deserialize)]
        struct Reply {
            info_hash: String,
            name: String,
            magnet: String,
            files: Vec<Entry>,
        }
        let reply: Reply = serde_json::from_value(value).context("Invalid torrent details")?;
        ensure!(is_hash(&reply.info_hash), "Invalid torrent details");
        let files = reply
            .files
            .into_iter()
            .filter(|file| !file.hidden)
            .map(|file| {
                check_relative(&file.saved_as)?;
                Ok(File {
                    index: file.index,
                    path: file.path,
                    saved_as: file.saved_as,
                    len: file.len,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(!files.is_empty(), "This torrent has no files");
        Ok(Self {
            info_hash: reply.info_hash,
            name: crate::model::safe_filename(&reply.name),
            magnet: reply.magnet,
            files,
        })
    }
}

fn check_allowed(store: &Store) -> Result<()> {
    ensure!(store.torrent_settings()?.enabled, OFF_MESSAGE);
    ensure!(!store.proxy_blocks_torrents()?, PROXY_MESSAGE);
    Ok(())
}

impl Store {
    /// Reads a .torrent file the user chose. Nothing connects to anyone for this.
    pub fn read_torrent_file(&self, path: &Path) -> Result<Details> {
        check_allowed(self)?;
        let path = path.to_str().context("This file path can't be used")?;
        let lease = self.torrents.lease(self)?;
        let reply = lease.call_blocking(json!({"cmd": "import", "torrent": path}));
        Details::parse(reply.map_err(|error| {
            if error.to_string().contains("torrent") {
                anyhow::anyhow!("That torrent file is damaged. Download the .torrent file again.")
            } else {
                error
            }
        })?)
    }

    /// Asks other people for a magnet link's file list, until they answer or `cancel` fires.
    pub fn fetch_torrent_details(
        &self,
        link: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<Details>> {
        info_hash(link)?;
        check_allowed(self)?;
        let lease = self.torrents.lease(self)?;
        let name = format!("preview-{}", uuid::Uuid::new_v4());
        lease.call_blocking(json!({
            "cmd": "add",
            "name": name,
            "magnet": link.trim(),
            "output": self.paths.data.join("torrents"),
            "choose_later": true,
            "peers": self.torrents.test_peers(),
        }))?;
        let details = (|| -> Result<Option<Details>> {
            while !cancel.is_cancelled() {
                let stats =
                    Stats::parse(lease.call_blocking(json!({"cmd": "stats", "name": name}))?)?;
                if let Some(error) = stats.error {
                    bail!("{error}");
                }
                if stats.state == "ready" {
                    let files = lease.call_blocking(json!({"cmd": "files", "name": name}))?;
                    return Details::parse(files).map(Some);
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(None)
        })();
        let removed = lease.call_blocking(json!({"cmd": "remove", "name": name}));
        let details = details?;
        removed?;
        Ok(details)
    }

    fn new_torrent_job(
        &self,
        link: &str,
        name: &str,
        folder: Option<&Path>,
        request_id: String,
        stage: Stage,
    ) -> Result<Job> {
        let hash = info_hash(link)?;
        if let Some(existing) = self.torrent_job(&hash)? {
            bail!("Already in your list: {}", existing.name);
        }
        let directory = match folder {
            Some(folder) => {
                crate::categories::validate_folder(folder)?;
                folder.to_owned()
            }
            None => self
                .download_folders()?
                .destination(&self.paths.downloads, name),
        };
        Job::new_torrent(link, name, directory, request_id, stage)
    }

    fn enqueue_torrent(&self, job: Job, files: Option<&Files>, queue: &str) -> Result<Job> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let job =
            crate::queue_store::enqueue_batch_in(&tx, std::slice::from_ref(&job), queue)?.remove(0);
        if let Some(files) = files {
            save_files(&tx, &job.id, files)?;
        }
        tx.commit()?;
        Ok(job)
    }

    /// Adds a torrent whose files were chosen in the Add download window.
    pub fn add_torrent(
        &self,
        details: &Details,
        chosen: Vec<usize>,
        folder: Option<&Path>,
        queue: &str,
        request_id: String,
    ) -> Result<Job> {
        ensure!(self.torrent_settings()?.enabled, OFF_MESSAGE);
        let files = Files {
            list: details.files.clone(),
            chosen,
        };
        files.validate()?;
        let mut job = self.new_torrent_job(
            &details.magnet,
            &details.name,
            folder,
            request_id,
            Stage::Chosen,
        )?;
        job.total = Some(files.chosen_size());
        self.enqueue_torrent(job, Some(&files), queue)
    }

    /// Adds a magnet link whose file list hasn't arrived. The download starts when someone
    /// has chosen from the list.
    pub fn add_magnet_for_later(
        &self,
        link: &str,
        folder: Option<&Path>,
        queue: &str,
        request_id: String,
    ) -> Result<Job> {
        ensure!(self.torrent_settings()?.enabled, OFF_MESSAGE);
        let hash = info_hash(link)?;
        let name = magnet_name(link).unwrap_or(hash);
        let job = self.new_torrent_job(link, &name, folder, request_id, Stage::Details)?;
        self.enqueue_torrent(job, None, queue)
    }

    /// The download for this torrent, if there is one.
    pub fn torrent_job(&self, info_hash: &str) -> Result<Option<Job>> {
        Ok(self.list()?.into_iter().find(|job| {
            job.torrent
                .as_ref()
                .is_some_and(|state| state.info_hash == info_hash)
        }))
    }

    pub fn torrent_files(&self, job: &str) -> Result<Files> {
        let data: Option<String> = self
            .connect()?
            .query_row(
                "SELECT data FROM torrent_files WHERE job_id=?1",
                [job],
                |row| row.get(0),
            )
            .optional()?;
        let files: Files = match data {
            Some(data) => serde_json::from_str(&data).context("Invalid saved torrent files")?,
            None => return Ok(Files::default()),
        };
        files.validate()?;
        Ok(files)
    }

    /// Records the chosen files. A running download picks them up from the scheduler.
    pub(crate) fn choose_torrent_files(&self, id: &str, chosen: Vec<usize>) -> Result<Files> {
        let mut files = self.torrent_file_list(id)?;
        files.chosen = chosen;
        files.validate()?;
        save_files(&self.connect()?, id, &files)?;
        Ok(files)
    }

    /// The saved list, which has no chosen files yet while someone is choosing.
    pub fn torrent_file_list(&self, id: &str) -> Result<Files> {
        let data: Option<String> = self
            .connect()?
            .query_row(
                "SELECT data FROM torrent_files WHERE job_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let data = data.context("This torrent's file list isn't known yet")?;
        serde_json::from_str(&data).context("Invalid saved torrent files")
    }
}

fn save_files(conn: &rusqlite::Connection, job: &str, files: &Files) -> Result<()> {
    conn.execute(
        "INSERT INTO torrent_files(job_id,data) VALUES(?1,?2)
        ON CONFLICT(job_id) DO UPDATE SET data=excluded.data",
        params![job, serde_json::to_string(files)?],
    )?;
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct Stats {
    state: String,
    error: Option<String>,
    finished: bool,
    progress_bytes: u64,
    total_bytes: u64,
    file_progress: Vec<u64>,
    downloaded_bytes: u64,
    uploaded_bytes: u64,
    download_bps: u64,
    upload_bps: u64,
    peers: usize,
}
impl Stats {
    fn parse(value: Value) -> Result<Self> {
        serde_json::from_value(value).context("Invalid torrent status")
    }
    fn phase(&self) -> Phase {
        match self.state.as_str() {
            "metadata" => Phase::TorrentDetails,
            "initializing" => Phase::CheckingSaved,
            "live" if self.peers == 0 => Phase::LookingForPeers,
            "live" => Phase::Downloading,
            _ => Phase::Saving,
        }
    }
}

enum Outcome {
    Stopped,
    /// The file list arrived, and someone has to choose from it.
    NeedsChoice,
    Finished,
}

/// Runs one torrent from the scheduler, in the same way as any other download.
pub(crate) async fn transfer(
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    let stage = job.torrent.as_ref().context("Missing torrent state")?.stage;
    if stage == Stage::Choosing {
        return Ok(None);
    }
    check_allowed(store)?;
    if let Some(file) = publish::recover(store, job).await? {
        forget(store, job)?;
        return Ok(Some(file));
    }
    let staging = publish::staging(job);
    tokio::fs::create_dir_all(&staging)
        .await
        .context("Cannot create the download folder")?;
    let files = store.torrent_files(&job.id)?;
    let lease = store.torrents.lease(store)?;
    let mut add = json!({
        "cmd": "add",
        "name": job.id,
        "magnet": job.url,
        "output": staging,
        "peers": store.torrents.test_peers(),
    });
    match stage {
        Stage::Chosen => add["only"] = json!(files.chosen),
        _ => add["choose_later"] = json!(true),
    }
    lease.call(add).await?;
    let id = job.id.clone();
    let outcome = tokio::select! {
        outcome = follow(store, job, &lease, limiter, cancel) => outcome,
        error = share_speed_limits(&id, &lease, limiter, cancel) => Err(error),
    };
    // Whatever happened, the engine must let go of the files before anyone touches them.
    let removed = lease
        .call(json!({"cmd": "remove", "name": id}))
        .await
        .map(|_| ());
    drop(lease);
    match outcome? {
        Outcome::Stopped | Outcome::NeedsChoice => Ok(None),
        Outcome::Finished => {
            removed?;
            let finished = publish::finish(store, job).await?;
            forget(store, job)?;
            Ok(Some(finished))
        }
    }
}

/// Removes what the engine remembers about a torrent.
pub(crate) fn forget(store: &Store, job: &Job) -> Result<()> {
    let Some(state) = &job.torrent else {
        return Ok(());
    };
    let folder = store.paths.data.join("torrents");
    for name in [
        format!("{}.resume.json", state.info_hash),
        format!("{}.info", state.info_hash),
    ] {
        match std::fs::remove_file(folder.join(name)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error).context("Cannot remove the torrent's saved details");
            }
            _ => {}
        }
    }
    Ok(())
}

async fn follow(
    store: &Store,
    job: &mut Job,
    lease: &Lease,
    limiter: &Limiter,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    let mut tick = tokio::time::interval(STATS_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut counted = 0u64;
    let mut saved = Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(Outcome::Stopped),
            _ = tick.tick() => {}
        }
        limiter.check_quota().await?;
        if let Some(chosen) = store.torrents.take_selection(&job.id) {
            lease
                .call(json!({"cmd": "select", "name": job.id, "only": chosen}))
                .await?;
        }
        let stats = Stats::parse(lease.call(json!({"cmd": "stats", "name": job.id})).await?)?;
        if let Some(error) = stats.error {
            bail!("{error}");
        }
        // Everything that crosses the network counts toward a data quota, in both directions.
        let traffic = stats.downloaded_bytes + stats.uploaded_bytes;
        if traffic > counted {
            limiter
                .record_payload(usize::try_from(traffic - counted)?)
                .await?;
            counted = traffic;
        }
        store.activity.set(&job.id, stats.phase());
        store.activity.set_torrent(
            &job.id,
            TorrentSnapshot {
                peers: stats.peers,
                download_speed: stats.download_bps,
                upload_speed: stats.upload_bps,
                uploaded: stats.uploaded_bytes,
                file_progress: stats.file_progress.as_slice().into(),
            },
        );
        if stats.state == "ready" {
            let details =
                Details::parse(lease.call(json!({"cmd": "files", "name": job.id})).await?)?;
            let files = Files {
                list: details.files,
                chosen: Vec::new(),
            };
            save_files(&store.connect()?, &job.id, &files)?;
            job.name = details.name;
            job.total = Some(files.list.iter().map(|file| file.len).sum());
            job.torrent.as_mut().context("Missing torrent state")?.stage = Stage::Choosing;
            return Ok(Outcome::NeedsChoice);
        }
        if stats.total_bytes > 0 {
            job.total = Some(stats.total_bytes);
            job.downloaded = stats.progress_bytes;
        }
        if stats.finished {
            return Ok(Outcome::Finished);
        }
        if saved.elapsed() >= SAVE_EVERY {
            store.checkpoint(job).await?;
            saved = Instant::now();
        }
    }
}

/// Holds a torrent to the same speed limits as every other download. Bytes are paid for
/// after they arrive, so a torrent with no peers doesn't use up what other downloads could.
/// Only ends if the helper does.
async fn share_speed_limits(
    id: &str,
    lease: &Lease,
    limiter: &Limiter,
    cancel: &CancellationToken,
) -> anyhow::Error {
    // A limit already in place holds from the first byte.
    let mut paid = limiter.rate_for(id).map(|_| 0);
    loop {
        let Some(rate) = limiter.rate_for(id) else {
            if paid.take().is_some()
                && let Err(error) = lease.allow(id, None).await
            {
                return error;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let window = (rate / 5).clamp(MIN_WINDOW, MAX_WINDOW);
        let until = paid.map(|bytes| bytes + window);
        let received = match lease.allow(id, until).await {
            Ok(received) => received,
            Err(error) => return error,
        };
        // A limit that was just set starts from here, and asks nothing for the past.
        let settled = *paid.get_or_insert(received);
        let mut owed = received.saturating_sub(settled);
        if owed == 0 {
            tokio::time::sleep(LIMIT_POLL).await;
        }
        while owed > 0 {
            let Some(granted) = limiter
                .acquire(id, owed.min(MAX_GRANT) as usize, cancel)
                .await
            else {
                std::future::pending::<()>().await;
                unreachable!();
            };
            owed -= granted as u64;
            paid = paid.map(|bytes| bytes + granted as u64);
        }
    }
}

/// How much room a folder's drive has. A folder that doesn't exist yet uses the nearest
/// one above it, because that's where it will be made.
pub fn free_space(folder: &Path) -> Option<u64> {
    let folder = folder.ancestors().find(|path| path.is_dir())?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // The path ends in a zero, and only the first output is asked for.
        let done = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (done != 0).then_some(free)
    }
    #[cfg(not(windows))]
    {
        let _ = folder;
        None
    }
}

/// Whether a torrent has its file list and waits for someone to choose from it.
pub fn waiting_for_choice(job: &Job) -> bool {
    job.status == Status::Paused
        && job
            .torrent
            .as_ref()
            .is_some_and(|state| state.stage == Stage::Choosing)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "c12fe1c06bba254a9dc9f519b335aa7c1367a88a";

    #[test]
    fn magnet_links_are_recognized_in_any_case() {
        assert!(is_magnet("magnet:?xt=urn:btih:abc"));
        assert!(is_magnet("  MAGNET:?xt=urn:btih:abc"));
        assert!(!is_magnet("https://example.com/magnet:"));
        assert!(!is_magnet("mag"));
        assert!(!is_magnet("€€€€€€€€"));
    }

    #[test]
    fn hashes_read_as_hex_or_base32() {
        assert_eq!(
            info_hash(&format!("magnet:?xt=urn:btih:{HEX}")).unwrap(),
            HEX
        );
        assert_eq!(
            info_hash(&format!(
                "magnet:?dn=a&xt=urn:btih:{}",
                HEX.to_ascii_uppercase()
            ))
            .unwrap(),
            HEX
        );
        assert_eq!(
            info_hash("magnet:?xt=urn:btih:YEX6DQDLXISUVHOJ6UM3GNNKPQJWPKEK").unwrap(),
            HEX
        );
    }

    #[test]
    fn bad_magnet_links_are_refused() {
        for link in [
            "magnet:",
            "magnet:?dn=only-a-name",
            "magnet:?xt=urn:btih:",
            "magnet:?xt=urn:btih:4f3a9c1e",
            "magnet:?xt=urn:btih:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "magnet:?xt=urn:btih:11111111111111111111111111111111",
            "magnet:?xt=urn:sha1:c12fe1c06bba254a9dc9f519b335aa7c1367a88a",
            "https://example.com/?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a",
        ] {
            assert!(info_hash(link).is_err(), "{link}");
        }
        assert!(
            info_hash(&format!(
                "magnet:?xt=urn:btih:{HEX}&x={}",
                "a".repeat(20000)
            ))
            .is_err()
        );
    }

    #[test]
    fn paths_from_the_helper_stay_inside_the_folder() {
        for bad in [
            "",
            "..",
            "a/../../b",
            "C:\\evil.dll",
            "C:evil.dll",
            "\\\\server\\share\\a",
            "/rooted",
            "a.txt:stream",
            "ends-with-dot.",
            "ends-with-space ",
            "wild*card",
        ] {
            assert!(check_relative(Path::new(bad)).is_err(), "{bad}");
        }
        for good in ["a.bin", "sub/b.bin", "sub/deeper/c d.bin"] {
            assert!(check_relative(Path::new(good)).is_ok(), "{good}");
        }
    }

    fn details_reply(saved_as: &str) -> Value {
        json!({
            "info_hash": HEX,
            "name": "Pack: one",
            "magnet": format!("magnet:?xt=urn:btih:{HEX}"),
            "files": [
                {"index": 0, "path": "a.bin", "saved_as": saved_as, "len": 5, "virtual": false},
                {"index": 1, "path": ".pad/1", "saved_as": ".pad/1", "len": 3, "virtual": true},
            ],
        })
    }

    #[test]
    fn details_hide_padding_and_clean_the_name() {
        let details = Details::parse(details_reply("a.bin")).unwrap();
        assert_eq!(details.files.len(), 1);
        assert_eq!(details.name, "Pack_ one");
    }

    #[test]
    fn details_with_an_unsafe_path_are_refused() {
        assert!(Details::parse(details_reply("../../outside.dll")).is_err());
    }

    #[test]
    fn chosen_files_must_exist() {
        let list = vec![File {
            index: 2,
            path: "a".into(),
            saved_as: "a".into(),
            len: 7,
        }];
        let valid = Files {
            list: list.clone(),
            chosen: vec![2],
        };
        assert!(valid.validate().is_ok());
        assert_eq!(valid.chosen_size(), 7);
        for chosen in [vec![], vec![3]] {
            let files = Files {
                list: list.clone(),
                chosen,
            };
            assert!(files.validate().is_err());
        }
    }
}

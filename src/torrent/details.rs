//! A running torrent's pieces, peers and trackers for the status window, and the changes the
//! window can ask for. The helper's answers describe strangers, so they are checked again
//! here: every list has a limit and every text a length. The window never talks to the
//! helper itself. It leaves requests with `Host`, and the worker that runs the torrent
//! passes them on, so nothing here can start `de-torrent.exe`.
use super::*;

/// The engine keeps at most 50 connections and 32 trackers per torrent.
const MAX_PEERS: usize = 64;
const MAX_TRACKERS: usize = 64;
const MAX_MAP: usize = 2048;
const MAX_URL: usize = 2048;
const MAX_ADDRESS: usize = 64;
const MAX_CLIENT: usize = 64;
const MAX_MESSAGE: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Pieces {
    pub total: u32,
    pub wanted: u32,
    /// Wanted pieces that are finished and passed their hash check.
    pub have: u32,
    pub in_progress: u32,
    /// Pieces read so far, while the files are being checked.
    pub checked: Option<u32>,
    /// One character per cell: `-` nothing wanted, `*` a piece in progress, and `0` to `9`
    /// for how much of the wanted pieces is finished, where `9` means all.
    pub map: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Peer {
    #[serde(default)]
    pub connection: de_torrent::details::ConnectionInfo,
    pub address: Option<String>,
    /// The name the peer gave, already cleaned by the helper.
    pub client: Option<String>,
    pub has_pieces: u32,
    pub download_bps: u64,
    pub upload_bps: u64,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    pub incoming: bool,
    pub receiving: bool,
    pub sending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerState {
    Waiting,
    Asking,
    Working,
    Failed,
    /// It leads to this computer or its network, so it is never asked.
    Refused,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Tracker {
    pub url: String,
    /// Added by the user, so it can be removed.
    pub added: bool,
    pub state: TrackerState,
    /// Why the last try failed. It can hold the tracker's own words.
    pub message: Option<String>,
    pub peers: Option<usize>,
    pub seeds: Option<u64>,
    pub leechers: Option<u64>,
    /// Seconds until the next announce or the next try.
    pub next_in: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    On,
    Off,
    Private,
    Waiting,
}

/// The DHT or peer exchange, and how many new peers it has found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Source {
    pub state: SourceState,
    pub peers: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct TorrentDetails {
    pub private: Option<bool>,
    pub pieces: Option<Pieces>,
    pub peers: Vec<Peer>,
    pub trackers: Vec<Tracker>,
    pub dht: Source,
    pub pex: Source,
}

fn plain(text: &Option<String>, max: usize) -> bool {
    text.as_ref()
        .is_none_or(|text| text.chars().count() <= max && !text.chars().any(char::is_control))
}

impl TorrentDetails {
    pub(crate) fn parse(value: Value) -> Result<Self> {
        let details: Self = serde_json::from_value(value).context("Invalid torrent details")?;
        let pieces_ok = details.pieces.as_ref().is_none_or(|pieces| {
            pieces.map.len() <= MAX_MAP
                && pieces.map.len() as u64 <= u64::from(pieces.total)
                && pieces
                    .map
                    .bytes()
                    .all(|cell| cell == b'-' || cell == b'*' || cell.is_ascii_digit())
                && pieces.have <= pieces.wanted
                && pieces.wanted <= pieces.total
                && pieces.checked.is_none_or(|checked| checked <= pieces.total)
        });
        let peers_ok = details.peers.len() <= MAX_PEERS
            && details.peers.iter().all(|peer| {
                plain(&peer.address, MAX_ADDRESS)
                    && plain(&peer.client, MAX_CLIENT)
                    && details
                        .pieces
                        .as_ref()
                        .is_none_or(|pieces| peer.has_pieces <= pieces.total)
            });
        let trackers_ok = details.trackers.len() <= MAX_TRACKERS
            && details.trackers.iter().all(|tracker| {
                tracker.url.len() <= MAX_URL
                    && !tracker.url.chars().any(char::is_control)
                    && plain(&tracker.message, MAX_MESSAGE)
            });
        ensure!(
            pieces_ok && peers_ok && trackers_ok,
            "Invalid torrent details"
        );
        Ok(details)
    }
}

/// What the status window can ask a torrent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Read every chosen piece again, and download the ones that fail.
    Recheck,
    AddTracker(String),
    RemoveTracker(String),
}

pub const RECHECK_LATER: &str =
    "Saved progress was cleared. The files are checked when the torrent starts.";
pub const ADD_WHILE_RUNNING: &str =
    "Start the torrent to add a tracker. Its details show whether it is private.";
/// The helper's own words when it refuses, kept here so the window can say it first.
pub const PRIVATE_TRACKERS_TEXT: &str =
    "This torrent is private, so it can only use the trackers it came with.";
const STOPPED: &str = "The torrent stopped before the change was made. Try again.";

/// Checks a tracker address the user typed, before it goes anywhere. The helper checks it
/// again with the rules for this computer's own network and for private torrents.
pub fn check_tracker(text: &str) -> Result<String> {
    const INVALID: &str = "Enter a tracker address that starts with http://, https:// or udp://";
    let text = text.trim();
    let url = Url::parse(text)
        .ok()
        .filter(|url| {
            text.len() <= MAX_URL
                && matches!(url.scheme(), "http" | "https" | "udp")
                && url.host_str().is_some_and(|host| !host.is_empty())
        })
        .context(INVALID)?;
    ensure!(
        url.scheme() != "udp" || url.port().is_some(),
        "A udp:// tracker address needs a port, such as udp://tracker.example.org:6969"
    );
    Ok(url.to_string())
}

impl Store {
    /// Trackers the user added to this torrent, oldest first.
    pub fn torrent_added_trackers(&self, id: &str) -> Result<Vec<String>> {
        let conn = self.connect()?;
        let mut statement =
            conn.prepare("SELECT url FROM torrent_trackers WHERE job_id=?1 ORDER BY rowid")?;
        let urls = statement
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(urls)
    }

    /// Saves sample data for screenshots, with no helper and no network.
    #[cfg(feature = "screenshot")]
    pub fn seed_torrent_preview(&self, id: &str, files: &Files, trackers: &[&str]) -> Result<()> {
        save_files(&self.connect()?, id, files)?;
        for url in trackers {
            self.save_added_tracker(id, url)?;
        }
        Ok(())
    }

    fn save_added_tracker(&self, id: &str, url: &str) -> Result<()> {
        self.connect()?.execute(
            "INSERT OR IGNORE INTO torrent_trackers(job_id,url) VALUES(?1,?2)",
            params![id, url],
        )?;
        Ok(())
    }

    fn delete_added_tracker(&self, id: &str, url: &str) -> Result<bool> {
        Ok(self.connect()?.execute(
            "DELETE FROM torrent_trackers WHERE job_id=?1 AND url=?2",
            params![id, url],
        )? > 0)
    }

    /// For a torrent no worker is running: what can be done without the helper. Returns the
    /// message to show when it worked.
    pub(crate) fn torrent_request_while_stopped(
        &self,
        id: &str,
        request: &Request,
    ) -> Result<Option<&'static str>> {
        let job = self.get(id)?.context("Download not found")?;
        let state = job.torrent.as_ref().context("This is not a torrent")?;
        match request {
            Request::Recheck => {
                ensure!(
                    job.status != Status::Complete,
                    "Finished torrents can't be checked again here."
                );
                ensure!(
                    state.stage == Stage::Chosen,
                    "Choose the torrent's files first."
                );
                forget_progress(self, &job)?;
                Ok(Some(RECHECK_LATER))
            }
            Request::AddTracker(_) => bail!(ADD_WHILE_RUNNING),
            Request::RemoveTracker(url) => {
                self.delete_added_tracker(id, url)?;
                Ok(None)
            }
        }
    }
}

async fn handle(store: &Store, lease: &Lease, id: &str, request: &Request) -> Result<()> {
    match request {
        Request::Recheck => {
            lease.call(json!({"cmd": "recheck", "name": id})).await?;
        }
        Request::AddTracker(url) => {
            let url = check_tracker(url)?;
            lease
                .call(json!({"cmd": "add_tracker", "name": id, "url": url}))
                .await?;
            if let Err(error) = store.save_added_tracker(id, &url) {
                // Keep the helper and the saved list the same.
                let _ = lease
                    .call(json!({"cmd": "remove_tracker", "name": id, "url": url}))
                    .await;
                return Err(error.context("The tracker could not be saved"));
            }
        }
        Request::RemoveTracker(url) => {
            let removed = lease
                .call(json!({"cmd": "remove_tracker", "name": id, "url": url}))
                .await;
            // A saved tracker the helper left out, such as one of a private torrent, still
            // goes from the list.
            if !store.delete_added_tracker(id, url)? {
                removed?;
            }
        }
    }
    Ok(())
}

/// Passes on what the status window asked for, then fetches fresh details while a window
/// shows them. Called by the worker that runs the torrent, between its own requests.
pub(super) async fn serve(store: &Store, lease: &Lease, id: &str) -> Result<()> {
    for (ticket, request) in store.torrents.take_requests(id) {
        let result = handle(store, lease, id, &request).await;
        store.torrents.settle(
            id,
            ticket,
            result.map(|()| None).map_err(|error| format!("{error:#}")),
        );
    }
    if store.torrents.is_watched(id) {
        let reply = lease.call(json!({"cmd": "details", "name": id})).await?;
        match TorrentDetails::parse(reply) {
            Ok(details) => store.activity.set_torrent_details(id, details),
            // Bad details only cost the window its lists, never the download.
            Err(_) => store.activity.clear_torrent_details(id),
        }
    }
    Ok(())
}

/// Requests nobody will pass on any more, because the torrent's worker has stopped.
pub(super) fn abandon(store: &Store, id: &str) {
    for (ticket, _) in store.torrents.take_requests(id) {
        store.torrents.settle(id, ticket, Err(STOPPED.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply() -> Value {
        json!({
            "private": false,
            "pieces": {"total": 4, "wanted": 3, "have": 1, "in_progress": 1, "checked": null, "map": "9*0-"},
            "peers": [{
                "address": "192.0.2.7:6881", "client": "qBittorrent/5.0.3", "has_pieces": 4,
                "download_bps": 1000, "upload_bps": 0, "downloaded_bytes": 5000,
                "uploaded_bytes": 0, "incoming": false, "receiving": true, "sending": false
            }],
            "trackers": [{
                "url": "udp://tracker.example.org:1337/announce", "added": false,
                "state": "working", "message": null, "peers": 50, "seeds": 212,
                "leechers": 37, "next_in": 1700
            }],
            "dht": {"state": "on", "peers": 41},
            "pex": {"state": "on", "peers": 12},
        })
    }

    #[test]
    fn details_from_the_helper_are_read() {
        let details = TorrentDetails::parse(reply()).unwrap();
        assert_eq!(details.pieces.unwrap().map, "9*0-");
        assert_eq!(
            details.peers[0].client.as_deref(),
            Some("qBittorrent/5.0.3")
        );
        assert_eq!(details.trackers[0].state, TrackerState::Working);
        assert_eq!(details.dht.peers, 41);
    }

    #[test]
    fn connection_indicators_use_reported_negotiation_and_old_helpers_stay_unknown() {
        use de_torrent::details::{ConnectionInfo, PeerTransport};
        assert_eq!(
            TorrentDetails::parse(reply()).unwrap().peers[0].connection,
            ConnectionInfo::default()
        );
        for (protocol, expected) in [("tcp", PeerTransport::Tcp), ("utp", PeerTransport::Utp)] {
            for encrypted in [true, false] {
                let mut value = reply();
                value["peers"][0]["connection"] =
                    json!({"transport":protocol,"encrypted":encrypted});
                assert_eq!(
                    TorrentDetails::parse(value).unwrap().peers[0].connection,
                    ConnectionInfo {
                        transport: expected,
                        encrypted: Some(encrypted)
                    }
                );
            }
        }
        let mut value = reply();
        value["peers"][0]["connection"] = json!({"transport":"not-a-protocol","encrypted":true});
        assert!(TorrentDetails::parse(value).is_err());
    }

    #[test]
    fn details_that_break_the_limits_are_refused() {
        let broken: [fn(&mut Value); 8] = [
            |value| value["pieces"]["map"] = json!("9x"),
            |value| value["pieces"]["map"] = json!("9".repeat(5)),
            |value| value["pieces"]["have"] = json!(4),
            |value| value["peers"][0]["client"] = json!("name\u{1b}[31m"),
            |value| value["peers"][0]["client"] = json!("x".repeat(65)),
            |value| value["peers"][0]["has_pieces"] = json!(5),
            |value| value["trackers"][0]["message"] = json!("a\nb"),
            |value| value["peers"] = json!(vec![value["peers"][0].clone(); 65]),
        ];
        for (number, change) in broken.into_iter().enumerate() {
            let mut value = reply();
            change(&mut value);
            assert!(TorrentDetails::parse(value).is_err(), "case {number}");
        }
    }

    #[test]
    fn typed_tracker_addresses_are_checked_before_they_go_anywhere() {
        assert_eq!(
            check_tracker("  udp://tracker.example.org:6969/announce ").unwrap(),
            "udp://tracker.example.org:6969/announce"
        );
        assert_eq!(
            check_tracker("https://tracker.example.net/announce").unwrap(),
            "https://tracker.example.net/announce"
        );
        for bad in [
            "",
            "   ",
            "tracker.example.org",
            "ftp://tracker.example.org/announce",
            "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
            "udp://tracker.example.org/announce",
        ] {
            assert!(check_tracker(bad).is_err(), "{bad:?}");
        }
        assert!(check_tracker(&format!("http://{}.org/", "a".repeat(2100))).is_err());
    }

    #[test]
    fn added_trackers_are_kept_in_order_and_removed_with_their_download() {
        let (_temp, store, job) = completed_folder_fixture();
        store
            .save_added_tracker(&job.id, "udp://b.example.org:1/")
            .unwrap();
        store
            .save_added_tracker(&job.id, "udp://a.example.org:1/")
            .unwrap();
        store
            .save_added_tracker(&job.id, "udp://b.example.org:1/")
            .unwrap();
        assert_eq!(
            store.torrent_added_trackers(&job.id).unwrap(),
            ["udp://b.example.org:1/", "udp://a.example.org:1/"]
        );
        assert!(
            store
                .delete_added_tracker(&job.id, "udp://b.example.org:1/")
                .unwrap()
        );
        assert!(
            !store
                .delete_added_tracker(&job.id, "udp://b.example.org:1/")
                .unwrap()
        );
        store
            .save_added_tracker(&job.id, "udp://c.example.org:1/")
            .unwrap();
        store.remove(&job.id).unwrap();
        let left: i64 = store
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM torrent_trackers", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn requests_wait_for_their_worker_and_outcomes_are_kept_by_ticket() {
        let host = Host::default();
        let (first, second) = (host.new_ticket(), host.new_ticket());
        assert_ne!(first, second);
        host.queue_request("job", first, Request::Recheck);
        host.queue_request(
            "job",
            second,
            Request::RemoveTracker("udp://a.example.org:1/".into()),
        );
        assert!(host.take_requests("other").is_empty());
        assert_eq!(host.take_requests("job").len(), 2);
        assert!(
            host.take_requests("job").is_empty(),
            "each request is passed on once"
        );
        host.settle("job", second, Err("refused".into()));
        assert_eq!(
            host.outcome("job", first),
            None,
            "only its own outcome counts"
        );
        assert_eq!(host.outcome("job", second), Some(Err("refused".into())));
        assert!(!host.is_watched("job"));
        host.watch("job");
        assert!(host.is_watched("job"));
        assert!(!host.is_watched("other"));
    }

    #[test]
    fn requests_left_when_a_worker_stops_are_answered() {
        let (_temp, store, job) = completed_folder_fixture();
        let ticket = store.torrents.new_ticket();
        store
            .torrents
            .queue_request(&job.id, ticket, Request::Recheck);
        abandon(&store, &job.id);
        assert_eq!(
            store.torrents.outcome(&job.id, ticket),
            Some(Err(STOPPED.into()))
        );
    }

    #[test]
    fn a_stopped_torrent_can_lose_saved_progress_and_trackers_but_not_gain_trackers() {
        let (_temp, store, mut job) = completed_folder_fixture();
        let finished = store
            .torrent_request_while_stopped(&job.id, &Request::Recheck)
            .unwrap_err();
        assert!(finished.to_string().contains("Finished"), "{finished}");
        job.status = Status::Paused;
        store.save(&job).unwrap();
        let state = job.torrent.as_ref().unwrap();
        let saved = store
            .paths
            .data
            .join("torrents")
            .join(format!("{}.resume.json", state.info_hash));
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(&saved, b"{}").unwrap();
        assert_eq!(
            store
                .torrent_request_while_stopped(&job.id, &Request::Recheck)
                .unwrap(),
            Some(RECHECK_LATER)
        );
        assert!(!saved.exists(), "the next start reads every file");
        let error = store
            .torrent_request_while_stopped(
                &job.id,
                &Request::AddTracker("udp://a.example.org:1/".into()),
            )
            .unwrap_err();
        assert_eq!(error.to_string(), ADD_WHILE_RUNNING);
        store
            .save_added_tracker(&job.id, "udp://a.example.org:1/")
            .unwrap();
        store
            .torrent_request_while_stopped(
                &job.id,
                &Request::RemoveTracker("udp://a.example.org:1/".into()),
            )
            .unwrap();
        assert!(store.torrent_added_trackers(&job.id).unwrap().is_empty());
    }
}

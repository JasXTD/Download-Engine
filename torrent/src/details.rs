//! What the status window shows about a running torrent: its pieces, peers and trackers.
//! Client names and tracker messages come from strangers, so they are cleaned before they
//! leave the engine, and every list has a limit.

use std::sync::Arc;

use librqbit_tracker_comms::{TrackerAnswer, TrackerObserver};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::time::{Duration, Instant};
use url::Url;

use crate::picker::Picker;

/// Cells in the piece map. More than any status window is wide.
pub const MAP_CELLS: usize = 1024;
pub const MAX_CLIENT_CHARS: usize = 40;
pub const MAX_MESSAGE_CHARS: usize = 160;

/// Characters that change the direction of text or hide in it. A stranger could use them to
/// make one name look like another.
fn is_hidden_or_directional(character: char) -> bool {
    matches!(
        character,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Text from a stranger, made safe to show: control, hidden and direction characters become
/// spaces, runs of spaces become one, and anything past `max` characters is cut off with `…`.
pub fn clean_text(text: &str, max: usize) -> Option<String> {
    let spaced: String = text
        .chars()
        .map(|character| {
            if character.is_control() || is_hidden_or_directional(character) {
                ' '
            } else {
                character
            }
        })
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() || max == 0 {
        return None;
    }
    if joined.chars().count() <= max {
        return Some(joined);
    }
    let mut cut: String = joined.chars().take(max - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    Some(cut)
}

/// One character per cell: `-` when the cell holds no wanted piece, `*` when a wanted piece in
/// it has started but isn't finished, otherwise `0` to `9` for how much of its wanted pieces
/// is finished, where `9` means all of them.
pub fn piece_map(picker: &Picker, cells: usize) -> String {
    let total = u64::from(picker.have().len());
    let cells = (cells as u64).min(total);
    (0..cells)
        .map(|cell| {
            let start = (cell * total / cells) as u32;
            let end = ((cell + 1) * total / cells) as u32;
            let (mut wanted, mut have, mut started) = (0u32, 0u32, false);
            for piece in start..end {
                if !picker.wanted().get(piece) {
                    continue;
                }
                wanted += 1;
                if picker.have().get(piece) {
                    have += 1;
                } else if picker.is_in_progress(piece) {
                    started = true;
                }
            }
            if wanted == 0 {
                '-'
            } else if started {
                '*'
            } else if have == wanted {
                '9'
            } else {
                char::from(b'0' + (u64::from(have) * 9 / u64::from(wanted)).min(8) as u8)
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerState {
    /// Not asked yet.
    Waiting,
    Asking,
    Working,
    Failed,
    /// It leads to this computer or its network, so it is never asked.
    Refused,
    /// The engine runs without trackers.
    Off,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TrackerDetails {
    pub url: String,
    /// Added by the user, rather than named by the torrent.
    pub added: bool,
    pub state: TrackerState,
    /// Why the last try failed. It can hold text the tracker sent.
    pub message: Option<String>,
    /// From the last answer, while the tracker keeps answering.
    pub peers: Option<usize>,
    pub seeds: Option<u64>,
    pub leechers: Option<u64>,
    /// Seconds until the next announce or the next try.
    pub next_in: Option<u64>,
}

struct TrackerEntry {
    url: Url,
    added: bool,
    state: TrackerState,
    message: Option<String>,
    answer: Option<TrackerAnswer>,
    next_at: Option<Instant>,
}

/// Every tracker of one torrent and what each last said. The tracker tasks write to it.
#[derive(Default)]
pub struct TrackerBoard(Mutex<Vec<TrackerEntry>>);

impl TrackerBoard {
    pub fn new(trackers: &[Url]) -> Arc<Self> {
        let board = Arc::new(Self::default());
        for url in trackers {
            board.add(url.clone(), false);
        }
        board
    }

    pub fn add(&self, url: Url, added: bool) {
        let mut entries = self.0.lock();
        if !entries.iter().any(|entry| entry.url == url) {
            entries.push(TrackerEntry {
                url,
                added,
                state: TrackerState::Waiting,
                message: None,
                answer: None,
                next_at: None,
            });
        }
    }

    pub fn remove(&self, url: &Url) {
        self.0.lock().retain(|entry| entry.url != *url);
    }

    pub fn refuse(&self, url: &Url) {
        self.update(url, |entry| {
            entry.state = TrackerState::Refused;
            entry.next_at = None;
        });
    }

    fn update(&self, url: &Url, change: impl FnOnce(&mut TrackerEntry)) {
        if let Some(entry) = self.0.lock().iter_mut().find(|entry| entry.url == *url) {
            change(entry);
        }
    }

    pub fn snapshot(&self, trackers_on: bool, now: Instant) -> Vec<TrackerDetails> {
        self.0
            .lock()
            .iter()
            .map(|entry| {
                let working = entry.state == TrackerState::Working
                    || (entry.state == TrackerState::Asking && entry.answer.is_some());
                let answer = entry.answer.filter(|_| working);
                TrackerDetails {
                    url: entry.url.to_string(),
                    added: entry.added,
                    state: if trackers_on {
                        entry.state
                    } else {
                        TrackerState::Off
                    },
                    message: entry
                        .message
                        .clone()
                        .filter(|_| entry.state == TrackerState::Failed),
                    peers: answer.map(|answer| answer.peers),
                    seeds: answer.and_then(|answer| answer.seeders),
                    leechers: answer.and_then(|answer| answer.leechers),
                    next_in: entry
                        .next_at
                        .filter(|_| trackers_on)
                        .map(|next| next.saturating_duration_since(now).as_secs()),
                }
            })
            .collect()
    }
}

impl TrackerObserver for TrackerBoard {
    fn asking(&self, tracker: &Url) {
        self.update(tracker, |entry| {
            entry.state = TrackerState::Asking;
            entry.next_at = None;
        });
    }

    fn answered(&self, tracker: &Url, answer: TrackerAnswer) {
        self.update(tracker, |entry| {
            entry.state = TrackerState::Working;
            entry.message = None;
            entry.answer = Some(answer);
            entry.next_at = Some(Instant::now() + answer.next);
        });
    }

    fn failed(&self, tracker: &Url, error: &str, retry_in: Duration) {
        self.update(tracker, |entry| {
            entry.state = TrackerState::Failed;
            entry.message = clean_text(error, MAX_MESSAGE_CHARS);
            entry.answer = None;
            entry.next_at = Some(Instant::now() + retry_in);
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    On,
    /// Turned off for this engine.
    Off,
    /// A private torrent may only use its own trackers.
    Private,
    /// Not used until a magnet link's details have arrived and the swarm runs.
    Waiting,
}

/// The DHT or peer exchange, and how many new peers it has found for this torrent.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct SourceDetails {
    pub state: SourceState,
    pub peers: usize,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PeerDetails {
    pub connection: ConnectionInfo,
    /// None only for test connections, which have no address.
    pub address: Option<String>,
    /// The name the peer gave, cleaned for showing.
    pub client: Option<String>,
    /// How many pieces the peer says it has.
    pub has_pieces: u32,
    pub download_bps: u64,
    pub upload_bps: u64,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    /// The peer connected to us.
    pub incoming: bool,
    /// The peer lets us download and has something we want.
    pub receiving: bool,
    /// We let the peer download and it wants something we have.
    pub sending: bool,
}

/// Negotiated facts, independent of the preferred encryption policy. In-memory
/// test streams and older helpers have no reported transport, hence Unknown.
#[derive(Serialize, serde::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerTransport {
    #[default]
    Unknown,
    Tcp,
    Utp,
}
#[derive(Serialize, serde::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub transport: PeerTransport,
    pub encrypted: Option<bool>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PieceDetails {
    pub total: u32,
    pub wanted: u32,
    /// Wanted pieces that are finished and passed their hash check.
    pub have: u32,
    pub in_progress: u32,
    /// Pieces read so far, while the files are being checked.
    pub checked: Option<u32>,
    pub map: String,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TorrentDetails {
    /// None until a magnet link's details have arrived.
    pub private: Option<bool>,
    /// None until the torrent's details are known and its files chosen.
    pub pieces: Option<PieceDetails>,
    pub peers: Vec<PeerDetails>,
    pub trackers: Vec<TrackerDetails>,
    pub dht: SourceDetails,
    pub pex: SourceDetails,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitfield::Bitfield;
    use librqbit_core::lengths::Lengths;

    #[test]
    fn text_from_strangers_is_cleaned_and_cut() {
        assert_eq!(
            clean_text("qBittorrent/5.0.3", 40).as_deref(),
            Some("qBittorrent/5.0.3")
        );
        assert_eq!(
            clean_text("  a\r\nb\tc\u{0}  ", 40).as_deref(),
            Some("a b c")
        );
        assert_eq!(
            clean_text("evil\u{202E}gpj.exe", 40).as_deref(),
            Some("evil gpj.exe"),
            "a right-to-left override must not reverse what follows it"
        );
        assert_eq!(
            clean_text("zero\u{200B}width\u{FEFF}", 40).as_deref(),
            Some("zero width")
        );
        assert_eq!(clean_text("\u{7}\u{1b}[31m", 40).as_deref(), Some("[31m"));
        assert_eq!(clean_text(" \u{202A}\u{0} ", 40), None);
        let long = "µ".repeat(100);
        let cut = clean_text(&long, 40).unwrap();
        assert_eq!(cut.chars().count(), 40);
        assert!(cut.ends_with('…'));
        assert_eq!(clean_text("abc def", 5).as_deref(), Some("abc…"));
    }

    fn picker(pieces: u32) -> Picker {
        let lengths = Lengths::new(u64::from(pieces) * 16384, 16384).unwrap();
        Picker::new(lengths, Bitfield::new(pieces), Bitfield::full(pieces), 7)
    }

    #[test]
    fn the_piece_map_has_one_cell_per_piece_when_there_is_room() {
        let mut picker = picker(5);
        picker.piece_passed(0);
        picker.piece_passed(4);
        let mut wanted = Bitfield::full(5);
        wanted.clear(2);
        picker.set_wanted(wanted);
        assert_eq!(piece_map(&picker, MAP_CELLS), "90-09");
    }

    #[test]
    fn a_started_piece_shows_as_in_progress() {
        let mut picker = picker(3);
        let peer = Bitfield::full(3);
        let requests = picker.next_requests(&peer, &|_| false, 1, std::time::Instant::now());
        picker.chunk_written(requests[0].chunk, 1);
        let map = piece_map(&picker, MAP_CELLS);
        assert_eq!(map.chars().filter(|cell| *cell == '*').count(), 1, "{map}");
        assert_eq!(picker.in_progress_count(), 1);
    }

    #[test]
    fn a_large_torrent_is_shrunk_to_the_cell_limit_with_fractions() {
        let mut picker = picker(4096);
        // The first quarter is finished, and one piece in four of the second.
        for piece in 0..1024 {
            picker.piece_passed(piece);
        }
        for piece in (1024..2048).step_by(4) {
            picker.piece_passed(piece);
        }
        let map = piece_map(&picker, MAP_CELLS);
        assert_eq!(map.len(), MAP_CELLS);
        assert!(map[..256].chars().all(|cell| cell == '9'), "{map}");
        assert!(map[256..512].chars().all(|cell| cell == '2'), "{map}");
        assert!(map[512..].chars().all(|cell| cell == '0'), "{map}");
        assert_eq!(piece_map(&picker, 0), "");
    }

    #[test]
    fn forgetting_progress_leaves_only_what_a_new_check_finds() {
        let mut picker = picker(3);
        picker.piece_passed(0);
        let peer = Bitfield::full(3);
        let requests = picker.next_requests(&peer, &|_| false, 1, std::time::Instant::now());
        picker.chunk_written(requests[0].chunk, 1);
        picker.forget_progress();
        assert_eq!(picker.have().count(), 0);
        assert_eq!(picker.in_progress_count(), 0);
        assert!(!picker.is_finished());
        assert_eq!(piece_map(&picker, MAP_CELLS), "000");
    }

    #[test]
    fn the_board_reports_each_tracker_in_order_with_what_it_said() {
        let one = Url::parse("udp://tracker.example.org:1337/announce").unwrap();
        let two = Url::parse("https://tracker.example.net/announce").unwrap();
        let mine = Url::parse("http://open.example.org/announce").unwrap();
        let board = TrackerBoard::new(&[one.clone(), two.clone()]);
        board.add(mine.clone(), true);
        board.add(one.clone(), true);
        let now = Instant::now();
        board.answered(
            &one,
            TrackerAnswer {
                peers: 50,
                seeders: Some(212),
                leechers: Some(37),
                next: Duration::from_secs(1800),
            },
        );
        board.failed(
            &two,
            "tracker said\u{202E} no\r\n",
            Duration::from_secs(100),
        );
        let list = board.snapshot(true, now);
        assert_eq!(list.len(), 3, "a tracker is listed once");
        assert_eq!(list[0].state, TrackerState::Working);
        assert!(!list[0].added);
        assert_eq!(
            (list[0].peers, list[0].seeds, list[0].leechers),
            (Some(50), Some(212), Some(37))
        );
        assert!(list[0].next_in.unwrap() >= 1799);
        assert_eq!(list[1].state, TrackerState::Failed);
        assert_eq!(list[1].message.as_deref(), Some("tracker said no"));
        assert_eq!(list[1].peers, None);
        assert_eq!(list[2].state, TrackerState::Waiting);
        assert!(list[2].added);

        // While a working tracker is asked again, its last answer still shows.
        board.asking(&one);
        assert_eq!(board.snapshot(true, now)[0].peers, Some(50));
        board.refuse(&mine);
        board.remove(&two);
        let list = board.snapshot(false, now);
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|entry| entry.state == TrackerState::Off));
        assert!(list.iter().all(|entry| entry.next_in.is_none()));
    }
}

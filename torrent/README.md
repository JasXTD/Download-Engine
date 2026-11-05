# Torrent engine

Download Engine's own BitTorrent engine. It builds `de-torrent`, a separate program that the
app starts only while a torrent is active, and drives with JSON lines on stdin and stdout.

This folder also works alone, with its own `Cargo.lock`, which is how the tests below and the
lab build it. `src/torrent` in the app is the other end of the pipe.

The formats and network protocols come from the crates copied into `../vendor/rqbit`. See
`../vendor/rqbit/patch.md` for what was changed in them. The engine itself is ours.

## Layout

| File | What it does |
| --- | --- |
| `src/meta.rs` | Reads and checks torrents. Everything else trusts what comes out of it. |
| `src/picker.rs` | Chooses what to ask each peer for. Pure logic with no network or disk. |
| `src/storage.rs` | Reads and writes across file boundaries. |
| `src/peer.rs` | One connection to one peer. |
| `src/admission.rs` | Connection reservations and per-source-IP limits. |
| `src/torrent.rs` | One torrent's life: details, checking, swarm, pausing. |
| `src/metadata.rs` | Fetches a magnet link's details from a peer. |
| `src/discovery.rs` | Finds peers through trackers and the DHT. |
| `src/resume.rs` | Remembers finished pieces between runs. |
| `src/details.rs` | What the status window shows: the piece map, peers, and each tracker's last answer. |
| `src/session.rs` | All torrents, the listener, and what they share. Decides what each magnet may use to find its details. |
| `src/router.rs` | Router setup (UPnP): asks the default gateway to forward the incoming port, and removes the rule again. |
| `src/reason.rs` | Errors the app recognises by a code and explains in the person's language. |
| `src/cli.rs` | The `de-torrent` program. The app builds it as `src/bin/de-torrent.rs`. |

## Rules to keep

- Everything from a peer, tracker, torrent file or magnet link comes from a stranger. Check
  each number before using it, and give each queue a limit.
- A tracker address must not lead to this computer or its network, and a tracker can't
  redirect us anywhere.
- Discovered peers and DHT nodes follow that same public-address policy. The legacy
  `allow_loopback_peers` option explicitly enables LAN traffic for isolated tests; the
  desktop helper never enables it. Explicit test/lab peer addresses remain supported.
- A discovered peer or DHT node on a service port is never contacted: everything below
  1024, and a short list of well-known non-torrent ports such as SIP 5060
  (`is_service_port` in the copied `librqbit-core`). Poisoned lists can't turn this client
  against someone else's server. Trackers, incoming connections and named peers don't use
  this rule.
- An unknown magnet's hash stays off DHT. Only hash-verified public metadata enables
  DHT discovery; trackerless magnets need cached metadata or a `.torrent` file, or the
  person's yes for that magnet (`dht_search`). Such a search only asks DHT nodes for peers
  and announces nothing until the metadata is checked and public. Private metadata found
  this way stops the torrent and is not saved. A search with no metadata ends after 10
  minutes. Without the DHT on, a yes changes nothing.
- Router setup runs only with `--upnp` and a listening port. It is the only code that may
  reach this computer's network, and it reaches only the private IPv4 default gateway from
  the routing table: answers from anywhere else are ignored, and every router address must
  be plain `http` to that gateway's IP, with no names, redirects or proxy. Answers stop at
  64 KiB and three seconds, XML with a DOCTYPE is refused, and only a few fields are read.
  It asks for one TCP rule to this computer with a one-hour lease, renewed every 30
  minutes, refuses permanent-only routers, never touches another device's rule, and on
  shutdown removes only a rule it still owns, named with its own process number.
- A piece counts only after its SHA-1 matches.
- File names on disk come only from `TorrentFile::path`. `display` is for showing.
- Reading from a peer must never wait on writing to it, or two peers can block each other.
- However a connection ends, what it held must go back to the picker. `Reader`'s `Drop`
  does this, so that a lost peer can't leave chunks stuck as requested.
- Tests must never reach a network. Torrents only connect out when `connect_out` is set,
  which test helpers leave off, and on Windows `connect` panics in a test build.
- Client names and tracker messages are a stranger's text. `details::clean_text` removes
  control, hidden and direction characters and cuts them short before anyone sees them.
- A tracker the user adds follows the rules for trackers in torrent files, and is only ever
  asked about a torrent known to be public. Private torrents refuse added trackers.

## Resource limits

- At most 32 trackers, each URL at most 2,048 bytes. At most four tracker requests per
  torrent and eight across the process, with permits released on cancellation. Sleeping
  trackers do not hold permits. HTTP bodies stop at 1 MiB even without Content-Length;
  announce intervals have a five-second minimum. HTTP and UDP request at most 200 peers
  and cap each response at 200 results even if the tracker ignores that request.
- At most 50 queued, connecting or active connections per torrent, and four per source IP
  across incoming and outgoing connections. IPv4-mapped IPv6 addresses share the IPv4
  limit. Reservations last until a connection closes, fails or is cancelled. The listener
  separately limits unfinished handshakes to 32 total and four per source IP.
- Discovery queues at most 128 addresses. Trackers, DHT and peer exchange share a budget
  of 5,000 unique addresses, including explicit peers and queued results; duplicate or
  rejected addresses do not spend it. Tracker announcements continue after the book fills.
- Metainfo accepts at most 10,000 files, 64 components per path, 1,024 raw bytes per
  component, 4,096 bytes per path, and 4 MiB of aggregate raw or decoded filename bytes.
  Collection limits apply during deserialization, before growing untrusted lists.
- A peer may supply up to 32 MiB of metadata. Reservations cover lazy chunks and the
  final contiguous result: 64 MiB per torrent and 128 MiB across the process. Reservations
  remain held through hash/structure validation and release on error or cancellation.
  A repeated metadata handshake ends that peer's attempt.
- The desktop starts the Windows helper with a 1 GiB process-memory limit as a backstop.
  Reaching it can stop the helper; the app reports that failure and retains staged data.

## Commands

Each line on stdin is a JSON object with a `cmd`, and gets one reply line: `{"ok":true,
"result":...}` or `{"ok":false,"error":"..."}`.

| Command | Fields | Result |
| --- | --- | --- |
| `files` | `torrent` (path) or `name` | The file list and info hash. |
| `add` | `name`, `output`, then `torrent` (path) or `magnet`, and optional `only`, `peers`, `choose_later`, `seed_only`, `file_name`, `trackers`, `dht_search` | The info hash. |
| `select` | `name`, `only` | Changes the chosen files at any time. |
| `details` | `name` | `private`, `pieces` (counts and a `map` of up to 1,024 cells), `peers`, `trackers`, and `dht` and `pex` with the peers each found. |
| `recheck` | `name` | Stops the swarm, forgets saved progress, and reads every chosen piece again. A running torrent starts again by itself. |
| `add_tracker`, `remove_tracker` | `name`, `url` | Adds a tracker the user typed, or removes one they added. Refused for private torrents. |
| `import` | `torrent` (path) | Saves the torrent's details, so its magnet link needs nothing else. Returns the same as `files`, with a `magnet`. |
| `remove` | `name` | Stops a torrent and lets go of its files, which stay on disk. |
| `configure` | `upload_limit`, `no_upload`, optional `upnp` | Changes uploading for every torrent at once. `upnp` turns router setup on or off; off waits until the rule is removed. |
| `allow` | `name`, `until` | Lets a torrent receive `until` bytes in all, or any amount with null. Returns `received`. |
| `stats` | optional `name` | Every torrent's state and progress, and session totals, or one torrent's. Both carry `router` while router setup is on. |
| `pause`, `resume` | `name` | |
| `stop_all`, `resume_all` | | The names affected. |
| `exit` | | Stops everything and saves progress. |

A torrent's `state` is one of `metadata`, `ready`, `initializing`, `live`, `finished`,
`paused` or `error`. A refused command's reply, and an `error` torrent's stats, carry a
`code` (`error_code` in stats) for the reasons in `src/reason.rs`: `needs_dht_consent`,
`dht_off`, `private_from_dht` and `dht_search_timeout`.

`router` is `{"state": …}` with one of `looking`, `working` (with `gateway` and `port`),
`no_router`, `taken` (with `port` and `other`, the other device's address or null),
`permanent_only`, `shared` (with `external`), `refused`, `unavailable` or `failed`. It
holds only addresses and a port, never the router's own text.

In a piece `map`, each character covers an equal run of pieces: `-` when none of them is
wanted, `*` when a wanted one has started but isn't finished, and `0` to `9` for how much of
the wanted ones is finished, with `9` only when all are. A tracker's `state` is `waiting`,
`asking`, `working`, `failed`, `refused` (it leads to this computer or its network) or `off`.

Options: `--port`, `--upload-limit`, `--download-limit`, `--state-dir`, `--no-seed`,
`--no-upload`, `--trackers`, `--dht`, `--upnp`. Trackers, the DHT, incoming connections and
router setup are off unless asked for, and `--upnp` does nothing without `--port`.
Set `DE_TORRENT_DEBUG=1` to write diagnostics to stderr.

The desktop passes `--port` only after an explicit incoming-connection opt-in.
An `add` request with `seed_only: true` requires known metadata and existing selected
files. It rehashes them, uses read-only handles, never creates or downloads skipped
files, and advertises only verified pieces. `file_name` maps a renamed single-file
torrent to its current safe file name. These sharing sessions do not overwrite
download resume state. Partial seeds report the full torrent's remaining bytes to
trackers, rather than claiming that an incomplete selection is a complete seed.

## Testing

```powershell
cargo test --features lab
```

These tests open no sockets, so they cause no firewall prompts. Peers are joined by
in-memory pipes. `src/network_tests.rs` uses real sockets and only builds on Linux, so
run it in a container with `--network none`.

The test lab in `artifacts/torrent-lab` runs real transfers with unchanged rqbit on a
private Docker network, using `run-lab.cjs test ours`.

## Not done yet

- HTTPS trackers only work in the app's build, which brings the TLS library. Built alone,
  this folder only reaches plain HTTP and UDP trackers.
- There is no peer encryption, uTP or web seeds. Router setup speaks UPnP IGD only (no
  NAT-PMP, PCP or IPv6 openings), and reads the routing table on Windows and Linux only.
- Peer exchange is only read. The engine doesn't send peer lists of its own.
- `add` reuses files already in `output`, so the app must choose a fresh folder for a new
  download.
- A skipped file can appear on disk at full size, as a sparse file holding only the
  bytes of pieces shared with a chosen file.

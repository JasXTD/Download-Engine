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
| `src/torrent.rs` | One torrent's life: details, checking, swarm, pausing. |
| `src/metadata.rs` | Fetches a magnet link's details from a peer. |
| `src/discovery.rs` | Finds peers through trackers and the DHT. |
| `src/resume.rs` | Remembers finished pieces between runs. |
| `src/session.rs` | All torrents, the listener, and what they share. |
| `src/cli.rs` | The `de-torrent` program. The app builds it as `src/bin/de-torrent.rs`. |

## Rules to keep

- Everything from a peer, tracker, torrent file or magnet link comes from a stranger. Check
  each number before using it, and give each queue a limit.
- A tracker address must not lead to this computer or its network, and a tracker can't
  redirect us anywhere.
- A piece counts only after its SHA-1 matches.
- File names on disk come only from `TorrentFile::path`. `display` is for showing.
- Reading from a peer must never wait on writing to it, or two peers can block each other.
- However a connection ends, what it held must go back to the picker. `Reader`'s `Drop`
  does this, so that a lost peer can't leave chunks stuck as requested.
- Tests must never reach a network. Torrents only connect out when `connect_out` is set,
  which test helpers leave off, and on Windows `connect` panics in a test build.

## Commands

Each line on stdin is a JSON object with a `cmd`, and gets one reply line: `{"ok":true,
"result":...}` or `{"ok":false,"error":"..."}`.

| Command | Fields | Result |
| --- | --- | --- |
| `files` | `torrent` (path) or `name` | The file list and info hash. |
| `add` | `name`, `output`, then `torrent` (path) or `magnet`, and optional `only`, `peers`, `choose_later` | The info hash. |
| `select` | `name`, `only` | Changes the chosen files at any time. |
| `import` | `torrent` (path) | Saves the torrent's details, so its magnet link needs nothing else. Returns the same as `files`, with a `magnet`. |
| `remove` | `name` | Stops a torrent and lets go of its files, which stay on disk. |
| `configure` | `upload_limit`, `no_upload` | Changes uploading for every torrent at once. |
| `allow` | `name`, `until` | Lets a torrent receive `until` bytes in all, or any amount with null. Returns `received`. |
| `stats` | | Every torrent's state and progress, and session totals. |
| `pause`, `resume` | `name` | |
| `stop_all`, `resume_all` | | The names affected. |
| `exit` | | Stops everything and saves progress. |

A torrent's `state` is one of `metadata`, `ready`, `initializing`, `live`, `finished`,
`paused` or `error`.

Options: `--port`, `--upload-limit`, `--download-limit`, `--state-dir`, `--no-seed`,
`--no-upload`, `--trackers`, `--dht`. Trackers, the DHT and incoming connections are off unless asked for.
Set `DE_TORRENT_DEBUG=1` to write diagnostics to stderr.

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
- There is no peer encryption, uTP, web seeds or router port mapping.
- Peer exchange is only read. The engine doesn't send peer lists of its own.
- `add` reuses files already in `output`, so the app must choose a fresh folder for a new
  download.
- A skipped file can appear on disk at full size, as a sparse file holding only the
  bytes of pieces shared with a chosen file.

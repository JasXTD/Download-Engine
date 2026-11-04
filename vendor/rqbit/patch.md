# rqbit protocol crates, modified for Download Engine

These nine directories are published crates from [rqbit](https://github.com/ikatson/rqbit),
copied here so that Download Engine can maintain them itself. They form a separate Cargo
workspace: `Cargo.toml` in this directory maps each crate name to its directory through
`[patch.crates-io]`, and the application's `Cargo.toml` does the same. The engine in
`torrent/` is what uses them.

| Directory | Version | crates.io SHA-256 |
| --- | --- | --- |
| `librqbit-bencode` | 9.1.0 | `0cf295fa65b3ad66bd363a5d3a278730352745b72ff0f5b6e6010612351924e5` |
| `librqbit-buffers` | 9.1.0 | `6a11501f6b2d68169725cf9e030d5ad7a9ede4b3839a9e16a27c4faef7507c8c` |
| `librqbit-clone-to-owned` | 9.1.0 | `c611ae410046c2e2cc90b0f300d53fe368d5129f26d822cede8f605f33f7fa84` |
| `librqbit-core` | 9.1.0 | `af3d5c9eec46a17b87123cee6280126056590143dd1bda7743681297e27737eb` |
| `librqbit-dht` | 9.1.0 | `558e74c5bcbb270fc19e1cf0517c26ed132f96c29b43b41f5ec5ce22470393a3` |
| `librqbit-dualstack-sockets` | 0.7.0 | `f7189f1ff66ca75055137a38b062ba7df9691d071b7172e5090997c62eb4b4de` |
| `librqbit-peer-protocol` | 9.1.0 | `88b018302dbb1d053dfe34aca20b200e9c434d2bc8ea27e41875771bd31008ef` |
| `librqbit-sha1-wrapper` | 9.1.0 | `4633bfa4c77dd7153be92584947d669a0ff8f61b82bb8bf283ff5f11ddee75be` |
| `librqbit-tracker-comms` | 9.1.0 | `3753f7f7a0dd00cf9fb088d2ba203c7e4df93028f922b5da5ac42e14cdf9b2e7` |

The upstream commits are in each `.cargo_vcs_info.json`: `4309d387` for the rqbit crates
and `e2f221ca` for `librqbit-dualstack-sockets`, which has its own repository.
`Cargo.toml.orig` in each directory is the upstream manifest and is left as published.

All nine are licensed under Apache-2.0 (`docs/licenses/common/Apache-2.0`). Only
`librqbit-dualstack-sockets` ships a license file of its own. The changes below are
Download Engine's and are released under the same terms. Every changed Rust file and
manifest says so near its top, and every added file carries its own note.

## Why

Download Engine needs BitTorrent's formats and network protocols, but not rqbit's client.
The hard, self-contained parts are kept here: bencode, torrent files, magnet links, the
peer wire format, trackers and the DHT. The download engine itself is written separately.
Everything that served rqbit's web interface or features we are not building was removed,
and small libraries were replaced, so that these crates add as little as possible to the
application.

## What changed

### Removed

- `librqbit-dualstack-sockets`: axum web server support and `src/multicast.rs`, which only
  local peer discovery and router port mapping need. `network-interface` went with it.
- `librqbit-sha1-wrapper`, `librqbit-core`: the aws-lc hash backend and its `sha1-ring`
  feature, because it brings in a large C library that nothing else here needs.
- `librqbit-core`: `src/directories.rs`. The application chooses where files live, so
  nothing may fall back to an rqbit folder. `DhtPersistenceConfig::config_filename` in
  `librqbit-dht` is now required.
- `librqbit-dht`: the `DHT_QUERIES_PER_SECOND` environment variable. A bad value stopped
  the process, and the limit is now the constant 250.
- Tests and tools that contact public servers or need extra libraries:
  `librqbit-dht/examples/dht.rs`, `test_announce` in `tracker_comms_udp.rs`, the test in
  `librqbit-dualstack-sockets/src/bind_device/tests.rs`, the benchmark in
  `librqbit-peer-protocol/benches/`, and test logging in `librqbit-dualstack-sockets`.

### Libraries replaced

| Library | Now |
| --- | --- |
| `hex`, `data-encoding` | `decode_hex` and `decode_base32` in `librqbit-core/src/hash_id.rs` |
| `atoi` | `parse_number` in `librqbit-bencode/src/deserialize.rs` |
| `byteorder` | The standard `to_be_bytes` and `from_be_bytes` |
| `urlencoding` | `push_percent_encoded` in `librqbit-tracker-comms/src/tracker_comms_http.rs` |
| `rand` | `librqbit-core/src/random.rs`, on `getrandom` |
| `backon` | `librqbit-core/src/backoff.rs` |
| `leaky-bucket` | `librqbit-dht/src/rate_limit.rs` |
| `async-stream` | `TrackerPeers` in `librqbit-tracker-comms/src/tracker_comms.rs` |
| `serde_with` | Serde's own reading of IP addresses from text |
| `dashmap` | `HashMap` behind a `parking_lot` lock in `librqbit-dht` |
| `futures` | `futures-util`, under the name `futures`, without default features |
| `crypto-hash` | `sha1` and `sha2` in `librqbit-sha1-wrapper` |

`bitvec`, `indexmap` and `tokio-stream` had no callers. `tracing` no longer uses its default
features, because nothing here uses its `#[instrument]` macro. The `sha1-crypto-hash` feature
is gone, because hashing now has one implementation and is always available.

### Problems fixed

All of these come from data that strangers control, and each has a regression test.

- Torrent resource limits: `librqbit-core/src/torrent_limits.rs` bounds file, path and
  tracker collections while decoding. Validation bounds raw and decoded filename bytes
  before constructing saved paths and duplicate sets. Magnets bound tracker count and
  URL length before adding them. Oversized input is rejected, not silently truncated.
- Trackers: HTTP bodies are read in chunks with a 1 MiB cap, including absent or dishonest
  Content-Length. HTTP/UDP polling has a five-second floor. Tracker
  sets are capped at 32 URLs of 2,048 bytes; semaphore permits bound simultaneous requests
  to four per torrent/eight process-wide without starving trackers behind sleeping monitors.
- Tracker peer results: HTTP and UDP request 200 peers and enforce that cap regardless of
  `numwant` compliance. HTTP compact IPv4/IPv6 results share one iteration limit; oversized
  dictionary lists are rejected during decoding. UDP validates the record boundaries and
  allocates only the first 200 addresses, ignoring extra complete records.
- Public endpoints: `librqbit-core/src/network.rs` shares the existing address policy with
  the engine and DHT. DHT filters discovered nodes/peers and guards request submission,
  incoming messages and final UDP sends, including bootstrap and persisted nodes.
  The RFC 8215 local-use translation prefix `64:ff9b:1::/48` is blocked because a
  network translator can use it to reach private IPv4 destinations.
  `allow_local_addresses` is an explicit isolated-LAN opt-in, off by default.
- DHT recursive work: each lookup admits at most 256 queued or active nodes and creates
  at most eight concurrent request futures, within a shared 4,096-node admission budget
  across torrent searches, bootstrap and routing refreshes. Pending endpoints and IDs
  remain deduplicated independently of cache eviction until completion or cancellation.
  Full queues discard
  additional discoveries without changing retry timestamps. Peer results share a bounded
  256-entry queue across IPv4/IPv6 discovery; a slow consumer cannot stall recursion or
  accumulate unlimited results. Cancelled requests also release response-tracking entries.
  Socket-free tests feed decoded responses through both lookup schedulers and cover floods,
  queue draining, duplicate nodes, timeouts and cancellation.

- File names on Windows: `validate` blocked only `..` and slashes, so `C:evil.dll`,
  `file.txt:stream`, `CON` and trailing dots passed, and a drive prefix replaced the
  download folder when joined to it. `src/safe_name.rs` (new) applies the rules of
  `safe_filename` in the application's `src/model.rs`, and `to_pathbuf` uses it.
  `ValidatedTorrentMetaV1Info::safe_name` does the same for the torrent's name. Names that
  would be the same file on Windows, ignoring case, are now rejected as duplicates.
- Base32 hashes: a padded hash decoded to too few bytes and crashed. Anything that does not
  decode to exactly the right length is now an error, and padding is accepted only at the
  end.
- Bencode numbers: a number too large for its type wrapped around or panicked. It is now an
  error, and `i-e` and `i+5e` are rejected. A huge number in a field nothing reads is still
  skipped, because bencode has no size limit.
- Magnet links: `so=0-18446744073709551615` crashed and a smaller range could fill the
  memory. Selections past `MAX_SELECTED_FILES` are ignored, like other values that fail to
  parse.
- Torrent lengths in `src/lengths.rs`: a piece length of 0 divided by zero, and huge sizes
  overflowed. These are now errors. Piece and chunk numbers are kept in 32 bits, so torrents
  past about 70 TiB are refused.
- Trackers: peers still queued when the last tracker stopped were lost. `TrackerPeers`
  delivers them before the stream ends.
- Trackers: a name that led to this computer or its network was used anyway.
  `is_local_address` and the `public_only` argument of `UdpTrackerClient::new` keep UDP
  trackers away from those addresses. The engine does the same for HTTP trackers.
- Extended handshakes: leaving out an extension failed the whole message, because bencode
  has no empty value. Private torrents must leave out peer exchange, so absent ids are now
  skipped when writing.

### Moved test files

Tests read `ubuntu-21.04-desktop-amd64.iso.torrent` from `librqbit-core/src/resources/test/`
and `extended-handshake.bin` from `librqbit-peer-protocol/src/resources/test/`. Upstream
keeps them in a crate that is not copied here.

## Not done here

- Nothing checks that a torrent's `pieces` field has one hash for every piece. `get_hash`
  returns `None` for a missing one, and the engine must treat that as an invalid torrent.
- A magnet link's `so=` values are not checked against the torrent's file list, and its
  `dn=` name is not made safe. The engine ignores `so=`, and the application cleans `dn=`.
- Peer encryption, rarest-first piece selection, web seeds, uTP, router port mapping and
  local peer discovery are not in these crates.
- `scripts/Build-LicenseBundle.cjs`, `scripts/release-source-inventory.json` and the notices
  must list these crates before the application can depend on them.

## Testing

On Windows, run this from this directory:

```powershell
cargo test --workspace --exclude librqbit-dualstack-sockets --no-fail-fast -- --skip test_dht_answers
```

The left-out tests open listening sockets, which can bring up Windows Firewall prompts. Run
them on Linux in a container with no network, where the whole workspace can be tested:

```sh
cargo test --offline --locked --workspace --no-fail-fast
```

## Checking or updating the copy

Download a crate, compare its SHA-256 with the table above, and compare the contents:

```powershell
$name = 'librqbit-dht'; $version = '9.1.0'
$file = "$env:TEMP\$name-$version.crate"
Invoke-WebRequest "https://static.crates.io/crates/$name/$name-$version.crate" -OutFile $file
(Get-FileHash $file -Algorithm SHA256).Hash
tar -xzf $file -C $env:TEMP
git diff --no-index "$env:TEMP\$name-$version" "vendor\rqbit\$name"
```

Only the changes listed above should differ. The first commit that added this directory
holds the crates exactly as published, so `git log -- vendor/rqbit` shows every later
change. When taking a newer rqbit release, compare it with these versions first and
re-apply the fixes above to whatever it changed.

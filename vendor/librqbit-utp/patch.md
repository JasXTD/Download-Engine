# librqbit-utp, maintained by Download Engine

Base: Igor Katson's `librqbit-utp` 0.7.0, published on crates.io on 2026-02-14.
Official source: <https://github.com/ikatson/librqbit-utp>.
Archive SHA-256: `4f3bfdc73944bc76cab24d5690a98816770040a654c449edf5ff2b9ba22626aa`.
The archive and its source files were checksum-verified; the user confirmed a clean
Malwarebytes scan on 2026-11-08. The upstream Apache-2.0 notice is retained in `LICENSE`.

## Changes

- Bound the dispatcher-to-stream packet queue to 128 datagrams. Drop packets on overload
  without blocking the shared UDP dispatcher or removing the connection. UDP loss recovery
  can retransmit them. The original unbounded queue allowed memory growth before the
  stream's receive-window checks.
- Match a SYN acknowledgment against both its sequence number and its connection ID.
  Previously, an acknowledgment with the wrong ID could consume a pending connection.
- Fix the extension serializer's link byte: adding a second extension must update the
  previous extension's next-type byte, not overwrite its length.
- Count pending connections toward the socket limit and limit active/pending connections
  to four per canonical source IP. Bound, deduplicate and expire cached SYNs.
- Add outgoing-only mode, which drops unsolicited SYNs without caching or replying.
- Track dispatcher and stream tasks for cancellation and joining. Expose immediate stream
  abort so Pause/Stop can discard buffered uploads instead of waiting for a graceful FIN.
- On abort, send one best-effort RESET with the connection's current sequence/ACK values.
  Previously cancellation left remote peers waiting, and encrypted libtorrent connections
  could reject subsequent attempts. Keep the send synchronous and nonblocking so endpoint
  permissions still apply; never flush buffered data, wait, retry or spawn cleanup work.
  A shared header snapshot tracks outgoing sequence/ACK changes; repeated abort is idempotent.
- Expose the default environment for a policy-filtering UDP transport wrapper.
- Keep the upstream in-memory unit tests. Adapt their packet injection to the bounded
  queue without reducing the data-integrity payload. Replace one numeric assertion helper
  with the identical absolute-error criterion; avoid process-wide environment mutation
  in test logging; allow binary payloads in the test formatter.
- Retain the upstream libutp-rs2 interoperability source for reference, but do not compile
  that separate native dependency or run its network tests on the user's Windows system.
  Socket integration tests belong in Download Engine's isolated Linux lab.
- Reduce the Cargo manifest to the library, its normal dependencies and the dependencies
  used by the in-memory tests. `Cargo.toml.orig` preserves the upstream authoring manifest.

## Review and evidence

The three original defects above were reproduced by failing regression tests before
their fixes. Review covered packet/header/extension parsing, receive and transmit queues,
pending connection admission, retry and inactivity timers, cancellation and task lifetime.
The local suite currently passes 80 tests, including arbitrary/truncated packet inputs,
overload/recovery, wrong connection IDs, outgoing-only mode, capacity release, shutdown,
abort notification in both directions, no buffered-data flush, and current RESET numbers.
This is a focused review and test result, not a claim that the library is free of bugs.

The library implements reliable UDP using **CUBIC**, not LEDBAT congestion control.
Do not advertise it as guaranteeing lower priority than browsing traffic. Keep it optional.

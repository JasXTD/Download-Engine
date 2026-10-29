# Performance measurements

## Download-list responsiveness — 3.4.1

Measured on 2026-10-29 against the 3.4.0 source at commit `9d9503e`, using the same opt-in fixture in both binaries. These are **unoptimized debug-build, headless egui whole-frame measurements**, not production frame rates or download-throughput improvements.

| 5,000-download view | Before mean frame time, median of blocks | After mean frame time, median of blocks | Reduction |
| --- | ---: | ---: | ---: |
| All files | 97.21 ms | 22.94 ms | 76.4% |
| Filename search | 93.61 ms | 26.69 ms | 71.5% |

[Raw measurements and executable hashes](benchmarks/3.4.1-ui.json) record two blocks per version, alternating before/after/after/before. Each scenario warms five frames and measures sixty frames at 1280×860 logical pixels. The queue contains 5,000 paused downloads; all data is synthetic and temporary. No Cargo or rustc processes were observed at any block boundary. The earlier measurements made during compilation are excluded. This is still one shared workstation and excludes GPU presentation, active network transfers and real user profiles.

The list now snapshots only rows being drawn, instead of cloning all matching jobs and their nested download metadata. Sidebar counts reuse one filename-search pass. Toolbar and selection indicators borrow selected records, and selection reconciliation uses a set of visible IDs. Empty searches skip lowercase filename allocations. Existing row order, selection semantics, virtual scrolling, commands and stored formats are retained.

Run the fixture with `cargo test --offline --locked --bin de desktop::ux_tests::large_download_list_frame_time -- --ignored --exact --nocapture --test-threads=1`. For comparisons, preserve both test executables and run them after compilation has finished. There is no timing pass/fail threshold.

### Engine assessment

This patch does not change transfer algorithms or claim faster Internet downloads. The existing optimized loopback benchmark completed all 15 ordinary-file, ranged-file and HLS transfers, including enabled-quota cases, with exact size and SHA-256 verification. Its timings varied while build work was active, so they are not used to claim a speed change.

The engine already uses bounded parallel ranges, adaptive tail splitting, buffered writes, connection policies, durable quota accounting, and single-part publication without another payload copy. Multi-part completion still reads and copies each saved part into an ordered staging file while hashing it. Reducing that extra disk copy is a candidate for a separate measured engine change, especially for large files on fast connections. A shared-file design would need a new durable record of completed byte ranges and crash/restart tests before it could replace the current per-part-length recovery mechanism. Raising connection counts, weakening validators, batching away durable quota commits or skipping flushes is not part of this patch.

### Checks

Desktop regressions cover selection, Unicode search, keyboard navigation and virtual scrolling, custom column widths and ordering, queue commands, translated controls and status-error tooltips. The native capture workflow is `node scripts/native_ui_smoke.cjs --download-list` with a screenshot build. It uses isolated paused profiles at minimum and wide sizes, dark/light themes, empty and filtered results, keyboard focus, German and Arabic. Human linguistic review and real Internet throughput remain separate from these automated checks.

## Large-file completion — 3.1.1

Measured on 2026-10-27 against 3.1.0. Same-volume, single-part HTTP/HTTPS and FTP downloads now hash and publish the saved part without writing another complete copy. Multi-part and cross-volume downloads retain asynchronous assembly. This changes the finishing phase, not the server's transfer speed.

### Results

Each case finishes an already-saved 512 MiB file. The final controlled follow-up uses three samples per case/version, with a separate temporary root cleared after each block and no other Cargo processes observed at block boundaries. Every sample verifies the exact output size and SHA-256. These local finishing times do not predict Internet download speed.

| Saved parts | 3.1.0 median (ms) | 3.1.1 median (ms) | Before / after timing range (ms) | Before / after process write bytes |
| --- | ---: | ---: | --- | --- |
| One | 1718.4 | 383.4 | 1702.9–2236.2 / 381.6–396.1 | 536,879,178 / 8,266 |
| Four | 1759.6 | 1790.3 | 1611.7–2305.4 / 1736.7–1803.8 | 536,879,178 / 536,879,178 |

Single-part finishing is about 4.5 times faster in this fixture. The repeatable I/O result is the eliminated payload rewrite: every single-part sample writes exactly 512 MiB fewer bytes. The remaining 8,266 bytes are metadata/database traffic. Both versions still read the full payload for checksum verification. These Windows process I/O counters are not physical device-write measurements. Multi-part timing ranges overlap; no multi-part speedup is claimed.

[Raw final measurements](benchmarks/3.1.1.json) retain the 12 controlled follow-up samples and all 36 earlier alternating samples, with background Cargo process counts. The earlier run had wide timing ranges and a test-fixture cleanup bug: its cached SQLite handle delayed temporary-directory deletion. All identified leftovers were removed, the fixture now closes its Store before deleting its directory, and an isolated regression run confirmed zero leftover entries. The timed production finishing code did not change for that test-only correction.

An earlier synchronous-copy experiment showed no reliable multi-part improvement, so that copy implementation was discarded. Its [complete measurements](benchmarks/3.1.1-experiment.json) are retained separately; they are not the final build's results.

### Method and implementation

- Optimized Windows x86-64 binaries; AMD Ryzen 7 3800X; rustc 1.99.0 (b940084d7, 2026-09-28). The frozen baseline is commit 3c09dd8f289b10489ebc36cbacf33a0c42c4c6cd, version 3.1.0.
- Each fixture creates and syncs deterministic bytes before timing, leaving a warm filesystem cache. Timing covers finalization: verification, any assembly, publication intent and publication. A separate output hash check follows the timer. It excludes network transfer, fixture creation and completion-cache cleanup.
- The initial blocks alternate before, after, after, before, before, after; the controlled follow-up runs one before block and one after block. Each block runs three samples of both cases. Only one benchmark process runs at a time, after this task's compilation, packaging and runtime tests have finished. Other local test activity is recorded. The follow-up's dedicated temporary roots and their removal are recorded as well.
- Single-part publication retains a guarded file handle through the completion transaction. The cache name remains a recovery anchor until completion is durable. Recovery checks file identity, size and the previously saved checksum; filename collisions never overwrite an existing file. The saved-job format is unchanged.
- Multi-part copies retain ordered asynchronous reads/writes. Cancellation joins outstanding writes before closing or removing staging; original parts remain available for retry. HLS/DASH and FFmpeg merging retain their existing paths.

To run the current finishing benchmark:

~~~powershell
cargo test --release --offline --locked --lib engine::finalization_tests::finalization_throughput -- --ignored --exact --nocapture --test-threads=1
~~~

The benchmark has no timing pass/fail threshold. It removes its temporary profiles and payloads. Compare independently built baseline and candidate executables on an idle machine for a cleaner latency estimate.

### Verification

Final optimized regressions cover single-part publication without a copy, ordered assembly, cancellation during hashing/copying and successful retry, checksum/size failures, failed publication-intent persistence, recovery without a duplicate, modified published bytes, open filename collisions, and a redirected Windows download folder. A real C:→D: test verifies both one-part and multi-part copying and interrupted-publication recovery; its temporary folder was removed.

The packaged 3.1.1 application downloaded 4,362,076,177 generated bytes over localhost. It paused at 33,554,432 bytes, resumed, was terminated after more than 4 GiB was saved, then resumed from the actual saved offset of 4,303,237,776 bytes. The final byte count and SHA-256 matched, with one job, one output file and no partial cache. The generated payload and isolated profile were removed after retaining the verification report.

Packaged HLS and DASH checks passed, including exact HLS bytes, merged DASH packet hashes/timing, successful decoding, checksums and Mark of the Web. Their fixtures were cleaned up. Library, ordinary-file, FTP, HLS/DASH, synchronization, storage-failure and pause/resume checks were run during implementation; affected engine and media suites were rerun after the final copy-path revision. Formatting, strict all-target/all-feature linting and release-version checks passed. These checks do not measure live Internet speed or re-test installer installation/uninstallation.

## Transfer performance — 3.1.0

Measured on 2026-10-27 against 3.0.2, using optimized Windows binaries and the same loopback fixtures. These are shared-workstation measurements: separate Cargo test runs remained active during every block. Treat the medians as observations for these fixtures, not predictions of Internet download speed. The ordinary-file differences are within the observed timing variation.

### Results

Each row has nine samples per version. Times include the download engine's transfer, verification, assembly, publication and cleanup; fixture creation and preview happen before timing. Every output is checked for exact size and SHA-256 after the timed run. Quota cases also assert the exact persisted received-byte count.

| Workload | 3.0.2 median (ms) | 3.1.0 median (ms) | Throughput ratio | Before / after timing range (ms) |
| --- | ---: | ---: | ---: | --- |
| Video segments | 3257.2 | 1063.9 | 3.06× | 3145.5–5455.3 / 1011.4–1598.1 |
| Video segments with quota | 2895.4 | 877.8 | 3.30× | 2685.7–3484.4 / 787.6–1391.2 |
| Single-stream file | 288.2 | 275.2 | 1.05× | 261.5–1147.7 / 251.5–851.8 |
| Parallel file ranges | 267.0 | 267.6 | 1.00× | 241.2–959.8 / 233.0–1208.1 |
| Parallel file ranges with quota | 543.2 | 309.9 | 1.75× | 441.5–1500.4 / 291.1–1219.9 |

Video peak simultaneous responses changed from 1 to 4 in every sample. The video gains are consistent across the observed ranges. File downloads with quotas show a lower median; broad ranges and background activity limit precision. Ordinary single-stream and ranged-file throughput was essentially unchanged.

[Raw measurements and background-process counts](benchmarks/3.1.0.json) include all 90 successful transfers. The initial pre-change warm-up run is excluded from the table.

### Method

- CPU: AMD Ryzen 7 3800X, 8 cores / 16 logical processors; Windows x86-64.
- Compiler: rustc 1.99.0 (b940084d7, 2026-09-28), x86_64-pc-windows-msvc, release profile.
- Baseline source: commit 235a1bfc3e3601616a4cc6b8ef626bd79d7947b2, application version 3.0.2. The baseline benchmark was built and copied aside before implementation.
- Video: 16 valid MPEG-TS segments, 770,048 bytes each (12,320,768 bytes total). The local server sends 64 KiB chunks with a requested 3 ms delay per chunk. Windows timer scheduling contributes to the observed durations; this models a paced source, not a real Internet path.
- Files: 32 MiB of deterministic bytes, sent in 16 KiB writes. The ranged endpoint supplies a stable strong ETag and supports four workers; the single-stream endpoint does not support ranges.
- Quotas: a fresh 1 GiB hourly allowance for each enabled case. Recording each received payload remains durable.
- Order: before, after, after, before, before, after. Each block runs three samples of every case. Only one benchmark process runs at a time, using fresh temporary databases and output folders. Compilation and visual-capture work had finished before measurement. Other test activity is recorded in the raw report.
- All fixtures are local. Temporary payloads and profiles are removed by the test harness.

To run the current version's benchmark:

~~~powershell
cargo test --release --offline --locked --test throughput -- --ignored --nocapture --test-threads=1
~~~

The benchmark is opt-in and has no elapsed-time pass/fail threshold. For a comparison, compile the same test fixture against each source version and keep both executables before alternating them. Use an idle machine for a cleaner result.

### Implementation and recovery guarantees

Video transfers use at most four workers, with the configured connection policy able to reduce that limit. Each track's first segment or initialization data is checked before the remaining segments are requested. Scheduling stays within eight positions of the first unfinished segment. Verified segments that finish out of order have durable receipts and remain reusable after pause, retry or application restart. Assembly uses playlist order.

HTTP/HTTPS workers coalesce small writes in a 256 KiB buffer. Live segment activity updates are limited to about ten per second per worker; aggregate progress checkpoints run every 500 ms through a blocking database worker. Frequent job/quota updates reuse a database connection and cached update statements. Quota transactions still commit received payload chunk by chunk.

Pause and cancellation join all workers after flushing and syncing their buffered file writes. Resume verifies completed media segments and resumes ordinary ranges from their saved file lengths. Restart clears both contiguous and out-of-order media receipts.

### Verification

The focused suites cover out-of-order HLS and DASH pause/reopen/resume, failed-segment retry while peers are active, lower global and per-site limits, the four-worker cap, corrupt saved segments, encryption/redirect boundaries, exact output and playable merged video/audio. Ordinary-file checks cover resumed byte offsets, browser-login persistence, synchronization queues, quota holds, storage faults, retries and publication recovery.

The buffered pause checks wait for received progress, pause the transfer, then compare the on-disk bytes with saved progress and verify that subsequent Range requests reuse those bytes. UI checks cover multiple active segments, sparse completion progress, dark/light native windows and the Failed label. Formatting, all-target/all-feature linting, JavaScript tests, release-version checks and packaging checks also passed. Live Internet benchmarks and real shutdown actions are outside this validation.

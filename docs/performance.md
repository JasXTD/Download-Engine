# Transfer performance — 3.1.0

Measured on 2026-10-27 against 3.0.2, using optimized Windows binaries and the same loopback fixtures. These are shared-workstation measurements: separate Cargo test runs remained active during every block. Treat the medians as observations for these fixtures, not predictions of Internet download speed. The ordinary-file differences are within the observed timing variation.

## Results

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

## Method

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

## Implementation and recovery guarantees

Video transfers use at most four workers, with the configured connection policy able to reduce that limit. Each track's first segment or initialization data is checked before the remaining segments are requested. Scheduling stays within eight positions of the first unfinished segment. Verified segments that finish out of order have durable receipts and remain reusable after pause, retry or application restart. Assembly uses playlist order.

HTTP/HTTPS workers coalesce small writes in a 256 KiB buffer. Live segment activity updates are limited to about ten per second per worker; aggregate progress checkpoints run every 500 ms through a blocking database worker. Frequent job/quota updates reuse a database connection and cached update statements. Quota transactions still commit received payload chunk by chunk.

Pause and cancellation join all workers after flushing and syncing their buffered file writes. Resume verifies completed media segments and resumes ordinary ranges from their saved file lengths. Restart clears both contiguous and out-of-order media receipts.

## Verification

The focused suites cover out-of-order HLS and DASH pause/reopen/resume, failed-segment retry while peers are active, lower global and per-site limits, the four-worker cap, corrupt saved segments, encryption/redirect boundaries, exact output and playable merged video/audio. Ordinary-file checks cover resumed byte offsets, browser-login persistence, synchronization queues, quota holds, storage faults, retries and publication recovery.

The buffered pause checks wait for received progress, pause the transfer, then compare the on-disk bytes with saved progress and verify that subsequent Range requests reuse those bytes. UI checks cover multiple active segments, sparse completion progress, dark/light native windows and the Failed label. Formatting, all-target/all-feature linting, JavaScript tests, release-version checks and packaging checks also passed. Live Internet benchmarks and real shutdown actions are outside this validation.

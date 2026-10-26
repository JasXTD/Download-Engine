# Native Rust YouTube extraction

Download Engine ports the anonymous VISIONOS/HLS extraction path from **yt-dlp
2026.08.19** into `src/youtube`. It reuses the existing Rust HLS transfer engine
and local FFmpeg merger. No Python, yt-dlp process, JavaScript engine, browser
cookie access, or new Cargo dependencies are required.

## Research and source map

The upstream repository is [yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp/tree/2026.08.19).
Its source is [Unlicense](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/LICENSE);
the license is included in `licenses/yt-dlp-LICENSE`. Bundled executable licensing
is a different question: no upstream executable is shipped here.

| Upstream source | Behavior adapted in Rust |
| --- | --- |
| [`youtube/_video.py`](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_video.py), `_VALID_URL` | Parse one video ID from watch, short, Shorts, embed and live URL forms; the first `v` wins. This port restricts hosts to official YouTube domains. |
| [`youtube/_base.py`](https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/yt_dlp/extractor/youtube/_base.py), `INNERTUBE_CLIENTS['visionos']` | VISIONOS client 101/version 1.02, device context and user agent. |
| `_base.py`, `extract_ytcfg`, `_extract_visitor_data`, `generate_api_headers`, `_call_api` | Read watch-page JSON as data, obtain visitor state, send the player request with matching context and headers. |
| `_video.py`, `_generate_player_context`, `_extract_player_response`, `_invalid_player_response` | Request playback information and reject mismatched video IDs. |
| `_video.py`, `_extract_formats_and_subtitles` | Select the returned HLS manifest and preserve quality/audio identities. Manifest parsing and transfer use Download Engine's existing engine. |

The research also traced [EJS 0.8.0's solver](https://github.com/yt-dlp/ejs/blob/0.8.0/src/yt/solver/solvers.ts)
and [n/signature extraction](https://github.com/yt-dlp/ejs/blob/0.8.0/src/yt/solver/nsig.ts).
That is a separate AST transformation and JavaScript execution path. It is **not
ported or bundled** in this implementation. Upstream itself marks extraction
without a JavaScript runtime as deprecated; the current client may change or
lose coverage. This implementation does not claim full yt-dlp compatibility.

## Runtime flow

1. A browser video review or pasted YouTube URL opens the native quality dialog.
2. Rust fetches the public watch page, parses configuration JSON without running
   scripts, and requests the anonymous player response with visitor context.
3. The response must identify the requested video and offer a finite, unencrypted
   HLS stream. Loading metadata and playlists cannot enqueue a download.
4. The user reviews video quality, audio, title-derived filename, queue and folder.
5. A version-5 media job stores the stable video ID and exact track choices.
6. Start/resume fetches fresh links and matches the original tracks. Completed
   segments are retained only when resource/encoding versions, segment identities,
   durations, sequences and byte ranges agree. Local hashes are checked again.
7. Selected tracks are downloaded, validated and remuxed into MP4. No remote URL
   reaches FFmpeg. Existing version-1 through version-4 HLS/DASH jobs are unchanged.

YouTube HLS audio formats 233/234 observed during live verification use ADTS AAC
with ID3 timestamps. `src/hls/aac.rs` validates frame lengths, encoding consistency,
metadata bounds and the continuous transport clock before merging. The initial
audio timestamp is preserved relative to video. This follows
[RFC 8216 section 3.4](https://www.rfc-editor.org/rfc/rfc8216.html#section-3.4).

## Boundaries

Metadata is capped at 8 MiB, individual requests at 25 seconds, and a complete
lookup at 60 seconds. Requests are cancellable. YouTube metadata uses a fixed
HTTPS origin, public pinned DNS, no automatic redirects, no proxies, and no
cookies. Media URLs must use HTTPS Googlevideo hosts; the existing HLS network
layer pins DNS and blocks cross-origin local/special-use destinations.
Lookup diagnostics contain only typed outcome/timing data.

This supports the public finished-video HLS subset, not JavaScript-challenged
URLs, direct adaptive formats, PO-token providers, signed-in/private/age-restricted
videos, live recording, DRM, playlists or subtitles. Unsupported results are
explicit errors. Video quality and availability remain dependent on the upstream
service. A selected encoding disappearing requires another review; no silent
quality or audio substitution occurs.

## Verification

Offline regression tests cover URL parsing, visitor propagation over real local
HTTP, malformed/oversized responses, redirects, cancellation, restrictions,
wrong-video responses, saved-job compatibility, track matching, expiring links,
AAC framing/timestamps, native dialog states and browser routing.

```powershell
cargo test --offline --locked -- --test-threads=1
cargo clippy --offline --locked --all-targets --all-features -- -D warnings
node --test tests/*.test.cjs
```

Live verification is deliberately opt-in and uses an isolated store under artifacts:

```powershell
cargo run --offline --locked --example youtube_probe -- 'https://www.youtube.com/watch?v=YE7VzlLtp-4'
cargo run --offline --locked --example youtube_probe -- 'https://www.youtube.com/watch?v=YE7VzlLtp-4' --pause-resume
```

The second command downloads a low-resolution test copy, pauses after verified
segments, forces saved CDN links to expire, reopens the store, resumes, and checks
completion. It does not touch the installed application's queue or download folder.

# Lua media providers — Media API 1

Download Engine 3.11.0 extends provider API 1 with segmented media and multiple
tracks. A personal Lua provider or installed Lua extension can resolve a finite
list of segments, process their bytes, and ask the core to mux selected tracks
into MKV or MP4. The existing single-file provider contract remains supported.

Use **Extensions → Scripts → New script**, select **Download provider**, and adapt
`examples/media-provider.lua`. Save it, enter the source URL and output name, then
choose **Add provider download**. The output extension follows the chosen container.
Normal queue controls, Pause, Resume, Stop, speed limits and quotas apply.
An extension packages the same Lua code as a job with `trigger.kind: "provider"`
and `provider_api: 1`; see [the extension guide](extensions.md). Installed providers
use their existing Source URL / Add provider download controls.

To try a complete installable example, follow the
[offline episode walkthrough](../examples/offline-episode/README.md). The builder
creates `dist/examples/Offline-Episode-1.0.0/`, including the extension JSON,
generated sample segments and a small PowerShell server. It reuses selected
upstream language/timeline helpers with their MIT notice. No app recompile is
needed to install or edit the example.

## Binary requests and session keys

**Download Engine 3.13.0** adds `xd.fetch_bytes`, `xd.handoff_key` and
`xd.session_key`. They are additive to provider/media API 1; use `xd.supports`
to check for them before running a provider on an older app.

`xd.fetch_bytes(url, options)` is available in a provider's resolver and in site
readers. It accepts the same method, header and body options as `xd.fetch`, but
its request body and returned `body` are binary Lua strings: NUL and non-UTF-8 bytes
are preserved. `xd.fetch` retains its existing text behavior. Both calls share
one 50-request budget, a 1 MiB request-body limit and an 8 MiB reply limit, plus
the same timeouts, public-address and declared-host checks, Direct connection
requirement and redirect rules. Neither inherits browser cookies or saved logins.

Inside `resolve` or `refresh`, a provider can hand off a key received from its own
protocol. This snippet illustrates a **raw AES key response** from a hypothetical
API, not a Widevine license exchange:

```lua
assert(xd.supports("xd.fetch_bytes") and xd.supports("xd.handoff_key"))
local reply = xd.fetch_bytes("https://media.example.invalid/session-key", {
    method = "POST",
    headers = { ["Content-Type"] = "application/octet-stream" },
    body = string.char(0, 1, 255), -- replace with your protocol's request
})
assert(reply.status == 200, "Key request failed")
xd.handoff_key("video", reply.body)
-- Return the ordinary file or media plan, without embedding a key in it.
```

The core seals the key for this resolver run, provider and key name. It starts
the processor with the protected handoff after closing the network-capable
resolver. The processor remains offline and obtains only an opaque AES handle:

```lua
-- Inside transform; use the counter and offset required by your own format.
local key = xd.session_key("video")
return xd.aes_ctr(key, counter_hex, bytes, offset)
```

- `xd.handoff_key(name, bytes)` accepts 16, 24 or 32 raw AES key bytes, at most 64
  unique names per resolution. Names contain 1-64 ASCII letters, digits, `_`, `-`
  or `.`. Duplicate names, wrong key sizes and calls outside resolve/refresh fail.
- `xd.session_key(name)` is processor-only, including during processor setup.
  Missing or invalid keys fail; there is no fallback to a different key. The
  existing `xd.key` and `xd.key_setting` APIs remain available for saved keys.
- The handoff adds no keys to saved plans, checkpoints, exports or packages.
  Pause/Stop, worker errors and normal completion release the run's handoff.
  Resume or a restart calls the resolver again; `refresh` must acquire and hand
  off every required key again. A failed or empty refresh never reuses old keys.
- A resolver sees any key bytes it receives and can contact its permitted hosts.
  Treat its code as trusted. The handoff prevents the core from persisting those
  bytes or exposing them to processor Lua as plaintext; it cannot erase strings
  that the resolver itself retains or prevent that resolver from forwarding them
  or explicitly putting them in returned metadata. Keep keys out of plans and errors.

These APIs transport binary data and AES keys. They do not implement service
authentication, Widevine challenge/license processing, or MP4 sample-encryption
parsing. Those remain separate implementation work.

## Resolve a plan

From 3.14.0, resolvers can use [authenticated HTTP sessions](#authenticated-http-sessions),
and processors can opt into [whole-segment callbacks](#whole-segment-callbacks).
Both are additive capabilities; check `xd.supports` before using them.

From **3.15.0**, [the provider core guide](lua-provider-core.md) covers additional
HTTP methods, explicit session handoff to segment downloads, unknown sizes,
stream processing, protected binary/RSA imports, cryptography and ASS subtitles.

Check `xd.media_api == 1` or `xd.supports("xd.media_plan")` before using this API.
`xd.media_plan(table)` validates the table and adds `kind: "media"` and
`media_api: 1`. It performs no I/O. Return its result from `resolve(request)` and
`refresh(request, previous)`:

```lua
return xd.media_plan({
    content_id = "movie-revision-1",
    container = "mkv",
    tracks = {
        {
            id = "video", kind = "video", format = "mp4", default = true,
            segments = {
                { id = "init", url = "https://files.example/video-init.mp4",
                  size = 1024, processing = "copy" },
                { id = "part-1", url = "https://files.example/video-1.enc",
                  size = 65536, processing = "lua" },
            },
        },
        {
            id = "audio-en", kind = "audio", format = "aac",
            language = "eng", title = "English", default = true,
            segments = {
                { id = "part-1", url = "https://files.example/audio.aac",
                  size = 16384, processing = "copy" },
            },
        },
    },
})
```

These URLs and sizes are illustrative. Provide actual metadata for your content.

| Field | Contract |
| --- | --- |
| `content_id` | Stable revision identifier. Change it when content or decryption parameters change. |
| `container` | `mkv` or `mp4`. |
| `tracks` | 1–16 tracks, in output order. Each has a unique `id`. |
| `kind` / `format` | Video: `mp4` or `mpegts`. Audio: `mp4`, `mpegts` or `aac`. Subtitle: `srt`, `webvtt` or `ass`. |
| `language`, `title`, `default` | Optional output metadata. Use language codes such as `eng` or `jpn`. At most one default track per kind. |
| Track `init_segment` | From 3.14.0: optional ID of the first segment, at most 8 MiB. Its original verified bytes are replayed to `prepare_track` on every processor run, including recovery. |
| `segments` | Ordered pieces of the same track, including initialization bytes when required. At most 16,384 across the plan (3.15.0+). |
| Segment `id`, `url`, `size` | Unique ID within its track and HTTP(S) address. Positive `size` is an exact byte count; omit it or use 0 for an unknown size (3.15.0+). Each downloaded segment is 1 byte–256 MiB. |
| Segment `session` | Optional non-secret name passed to `xd.handoff_session` during this resolution (3.15.0+). |
| `range_start` | Optional start offset in a larger resource; a known positive `size` gives the range length. The server must return the exact requested range. |
| `processing` | `copy` (default), `lua`, `lua_segment`, or `lua_stream` (3.15.0+). Copy leaves bytes unchanged; Lua calls `transform` per chunk; `lua_segment` calls `transform_segment` for the entire bounded segment; `lua_stream` calls `transform_stream` with temporary readers/writers. |
| `input_sha256`, `output_sha256` | Optional lowercase SHA-256 of that segment before/after processing. Supply trusted digests when available. |

IDs contain only ASCII letters, numbers, `.`, `_` and `-`, up to 128 bytes. The
serialized plan is capped at 4 MiB (3.15.0+). Unknown fields, empty tracks, duplicate IDs,
invalid ranges and unsupported formats fail before a segment is requested.

Tracks are concatenated in order, not treated as independent complete files:
for fragmented MP4, place its initialization segment first; later pieces must
belong to that same track and timeline. Byte slices of one complete MP4 also work.
Concatenating unrelated MP4 movies is not supported. Subtitle segments must form
one valid SRT, WebVTT or ASS document with timestamps on the same timeline as the media.
MKV preserves ASS styling; MP4 converts subtitles to `mov_text` and loses that styling.

## Process bytes

The same four callbacks are required as for file providers. For a media plan:

```lua
transform = function(context, bytes, offset)
    local key = xd.key_setting("content_key") -- installed extension AES setting
    local counter = "000102030405060708090a0b0c0d0e0f" -- public example nonce
    return xd.aes_ctr(key, counter, bytes, context.track_offset + offset)
end
```

`context` contains `content_id`, `track_id`, `kind`, `segment_id`, `segment_index`
(zero-based), `track_offset`, and the segment's `size`. `offset` starts at zero
within each segment. Choose either segment-relative or track-relative offsets
according to your encryption format, with the correct key and counter for that
track/segment. Never reuse an AES-CTR key/counter combination for unrelated data.

For `processing: "lua"`, chunks are at most 256 KiB. Return exactly as many bytes as received. For clear
initialization data or subtitles, use `processing: "copy"`. For private keys,
use an extension `aes_key` setting or the personal-provider `xd.key` token API.
The resolver can fetch metadata on its permitted public hosts. The processor
has no network, filesystem, process execution, saved browser login or state store.
Plan URLs never enter processor callbacks or the saved media plan.

After processing, `finish(plan, result)` receives the URL-free media plan and
`{ size = <total plaintext bytes>, tracks = <count>, stage = "before_mux" }`.
Return `true` to proceed. This is before muxing; the final container's size/hash
may differ. Rust verifies supplied segment hashes independently of this callback,
then hashes the muxed output before publishing it without overwriting another file.

Hashes are optional because some sources do not publish them. Without trusted
expected hashes, checkpoint hashes detect later local corruption but do not
authenticate the source or prove that a key is correct. AES-CTR alone provides no
integrity protection. An expected final-file checksum supplied through the normal
download controls is checked too.

## Muxing and recovery

The core downloads one segment at a time with bounded memory. Completed encrypted
and processed segments are flushed and checkpointed. Pause/Stop retains them;
an interrupted segment restarts from its beginning. Resume or crash recovery
refreshes the source URLs, requires unchanged content metadata and verifies saved
segment hashes before reuse. Changed content or corrupted caches require explicit
Restart. Editing a library script never changes a queued job's frozen code.

Processed output across the whole job may not exceed twice the verified input
size plus 16 MiB. Saved output counts toward this limit when resuming. The core
passes the remaining allowance into each segment callback and checks disk space
for temporary copies and muxing, with 64 MiB of free-space headroom. Processing
progress counts verified output bytes; its total stays unknown until transforms
finish. Failed transforms clear their incomplete output while preserving completed
segments. Free-space checks cannot reserve storage against unrelated processes.

Muxing uses the bundled FFmpeg and a fixed argument list. Lua cannot supply an
executable, command-line options, local paths or URLs to FFmpeg. Only guarded,
verified local track files are opened; external MP4 references are disabled.
The core maps the first stream of the declared kind from each track, copies video
and audio without re-encoding, and keeps language/title/default metadata. MP4
subtitles use the fixed `mov_text` codec; codec/container incompatibilities fail
without publishing a partial result. Muxing is cancellable, has a 30-minute limit
and a bounded output size; retry reconstructs tracks from saved segments.
Processor and muxer diagnostic text cannot leak file bytes through an extension's
download error into another network-capable job.

Explicitly entered local origins are usable for local fixtures. Different origins
must be public, cross-origin redirects are refused, HTTPS cannot downgrade to
HTTP, and an extension holding a secret can only return its declared hosts. No
browser cookies or saved server credentials are inherited by segment requests.

The first saved media plan raises profile data/writer compatibility to 7. Older
builds refuse that profile rather than dropping new media state. Existing file
provider checkpoints keep their original format. Keep a profile backup before
downgrading; use a separate profile with older builds.

## Authenticated HTTP sessions

**3.14.0** adds `xd.http_session(origin, options)` in site readers and provider
resolvers. A personal script may use it directly. An installed extension must
declare `"http_sessions": true` alongside its `permissions.network` list. The
install review shows this access, and an update adding it needs approval. A frozen
job cannot gain permission from a later extension update; removing permission
disables it for future worker runs.

```lua
assert(xd.supports("xd.http_session"))
local session = xd.http_session("https://api.example.invalid", {
    cookies = { login = xd.setting("login_cookie") }, -- extension secret setting
    headers = { ["X-Client"] = "My extension" },
})
local reply = xd.fetch_bytes("https://api.example.invalid/session", {
    session = session,
    method = "POST",
    headers = { ["Content-Type"] = "application/octet-stream" },
    body = string.char(0, 1, 255),
})
assert(reply.status == 200)
```

This is a supplied-cookie HTTP example, not a service login implementation.
The origin must have no path, query or fragment. Production authenticated origins
require HTTPS. A literal loopback HTTP origin is accepted as a handle for isolated
tests, but creating it does **not** relax fetch's existing public-address policy.
The normal application still refuses a loopback fetch.

- At most 16 handles per run; each accepts 50 cookies, 32 headers and 16 KiB of
  credential names/values. Cookie values are raw cookie values, without surrounding
  quotes. Use a `secret` setting for user-supplied login material.
- Pass the handle in the `session` option of either `xd.fetch` or `xd.fetch_bytes`.
  The existing 50-request budget is shared across every handle and ordinary fetch.
  Request/response limits, declared hosts, public-address checks and Direct-only
  routing still apply.
- Each handle has a separate cookie jar. Server cookies respect path, expiry,
  deletion and Secure attributes. Domain attributes never expand access beyond
  the exact scheme, host and port selected when creating the handle.
- A session request cannot follow a redirect to another origin or override a
  session header. Ordinary fetches never inherit its credentials. Reserved headers
  such as Cookie, Host, Content-Length and proxy headers remain blocked.
- Handles are opaque and cannot be encoded in JSON or a download plan. They last
  for one worker run, do not import browser/saved logins, and are unavailable to
  processors and automation jobs. Resume must create the session again.
- Segment requests never inherit a session implicitly. From 3.15.0, use
  [explicit session handoff](lua-provider-core.md#credentials-for-segment-downloads)
  to authorize selected segments. Single-file provider transfers still require
  usable download URLs.

Installing an extension with this permission raises profile compatibility to 8.

## Whole-segment callbacks

**3.14.0** adds `xd.segment_api == 1` and
`xd.supports("media.segment_processing")`. Set a segment's processing mode to
`"lua_segment"` when its format requires inspecting the whole piece or changing
its length. The input may be 1 byte–8 MiB; the returned binary Lua string may be
1 byte–16 MiB. Larger segments must use a different plan/processing strategy.

Add these callbacks to the ordinary provider callback table:

```lua
local tracks = {}
return {
    resolve = resolve,
    refresh = refresh,
    transform = function(_, bytes) return bytes end,
    prepare_track = function(context, original_init_bytes)
        tracks[context.track_id] = parse_my_initialization(original_init_bytes)
        return true
    end,
    transform_segment = function(context, original_segment_bytes)
        return rewrite_my_segment(tracks[context.track_id], original_segment_bytes)
    end,
    finish = function() return true end,
}
```

`resolve`, `refresh`, `parse_my_initialization` and `rewrite_my_segment` are functions supplied by the
extension author; the app does not implement them. Keep `resolve`, `refresh`,
`transform` and `finish` in the table for the existing provider contract. A provider
using only whole-segment/copy modes can give `transform` a simple pass-through body.

Set a track's `init_segment` to its first segment ID when parsing requires
initialization. Before processing that track, the core checks and replays the
original initialization bytes to `prepare_track`, even when that segment's output
was already saved. Return `true` to acknowledge successful setup. Initialization
itself is processed according to its own `processing` mode.

`transform_segment` receives the same URL-free context as `transform`; its offset
within the segment is zero. `context.track_offset` counts **input** bytes, not
rewritten output bytes. `context.output_offset` counts bytes already written to
the output track, including reused checkpointed pieces, so container offsets can
be rewritten correctly after a restart. Return the complete rewritten piece. Each piece must be
processable using initialization and its own metadata: completed media pieces
are skipped during recovery, so do not depend on callbacks for earlier pieces.

Lua still has no files, network or processes in this phase. IPC remains chunked
at 256 KiB; the worker assembles only the bounded segment. Stop/Pause and the
callback timeout terminate a busy worker. `finish` receives the actual total
processed byte count before muxing. Checkpoints verify each output's own length
and hash, including after a restart; malformed or changed output fails before muxing.
Existing `copy` and `lua` jobs keep their same-length behavior and receipt format.

This enables MP4 parsing, sample processing and container rewriting in Lua, but
does not supply an MP4/CENC parser or licence protocol. The offline regression
fixture strips a small teaching envelope from encrypted MP4 pieces and compares
the final encoded packets with the originals; that envelope is not CENC/Widevine.
The separate [offline CENC lab](../examples/offline-cenc/README.md) supplies an
original Lua parser for a defined sample-level AVC/AAC CENC subset on 3.15.0,
with synthetic license fixtures. It is an author example, not built-in site
support or a complete CENC/Widevine implementation.
Using `init_segment` or `lua_segment` raises profile compatibility to 8.

## Offline verification

Use existing project dependencies and media tools; the test downloads nothing:

```powershell
cargo test --offline --locked --lib provider::
cargo build --offline --locked --bins
node scripts/media_provider_smoke.cjs target/debug
```

The smoke test uses `tools/ffmpeg/ffmpeg.exe` and the already-staged
`artifacts/ffmpeg-tools/ffprobe.exe`. On native systems set `MEDIA_TEST_FFMPEG`
and `MEDIA_TEST_FFPROBE` to the approved native tools. It generates its own video,
two audio tracks and subtitles, encrypts them with public test keys, serves them
only on loopback, and runs the real desktop/CLI/Lua workers. It checks MKV/MP4
tracks and media packet hashes, extension key settings, pause/resume, crash
recovery, corrupted segments, wrong keys, Stop and redirect restrictions. Its
temporary profile, media and server are removed; a summary remains in artifacts.

The bundled implementation does not currently include Widevine license handling
or Crunchyroll authentication and stream discovery. These are implementation
limits, not additional [software license conditions](../README.md#license).
Developers can extend the Lua code and core APIs; the offline tests above validate
the generated sample format, not compatibility with a commercial service.

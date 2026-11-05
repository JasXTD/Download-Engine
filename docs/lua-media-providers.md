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

## Resolve a plan

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
| `kind` / `format` | Video: `mp4` or `mpegts`. Audio: `mp4`, `mpegts` or `aac`. Subtitle: `srt` or `webvtt`. |
| `language`, `title`, `default` | Optional output metadata. Use language codes such as `eng` or `jpn`. At most one default track per kind. |
| `segments` | Ordered pieces of the same track, including initialization bytes when required. At most 2,048 across the plan. |
| Segment `id`, `url`, `size` | Unique ID within its track, HTTP(S) address and exact byte count. Each segment is 1 byte–256 MiB. |
| `range_start` | Optional start offset in a larger resource; `size` gives the range length. The server must return the exact requested range. |
| `processing` | `copy` (default) or `lua`. Copy leaves bytes unchanged. Lua calls `transform` for each chunk. |
| `input_sha256`, `output_sha256` | Optional lowercase SHA-256 of that segment before/after processing. Supply trusted digests when available. |

IDs contain only ASCII letters, numbers, `.`, `_` and `-`, up to 128 bytes. The
serialized plan is capped at 512 KiB. Unknown fields, empty tracks, duplicate IDs,
invalid ranges and unsupported formats fail before a segment is requested.

Tracks are concatenated in order, not treated as independent complete files:
for fragmented MP4, place its initialization segment first; later pieces must
belong to that same track and timeline. Byte slices of one complete MP4 also work.
Concatenating unrelated MP4 movies is not supported. Subtitle segments must form
one valid SRT or WebVTT document with timestamps on the same timeline as the media.

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

Chunks are at most 256 KiB. Return exactly as many bytes as received. For clear
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

This API does not implement Widevine, license acquisition, service logins or any
particular commercial site's media format. Testing is limited to owned offline
fixtures; it is not evidence of compatibility with Crunchyroll or another service.

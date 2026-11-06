# Provider core APIs — Download Engine 3.15.0

These APIs supply transport, credential storage, cryptographic operations and
bounded media processing. Login flows, request formats, license messages,
manifest parsing and MP4 sample handling belong to your Lua extension. No site
integration is enabled automatically by these helpers. Validation uses generated
offline data only; it does not establish compatibility with a particular service.

The provider and media API versions remain **1**. Check additive capabilities
with `xd.supports` before using them on an older app. The editor's provider API
completion list includes the new functions.

## HTTP methods

`xd.fetch` and `xd.fetch_bytes` accept `GET`, `HEAD`, `POST`, `PUT`, `PATCH`,
`DELETE` and `OPTIONS`. GET and HEAD have no request body. The other methods can
send up to 1 MiB; `fetch_bytes` preserves binary bodies and replies. HEAD returns
headers with an empty body, even when Content-Length describes a large resource.

The existing 50-request budget, 8 MiB response limit, host permissions and public
address checks remain in force. Redirects preserve methods and bodies for 307/308;
303 changes non-HEAD requests to GET; 301/302 changes POST to GET. A body is never
forwarded to another origin. An authenticated session refuses another origin
entirely. Scripts still have no browser-cookie or arbitrary filesystem access.

```lua
local reply = xd.fetch_bytes(api_url, {
    method = "DELETE",
    session = api_session,
    headers = { ["Content-Type"] = "application/octet-stream" },
    body = request_bytes,
})
assert(reply.status == 200 or reply.status == 204)
```

## Credentials for segment downloads

An HTTP session can now be handed to the core downloader. The extension must
declare `permissions.http_sessions`, as it does for authenticated fetch calls.
Adding that permission requires review. Personal provider scripts can also use it.

Inside `resolve` or `refresh`:

```lua
assert(xd.supports("xd.handoff_session"))
local media_session = xd.http_session("https://media.example.invalid", {
    cookies = { session = supplied_cookie },
    headers = {
        Authorization = "Bearer " .. supplied_token,
        Origin = "https://www.example.invalid",
        Referer = "https://www.example.invalid/player",
        ["User-Agent"] = "MyProvider/1",
    },
})
xd.handoff_session("media", media_session)
-- In each relevant segment: session = "media"
```

Names are non-secret identifiers of 1–64 ASCII letters, digits, `_`, `-` or `.`.
Use at most eight sessions per resolution and 1 MiB of encrypted session data
in total. Duplicate names and missing references fail. Header values and cookie
values are protected by the OS for this run, provider and session name. They do
not enter the saved plan, checkpoint or processor. Cookies learned by the
resolver before it returns are included, and segment responses can update the
downloader's jar. Cookie paths, expiration and Secure attributes still apply.

Each session belongs to exactly one origin (scheme, host and port). The core
checks that origin before every request and redirect. Give each different CDN
origin its own session. Browser/profile logins are never inherited. Resume starts
a new resolver: reacquire and hand off the required sessions again. Lua that
receives supplied secrets remains trusted code; do not put secrets in metadata,
URLs or error messages.

## Unknown sizes and larger plans

Omit a segment's `size`, or use `size = 0`, when its exact size is unknown. The
core streams and measures it; no HEAD request per segment is necessary. A known
positive size is still mandatory for `range_start`. Content-Length, when present,
is enforced. Empty, oversized, truncated HTTP responses and digest mismatches
fail. Without a trusted digest, a complete chunked response cannot prove that the
publisher intended additional bytes; supply `input_sha256` when available.

Plans support up to **16,384 segments**, 16 tracks and 4 MiB of serialized plan
data. Each input segment remains capped at 256 MiB. Progress has no total until
all unknown sizes are measured. Checkpoints retain the measured sizes and SHA-256
digests. Resume rechecks them and reuses completed segments; an interrupted
segment starts again. The saved plan retains its original size declarations so
refresh can compare the same content identity. Processor contexts contain the
measured input size and actual input/output offsets.

## Streaming container processing

`processing = "lua_stream"` calls `transform_stream(context, input, output)`.
Use it when a segment exceeds the existing 8 MiB whole-string callback limit or
when rebuilding a container without retaining its payload in Lua memory.

```lua
assert(xd.supports("media.stream_processing"))

local function transform_stream(context, input, output)
    local offset = 0
    while offset < input:size() do
        local bytes = input:read(offset, 256 * 1024)
        -- Parse headers and transform selected bytes using your own format logic.
        output:write(bytes)
        offset = offset + #bytes
    end
    return true
end

-- Include transform_stream in the provider's returned callbacks table.
-- resolve, refresh, transform and finish are still required.
```

`input:read(offset, count)` uses a zero-based byte offset and accepts at most
256 KiB per call; it returns fewer bytes at EOF. It supports random reads within
the current input. `output:write(bytes)` appends 1 byte–256 KiB at a time, with a
512 MiB maximum output per segment. Return `true` after writing a nonempty result.
Across all tracks, processed output is limited to twice the verified input bytes
plus 16 MiB of job-wide overhead. The writer enforces the remaining allowance
before writing; resumed segments consume the same budget. This limit also applies
to `lua_segment` output. Processing checks space for scratch files, assembled
tracks and the final container, keeping at least 64 MiB free. Space checks are
repeated during writes; they are not an exclusive operating-system reservation.

The worker owns anonymous temporary files. No path or raw file handle reaches
Lua. Captured readers/writers expire as soon as the callback returns or fails.
Pause, Stop and the callback deadline terminate the owned worker and close its
temporary files. The core verifies hashes and stores the completed output before
recording a reusable checkpoint. `init_segment` / `prepare_track` still replay
the first original initialization segment (up to 8 MiB) on each processor run.

## Protected binary credentials and RSA keys

Declare explicit settings in your extension manifest:

```json
"settings": [
  { "id": "client_data", "type": "binary_secret", "label": "Client data" },
  { "id": "signing_key", "type": "rsa_private_key", "label": "Signing key" }
]
```

In **Extensions → Installed → your extension → Settings**, choose **Import file…**.
The selected file is copied into OS-protected storage, with a 64 KiB limit.
**Replace file…** and **Clear** replace or remove the saved value. The original
file is not changed. Values are not included in extension packages or settings
exports. CLI equivalents:

```powershell
.\de-cli.exe extensions import-secret example.provider client_data .\owned-client.bin
.\de-cli.exe extensions import-secret example.provider signing_key .\owned-private.pem
```

Only web jobs receive these settings. Provider processors and automation jobs
receive neither. `xd.binary_setting("client_data")` returns the exact binary
bytes to a web job. `xd.rsa_key_setting("signing_key")` returns an opaque RSA
handle; it does not export the private key. Removing or changing a setting's type
invalidates its saved value through the usual extension update review.

An RSA key must be an unencrypted 2048–4096-bit, two-prime private key in PKCS#1
or PKCS#8 PEM/DER format. A script which already has key bytes can use
`xd.rsa_private_key(bytes)` to import an opaque handle without filesystem access.

## Generic cryptography

All strings below are binary Lua strings unless stated otherwise.

| Function | Contract |
| --- | --- |
| `xd.random_bytes(count)` | 1–4096 bytes from the OS cryptographic random source. |
| `xd.digest(algorithm, bytes)` | `sha1` or `sha256`; up to 1 MiB input; binary digest. |
| `xd.hmac(algorithm, key, bytes)` | `sha1` or `sha256`; key up to 64 KiB, message up to 1 MiB. |
| `xd.rsa_sign(handle, algorithm, bytes)` | `pss-sha1`, `pss-sha256` or `pkcs1-sha256`; message up to 1 MiB. PSS uses matching MGF1 and a digest-length salt. |
| `xd.rsa_decrypt(handle, algorithm, ciphertext)` | `oaep-sha1` or `oaep-sha256`; matching MGF1 hash, empty label; returns binary plaintext. |
| `xd.aes_decrypt_blocks(key_or_handle, blocks)` | AES-128/192/256 ECB decryption without padding; complete 16-byte blocks, at most 256 KiB plus one block. |

The existing `xd.aes_blocks`, `xd.aes_ctr`, `xd.handoff_key` and `xd.session_key`
remain available. Lua can combine block primitives into its own format-specific
modes. SHA-1 options are provided for protocols requiring them. RSA uses Windows
CNG, Linux OpenSSL or macOS Security.framework; HMAC/digests reuse the already
locked RustCrypto libraries. No service SDK, device identity or credentials are
bundled with these APIs.

## ASS subtitles

Use `kind = "subtitle", format = "ass"`. Segments must form one valid ASS
document on the same timeline as the video. Choose **MKV** to preserve ASS style
definitions and inline overrides. MP4 uses `mov_text`, which does not preserve
ASS styling. Font attachments are not part of this API; the player's available
fonts affect rendering.

## Compatibility and offline checks

Using the new credential setting types, unknown-size plans, stream processing,
ASS tracks or larger plans raises the profile's minimum writer version to **9**.
Older apps then refuse to write the affected profile. Existing fixed-size and
whole-segment checkpoints retain their original serialization and bindings.

Run the focused Rust provider, extension and HTTP tests, followed by
`scripts/provider_core_smoke.cjs`, `scripts/provider_smoke.cjs` and
`scripts/media_provider_smoke.cjs`. Supply a built package-shaped binary folder
and an evidence folder. The scripts generate harmless bytes, RSA keys and media,
use no live service accounts, and remove their temporary profiles and files.

For a complete Lua sample-processing path, see the
[offline CENC provider lab](../examples/offline-cenc/README.md). It exercises a
synthetic signed license exchange, protected imports, sample/subsample AES-CTR,
initialization replay, multi-track muxing and ASS. Run
`scripts/offline_cenc_smoke.cjs` with the same binary/evidence arguments. The lab
uses the existing 3.15.0 core and removes generated keys, profiles and media.
Its limited CENC parser and invented license format do not establish live-service
compatibility; the guide documents the separate protected-host and local-media
checks and the unavailable independent encrypted-input decoder check.

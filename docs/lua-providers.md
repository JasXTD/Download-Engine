# Lua download providers — API 1

Download Engine 3.5.0 runs user-written Lua providers from the existing Scripting
page. A provider resolves a direct file URL and transforms downloaded bytes.
Download Engine owns the network transfer, files, checkpoints and final hash
verification. No recompilation or separate plugin system is needed.

This release was tested with generated encrypted fixtures and a local HTTP
server only. It does not establish compatibility with Crunchyroll, Widevine,
DRM licenses, or any other service's encryption/container format.

## Use a provider

1. Open **Tools → Scripting → Loaded scripts → New script**.
2. In **Script settings**, keep **Lua** and choose **Download provider** as the
   trigger. The untouched starter is replaced with a provider template.
3. Adapt its source resolution, trusted size/hashes and byte transformation to
   your own data. The bundled `examples/download-provider.lua` uses native
   AES-CTR; `examples/download-provider-custom-ctr.lua` builds CTR in Lua using
   the AES block primitive. Both contain public test keys and deliberately
   incomplete hash placeholders. They are not ready-made service integrations.
4. **Save script**. Enter the **Source URL** and optionally the output **File
   name**, then choose **Add provider download**. It enters Main queue and uses
   your usual download folders. The explicit submission runs even if the
   definition was imported with automatic triggers disabled.
5. Use the normal download list/status window to **Pause**, **Resume**, **Stop**
   or inspect a failure. Processing appears as **Processing with Lua provider**.
   Network byte progress can already be 100% while processing is still running.

Provider downloads retain the exact saved code and revision used when added.
Editing or removing the library script does not change existing jobs. Add a new
download to use edited code. Restart clears its transfer/processing checkpoints
and resolves the file again, still using that frozen revision.
Duplicate and Redownload also keep that revision and resolve a fresh plan.
Provider links refresh through the script; browser-assisted replacement links,
saved-login dialogs and remote ZIP previews do not apply to these jobs.

Provider execution belongs to the download, rather than the automation Run/Stop
buttons. Errors appear on the download. The download list export omits provider
jobs because a plain URL cannot preserve their processing behavior; share the
reviewed `.de-script.json` package separately.

The Core API menu and autocomplete show functions available to the provider.
Full editor remains available. Script packages carry `provider_api: 1`, import
as disabled drafts, and are rejected by versions without this contract.

From the CLI, after saving a provider:

```powershell
.\de-cli.exe scripts download SCRIPT_ID "https://example.com/encrypted.bin" --name "plain.bin" --queue main
.\de-cli.exe pause DOWNLOAD_ID
.\de-cli.exe resume DOWNLOAD_ID
```

`--request-id UUID` makes a retried add command idempotent. An automation worker
cannot start a provider through this CLI command; submission is an explicit
desktop/CLI action. The current MCP tools do not expose provider submission.

## Callback contract

Return a table with all four functions. `xd.provider_api == 1`,
`xd.api_version == 1`, and `xd.script_kind == "provider"` describe this runtime.

| Callback | Arguments and result |
| --- | --- |
| `resolve(request)` | `request.url` is the URL entered by the user. Return the plan below. |
| `refresh(request, previous)` | Return a plan for the same content, possibly with a renewed URL. `previous` contains size, hashes and `transform_id`, with no URL. |
| `transform(plan, bytes, offset)` | Return a binary Lua string of exactly the input length. `plan` contains size, hashes and `transform_id`, with no URL. `offset` is the absolute input byte offset, starting at zero. |
| `finish(plan, result)` | Called after the core verifies the plaintext. `result.size` and `result.sha256` describe it. Return `true`. |

The plan has exactly these fields; unknown fields are rejected:

```lua
{
    url = "https://example.com/direct-file.bin",
    size = 2097189,
    input_sha256 = "64 lowercase hex characters for the encrypted source",
    output_sha256 = "64 lowercase hex characters for the final output",
    transform_id = "my-format-v1",
}
```

Size must be positive and fit in a signed 64-bit integer. Both hashes are
mandatory. The URL can change on refresh, but size, hashes and `transform_id`
must remain identical when resuming. Changing content requires Restart.
The plan schema has no fields for filesystem paths, authentication bindings or
keys. Only size, hashes and `transform_id` are persisted. Resolved URLs are
transient and reacquired on resume; keep `transform_id` free of secrets. The
original URL entered by the user remains ordinary download metadata.

The resolver and processor use separate fresh worker processes with different
capabilities, enforced before the source is evaluated. `xd.phase` is
`"resolver"` for `resolve`/`refresh` and `"processor"` for `transform`/`finish`.
The resolver has `xd.fetch` and never receives file bytes. The processor has no
`xd.fetch`, including during module initialization; it rejects resolve/refresh
operations. Resolver locals never carry into a processor. Gate initialization
by `xd.phase` when it needs a phase-specific helper. This security restriction
also applies to existing providers that previously fetched inside transforms or
populated processor locals during refresh.

Resume creates fresh workers. `refresh` must work without an earlier `resolve`
call in that Lua instance. Load private keys in the processor with `xd.key`,
as described below. Transform each chunk independently using its absolute
offset; do not depend on global counters surviving a pause. Chunks are at most
256 KiB. API 1 supports same-length transforms, not decompression, arbitrary
remuxing, multiple tracks, or transformations that change byte offsets.

## Core helpers and boundaries

- `xd.aes_ctr(key_or_handle, counter_hex, bytes, offset)` uses Windows CNG AES-128,
  AES-192 or AES-256. Counter is exactly 16 bytes, interpreted as a full
  big-endian 128-bit counter. Non-block-aligned offsets are supported; counter
  wrap and invalid key lengths are refused.
- `xd.aes_blocks(key_or_handle, blocks)` encrypts complete 16-byte AES blocks without
  padding (at most 256 KiB plus one block). It is a primitive for custom Lua
  constructions, as demonstrated by the second example.
- `xd.key(protected_token)` is available only in the processor. It returns an
  opaque AES key handle accepted by both crypto helpers, without exposing key
  bytes to Lua. Raw hex keys remain supported for public fixtures.
- `xd.fetch` is available only in the resolver and retains the site-reader GET/POST rules: public addresses only,
  checked/pinned DNS and redirects, bounded requests and replies, no browser
  cookies or saved login access. Normal proxy preferences apply.
- JSON, URL and HTML helpers remain available. `xd.supports(name)` reports the
  actual function set. There is no `xd.command`, download mutation, state store,
  filesystem, process, native module, `os`, `io`, `debug`, or dynamic code loader.
- The transfer core can access the origin explicitly entered by the user,
  including a local fixture server. Resolved URLs on other origins must be
  public. Provider file requests reject cross-origin redirects; resolve the
  final direct URL in the script. Metadata `xd.fetch` does not inherit this
  local-origin grant.

Lua has a 64 MiB allocator limit; an owned Windows worker has a 256 MiB process
limit and may not launch children. Each callback is limited by the saved timeout,
capped at 60 seconds. Pause/Stop kills a busy owned worker without waiting for
that timeout. Workers are restricted processes, not an OS permission sandbox;
review scripts before running them. Frozen job source is encrypted using the
existing Windows user-scoped DPAPI helper. Library definitions and script exports
still contain their source: use protected tokens instead of private key literals.
Keys passed to crypto functions are not written to processing checkpoints.

## Private AES keys

After saving a provider, run `de-cli.exe scripts protect-key SCRIPT_ID` with a
32-, 48- or 64-character hexadecimal AES key supplied through **redirected stdin**
from your secret manager. The command refuses terminal input and key arguments;
do not put the secret in command history. It runs locally without starting the
desktop or writing a command receipt. Its JSON result contains only a
`result.token` ciphertext string beginning with `provider-key-v1:`.

Use that token in the provider:

```lua
local key
if xd.phase == "processor" then
    key = xd.key("provider-key-v1:PASTE_PROTECTED_TOKEN_HERE")
end
-- In transform(plan, bytes, offset):
-- return xd.aes_ctr(key, counter_hex, bytes, offset)
```

Tokens are immutable and bound to the saved provider ID and Windows account.
Editing that provider preserves access; importing a package creates a new ID
and requires a newly provisioned token. The token and opaque Lua handle cannot
be used to read a different provider's key. The resolver cannot unlock tokens,
and Lua cannot extract or JSON-encode the handle. Keep the token private as well:
DPAPI protects stored data, not against arbitrary code already running as your
Windows user. No new dependencies or external credential service are used.

AES-CTR alone provides no authenticity. Supply hashes from a trusted source;
the test fixture's known plaintext hash only proves the fixture result. A
successful `finish` callback never overrides a failed core hash check.

## Recovery and compatibility

The core downloads encrypted bytes through its regular range/resume, proxy,
speed-limit and quota pipeline. It checks the complete source hash before Lua
processing, flushes output before atomically replacing its checkpoint, validates
saved plaintext before resuming, and publishes without overwriting an existing
file. Only an uncommitted private tail is truncated. Corrupt or incompatible
checkpoints fail with an instruction to restart. Completed output follows the
normal antivirus/completion flow.

Like reviewed video streams, provider jobs in synchronization queues retain their
reviewed content instead of automatically checking for a different file version.

Saving a provider definition upgrades the profile's required writer/data version
to 3. Opening an older provider profile atomically protects frozen job source and
removes resolved URLs from active saved plans while preserving content and
checkpoint bindings. The original Windows account is required to reopen frozen
jobs. A failed upgrade rolls back rather than discarding a download.

This upgrade does not erase historical SQLite pages, WAL files, exported scripts
or backups. If private keys or signed URLs were previously stored there, remove
private literals from library scripts and rotate those secrets as appropriate.

Older builds then refuse that profile, preventing them from treating a provider
job as an ordinary encrypted file. Existing downloads, settings and files stay
in place. Keep a profile backup before downgrading; use a separate profile for
older versions.

## Offline developer verification

No additional dependencies are needed beyond the repository's existing Rust
and Node.js prerequisites. Build and run the actual application tests:

```powershell
cargo test --offline --locked --lib provider::
cargo build --offline --locked --bins
node scripts/provider_smoke.cjs target/debug
```

The smoke test generates harmless bytes, encrypts them with Node's AES-CTR,
runs the real desktop/CLI/provider workers in a private profile, and checks
native and Lua-written CTR output, idempotent submission, renewed-URL network
resume, processing pause/restart/resume, frozen code, corrupt checkpoints,
chunked transfers and size limits, wrong keys, memory/time limits, busy-worker
cancellation and restricted metadata access. The server
binds only to `127.0.0.1`. It cleans up its owned desktop, server, fixture files
and profile. Results are recorded under
`artifacts/lua-provider-integration-20261029/`.

The earlier [standalone experiment](lua-provider-lab.md) is retained for its
known-answer tests and comparison. Its `experimental-0` callback argument order
and key-grant helpers are different; use the new examples in the installed app.

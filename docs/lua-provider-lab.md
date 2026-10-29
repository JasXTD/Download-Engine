# Offline Lua provider experiment

For the integrated app in 3.5.0, use [Lua download providers — API 1](lua-providers.md)
and `examples/download-provider.lua` or `examples/download-provider-custom-ctr.lua`.
This document retains the earlier standalone experiment and its different API.

This is a **developer proof of concept**, separate from the installed app. It
demonstrates user-written Lua processing of encrypted bytes, with a Rust host
controlling input, chunk boundaries, checkpoints, cancellation and publication.
The experiment uses the project's existing Lua 5.4/mlua dependency and Windows
cryptography. It does not add an installed tool or change normal downloads.

The examples are **not yet supported by Scripting → Import script**. Their
`experimental-0` interface is separate from the app's stable Lua API 1. They are
not a Crunchyroll downloader and contain no code from that project. Passing the
sample proves this extension mechanism; it does not establish Widevine,
Crunchyroll authentication, media-container or live-service compatibility.

## Run it on Windows

Use the repository's existing Rust/MSVC build environment. From the source root:

```powershell
cargo run --offline --locked --example lua_provider_lab
```

No packages are downloaded by this command. If the locked dependencies or build
tools are not already installed, Cargo will fail; the demo does not install them.

The host generates a harmless file of **2,097,189 bytes**, encrypts it using a
public test key, and reads `examples/lua-provider.lua` from disk. It processes
three chunks, pauses at **196,617 bytes**, then constructs a fresh Lua runtime.
It replaces the sample's source reference and removes the old grant, invokes
`refresh`, validates the checkpoint and continues from the saved offset.

The final plaintext must match the generated file byte for byte. A second run
must reuse the completed result without processing it again. JSON output reports
these checks, the plaintext SHA-256, the resumed offset and
`sample_files_cleaned: true`. The private temporary directory is removed before
success is printed. No generated file is opened or executed.

## Write the processing logic in Lua

The second example constructs AES counters and performs the XOR decryption in
Lua. Rust only supplies the AES block primitive and the permitted key:

```powershell
.\target\debug\examples\lua_provider_lab.exe --script .\examples\lua-provider-custom-ctr.lua
```

Both examples should produce the same plaintext hash. The report's
`script_sha256` will differ. **The executable reads the Lua file at runtime**:
after the host is built, editing or switching Lua files needs no Rust rebuild.
If `CARGO_TARGET_DIR` is configured, use its `debug\examples` directory instead.

Start with a copy of either Lua file. Keep these four functions in the table
returned by the script:

| Callback | Contract |
| --- | --- |
| `resolve(context)` | Return the source plan for `context.asset`. |
| `refresh(context, previous)` | Return a fresh source reference for the same content on resume. |
| `transform(bytes, offset, plan)` | Return a binary string with the same length as `bytes`; offset is zero-based in **bytes**. |
| `finish(plan, output)` | Validate the final summary and return `true`. The host independently verifies the expected hash. |

Every chunk must be processable from its offset and plan alone. Do not depend on
mutable Lua globals or assume one call per file. Lua state is lost when paused;
the host can start a new interpreter for resume. Stateful or length-changing
formats require a different checkpoint contract and are outside this experiment.

The lab's host functions are deliberately small:

| Host function | Meaning |
| --- | --- |
| `xd.metadata("sample-episode")` | Return the granted local fixture metadata; **no HTTP request**. |
| `xd.aes_ctr(key_id, counter_hex, bytes, offset)` | Native AES-CTR over one bounded chunk. Counter is 16 bytes encoded as 32 hex characters; increment is full 128-bit big endian. |
| `xd.aes_blocks(key_id, bytes)` | AES encryption of complete 16-byte blocks, without padding. The custom Lua example uses it to create a CTR keystream. |

Keys are referenced by grant names, not file paths or literal key bytes in Lua.
Source identifiers map to files held by Rust; Lua cannot substitute an arbitrary
local path. The metadata includes the source size, encrypted and plaintext
SHA-256 values, key ID, counter and transform ID. Changing those content fields,
the script, the key or the chunk size invalidates a saved run. A renewable source
reference alone may change.

The fixed key and counter are **public test material only**. AES-CTR does not
authenticate ciphertext. Here, the trusted fixture supplies the expected
plaintext hash, which Rust checks before publication. A real format needs an
appropriate integrity/authentication scheme; a hash supplied by an untrusted
source does not establish authenticity.

## What is checked

```powershell
cargo test --offline --locked --example lua_provider_lab
```

The focused tests cover:

- Native AES and custom Lua CTR against independent NIST test vectors, including
  unaligned offsets and counter overflow.
- Binary bytes, partial final chunks, pause and resume in a fresh interpreter,
  renewed source references, and repeat completion without duplicate processing.
- A separate custom Lua byte transformation without any native crypto call.
- Wrong keys, source corruption, changed scripts/keys/settings, damaged or missing
  checkpoint data, and an uncommitted write tail after simulated interruption.
- Cancellation of busy Lua, callback timeout, Lua memory exhaustion, wrong output
  types/lengths and final validation failures.
- Missing grants, arbitrary source names, pre-existing output preservation and
  rejection of unexpected hard links.

All test files live in disposable directories. The standalone run reports cleanup
errors; tests also check explicit directory removal. A forced termination of the
whole process can leave a `download-engine-provider-lab-*` folder in the user's
temporary directory. Neither the demo nor the tests touch the app's databases,
queues, browser registration, antivirus settings or installed files.

## Boundaries before app integration

The lab has no networking, login, DRM-license handling, browser cookies, media
muxing, persistent credential vault or user-facing permission controls. Source
refresh is simulated by changing a local grant. Its serial processing loop is
not a test of the app's HTTP scheduler or parallel transfer engine.

Scripts have a 64 KiB source limit and a 16 MiB Lua memory limit. Chunks are at
most 256 KiB and files at most 64 MiB. Callbacks get instruction-hook time and
cancellation checks. `io`, `os`, `package`, `debug`, dynamic loaders, coroutines,
`pcall` and `xpcall` are absent. This restricted **in-process lab is not an OS
security boundary**; use reviewed code. Production integration would need owned
worker processes, hard cancellation/time limits and a complete threat review.

Checkpoints are written after output is flushed. Resume verifies the encrypted
source and saved plaintext prefix, discards only an uncommitted private tail,
then processes the remaining ciphertext. The original encrypted input is never
overwritten. Publication uses a new hard link after verification and keeps the
partial file as its recovery anchor. This models interruption recovery but is
not a real power-loss or physical disk-failure test.

The next engineering step, if requested, is adapting these contracts to the app's
existing transfer worker, secret storage, capability prompts and versioned script
packages. The prototype deliberately makes no compatibility promise for those
future APIs. There is no app version bump, dist rebuild or release associated
with this experiment.

## References

- [Lua 5.4 binary strings and embedding](https://www.lua.org/manual/5.4/manual.html)
- [Windows CNG symmetric keys](https://learn.microsoft.com/en-us/windows/win32/api/bcrypt/nf-bcrypt-bcryptgeneratesymmetrickey)
- [NIST SP 800-38A, sections F.5.1 and F.5.5](https://csrc.nist.gov/pubs/sp/800/38/a/final)

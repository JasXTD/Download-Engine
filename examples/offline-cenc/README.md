# Offline CENC provider lab

An original Lua example for **Download Engine 3.15.0 or newer**. It demonstrates
sample-level MP4 decryption, a synthetic binary license exchange, multiple tracks
and styled ASS subtitles using the existing provider APIs. No Rust changes or
additional dependencies are needed.

This is an offline developer lab, **not a Crunchyroll or Widevine integration**.
It uses generated test patterns, a sine wave, temporary RSA/AES keys and an
invented license format. It contains no accounts, device credentials or service
endpoints. Its original code uses the repository's [MIT license](../../LICENSE).

## Run the checks

From the source checkout, with the project's existing Node.js, built app and
approved FFmpeg/ffprobe available:

```powershell
node scripts/offline_cenc_smoke.cjs dist/IDM-EX-UnsignedDev artifacts/offline-cenc-3.15.0
```

The first argument is a package-shaped binary folder containing `de`, `de-cli`
and the app's required runtime resources. The second is the evidence folder.
`MEDIA_TEST_FFMPEG` and `MEDIA_TEST_FFPROBE` can point to already-installed native
tools. The Windows run was exercised; this does not establish macOS/Linux
runtime compatibility for the new example.

The harness generates media, constructs a test extension, installs it into its
own temporary profile, and starts its own app instance and loopback-only media
server. Update checks and notifications are disabled in that profile only. It
does not change your normal profile, browser registration, proxy, hosts file or
antivirus configuration. Windows protected imports need a normal user context;
a restricted execution sandbox may prevent DPAPI from working.

On completion or failure, the harness closes its app/server and removes its
temporary profile, generated media, downloaded movies and key material. Only
`verification.json` in the evidence folder remains. The source folder is an
author template: `catalog.lua` and `fixture_credentials.lua` are deliberately
generated during testing, not checked in or usable as service credentials.

## What runs in Lua

1. `lib/license.lua` builds and signs a binary request with RSA-PSS/SHA-256.
   The local Node fixture issuer independently verifies the signature and returns
   RSA-OAEP/SHA-256-wrapped session material, AES-CTR-encrypted content keys and an
   HMAC-SHA-256. Lua authenticates and decodes that reply and checks the content
   identity, request nonce, key IDs and exact lengths. This format is invented
   for the test; it is not a real service protocol or a production protocol design.
2. `provider.lua` resolves the generated track catalog and hands the decoded keys
   to the core using `xd.handoff_key`. The saved plan has only nonsecret key IDs,
   segment identities, sizes and hashes.
3. `lib/cenc.lua` parses encrypted sample entries and `tenc`, then uses `tfhd`,
   `trun`, `senc`, `saiz` and `saio` to locate encrypted bytes. It resets CTR for
   each sample and advances it only for encrypted bytes across subsamples.
4. Streaming callbacks replace encryption metadata with same-sized `free` boxes,
   restore the original codec entry and decrypt media bytes in bounded chunks.
   Keeping box lengths unchanged preserves fragment data offsets. Original
   initialization is replayed by `prepare_track` after each fresh processor starts.
5. The core owns downloading, hash checks, cancellation, checkpoints and muxing.
   Its bundled FFmpeg receives only the resulting clear tracks and the ASS file.

The synthetic license request/reply travels through local worker IPC and a
temporary fixture, **not an HTTP license endpoint**. Binary HTTP transport has
separate core tests. This lab does not claim an end-to-end live license exchange.

## Protected credentials and local fixtures

There are two explicit checks:

- A protected variant imports the generated RSA identity and binary license
  through the real CLI. Lua decodes the reply, but the app correctly rejects
  the resulting localhost media plan because an extension with protected
  credentials may only return its declared domain names. No media request occurs.
- The full download variant uses generated, disposable fixture material in a
  temporary module and has no protected settings. It can use the exact local
  source URL explicitly submitted by the harness. Content keys still pass through
  the real opaque handoff API to the processor.

The harness does not relax either permission boundary. For an actual service
adapter, replace the fixture inputs with `xd.rsa_key_setting`,
`xd.binary_setting` and authenticated HTTP calls as appropriate, and declare the
real API and media hosts. Never put actual credentials in a Lua package.

## Supported CENC subset

| Item | This example |
| --- | --- |
| Encryption scheme | `cenc`, AES-128 CTR |
| Codecs | AVC (`avc1`/`encv`) and AAC (`mp4a`/`enca`) |
| Initialization | One track and one sample description; version-0 `tenc` |
| Fragment layout | One `moof`, one `traf`, one `trun`, one `mdat`; `default-base-is-moof` and explicit data offset |
| Sample sizes | Per-sample `trun` sizes or a `tfhd` default |
| IVs | 8 or 16 bytes per sample |
| Encryption records | In-fragment `senc` plus matching `saiz`/`saio`; whole samples or clear/encrypted subsamples |
| Output | Clear tracks assembled by the app; MKV preserves ASS styling |

Unsupported layouts fail explicitly. This parser does not implement `cbcs`,
constant IVs, `seig` sample groups/key rotation, multiple sample descriptions,
multiple runs/tracks in one fragment, external auxiliary data, 64-bit box sizes,
HEVC, or a general DASH/XML/protobuf parser. Those are additional Lua development
tasks; this example alone does not demonstrate every real service format.

The structure follows the [W3C CENC stream-format overview](https://www.w3.org/TR/eme-stream-mp4/).
The fixture writer and Lua parser are original code, not copied from a service
downloader or DRM SDK.

## Evidence and limits

The checks compare every processed fragment with the expected clear bytes and
compare muxed video/audio packets with the originals. FFmpeg also decodes the
clear reconstructed fixtures and compares their frame hashes. The tests exercise
protected imports, malformed license replies, invalid box/auxiliary metadata,
Pause/restart/Resume, forced app termination and Stop all with unfinished work.
Recovered jobs must keep their IDs and reuse verified input segments; Stop all
must cancel both test jobs without late checkpoint writes.

Our bundled FFmpeg deliberately rejects PSSH-bearing encrypted input, so it is
**not** an independent decoder oracle for the encrypted fixture. That protection
was left intact. Successful checks demonstrate our generated CENC subset and
clear-output interoperability, not certification against every CENC producer,
Widevine compatibility or real Crunchyroll operation.

The result answers a narrower question: the tested license/key/segment workflow
can be implemented in Lua on the existing core. A live adapter still needs its
own authentication, discovery, manifest and license-protocol implementation,
additional container variants as required, and separate integration testing.

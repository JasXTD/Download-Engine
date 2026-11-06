# Offline episode extension

A working Lua extension for **Download Engine 3.11.0 or newer**, following the
track-selection and segmented-download workflow of
[CuteTenshii/crunchyroll-downloader](https://github.com/CuteTenshii/crunchyroll-downloader).
It adapts that project's language selection and segment-template helpers to Lua;
see [NOTICE.md](NOTICE.md) for the pinned source, modifications and MIT notice.

The video is a four-second generated test pattern. The two audio choices are
synthetic tones, labelled with Japanese and English locale codes to demonstrate
track selection. Subtitles are generated sample text. **This bundled version uses
offline samples.** Crunchyroll authentication, Widevine license handling and live
service discovery are not implemented.

The Lua source is editable and MIT licensed. You can modify and redistribute it
while retaining the required notices in [NOTICE.md](NOTICE.md). The missing
features describe this version's implementation, not restrictions on what users
may develop. Features beyond the exposed Lua APIs may also need core changes.

Download Engine 3.15.0 adds [generic provider core APIs](../../docs/lua-provider-core.md)
for binary/RSA imports, cryptography, scoped HTTP sessions, unknown sizes,
streaming processing and ASS subtitles. This demo keeps its small offline workflow;
the new helpers do not turn it into a live Crunchyroll integration.

For sample-level encryption instead of this example's whole-fragment envelope,
see the [offline CENC provider lab](../offline-cenc/README.md). It adds a Lua
parser for a defined AVC/AAC CENC subset and a synthetic license exchange, tested
with generated files. It also remains an offline example.

## Try the bundled demo on Windows

Download Engine 3.12.0 packages include this demo in their own `plugins` folder.
Discovery only lists it: you still review, install and enable it yourself.

1. Open **Extensions → Installed → From plugins folder → Open folder**, then open
   the `offline-episode` subfolder. Alternatively, extract the whole standalone
   `Offline-Episode-1.0.0.zip`. Keep its folders together.
2. Open PowerShell in the demo folder and run:

   ```powershell
   .\Start-Server.ps1
   ```

   Keep that window open. It prints an address such as
   `http://127.0.0.1:54321/sample-episode`. The port is selected automatically.
   This small local server is the pretend website holding the sample segments.
   It accepts only loopback requests for the generated files; it installs nothing.
3. In Download Engine, choose **Review install** beside the discovered demo.
   If using the standalone ZIP, choose **Extensions → Install from file…** and
   select `offline-episode.de-extension.json` instead. Review, install, and enable it.
4. Leave the defaults for the first run. In its provider job, paste the printed
   address into **Source URL**, enter `Sample episode` as the output name, and
   choose **Add provider download**.
5. The result is an MKV containing video, two audio tracks and English subtitles.
   Your media player's audio/subtitle menus let you switch between them.
6. Try different settings and add another download. Use **Pause/Resume/Stop** as
   usual. Stop retains completed segments. To make progress easier to observe,
   restart the sample server with a fixed port and a small delay:

   ```powershell
   .\Start-Server.ps1 -Port 54321 -ChunkDelayMs 40
   ```

   Use its printed address. If a paused download needs the server restarted,
   restart on the same port as that download's source URL.
7. Press **Ctrl+C** in PowerShell when finished. Remove the demo downloads from
   the app before removing the extension if any remain unfinished. Remove the
   extension when no longer needed. You may delete a separately extracted bundle.
   The bundled package will stay available for review after uninstalling the
   extension; it will not reinstall or enable itself. Downloaded
   sample movies remain in the folder you selected until you delete them.

If PowerShell policy blocks the launcher, follow your machine's approved script
policy; the demo does not change execution policy or require administrator access.
The extension itself is ordinary portable Lua. This bundle's launcher is a
PowerShell script; macOS/Linux users need an already-installed PowerShell 7 to use
it. Only the Windows demonstration was exercised here.

## Settings to try

| Setting | Example | Result |
| --- | --- | --- |
| Video quality | `144p` or `90p` | Chooses a different generated representation. |
| Audio locales | `ja-JP,en-US` | Japanese-labelled tone is default; English-labelled tone is also included. |
| Audio locales | `en-US,ALL` | English first, then remaining available tracks. |
| Audio locales | `ALL` | Follows upstream's preferred Japanese locale, then stable order. |
| Subtitle locales | `en-US`, `ja-JP`, `ALL`, `NONE` | Selects sample subtitles or omits them. |
| Output container | `mkv` or `mp4` | Core muxing preserves encoded video/audio; MP4 converts subtitle text to mov_text. |

The first selected audio/subtitle is the default for its kind. Repeated locale
names are deduplicated. Unknown locales fail with a clear error before any media
request; this is intentionally stricter than upstream's optional-locale skipping.
Track/container choices are made when a download first resolves. On Resume this
example rebuilds from the saved selection, so changing those preferences does
not alter a partially downloaded episode. The app freezes the extension's code;
settings and saved keys are read again when a worker starts.

The default uses a **public sample key**, so no credential setup is needed.
To demonstrate protected key settings, save the following value in **Optional
sample AES key** and turn on **Use the optional saved AES key**:

```text
603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4
```

The processor receives an opaque key handle through `xd.key_setting`; the
resolver does not receive the key. A wrong key fails plaintext verification and
does not publish a movie. This key is public fixture data, not a service secret.

## How it works

1. `provider.lua` reads the quality/language settings and the generated catalog.
   `lib/upstream.lua` supplies adapted language ordering and finite timeline / URL
   template expansion. There is no network metadata lookup in this fixture.
2. `resolve` returns `xd.media_plan`: clear MP4 initialization data followed by
   encrypted fragments, plus the chosen audio and subtitle tracks. Each segment
   has an exact size and ciphertext/plaintext SHA-256.
3. Download Engine fetches the segments, manages queue controls and stores recovery
   checkpoints. The example accepts only the explicit local sample URL. The
   required network declaration is a reserved `.invalid` name; no public host is
   contacted and no hosts-file change is needed. The core permits segment
   downloads from the same local origin the user explicitly entered.
4. `transform` uses `xd.aes_ctr` and `context.track_offset + offset`. This fixture
   encrypts whole fragment bytes with a per-track counter, leaving initialization
   bytes clear. It is **not MP4 CENC/Widevine sample encryption**.
5. `finish` checks the completed byte counts. The core verifies hashes and invokes
   its bundled FFmpeg to assemble the selected tracks into MKV or MP4.

This demonstrates the extension/core boundary. It does not establish compatibility
with a commercial service. A service adapter also needs its own authentication,
manifest parsing, encryption format and renewal/cleanup behavior.

## Files and editing

- `offline-episode.de-extension.json`: ready-to-install package.
- `extension/`: author folder, including the generated `catalog.lua`. Edit the Lua
  here and rebuild with `de-cli extensions pack`; the installed Code tab is read-only.
- `fixtures/`: generated segments and a strict server route list.
- `catalog.json`: readable metadata for understanding/testing the fixture.
- `Start-Server.ps1`: local sample server; no app configuration changes.
- `SHA256SUMS`: hashes of every other file in this bundle.
- `NOTICE.md`: source provenance and required license notice. The notice is also
  embedded in `lib/upstream.lua`, so it travels inside the installable package.
- `LICENSE`: the MIT license for Download Engine's original example code.

After editing an already-installed example, increase its extension version in
`extension/manifest.json` (for example `1.0.1`), then use the app's CLI:

```powershell
& 'C:\path\to\Download Engine\de-cli.exe' extensions pack .\extension --out .\my-episode-1.0.1.de-extension.json
```

Install the new package and review the update. You do not need to recompile
Download Engine. Pack refuses an existing output filename.

## Rebuild from the Download Engine source checkout

Use the project's existing Node.js, built CLI and approved bundled FFmpeg. The
builder generates all media locally and uses a temporary profile only to pack
the extension. It never installs it into your normal profile.

```powershell
node scripts/build_offline_episode_demo.cjs
```

Default output: `dist/examples/Offline-Episode-1.0.0/`. Choose a new output folder
as the first argument to build another copy. The builder refuses to overwrite an
existing folder. `MEDIA_TEST_FFMPEG` and `MEDIA_DEMO_CLI` can select already-approved
native binaries on other platforms.

To run the isolated demonstration checks with the existing approved `ffprobe`:

```powershell
node scripts/offline_episode_smoke.cjs dist/examples/Offline-Episode-1.0.0
```

The test creates and removes its own profile, processes and downloaded movies.
The reusable demo bundle stays available for you to try.

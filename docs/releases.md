# Versioning and release notes

Every delivered update gets a new version and a dated entry in `CHANGELOG.md`.
Use a patch increment for fixes, UI or documentation refinements, a minor increment
for compatible features, and a major increment for breaking changes. Work in
progress can use one pending version across its internal builds and corrections.

`Cargo.toml` is authoritative. Keep the `download-engine` package entry in `Cargo.lock`
and `extension/manifest.chromium.json` / `extension/manifest.firefox.json` aligned.
Chrome and Edge share the Chromium manifest. About and both CLI version displays
use Cargo's full major.minor.patch value. Windows executable properties and the
installer version also derive from that value.

Add a heading such as `## 1.0.1 — 2026-10-16` at the top of the release history,
followed by concise user-facing changes. Keep older entries below it, newest first.
Use paragraphs and `###` subheadings separated by blank lines; Help → Changelog
renders these offline with the same text layout as the Guide. Do not add HTML or
instructions that require network access. Preserve the distinction between new
changes and earlier development work that reused a version number.

## Build and verify

After updating sources and the release notes, run focused tests and visual checks
for the affected behavior. Then run these commands from the project root:

```powershell
cargo fmt --all --check
cargo clippy --offline --locked --all-targets --all-features -- -D warnings
cargo build --release --offline --locked --bins
.\scripts\Test-Release-Version.ps1 -CheckBinaries
.\scripts\Test-ReleasePackaging.ps1
node scripts/Build-LicenseBundle.cjs --check
.\scripts\Package.ps1 -SkipBuild -AllowUnsignedRelease
.\scripts\Build-Installer.ps1 -SkipBuild -AllowUnsignedRelease
.\scripts\Package-ReleaseSources.ps1
.\scripts\Write-ReleaseChecksums.ps1
```

Both packaging entry points verify Cargo, the lockfile, browser manifests and
the newest dated changelog entry before building. They also reject binaries whose
Windows version metadata differs from the release, including with `-SkipBuild`.
These checks catch inconsistent versions; they do not detect source edits made
without a version bump. The release author must choose and increment the version.

Check Help → Changelog, Help → About, `de-cli.exe --version` and `--help`,
the packaged manifests, and the packaged `CHANGELOG.md`. Ensure the packaged
executables match the release build. The versioned ZIP is the canonical portable
artifact and is always assembled from a fresh whitelist.

Public artifacts are written to `dist/releases/<version>`. Every public packaging
entry point rejects unexpected files, subdirectories and links in that version's
folder; checksum generation additionally requires all six payloads. The completed
folder contains exactly those six files plus `SHA256SUMS-<version>`. Never upload
the entire `dist` tree. Existing older builds and private test profiles are not moved.

All packagers validate `scripts/release-source-inventory.json` before copying
sources or bundled directories. New source files need an explicit inventory review.
Sensitive names require a documented path/SHA-256 exception; currently only public
FTPS test identities and pinned FFmpeg build logs have one. Source packaging also
validates the staged bytes before replacing an existing archive. See the GitHub
release guide for inventory maintenance and the limits of credential scanning.

`dist/IDM-EX-UnsignedDev` is a stateful local test profile, not a distributable
folder. It can contain private queue history, diagnostics, browser registration
and downloads. To refresh it for packaged smoke tests, close the application and
any MCP server using that copy, then run:

```powershell
.\scripts\Package.ps1 -SkipBuild -AllowUnsignedDevelopment -UpdateLocalPortable
```

Disconnecting an MCP client does not stop its HTTP server. Desktop-owned MCP stops
when the app exits; a terminal server uses the existing de-cli.exe and must be
stopped separately. The packaged docs/mcp.md, docs/mcp-ai-guide.md and Help → MCP
setup describe connection and usage. Historical versioned archives can coexist.
Unsigned development archives and the test installer are local test artifacts.
Public unsigned GitHub releases use `-AllowUnsignedRelease` and explicit
`Unsigned`/`UnsignedSetup` filenames. They include the source/license materials;
see `docs/github-release.md`. Signing and browser-store publication are separate.
For a signed build, supply `-SignTool`, `-CertificateThumbprint` and `-TimestampUrl`
instead of either unsigned switch. Do not mix signing modes.
For signed public packages, also pass `-Signed` to `Package-ReleaseSources.ps1`
and `Write-ReleaseChecksums.ps1`. A version folder cannot mix signed and unsigned
Windows package names.

## Language release checks

Update `locales/desktop.json`, `locales/extension.json` and the additional-language
`locales/additional.json` catalog. Keep `locales/languages.json`, the desktop enum,
extension picker and Setup entries aligned when adding a language. Generate and
verify extension assets before packaging:

```powershell
node scripts/Build-Locales.cjs
node scripts/Build-Locales.cjs --check
node --test tests/*.test.cjs
node scripts/localization_ui_smoke.cjs
cargo build --offline --locked --features screenshot --bins
node scripts/native_ui_smoke.cjs --localization
```

The browser UI check requires the existing Playwright runtime and installed Chrome
and Edge. It renders production UI and CSP with simulated extension APIs, using
temporary browser contexts. It does not claim live native messaging or Firefox
runtime coverage. The native check uses isolated paused profiles and the real
egui renderer. Both checks use the language registry. Review the generated images
for all supported languages, including Arabic direction and longer labels.

After building the test installer, `node scripts/installer_languages_smoke.cjs`
installs, upgrades and uninstalls each registered language in temporary directories. It
checks first-run language inheritance, saved preference preservation, localized
extension payloads, preservation of test user files and unchanged native browser
registrations, then removes its fixture data. It requires no antivirus or system
proxy changes. See `docs/localization.md` for translation scope and provenance.

For a focused packaging check, pass `--languages en,zh-CN,ar`. Each selected
language still checks every bundled browser locale, the Extensions guide and all
hash-pinned FFmpeg files. Omit the option to test every installer language.

## Windows memory checks

Run the UI lifecycle check alone so other tests cannot distort its process counters:

```powershell
cargo test --offline --locked --bin de desktop::memory_tests::repeated_pages_dialogs_and_history_cleanup -- --ignored --exact --nocapture --test-threads=1
node scripts/memory_packaged_soak.cjs dist/IDM-EX-UnsignedDev
```

The first command cycles pages, dialogs and history removal using egui and a fresh
database. It prints private memory and Windows handle/resource counts. It does not
create GPU windows. The second runs the packaged desktop, local downloads,
pause/resume, Lua and PowerShell runs, script cancellation, desktop-owned MCP, and
24 real download-status windows, followed by MCP reads with a fixed history size.
Add `--windows-only` to skip repeating the mixed download/script phase. Node must
provide `node:sqlite`.

The packaged run records once-per-second counters and settled checkpoints under
`artifacts/memory-packaged-*`, closes its own processes, and removes its generated
download payloads. Personal profiles, browser registrations and OS proxy settings
are untouched. Only the child counter-reader process bypasses PowerShell's script
execution policy; no persistent policy is changed.

Compare settled samples after warm-up and inspect private bytes, threads, handles,
GDI and USER resources together. Download/run history remains in the isolated
profile and its size is reported. Passing workload checks or a stable working set
alone does not prove the absence of leaks; these are bounded soak tests, not heap
allocation tracing or a multi-day stability claim.

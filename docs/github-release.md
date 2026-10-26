# Publishing an unsigned GitHub release

Download Engine's original code is MIT licensed. Third-party components retain their
own licenses; read `THIRD-PARTY-NOTICES.md`. No GitHub repository URL, publisher
certificate or browser-store approval is assumed by these scripts.

## Set up private security reporting

Publish the root [SECURITY.md](../SECURITY.md) on the repository's default branch.
Before publishing the release, enable **Private vulnerability reporting** in
**Settings → Advanced Security**, and enable notifications for security reports.
Check that **Report a vulnerability** is available on the repository's security
advisories page. See [GitHub's setup instructions](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository).

This is a repository setting: the policy file and packaging scripts do not enable
it. The policy explains how to request a private contact if reporting is unavailable.
Keep vulnerability details out of public issues. The policy also ships with the
portable app, installer and source archive.

## Build the release

Install the prerequisites described in the README (Rust/MSVC build tools, Node.js
and Inno Setup). For a fresh Cargo cache, run `cargo fetch --locked` before the
offline checks below. Git checkouts and the source-only ZIP exclude executable
files: copy the matching `tools/ffmpeg/ffmpeg.exe` from the portable release, or rebuild it using
`tools/ffmpeg/source/README.md` and update the documented hash pins. Then run
`scripts/Prepare-MediaTools.ps1` to verify the complete media bundle.

Keep `tools/ffmpeg/ffmpeg.exe` locally as an ignored packaging input. Publish it
inside the installer and portable release assets, not as a tracked source file.
FFmpeg's corresponding source archives, build recipe and license notices remain
tracked and are included in the release packages. The packagers copy the verified
local executable explicitly; Git ignore rules do not exclude it from those assets.
The removal from the current source tree does not rewrite earlier commits or tags.

Use the current version in Cargo.toml, Cargo.lock, both extension manifests and
the newest changelog heading. From the project root:

```powershell
node scripts/Build-Locales.cjs --check
node scripts/Build-LicenseBundle.cjs --check
cargo fmt --all --check
cargo clippy --offline --locked --all-targets --all-features -- -D warnings
cargo build --release --offline --locked --bins
.\scripts\Test-Release-Version.ps1 -CheckBinaries
.\scripts\Test-ReleasePackaging.ps1
.\scripts\Package.ps1 -SkipBuild -AllowUnsignedRelease
.\scripts\Build-Installer.ps1 -SkipBuild -AllowUnsignedRelease
.\scripts\Package-ReleaseSources.ps1
.\scripts\Write-ReleaseChecksums.ps1
```

Use the signed parameters documented in `releases.md` if a publisher certificate
is available. `-AllowUnsignedRelease` is an explicit choice; it cannot be mixed
with signing parameters or `-AllowUnsignedDevelopment`. It produces a normal
per-user installer with the stable application identity, not a smoke-test install.
Windows may display an unknown-publisher warning. Do not describe it as signed.

For version 2.0.0, upload only the following files from
`dist/releases/2.0.0` after checksum generation succeeds:

- `DownloadEngine-2.0.0-Unsigned-windows-x64.zip` — portable application.
- `DownloadEngine-2.0.0-UnsignedSetup.exe` — Windows installer.
- `download-engine-chrome-2.0.0.zip`, `download-engine-edge-2.0.0.zip`, and
  `download-engine-firefox-2.0.0.zip` — extension source/developer-loading packages.
- `DownloadEngine-2.0.0-Source.zip` — matching application and dependency sources.
- `SHA256SUMS-2.0.0` — checksums for the assets above.

The version folder rejects unexpected files, links, subdirectories, old versions
and test artifacts. Do not use a wildcard upload against `dist`. Older outputs
are left in place for recovery; no script moves or deletes private test profiles.
For signed builds, pass `-Signed` to the source and checksum scripts too. Do not
mix signed and unsigned Windows artifact names in one version folder.

The app and installer already contain FFmpeg's exact source archives, build
recipe, GPL text and build records, plus the required MPL source archives.
Keep those files with redistributions. The additional source ZIP makes the
application build inputs easy to download without a Git checkout. The build
recipe records toolchain versions; it is not a promise of byte-identical output
from future compiler/OS versions.

Publish the matching source tree/tag, copy the newest changelog entry into the
GitHub release notes, and state clearly that the Windows binaries are unsigned.
Do not upload `dist/IDM-EX-UnsignedDev`, `artifacts`, private queues, downloaded
files, browser registrations, or old `TestSetup`/`UnsignedDev` archives.
The source packaging script uses an exact file inventory rather than recursive
directory inclusion. Private top-level data/build folders are never candidates.

Chrome and Edge developer-loading instructions are in the
[browser setup reference](reference.md#connect-a-browser). The Firefox ZIP is not
a signed XPI; permanent installation in ordinary Firefox requires
Mozilla signing. Browser-store publication and signatures are separate from
publishing this GitHub release.

## Maintaining the source and license bundle

Review additions and removals in `scripts/release-source-inventory.json` when source
files change. Do not regenerate that list blindly from a working directory. Any
unexpected file below an approved source root blocks packaging. Known dependency
and Python cache directories are excluded; linked paths are rejected before traversal.

Sensitive filenames (environment files, signing keys, DPAPI data, databases and
logs) are denied even if added to the inventory. Exceptions must have an exact path,
SHA-256 and review reason. The two FTPS PFX files are deliberately public localhost
test identities with a public password; they are included only in the source ZIP
with their reviewed hashes. The four shipped FFmpeg build logs also have hash
exceptions and remain covered by the media manifest. Changed exception bytes require
another review, never an automatic pin update.

Text checks also reject private-key blocks and recognizable AWS/GitHub credentials,
and a SQLite header is rejected even under a different extension. These checks do
not recognize every possible secret or replace human review. Dependency archives
retain their separate Cargo/media checksum verification. The source packager checks
the staged inventory and content again before publishing the ZIP; a validation
failure preserves the prior archive. `Test-ReleasePackaging.ps1` covers these guards
using synthetic data, public test identities and isolated temporary directories.

`Build-LicenseBundle.cjs` reads only the locked local Cargo cache, checks archive
hashes against Cargo.lock, and generates the Rust license index, preserved notice
files and selected source archives. Run it without `--check` after reviewing a
dependency update. Review newly encountered licenses and missing notice files.

`vendor/epaint` is a modified copy of epaint used through `[patch.crates-io]`; it is
a source root and its files are in the inventory. The license script accepts no other
local dependency, takes epaint's notices and included source from the original crate
(checked against the crates.io checksum recorded in `patchedCrates`) and marks it as
modified. Update both when egui is upgraded; see `vendor/epaint/patch.md`.

To rebuild FFmpeg, follow `tools/ffmpeg/source/README.md`. Its source archives are
pinned by SHA-256. Review changed source, build the helper, test HLS/DASH and
cancellation, then deliberately update its manifest and the runtime hash pin.
Packaging rejects missing or altered source, notices or media binaries.

To publish later releases, increment the version and regenerate the packages and
checksums. Do not edit an existing release asset silently.

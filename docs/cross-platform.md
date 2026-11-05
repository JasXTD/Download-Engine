# macOS and Linux

The native builds use the same Rust download engine and egui interface as Windows.
Version 3.10.0 adds macOS Apple Silicon and Intel packages and a Linux x64 archive
alongside the Windows installer and portable ZIP. Platform adapters preserve the
existing UI and engine. Read the requirements and validation limits below before
choosing a package.

## What is shared

Downloads, queues, schedules, torrents, browser integration, website grabbing,
video merging, Lua automation/providers, localhost MCP, themes and language
preferences use the existing interface. Files and magnet links still open a review
before downloading; torrents remain off until the user enables them.

Windows tray notifications, automatic clipboard offers, the floating drop box,
Windows startup registration, Explorer antivirus integrations, enterprise SSPI
login, and completion power actions are Windows integrations. Their Windows-only
controls are not offered by the native builds. Linux/macOS can use ClamAV or a
configured native command-line scanner. PowerShell automation needs an existing
PowerShell 7 installation; Lua is included. No platform installs a scanner or
PowerShell automatically.

## Linux

Builds made on Debian 12 require glibc 2.36 or newer, OpenSSL 3, D-Bus, and the
desktop's graphics/input libraries. Use an X11 or Wayland desktop with
`xdg-desktop-portal` and its GNOME/KDE/GTK backend for native file/folder dialogs.
On Debian 12, X11 also requires `libxkbcommon-x11-0` (which pulls in
`libxcb-xkb1`). A minimal Xfce installation may omit it; without it the window
library cannot initialize. Install runtime libraries through your distribution's
package manager before launching the extracted app.
Saved credentials need an unlocked Secret Service keyring, such as GNOME Keyring
or a compatible KDE service. A missing or locked keyring produces an error; the
application does not fall back to plaintext secrets.
If a fresh desktop account has no keyring yet, create and unlock one using the
desktop's password manager before saving credentials in Download Engine.

Language coverage uses installed system fonts. CJK, Thai and Arabic may need the
corresponding font packages from your distribution. The app does not download
fonts or change your system's font configuration.

Extract the archive into a permanent folder, then run `./de`. To add this copy to
the application menu, run `bash ./install-desktop.sh` as your normal desktop user.
To also select it as the default `.torrent` and magnet handler, run
`bash ./install-desktop.sh --set-defaults`. This changes only that user's handlers.
Keep the extracted folder in place after registration. Removing
`~/.local/share/applications/com.xetro.downloadengine.desktop` removes its menu
entry (use your `XDG_DATA_HOME` instead when configured).
The extraction path may contain spaces and Unicode, but not `=` or control
characters; the registration script rejects those paths before making changes.

The profile is under `$XDG_DATA_HOME/Download Engine`, or
`~/.local/share/Download Engine` by default. Downloads use the current XDG Downloads
folder, with `~/Downloads` as the fallback. The archive uses a normal user profile;
an explicit `DOWNLOAD_ENGINE_HOME` or `portable.json` can select separate storage.

## macOS

Choose `macos-arm64` for Apple Silicon or `macos-x64` for Intel. The package
contains `Download Engine.app`. Keep it in its final location before
connecting browsers. `.torrent` files and magnet links are declared in the app
bundle and enter the same review inbox as pasted links. Finder's **Open With** can
choose the app for torrent files. Dragging local torrent files onto the window also
opens a review.

The profile is in `~/Library/Application Support/Download Engine`; the default
destination is `~/Downloads/Download Engine`. Saved secrets use the user's Keychain.
An exported Windows profile does not make DPAPI-protected secrets readable on a
Mac or Linux account: enter those credentials again.

CLI helpers are inside the app bundle:

```sh
"/Applications/Download Engine.app/Contents/MacOS/de-cli" --help
"/Applications/Download Engine.app/Contents/MacOS/de-cli" list
```

The bundles have an ad-hoc signature for local integrity checks. They are not
Apple Developer ID signed or notarized, so macOS may block the first launch.
If you trust the package, use macOS's per-app **Privacy & Security → Open Anyway**
review after attempting to open it; follow [Apple's opening guidance](https://support.apple.com/en-gb/102445)
and do not disable Gatekeeper globally. Native
tests, package smoke checks and UI rendering ran on both Mac architectures in CI.
Interactive Finder and Keychain permission prompts still need hands-on validation
on a real Mac; CI does not prove those interactions.

## Browser integration

Open **Preferences → Browser & AI → Browser integration**. Install the same
Chrome/Edge/Brave/Firefox extension and connect its ID. Native messaging manifests
go into that browser's per-user directory; no root access is needed. Registration
refuses to overwrite a different application's copy. Move the app first, then
disconnect/reconnect it if the installation path changes.

## Build and validate

Use Rust 1.99.0, a native C/C++ toolchain, make and pkg-config. Packaging also uses
Node.js for dependency notices and Perl's standard JSON::PP module to copy only
files listed in the reviewed source inventory. Linux additionally
needs the OpenSSL and D-Bus development packages. The project's locked Cargo
dependencies must already be available for the offline build commands.

Credential storage uses `keyring-core` with the encrypted D-Bus Secret Service
store on Linux and the user Keychain store on macOS. Keep the existing service
names, account identifiers, and Linux `target=default` attribute when changing
these adapters; saved logins and the protected-data master key depend on them.
A missing or locked store must remain an error, and a failed connection must be
retryable without restarting the application.

macOS TLS tests use the installed `openssl` command to create temporary 30-day
localhost identities. Their keys are removed after loading; tests never change
the system trust store or disable certificate/hostname verification.

The macOS CI job uses a disposable Keychain. Its synthetic storage key explicitly
trusts only the test executables Cargo built in that checkout, because each is a
separate application to Keychain. The fixture key is removed after the suite and
the entire test keychain is removed on job cleanup. This does not change the
application's normal Keychain policy; interactive permission prompts still need
verification on a real Mac desktop.

```sh
bash scripts/Build-UnixFfmpeg.sh "$PWD/artifacts/native-ffmpeg"
export DOWNLOAD_ENGINE_FFMPEG_SHA256="$(awk '/  ffmpeg$/ {print $1}' artifacts/native-ffmpeg/build-record/binaries.sha256)"
export DOWNLOAD_ENGINE_FFMPEG_PATH="$PWD/artifacts/native-ffmpeg/ffmpeg"
cargo test --locked --offline --all-targets -- --test-threads=2
cargo test --locked --offline --manifest-path torrent/Cargo.toml -- --test-threads=2
export DOWNLOAD_ENGINE_NATIVE_FFMPEG="$PWD/artifacts/native-ffmpeg"
bash scripts/Package-Unix.sh "$PWD/dist/native"
```

Run tests in a disposable desktop account/container with an unlocked test keyring,
never a real download profile. The native FFmpeg build uses the repository's pinned
FFmpeg/x264 sources and security patch. Its checksum is compiled into the app;
replacing the helper without rebuilding the matching app is rejected. Packages
include its source, notices, rebuild script, native Rust dependency notices and
all four application executables. Packaging downloads and installs nothing.

On Unix, each merge copies the helper into a private temporary directory and
checks that copy against the compiled checksum. Launches use the open directory
handle, so replacing the original executable or renaming a parent cannot redirect
execution after verification. The temporary filesystem must permit execution;
otherwise merging fails without falling back to the original pathname. Normal
completion, cancellation and failed launches release the private copy.

Native torrent tests cover concurrent first writes, pause/resume and file-path
confinement. Unix storage uses exclusive file creation followed by opening an
existing file when necessary, avoiding a macOS concurrent-creation failure while
retaining symlink and hard-link protections. CI runs this suite before the longer
application build so storage failures are reported early.

Native packages exclude FFmpeg's raw `config.log`, which contains the build
environment. Only the explicitly listed configuration, version, hash and test
records are copied.
Mac dependency notices also include the pinned objc2 workspace license files
omitted from the published crates, with their provenance and original source
archives. Packaging verifies the recorded revisions and notice hashes offline;
Rust implementation files such as `copying.rs` are not treated as license texts.

Helper memory protection follows platform facilities: Windows Job limits, Linux
address-space limits, and a macOS watchdog sampling each owned helper's physical
memory every 25 ms. The macOS guard can allow a brief burst between samples; it is
not an allocation-level limit. Lua additionally retains its allocator limit.
The watchdog stops before the child is reaped so an old PID cannot target a new
process. Helper exit/cancellation also closes its owned process group.

On Linux, `bash scripts/Test-LinuxDesktop.sh` checks desktop-entry parsing and
magnet forwarding using a harmless stub in a temporary directory. It needs the
existing GCC and `gio-unix-2.0` development files and must run as a non-root user.
It does not change default file associations.

`bash scripts/Test-UnixPackaging.sh` verifies that native packages copy only
reviewed source files and reject source symlinks or paths outside the checkout.
It also verifies that environment dumps and unlisted FFmpeg build records are
excluded and that linked build records are rejected.
Its fixtures are temporary and are removed afterward.

Validation evidence and remaining platform checks are recorded in
`artifacts/cross-platform/` during development. A passing Windows build or Linux
test does not substitute for macOS compilation, Keychain checks or Finder testing.

Interactive Linux checks also ran in a disposable Debian 12/Xfce X11 desktop
under WSL2, with a fresh user, software rendering and only local fixture traffic.
They covered native folder/open/save dialogs, Unicode paths, cancellation,
overwrite confirmation and duplicate-safe list import. GNOME Keyring's unlock
and cancel prompts were exercised; an authenticated download retained its saved
progress across an app restart and resumed with the stored credentials.
Firefox ESR's temporary extension connected to the native host, captured one
download without duplicating it, and disconnected/reconnected successfully.
The downloaded fixtures were checked against their expected SHA-256 hashes.
Evidence is in `artifacts/linux-desktop-vm/` during development.

These results cover that X11 desktop and Firefox configuration. Wayland, other
desktop/portal implementations, other Linux browsers and real macOS desktop
prompts still need separate interactive checks. The Firefox test used a temporary
add-on, not a store-signed installation.

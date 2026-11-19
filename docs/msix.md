# Microsoft Store package

Download Engine keeps both Windows distribution formats:

- Inno Setup installer and portable ZIP for GitHub/website downloads.
- MSIX for submission to Microsoft Store. Preparing this package does not publish it.

Both use the same Rust application and interface. The MSIX includes the desktop,
native browser host, CLI, torrent helper, browser extensions, bundled Lua plugins,
FFmpeg and the same source/license notices as the portable payload.

## Build

The public package identity in `installer/msix/identity.json` comes from Partner
Center's **Product management > Product identity**. Keep the name, publisher and
publisher display name exactly as assigned. These are public identifiers, not
credentials. Do not commit signing keys or Partner Center credentials.

Use the existing Windows SDK (MakeAppx) and normal project build prerequisites:

```powershell
./scripts/Build-MSIX.ps1
# Only after the current sources have already been built:
./scripts/Build-MSIX.ps1 -SkipBuild
```

The script reuses the existing clean portable packaging process, strips the
portable profile marker and manual registration script, adds the Store manifest
and correctly sized icons, and runs MakeAppx with validation enabled. It does not
read the stateful `dist/IDM-EX-UnsignedDev` directory or include personal downloads,
profiles or credentials. It installs nothing and does not change certificate trust.

Outputs go to `dist/msix/`:

- `DownloadEngine-<version>-x64-StoreUpload.msix`
- `SHA256SUMS-MSIX-<version>`

The package version is `<major>.<minor>.<patch>.0`; the Store reserves the fourth
component. The current manifest supports Windows 10 build 19041 and newer, x64.
Continue using `scripts/Build-Installer.ps1` for the separate Inno Setup installer.

The manifest declares Microsoft's `Microsoft.VCLibs.140.00.UWPDesktop` framework
(minimum 14.0.30704.0), which supplies the `VCRUNTIME140.dll` imported by our MSVC
binaries. Microsoft Store installs and services this dependency. Local sideload
tests must already have a compatible framework; the build script does not download
or install one.

**The local Store-upload package is unsigned.** An `.msix` extension is not a
signature. Microsoft signs the package after certification; this does not sign
the separate GitHub installer. Do not publish the unsigned upload as a signed,
direct-install download. Local installation tests require an isolated development
environment and appropriate development signing/registration.

## Differences in the Store installation

- Microsoft Store manages application updates. The app's update dialog opens the
  Store update page; automatic GitHub version checks are skipped for packaged apps.
- Windows owns the `.torrent` and `magnet:` registrations. They open the existing
  review flow, and never enable torrents or replace a default app without consent.
- The Windows startup task is disabled initially. Enabling it uses the Windows
  StartupTask API; a choice disabled by the user in Task Manager is respected.
- Native-host manifests are written under
  `%LOCALAPPDATA%/Download Engine/Browser/<package-family>/`, not in WindowsApps.
  Their executable path uses the package-specific `download-engine-host.exe` app
  execution alias, so a Store update does not leave an old version's executable
  path in Chrome/Edge/Brave/Firefox registration.
- Browser connections remain explicit and limited to the reviewed extension ID.
  Another installed/portable copy's connection is not overwritten. Use Browser
  setup to disconnect before switching copies or uninstalling the Store edition.
  External native-host registry entries are shared with browsers and therefore
  may remain after an uninstall; they do not contain cookies or account secrets.
- The bundled `plugins` folder is read-only. Bundled discovery still works; use
  **Install from file** in Extensions for custom packages and review their permissions.
  Installed Lua code and settings remain in the writable application profile.
- Existing installed profiles and download destinations are preserved. MSIX never
  ships `portable.json`, so it cannot put a queue inside its read-only install folder.

## Certification and validation

The manifest requests `runFullTrust` for the native desktop application and
`unvirtualizedResources` for interoperability with browsers and the existing
per-user profile. Windows 11 exclusions are limited to the Download Engine/IDM_EX
AppData folders and the four native-host registry keys; Windows 10 uses the older
whole-AppData/HKCU opt-out. No elevation, drivers, services, firewall rules or
automatic port mappings are requested. Explain these capabilities to the Store
reviewer; their acceptance is a Microsoft certification decision.

Run `scripts/Test-MSIX.ps1` and the focused browser registration, startup, updates
and shell argument tests before packaging. MakeAppx validation is not a substitute
for installation/runtime testing. In an isolated Windows environment, verify:

1. Install, launch, download/pause/resume and tray behavior.
2. Native messaging through each supported browser, including after an MSIX update.
3. File/magnet activation, keeping the torrent consent and input validation intact.
4. Startup enable/disable and Task Manager overrides; it must initially be off.
5. Custom extension import, bundled plugins, media helper access and writable data.
6. Store update UI and uninstall, preserving downloaded files and unrelated copies.

Upload through the matching Partner Center product only after those checks and
the Windows App Certification Kit review. Complete listing, screenshots, privacy
policy, age rating and capability explanations separately. No upload is performed
by the packaging scripts.

References: [manual MSIX packaging](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion),
[Store signing](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options),
[desktop runtime dependencies](https://learn.microsoft.com/en-us/windows/msix/msix-troubleshooting-guide#missing-dependencies),
[flexible virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization),
[desktop integration](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/desktop-to-uwp-extensions).

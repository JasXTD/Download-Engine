# Extensions

An extension is one JSON file holding a manifest and Lua files. It adds site support and automation as a single package, and runs on the same Lua workers as your own scripts. Each job keeps its own worker and its own limits: a package groups jobs but never merges their powers.

Extensions are not the Download Engine browser extension. That is the add-on you load into your browser, and the guides call it the *browser extension*.

Your own scripts stay on the **Scripts** tab and work as they always have. Write a script for something just for you. Make an extension for something you want to share or keep updated.

The app's original code and bundled Lua example are [MIT licensed](../LICENSE).
You can modify and redistribute them while retaining the required notices.
The capabilities and permission checks described here are features of the current
runtime, not additional license conditions. Features beyond the exposed APIs may
also require changes to the app; each third-party extension retains its own license.

From 3.13.0, provider extensions can use [binary HTTP and session-key handoff](lua-media-providers.md#binary-requests-and-session-keys).
The key handoff is temporary and provider-bound; it does not grant network access
to processors or change an extension's declared-host permissions.

From 3.14.0, [authenticated HTTP sessions](lua-media-providers.md#authenticated-http-sessions)
require `permissions.http_sessions`. Adding that permission requires update review.
Media jobs can use [whole-segment callbacks](lua-media-providers.md#whole-segment-callbacks)
to rewrite container bytes and restore initialization on resume.

For a runnable example, see the [offline episode extension](../examples/offline-episode/README.md).
From 3.12.0 it ships in the application's `plugins` folder; its companion bundle also contains an installable JSON, generated encrypted media and
a loopback sample server. It adapts helpers from crunchyroll-downloader under MIT
and demonstrates language/quality selection, protected keys and MKV/MP4 muxing.
This bundled version uses generated offline media; it does not implement Crunchyroll
authentication or Widevine license handling. Build it with
`node scripts/build_offline_episode_demo.cjs` from the source checkout.

## The application's plugins folder

From **3.12.0**, open **Extensions → Installed → From plugins folder**. The app
checks this folder in the background at launch, when opening Installed, and when
you choose **Refresh**. **Open folder** opens the exact location for this copy:

| App layout | Folder |
| --- | --- |
| Windows installer or portable | `plugins` beside `de.exe` |
| Linux archive | `plugins` beside `de` |
| macOS app bundle | `Download Engine.app/Contents/Resources/plugins` |

Place `.de-extension.json` files directly in this folder, then choose **Refresh**.
Unpacked Lua files and nested folders are not scanned. There is **no profile
plugins folder**, and changing the current directory or `DOWNLOAD_ENGINE_HOME`
does not change discovery. A protected install directory may need administrator
permission to add files; **Install from file…** remains available from any folder.
On macOS, use **Install from file…** for your own packages: modifying a signed
`.app` bundle can invalidate its resource signature. The bundled demo is included
before the package is signed.

Choose **Review install** or **Review update** beside a discovered package. The
normal permission review still applies; new installs start disabled. Discovery
never runs Lua, starts a sample server, grants access or silently updates code.
It installs the exact bytes reviewed, even if the source file changes afterward.

Keep one file per extension ID. Duplicate IDs are blocked. A package with the same
version but different contents needs a version increase from its author; older
versions do not replace newer installed versions. Invalid packages show an error
without hiding valid ones. Scans accept up to 64 packages, 1 MiB each, 16 MiB total,
and at most 1,024 directory entries; linked files/folders are refused.

Reviewed installed copies and their enable/permission settings stay in the app's
normal database. Deleting a package file does not uninstall it. Removing an
installed extension makes its source package available for review again, if the
file remains here. Refreshing or reopening never enables it automatically.

The bundled **Offline episode** demo uses `offline-episode.de-extension.json`;
its files and instructions are under `plugins/offline-episode/README.md`.

## Install, update and remove

Open **Extensions** in the top toolbar. Choose **Review install** for a discovered
package, or **Install from file…** to pick a `.de-extension.json` from elsewhere.

- **Install.** The review shows the publisher, what the extension can do and its jobs. Nothing runs yet. It installs turned off, so tick **Enabled** when you are ready. File access is only a request, and stays off unless you tick it.
- **Update.** Open a newer file with the same ID. The review shows what changed. You must approve new access, a new publisher, every added job, and any job that now runs at other times, on other pages or from another file. This includes replacing a job's ID, even when its name and kind stay the same. **Not now** leaves the installed version exactly as it was. Settings, keys, stored values and run history are kept, except a saved value the new version can't use, which the review names. Downloads already running keep the code they started with. Going back to an older version means removing the extension first.
- **Turn off.** Untick **Enabled**. Its running jobs stop and its running downloads pause. Nothing it owns starts until you turn it back on, so a waiting download fails with a message saying so.
- **Remove.** Its settings, keys and stored values are deleted. Run history stays, and files you downloaded are kept. Removing is refused while unfinished downloads still use it.

**Publishers are not verified.** Anyone can make a file with any ID and any publisher name, so install extensions only from people you trust. Review the **Code** tab, which is read-only. **Copy to my scripts** makes an editable copy of one file as a personal script.

From the CLI:

```powershell
.\de-cli.exe extensions list
.\de-cli.exe extensions show example.videohost
.\de-cli.exe extensions review .\videohost.de-extension.json
.\de-cli.exe extensions install .\videohost.de-extension.json
.\de-cli.exe extensions enable example.videohost
.\de-cli.exe extensions set example.videohost quality 720p
Get-Content .\key.txt | .\de-cli.exe extensions set-secret example.videohost content_key
.\de-cli.exe extensions remove example.videohost
```

`extensions install` takes `--allow-files` to grant requested file access, `--queue NAME` to choose the queue for Queue completed jobs, and `--approve` to accept an update that needs approval. An update that adds a job requires `--approve` even if the extension already has that kind of job or is currently turned off. `extensions review` shows what an update changes without saving anything. Scripts cannot run these commands.

## What each job can do

| Job | Sees downloads | Reaches the web | `xd.store` | Changes files | Secrets and keys |
| --- | --- | --- | --- | --- | --- |
| Automation | Names, folders, status and errors. Never address fields. | Only through `xd.add`, declared hosts only | Its own values | Any downloaded file, only if you allow it | None |
| Site reader | No | `xd.fetch` / `xd.fetch_bytes`, declared hosts only | No | No | `secret` settings |
| Provider resolver | No | `xd.fetch` / `xd.fetch_bytes`, declared hosts only | No | No | `secret` settings; can hand off received AES keys |
| Provider processor | Only the bytes of its own download | No | No | No | `aes_key` settings and this run's opaque session-key handles |

Automation jobs can also pause, resume and retry downloads, create, start and pause queues, change a queue's limit, and show notifications. The review lists all of this.

Jobs share only read-only settings, and `xd.store` values stay with the job that wrote them. Automation jobs can read a download's name and error text, so the app replaces a provider processor's error with a fixed message: it sees file bytes, and nothing it writes may reach the web. A resolver's error stays readable, so never put a secret in one.

Rust enforces these limits, not Lua:

- `xd.fetch` and `xd.fetch_bytes` check every request and redirect against the declared hosts and share the same request budget.
- `xd.add` refuses addresses on undeclared hosts. A declared host that redirects elsewhere is outside this limit, because normal downloads follow redirects. These downloads can't reach this computer or your local network, even when a declared name points there.
- A package with a `secret` setting may only return addresses on declared hosts, because the app opens a single returned stream by itself. This includes a download that started before an update added the secret.
- Extensions cannot use `xd.command("automation", …)`, `request` or `cancel`, so they cannot change your routing rules. `xd.restart` is refused too, because it discards partial downloads.

Extensions get no browser cookies, saved logins, `os`, `io`, `debug`, `package` or `load`, and they can run no PowerShell. These are application limits, not an operating system sandbox. Workers run under your Windows account with memory and child-process limits. Only install code you trust.

Built-in site readers always handle their own sites. For other pages, your own scripts go first, then extensions in the order you installed them. The Extensions page says when a job never runs because something else handles the same pages first.

## The package file

Authors work in a folder holding `manifest.json` and `.lua` files, then build the package:

```powershell
.\de-cli.exe extensions pack .\videohost --out .\videohost.de-extension.json
```

Pack refuses links and any file that is not `.lua`, so nothing is left out by accident. It never replaces an existing file.

```json
{
  "format": "download-engine-extension",
  "format_version": 1,
  "id": "example.videohost",
  "name": "VideoHost",
  "version": "1.2.0",
  "publisher": "Example",
  "description": "Adds VideoHost pages to Find videos.",
  "homepage": "https://example.com/videohost",
  "license": "MIT",
  "api_version": 1,
  "provider_api": 1,
  "permissions": {
    "network": ["videohost.example", "*.cdn.videohost.example"]
  },
  "settings": [
    {
      "id": "quality",
      "type": "choice",
      "label": "Preferred quality",
      "options": ["1080p", "720p", "480p"],
      "default": "1080p"
    },
    { "id": "api_token", "type": "secret", "label": "API token" },
    { "id": "content_key", "type": "aes_key", "label": "Content key" }
  ],
  "jobs": [
    {
      "id": "reader",
      "name": "VideoHost pages",
      "entry": "reader.lua",
      "timeout_seconds": 30,
      "trigger": { "kind": "site_reader", "pages": ["videohost.example/watch/*"] }
    },
    {
      "id": "decrypt",
      "name": "Decrypt episodes",
      "entry": "provider.lua",
      "timeout_seconds": 60,
      "trigger": { "kind": "provider" }
    },
    {
      "id": "tidy",
      "name": "Tidy finished files",
      "entry": "tidy.lua",
      "timeout_seconds": 60,
      "change_files": true,
      "trigger": { "kind": "download_completed", "queue": null }
    }
  ],
  "files": {
    "reader.lua": "local parse = require('lib.parse')\nreturn parse.read(xd.fetch(event.url))\n",
    "provider.lua": "return { resolve = resolve, refresh = refresh, transform = transform, finish = finish }\n",
    "tidy.lua": "print('tidy')\n",
    "lib/parse.lua": "local M = {}\nfunction M.read(page) return { items = {} } end\nreturn M\n"
  }
}
```

In a folder, `manifest.json` holds everything except `files`.

### Manifest fields

Unknown fields are refused.

| Field | Required | Rule |
| --- | --- | --- |
| `format` | Yes | Exactly `download-engine-extension`. |
| `format_version` | Yes | Exactly `1`. |
| `id` | Yes | Lower-case words with at least one dot, up to 64 characters. It never changes, because updates find the installed copy by it. |
| `name` | Yes | Up to 64 characters, no control characters. |
| `version` | Yes | `MAJOR.MINOR.PATCH`. An update must be higher. |
| `publisher` | Yes | Up to 64 characters. Shown as not verified. |
| `description` | Yes | Up to 200 characters. |
| `homepage` | No | An `https://` address. |
| `license` | No | Up to 64 characters. |
| `api_version` | Yes | Must equal the app's Lua API version. |
| `provider_api` | Only with a provider job | Must equal the app's provider API version. |
| `permissions.network` | With site reader or provider jobs, or to use `xd.add` | Up to 32 patterns: `host` or `*.host`. `*.host` covers subdomains, not the bare host. No ports, paths or IP addresses. A bare `*` is refused. |
| `permissions.http_sessions` | Optional, default false; 3.14.0+ | Lets site reader/provider jobs use explicitly supplied cookies and headers in exact-origin, run-local HTTP sessions. No browser import or processor access. Adding it requires update approval. |
| `settings` | No | Up to 32 entries. |
| `jobs` | Yes | 1 to 16 entries. |
| `files` | Yes | Up to 64 Lua files. |

### Job fields

| Field | Required | Rule |
| --- | --- | --- |
| `id` | Yes | Lower-case letters, digits and hyphens, up to 32 characters. It stays the same across updates. |
| `name` | Yes | Up to 64 characters. |
| `entry` | Yes | A path in `files`. |
| `timeout_seconds` | Yes | 1 to 3600. Provider callbacks stay capped at 60 seconds. |
| `trigger` | Yes | `manual`, `startup`, `interval`, `download_completed`, `download_failed`, `queue_completed`, `site_reader` or `provider`. |
| `change_files` | No | Automation jobs only. A request the person installing it must grant. It covers every downloaded file, not only those this job added. |

A package cannot name a local queue. `"queue": null` means every queue. For `queue_completed`, write `"queue": ""` and the install review asks which queue to use. Each site reader page's host must be covered by `permissions.network`. A `*.host` page reaches every subdomain, so it needs a `*.host` pattern, not one named subdomain.

### Settings

Every setting has `id`, `type` and `label`. Lua can read settings but never write them.

| `type` | Extra fields | Who can read it |
| --- | --- | --- |
| `string` | `default`, `max_length` | Every job |
| `integer` | `default`, `min`, `max` | Every job |
| `boolean` | `default` | Every job |
| `choice` | `options`, `default` | Every job |
| `secret` | None | Site reader and provider resolver jobs |
| `aes_key` | None | Provider processor jobs, as an opaque handle |
| `binary_secret` | None; import a 1 byte–64 KiB file (3.15.0+) | Site reader and provider resolver jobs, as binary bytes |
| `rsa_private_key` | None; import a PKCS#1/PKCS#8 PEM/DER private key (3.15.0+) | Site reader and provider resolver jobs, as an opaque handle |

Secrets and keys use OS-protected storage and can be replaced or cleared but not shown again. A `secret` reaches Lua as text and a `binary_secret` as bytes, so trusted script code could still disclose them. See [credential import and crypto contracts](lua-provider-core.md#protected-binary-credentials-and-rsa-keys).

### Files and limits

Paths use lower-case letters, digits, `_`, `-` and `/`, and must end in `.lua`. Library files stay in memory. An automation or site reader job's entry file sits in its run folder while it runs, like any script's, but saved settings and keys never do. Binary Lua chunks are refused.

| Limit | Value |
| --- | --- |
| One Lua file | 64 KiB |
| All Lua files | 512 KiB |
| A package with a provider job | 496 KiB of JSON-escaped code, counting every file plus the job's entry file again, because each download keeps its own copy |
| Package file | 1 MiB |
| Lua memory | 64 MiB |
| Worker process memory | 256 MiB |

### Lua additions

Extension jobs use Lua API 1 with these additions. Personal scripts do not have them.

| Addition | What it does |
| --- | --- |
| `require("lib.parse")` | Loads `lib/parse.lua` from this package only, once per run. |
| `xd.setting(id)` | Reads one setting this job may hold. |
| `xd.key_setting(id)` | Provider processor only. Returns an opaque key handle for `xd.aes_ctr` and `xd.aes_blocks`. |
| `xd.binary_setting(id)` | Web jobs only. Reads an explicitly imported binary credential. |
| `xd.rsa_key_setting(id)` | Web jobs only. Returns an opaque imported RSA private-key handle. |
| `xd.extension` | A table with `id`, `version` and `job`. |

Check `xd.supports("xd.setting")` to tell whether the code is running in an extension.

## Compatibility

Provider jobs can also return `xd.media_plan(...)` for segmented media, multiple
tracks and core-controlled MKV/MP4 muxing in Download Engine 3.11.0 or newer.
Check `xd.media_api == 1` in the Lua entry. Protected `aes_key` settings work in
media processors too. See [the media-provider guide](lua-media-providers.md).

From 3.13.0, provider jobs can also use `xd.fetch_bytes` in the resolver and pass
newly received AES keys through `xd.handoff_key` to processor `xd.session_key`.
These APIs also work in personal providers. Detect them with `xd.supports` in
the appropriate phase; they do not change the extension's reviewed permissions.

The first install raises the profile's data version, so older builds refuse that profile rather than misread it. Keep a profile backup before downgrading. Profiles without extensions are unaffected.

Not available yet: online updates, signatures, a public store, buttons or panels added by an extension, browser logins, and replacing a built-in site reader. An extension's own text, such as its description and job names, shows as its author wrote it.

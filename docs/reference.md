# Download Engine detailed reference

For a quick introduction and setup, see the [README](../README.md). This guide
keeps the detailed feature behavior, limits, troubleshooting and verification
notes. Paths and commands are relative to the project root unless noted otherwise.

- [Install and data locations](#install-or-use-the-portable-version)
- [Download controls](#start), [queues and batches](#queues-and-batches)
- [Browser setup](#connect-a-browser), [video downloads](#video-links-in-the-browser)
- [Website grabber](#website-grabber), [proxy settings](#proxy-settings)
- [Security](#security), [diagnostic logs](#local-diagnostic-logs)
- [Build and verify](#build-and-verify), [code layout](#code-layout)


An original Windows download manager built with Rust and a native egui interface.
Browser add-ons use plain JavaScript. There is no TypeScript or Node requirement to run it.

Download Engine's original code is available under the [MIT license](../LICENSE).
Bundled dependencies retain their licenses; see [third-party notices](../THIRD-PARTY-NOTICES.md).

English, Simplified Chinese, Traditional Chinese, Malay, Japanese, Korean,
Vietnamese, Thai, Polish, Russian, Ukrainian, Arabic, German, French, Swedish,
Norwegian Bokmål, Swiss standard German, Italian and Romansh are available in the
desktop app, offline quick guide, Windows installer and all three extensions.
Choose a desktop language in **Preferences → Appearance → Language**. Each extension
has its own selector in **Browser options**. See [Languages](localization.md)
for defaults, font support and translation maintenance.

Download Engine was called XDownloader before 2.0, with programs named
idm-ex.exe, idm-ex-cli.exe and idm-ex-host.exe. The programs are now de.exe,
de-cli.exe and de-host.exe. Upgrades keep existing data and download folders,
saved site sign-ins, the Start with Windows entry and browser connections; these
keep their internal IDM_EX / idm-ex / XDownloader identifiers.

## Install or use the portable version

Public unsigned Windows builds are in **dist\releases\&lt;version&gt;**:
**DownloadEngine-&lt;version&gt;-UnsignedSetup.exe** and
**DownloadEngine-&lt;version&gt;-Unsigned-windows-x64.zip**. These files have no
Authenticode publisher signature; Windows may show an unknown-publisher warning.
See [GitHub release instructions](github-release.md) for packaging, source
and license materials, and SHA-256 checksums. Setup installs per-user in
**%LOCALAPPDATA%\Programs\Download Engine** without administrator access and adds a Start
menu shortcut; a desktop shortcut is optional. Upgrades stay in the existing folder,
including **%LOCALAPPDATA%\Programs\IDM EX** from before 2.0, replace the old
programs and shortcuts, and point Start with Windows and connected browsers at
de.exe and de-host.exe. The final screen can open Browser setup.

The stateful local unsigned development profile uses
**dist\IDM-EX-UnsignedDev\de.exe**. It preserves private test history and
downloads and must not be copied or repackaged for distribution. Clean portable
test builds use **dist\DownloadEngine-&lt;version&gt;-UnsignedDev-windows-x64.zip**; test installers
use **dist\DownloadEngine-&lt;version&gt;-TestSetup.exe**. Signing and browser-store publication
remain separate release steps.

Installed copies keep queue history in **%LOCALAPPDATA%\Download Engine**. Each launch
asks Windows for the current Downloads folder, including a location moved to another
drive; new files use its **Download Engine** subfolder. A profile created before 2.0
keeps **%LOCALAPPDATA%\IDM_EX** and uses the **IDM EX** subfolder of the current
Downloads location until a **Download Engine** data folder exists. The folder is
looked up without creating it; normal startup prepares it and reports errors. An
unavailable lookup is not replaced with a guessed user-profile path. Existing
downloads and partial files keep their recorded locations and progress. Use **Tasks → Exit** (or **Exit** in the tray menu) before an
upgrade or uninstall. Uninstall preserves queue history, partial files, and
downloads, and removes only browser registrations pointing to the removed copy.
Cancelling uninstall leaves those registrations alone. Setup refuses to install
over a portable folder. Portable history is not moved or merged automatically.

## Start

Open **Help → Guide** (or press **F1**) for offline instructions, a topic list beside
the article, and copyable Lua and PowerShell examples. The same [user guide](user-guide.md) ships
in the portable package and installer. It covers downloads and recovery, browser
integration, queues and schedules, automation, CLI usage, settings and diagnostics.

Open **Help → Changelog** for offline release notes, newest first. The same
[changelog](../CHANGELOG.md) ships in both packages. Every delivered update gets a
new version and release note; see the [release workflow](releases.md).

The **Scripting** page in the top toolbar can route new downloads by site or
file type, apply a speed limit, and start another queue after completion. Rules
have enable switches and a text run history. The packaged **de-cli.exe**
provides JSON commands for PowerShell and other scripts, using the same scheduler
as the app. See [CLI and automation](automation.md) for commands and examples.

**Scripting → Scripts** adds a built-in Lua editor with optional PowerShell, Save, Run, Stop, timeouts
and persistent coloured text logs. Choose manual, startup, interval, download
completion or queue completion triggers. The included example uses
Lua's `xd` app API and `event` for trigger context. PowerShell scripts use
`Invoke-DownloadEngine` and `$Event`. Scripts run while the app is open or in the tray; only run code
you trust. Start with **New script → Save script → Run script**, then scroll to
Run logs. Open **Lua guide** above the Lua editor or
**Help → Guide → Lua scripting** for a first-run walkthrough, working download
and completion-trigger examples, the app API and troubleshooting. Each Lua block
has **Copy script**. The [Lua scripting guide](user-guide.md#lua-scripting)
ships offline in both packages. Help → Guide also covers recovery and retention.

**MCP** on the toolbar opens a popup with AI connection controls: Start/Stop, port, View only
or Allow changes, optional startup with the app, and a real connection check.
Use **Copy client config** to connect a compatible local AI client over HTTP
(default `http://127.0.0.1:17842/mcp`). MCP starts disabled with view-only
access. **Setup guide** on the MCP popup (or **Help → Guide → MCP integration**) explains the steps. A terminal server is also
available through `de-cli.exe mcp`; `mcp-config` prints its connection settings
and `mcp --read-only` restricts it to inspection. CLI flags are independent of the desktop settings.
Downloads, queues and saved-script controls use the same scheduler and receipts
as the CLI. Closing the popup keeps your current page and pending edits. The server stays running when the popup closes or clients disconnect. See the
[MCP connection guide](mcp.md) and [AI usage guide](mcp-ai-guide.md).
AI clients can read the latter through `get_usage_guide`.

For the local development build, open **de.exe** inside
**dist\IDM-EX-UnsignedDev** (or in the extracted portable package).
Choose **Add URL**, paste a direct HTTP/HTTPS/FTP/FTPS file URL, select a queue,
and optionally choose a filename and an absolute destination folder. Select
**Start download**, or **Add to queue** when files are being held for later. An
idle, unscheduled Main queue starts automatically. Cancelling a
queue choice in this form leaves the current download view unchanged.
FTP filenames come from the URL unless you supply a name. For HTTP/HTTPS,
leave Filename blank to use a server-provided name when available. New unnamed
downloads read Content-Disposition from the first accepted transfer response,
preferring its Unicode filename* form (UTF-8, with legacy ISO-8859-1 support).
Missing or unusable metadata keeps the URL-derived name. A malformed extended
parameter can fall back to a valid plain filename in the same header; duplicate
parameters, multiple headers, or headers over 8 KiB are ignored.
Server paths are discarded and the final name uses the same Windows filename
sanitization and non-overwriting publication as other downloads. Names you type
and browser-supplied names take priority, even if they match the URL name.

Once a transfer starts, its name stays fixed across retries, pause/resume,
restarts, and expired-link replacement. Existing history keeps its names.
The save folder shown when adding stays fixed too; a later server name can
change the displayed category, but does not move the download to another folder.
The parser follows [Content-Disposition](https://www.rfc-editor.org/rfc/rfc6266)
and [extended filename parameters](https://www.rfc-editor.org/rfc/rfc8187).

The native interface follows the Classic HTML mockup. The toolbar offers **Add URL**,
**Resume**, **Pause**, **Pause all**, **Remove**, **Open folder**, **Clear finished**,
**Scheduler**, and **Preferences**. A file without a resume plan shows **Start again**
instead of Resume. Open folder launches File Explorer directly; there is no extra
confirmation dialog. Remove is disabled during an active transfer.

Press **Ctrl+N** to add a download, or **Ctrl+F** to focus filename search from
any page. While search has focus, **Escape** clears it; a second Escape releases
focus. Shortcuts leave open dialogs and menus alone. **×** and **Escape** close
Add download and keep its URL, name and folder draft; login fields are cleared.

Use **Browse…** beside destination fields to choose a Windows folder. Cancelling
the picker leaves the draft unchanged. Checksum, authentication, and ZIP preview
options are under **Advanced** in Add download. **Use login** reveals username and
password fields directly in the form. Preferences opens a popup with Appearance,
Downloads, Connection, Browser & AI, Antivirus, and Startup & tray categories.
Changes to the popup's controls wait for **Save**; **Cancel**, **×**, or Escape
discards pending changes. Themes preview while editing. **Reset** changes only
the current category's draft. Detailed editors retain their own explicit saves;
closing them returns to the popup. Table-column edits stay in its pending draft.
Dialogs share the same compact frame, icon header, top-right **×**, and action
buttons. Longer forms scroll while their actions remain visible. Dialogs have
no separate Cancel or Close button, except Preferences' explicit Cancel action:
the top-right **×** or Escape dismisses them
without saving. Destructive confirmations use a red action.
Pending saves must finish before their dialog can close.

**Ctrl+click** toggles downloads; **Shift+click** selects a range. With the list
focused, use **↑/↓**, **Home/End**, **Shift** to extend a range, and **Ctrl+A** to
select the visible downloads. **Space** pauses or resumes the selection;
**Delete** removes it. Toolbar Resume and Pause affect eligible selected files.
Remove and Restart ask before discarding saved partial progress; clearing completed
history keeps the finished files. For completed downloads, **File actions → Delete file…**
(or **Delete files…** for a selection) shows the saved paths and asks before permanently
deleting the files and their history entries. Files do not go to the Recycle Bin.
Missing or locked files keep their history entries and report an error; use **Clear
from history** to forget a file that was already moved or deleted. Search and other text fields keep their normal
editing shortcuts. Successful saves use green feedback; errors remain red.

The sidebar filters the selected queue by status and file type, and switches queues.
**Manage queues** shows a card per queue with file counts, progress, Start/Pause,
Schedule and View downloads. The **⋯ → Download at once** submenu sets the queue's
concurrency from 1 to 8; the menu also contains completion
actions, rename, clear-finished and empty-queue deletion controls.
**Tools → Browser setup** and **Preferences → Browser & AI → Browser integration** open the same
browser connection dialog. **Tools** also contains video preview, website grabber,
and folder settings. **Help → About** shows a rounded information card with a purple
download badge, the app name, a prominent blue version heading, **Created by Xetro**, and
**GitHub (coming soon)**. It supports both themes and closes with **×** or Escape.
The version follows the full major.minor.patch build metadata. Set
`GITHUB_URL` in `src/desktop/about.rs` when the repository URL is ready; until then,
the placeholder does not open a browser. Browser connection status is absent from
the sidebar. Sidebar labels can be switched off in **Preferences → Appearance**.

The default table shows file name, size, status with progress, speed, time left when
space permits, and file actions. Existing saved column choices and custom layouts
are retained. **View → Table columns → Reset** applies the new arrangement
after Save. Separate progress, row number, and added-date columns remain available.

Drag a table heading left or right to reorder columns. Drag its right-hand
divider to resize; double-click a divider to restore that column's automatic
width. Release saves the layout; Escape cancels an in-progress drag. Widths and
order survive restarts and are shared across queues. Oversized layouts scroll
horizontally while header and rows stay aligned.

**Preferences → Table columns** also offers visibility, order arrows, numeric
widths, Auto, and **Reset**. Save applies the draft; **×** leaves the
saved layout alone. File name and Actions cannot be hidden. Narrow-window
hiding of row number, Added, and Time left remains responsive and does not erase
their saved positions or widths.

Click a filename or row to select it and open its details. Download rows have no
checkboxes. The details panel shows the source, saved location, resume support,
and errors. Close it with **×** or uncheck **View → Show details** to make room for
the table. Clicking another filename reopens it. **File actions**, the row menu,
and right-clicking a row provide authentication, link recovery, checksums, and other
file operations. **Move to** includes queue destinations and **Move up / Move down**.

Double-click a download, or choose **Download status** in its right-click or
three-dot menu, to follow it in a separate compact window. It shows percentage,
downloaded size, speed, estimated time remaining and the worker's current activity.
Its tabs are **Status**, **Speed limit** and **When done**. Expand **Show details**
for resume support, source, destination, and per-connection progress or saved media
segments. Unknown totals and unavailable estimates remain blank or
marked unknown; media progress counts completed segments.

Manual single downloads (including browser handoffs and reviewed video downloads)
open their status automatically once they start. **Preferences → Downloads → Download
status window** turns automatic opening off. Batch, queue, CLI and automation starts
stay in the background. **Pause** keeps the window open and saved progress; close
the window afterwards to stop following it. **Hide**, Escape and the close button
only hide the window and downloads continue. **Resume / Start immediately** starts this file
without starting other queued files. A completed file offers **Open file** (using
the existing antivirus review when enabled) and **Open folder**. Switching queues,
filters or pages in the main window does not change an open status window.

Filter the selected queue with **All downloads**, **Downloading**, **Unfinished**,
and **Completed**. Unfinished includes downloading, paused, queued, and failed entries.
**View → Status filter** also offers In progress, Queued, and Failed. Filename
search ignores letter case and combines with status and file-type filters. Click an
active file type again, or its **×** chip above the table, to clear that type filter.
**Clear** beside the search box clears only the search. Queue actions apply to the entire selected
queue, including files hidden by filters. **Clear finished** removes completed entries
from that queue's history and keeps their saved files.
Changing a queue or filter returns the list to the top; progress updates keep
your scroll position. An empty search result offers **Clear filters** to restore
all files in that queue. An empty queue offers **Add download**.

**Open file** explicitly asks Windows to open a completed local file using its
normal association and security prompts. **Duplicate** and **Redownload** create
new entries in the selected queue, keeping the original file and history. They
start from zero with a fresh filename collision check; credentials and partial
bytes are never copied. A duplicate of an authenticated item stays paused while
you supply its own credentials.

Download Engine defaults to the charcoal **Dark** theme with mint-green accents,
blue selections, and amber paused progress. Existing saved appearance choices
are preserved. **Preferences → Appearance** contains System/Light/Dark theme,
compact rows, sidebar labels, details, and table columns. Theme and appearance
choices apply after **Save**; choose **System** to follow Windows. Short windows
use compact rows automatically.
**Preferences → Startup & tray** opens window-close and notification options.
The top-right **×** discards unsaved changes; use
**Save** to apply them. These settings persist across restarts.
Browser integration and About
also close with **×**. Escape continues to dismiss these dialogs.
Time left appears only with a known remaining size and measured transfer rate.

The portable package keeps its queue and partial files in **data** and finished
files in **downloads**, beside the executable. Keep both executables and
portable.json together. Without portable.json, development builds use the
installed profile folders above.
DOWNLOAD_ENGINE_HOME (or IDM_EX_HOME from before 2.0) can override these locations for isolated testing.
The queue keeps its SQLite WAL connection alive while the app is open, avoiding
repeated checkpoints during progress polling. Saves retain full synchronization;
the final store handle releases the database at shutdown. For a manual backup,
exit the app, let browser handoffs finish, and copy the whole **data** folder.
Do not copy only queue.sqlite3 while the app is running.

The Main queue starts automatically with a limit of three simultaneous files.
Each queue can run 1–8 files, with a combined maximum of eight across all queues,
and four connections per large file by default, configurable up to 32. Queue history survives restarts.
Large ranged downloads now adapt as connections finish: a free connection helps
with the largest unfinished range. Downloads begin with four ranges at 8 MiB;
smaller files use one. Rebalancing requires at least 4 MiB remaining in a range,
keeps each new piece at least 2 MiB, and never exceeds the configured active connections
or 256 saved pieces per file. Global and per-file speed limits still apply.
Servers that cap total throughput may see little benefit.
Actions on a just-paused or completed file wait for its previous worker to finish
releasing files before they run. This prevents an immediate Resume, Restart, or
Remove from being rejected during the final cleanup step.
Exiting the app pauses active downloads; use Resume when ready.
A desktop started by the extension runs independently of the browser. Native
launches inherit no browser handles, so closing a browser does not leave its
pipes held open by the app.
An already-running copy owns the queue; launching that copy again restores its window.

Resume requires byte-range support and a strong ETag from the server. Otherwise,
the **Start again** action downloads the entire file. If the remote file changes,
use **Restart** to discard the partial copy and fetch the new version.
Saved parts keep their physical identity when split and are assembled in byte
order. Pause/resume and replacement-link checks use the same saved layout.
Existing queues and fixed-range partial downloads remain readable. Once a file
has an adaptive layout, its queue requires this build or newer; older builds
reject that layout. Update the desktop and native host together.

Existing destination files are preserved; repeated names receive a numeric suffix.
Use a local NTFS folder: safe completion currently requires hard-link support.
Allow free disk space for both partial data and the assembled file.

## Optional antivirus scanning

Open **Preferences → Antivirus → Antivirus scan**, choose a scanner, and enable scanning.
It is off by default. When enabled, newly completed downloads are sent to the
selected scanner. With Defender, ClamAV, and compatible command-line scanners,
**Open file** runs a fresh scan, displays the result, and requires a second
explicit Open click after **No threats found**. Errors, detections, incomplete
results, cancellation, and timeouts block that action. Malwarebytes, Kaspersky and Bitdefender use manual
result review, as described below. Opening a folder remains available.

The file menu's **Antivirus scan / result** shows the last report and offers
**Scan again**, including when automatic scanning is off. Completed-file details
show the last scan status. Reports include scanner, time, and a SHA-256 of the
scanned file; they survive restarts, but historical reports never authorize an
Open action. Removing a history entry also removes its scan report.

Available scanners:

- **Microsoft Defender** is located automatically in Windows' installed platform
  directory. It uses a custom file scan with remediation disabled; it does not
  change Windows antivirus settings. Defender must be available and active.
  Its exit code 2 is ambiguous (detection, required action, or error), so Download Engine
  reports **Scan needs attention** and blocks opening.
- **ClamAV** uses your installed **clamscan.exe**. Update signatures with
  FreshClam first. Download Engine requires a confirmed single-file scan and enables
  alerts for encrypted files and exceeded scan limits. It does not remove or
  quarantine files. Such alerts block opening just like detections.
- **Other antivirus (command line)** requires an absolute local scanner .exe,
  one argument per line with **{file}** exactly once, and distinct no-threat and
  detection exit codes from the vendor. Do not put shell quotes around the
  placeholder. You must verify the scan-only command waits for completion and
  reports skipped/incomplete scans as errors. The configuration executes the
  selected program with your user permissions; configure only a trusted AV.
  Shell interpreters, scripts, and known Malwarebytes GUI/Toolset launchers are
  rejected. Other products need their own supported command-line interface.
- **Malwarebytes, Kaspersky and Bitdefender (review in their apps)** request
  scans through the installed consumer product's Windows Explorer action:
  **Scan with Malwarebytes**, **Scan for viruses** (Kaspersky), or **Scan with
  Bitdefender**. Select the corresponding provider and optionally enable
  **Automatically request antivirus scans**. Defender activation is unnecessary.
  These integrations require an English individual Explorer scan command, a
  standard Program Files or Program Files (x86) vendor installation, and a running
  Explorer desktop session. Submenu-only or translated commands that Explorer's
  automation API does not expose are unsupported and fail closed. The helper
  verifies the selected vendor's registered local DLL, matches one exact action,
  and never invokes a default Open action or guesses a shell command.
  Kaspersky Endpoint policy controls, Bitdefender BEST CLI, and Malwarebytes
  Breach Remediation/Toolset are separate products and are not automated here.
  **Check setup** checks configuration and readable installed files only; it does
  not run a scan or prove that the engine/signatures/license are ready.
  **Scan requested — review in antivirus** acknowledges dispatch, not completion,
  acceptance by a busy scanner, or a clean verdict. The antivirus controls its
  normal protection and quarantine behavior. Review the actual report there.
  **Open file** requests a fresh scan, requires **I reviewed this file's result
  in the antivirus app**, then **Open after review**. Results are not imported.
  Historical reports never authorize opening by themselves.

The timeout is configurable from 10 to 3600 seconds (default 300) for the scanner
process. For external consumer integrations it limits only the handoff. Requests are sent one at
a time; the antivirus manages the scans and may require attention when busy.
Cancelling, changing settings, or exiting stops owned scanner/helper work;
an already requested external scan continues in its own app and must be
cancelled there. A bounded helper asks the running Windows Explorer to invoke
the exact selected antivirus action, so the antivirus handoff can outlive the helper.
It never falls back to the downloaded file's default Open action.
Queue auto-exit is cancelled while owned scan work is outstanding. A verified
Open review retains read-only file and parent-folder handles until confirmation, to
  prevent Windows file or directory replacement between scanning and opening.
Network paths and reparse-point folders require a regular local copy for this
scan integration. File hashing is cancellable and runs outside the UI thread.
External handoffs release their initial handles after dispatch so the antivirus
can manage detections. After manual confirmation, Download Engine reacquires protected file
and parent handles, rehashes the current bytes against the request, rechecks app
state, and retains the handles through Open. That verification is cancellable and
runs on the scanner worker rather than the UI thread.

Download Engine does not upload files to an analysis service, change antivirus exclusions,
request elevation, or remove Windows' Internet-origin marker. The chosen AV's
own settings and cloud behavior still apply. **No threats found** is that
scanner's result, not a guarantee that a file is safe. Disabling the option
restores ordinary Windows opening and its security prompts; this setting only
controls opening through Download Engine.

Vendor references: [Microsoft Defender CLI](https://learn.microsoft.com/en-us/defender-endpoint/command-line-arguments-microsoft-defender-antivirus),
[ClamAV scanning](https://docs.clamav.net/manual/Usage/Scanning.html),
[ClamAV flags and exit codes](https://github.com/Cisco-Talos/clamav/blob/main/docs/man/clamscan.1.in),
[Malwarebytes file scans](https://help.malwarebytes.com/hc/en-us/articles/34572984219035-Scan-a-file-or-folder-with-Malwarebytes),
[Kaspersky context-menu scans](https://support.kaspersky.com/help/Kaspersky/Win21.5/en-US/199673.htm),
[Bitdefender consumer guide](https://www.bitdefender.com/content/dam/bitdefender/consumers/case-studies/total-security/EN.pdf),
[Bitdefender consumer/enterprise CLI distinction](https://community.bitdefender.com/en/discussion/90097/command-line-scan-for-my-custom-business-software).

## Categories and automatic save folders

New downloads are sorted into **Documents**, **Programs**, **Compressed**,
**Music**, **Video**, or **Other** folders under the normal download folder.
The same rules apply to individual URLs, batch URLs, browser link actions, and
accepted automatic browser handoffs. Categories use the final extension of the
sanitized filename, ignoring letter case: `.PDF` is Documents, `.tar.gz` is
Compressed, and `.pdf.exe` is Programs. Images, unrecognized extensions, and files
without an extension go to Other. Categories do not identify a file's actual
contents or establish that it is safe.

Open **Preferences → Downloads → Download folders** (also available in the Tools menu) to choose
an absolute local folder for each category. A blank field uses the named subfolder
under the default download folder. Hover a folder for its full default path, or a
category for example file types. Turn off **Automatically sort new
downloads by category** to save new files directly in the normal download folder.
Save applies all folder settings together; **×** discards the draft. These
settings persist across app and native-host restarts. Installed defaults are
resolved again from Windows on each launch; restart the app after moving the Windows
Downloads folder. Portable defaults follow the app's current location. Custom
absolute folders stay where you configured them and may need changing on another PC.

In **Add download** or **Add batch**, leave **Save to** blank to use these rules.
Entering an absolute local folder overrides automatic sorting for that download
or the entire batch. The Add dialog previews the category and destination using
the chosen filename. Folders are created when a download starts, and folder
errors appear on that download; no fallback silently saves it elsewhere.

Existing queued, paused, failed, and completed downloads keep their stored
folders. Changing rules does not move files, rewrite history, or redirect a
resume, retry, restart, or already-prepared browser handoff. Old history can be
filtered by category immediately. Status and category filters combine within
the selected queue; queue controls still apply to the whole queue. Adding a file
in the app or reviewing a notification clears the filters so that file is visible.

## Tray and notifications

The Windows notification-area icon has **Open Download Engine**, **Pause all downloads**,
**Preferences**, and **Exit**. It may be inside Windows' hidden-icons menu.
The tray's Preferences command opens the same popup over the current page.
Choose **Tasks → Hide to tray** to keep downloads running with the window hidden.
In **Preferences → Startup & tray**, enable **Close window to the tray** to make the
window's close button behave the same way. This setting is **off by default**.
If the tray becomes unavailable, the app restores its window and closing exits.

**Exit** always quits, including while hidden: active transfers settle to
Paused, partial files are retained, and pending retry deadlines keep their existing
shutdown behavior. Tray **Pause all downloads** pauses current active, queued,
and retrying files across all queues. It does not disable queues or their schedules;
newly added files can still start. Use **Pause all** to stop queue dispatch.
The tray command also keeps files paused when a queue pause is still settling;
starting that queue or reopening the app cannot undo the individual pause.

Completion and needs-attention notifications are enabled by default, with separate
switches in **Preferences → Startup & tray**. They cover all queues, even while hidden.
Temporary errors that will retry stay quiet, and old history does not notify again on startup.
Several completions are grouped behind the current notification. Clicking reviews
that download (the latest one for a group) in the app; it never executes a file.
If a dialog has unsaved input, selection waits until the dialog closes.

Notifications contain filenames, not source URLs or server error details. Old
banners are retired before the next batch so stale clicks cannot select another
file. Windows notification settings and quiet modes control delivery; accepting a
notification does not prove it appeared. Closing or restarting the app clears its
transient notifications. No taskbar preference or notification policy is changed.
The native integration follows [Windows notification-area APIs](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw).

## Speed limits

Choose **Speed limits** at the bottom of the window. Set a total limit for all
queues, and optionally a separate limit for the selected unfinished file. Enter
KiB/s (1,024 KiB/s = 1 MiB/s); uncheck a limit for unlimited speed. Supported
limits are 1 KiB/s to 1 GiB/s. Choose **Apply limits** to save and apply changes
immediately, including to active transfers.

All files and range connections share the total budget. A file's connections
share its own budget, and both limits apply together. Replacement-link byte
checks also use these budgets. Settings survive pause, retry, restart, and app
relaunch; removing a file removes its individual limit. The table shows a file's
cap beneath its measured speed, and the footer shows the enabled total cap.

Limits pace downloaded file data with a small burst allowance (up to 100 ms of
credit, capped at 1 MiB). HTTP/TLS headers, metadata probes, and operating-system
or HTTP buffering can briefly exceed the displayed payload rate; this is not
system-wide network shaping. Unlimited remains the default.

## Hourly and daily data quotas

Open **Preferences → Downloads → Data quotas** to set either or both allowances.
They are off by default. Enter MiB (1,024 MiB = 1 GiB), then choose **Apply quotas**.
Limits apply across every queue, file, range connection, and replacement-link
verification. They can be changed while downloading; changing or disabling a
limit preserves the usage already recorded in its current window.

The hourly window repeats every hour and the daily window every 24 hours from
the first time quotas are enabled, rather than at local midnight. Settings show
the next local reset time and current usage. Both counters track received file
data while either limit is enabled; no new usage is counted with both disabled.
Counters and window boundaries survive app restarts, and setting the clock back
does not grant another allowance. When both limits are reached, downloads wait
until both permit more data.

At the limit, resumable files retain their partial data and wait in the queue
without consuming a retry attempt. They continue automatically after reset,
subject to the queue's running state, schedule, and concurrency settings.
Manually paused downloads stay paused. A server without safe resume information
requires manual action: raise or disable the quota, then choose **Start again**.
This avoids repeatedly downloading the same prefix each window. A link check
interrupted by a quota keeps the original URL and saved bytes; rerun it later.

Usage counts received file payload, including retransferred bytes after retries
and replacement-link verification. Deleting a download does not refund usage.
Chunks are accounted when their database write is processed. An in-flight chunk
may cross a reset or exceed the allowance; this is not an exact ISP meter.
Browser traffic, website-grabber HTML scans, metadata probes, HTTP/TLS overhead,
and data buffered by the OS or HTTP stack but not delivered to the engine are
excluded. Existing speed limits still apply independently.

## Recover an expired download link

Select a paused or failed file, choose **Update download link**, and paste a fresh
HTTP/HTTPS file URL. **Check link** verifies availability and compares every saved
byte with the replacement. This uses bandwidth for the saved portion and can take
as long as downloading that portion again. You can cancel the check.

If the saved data matches and the server supports verified byte ranges, choose
**Update and resume**. Otherwise, the dialog explains why a restart is needed;
**Update and restart** explicitly discards the partial data. No saved bytes means
**Update and start**. The file keeps its name, folder, download ID, queue, and
position. Confirming wakes an idle, unscheduled Main queue. Other paused or full
queues keep it waiting; choose **Start immediately** from the file's menu to run
it ahead of that queue. Existing completed files are
preserved and filename collisions still receive a suffix.

Until you confirm, checks, failed checks, and cancellation keep the original URL
and saved bytes. A check
holds this download inactive until you confirm or cancel; other downloads can
continue. The app rejects login pages, bad ranges, changing validators, and
unsupported links. A file with pending publication must use Resume to recover
completion before its link can change. Saved credentials are reused only for a
replacement on the same origin (scheme, hostname, and port). A different server
is checked anonymously; accepting its link removes the old saved credentials.
Browser-login snapshots are reused only on their original server; moving to another origin removes the snapshot.

Matching size and ETag alone cannot identify a file at a different URL
([HTTP validator scope](https://www.rfc-editor.org/rfc/rfc9110.html#section-8.8.1)).
After comparing the saved bytes, Download Engine pins the replacement's strong ETag, size,
and final URL for subsequent ranges. This relies on the server honoring its
validator and does not establish file authenticity or malware safety.

## Browser recovery and connection settings

Browser-assisted recovery is also available in **Update download link → Find a fresh link
in the browser**. Open the original webpage and download the same file with the updated
Chrome, Edge, Brave or Firefox extension. It matches the new filename and known size, pauses
the browser copy, and asks the desktop to check saved bytes. Only confirming the
desktop result updates the existing job and permits cancellation of the browser copy.
Cancellation, lost app access or expiry releases it back to the browser. Recovery does
not import new cookies, and needs the optional downloads permission; automatic capture
can remain off. If matching cannot identify the file, paste its fresh URL manually.
See **Help → Guide → Downloads & recovery** for the full walkthrough.

**Preferences → Connection → Download connections** saves a default and up to 100 hostname rules.
Each file can use 1–32 HTTP range workers; the UI offers 1, 2, 4, 8, 16 and 32.
Exact/more-specific rules override broader subdomain rules. For redirects, the lower
matching source/final-host limit wins; the default applies when neither host matches.
Limits are per file, not aggregate per server. Active files adopt changes after
pause/resume, including existing multi-part plans. Small and non-range files use one
connection. FTP, HLS and DASH retain their existing transfer behavior.

## Downloads that require authentication

Download Engine supports **HTTP Basic** username/password credentials and **bearer tokens**
for direct file URLs. In **Add download → Advanced**, check **Use login** and enter
the username and password in the fields that appear. HTTP/HTTPS also offers a
**Bearer token** option. Choose **Start download**, or **Add to queue** for a paused
queue. The file is created paused while Windows saves the login, then queued only
after that succeeds. A failed save stays in the form; **Retry login** reuses the
same paused entry. Cancelling before submission creates no download; cancelling
after a failed save leaves that entry paused. Cancelling, changing login method,
unchecking Use login, or editing the URL clears the entered credentials.

For an existing file, pause it or select a failed download, then choose
**Authentication** in its details. Select **Username / password**, **Bearer
token**, **NTLM**, or **Negotiate (Kerberos / NTLM)**, enter the values (without
the `Bearer ` prefix for tokens), and choose **Save and
continue**. Clear **Continue the download after saving** to leave it paused.
Continuation still respects the queue's schedule and concurrency limit. HTTP 401
and 403 errors include guidance for correcting credentials or an expired link.

Changing or removing credentials when partial data exists requires explicit
consent to discard that data and restart. This prevents mixing downloads from
different accounts. The filename, folder, download ID, and queue position stay
unchanged. The **None** method removes saved credentials. Credentials cannot be
changed during a transfer, link verification, or pending file publication.

Credentials are stored per download in **Windows Credential Manager**, under
opaque `IDM EX/Downloads/v1/` entries. The queue records only the entry ID, method,
and server origin. Credentials survive pauses and restarts on the same Windows
account and computer; they are not part of a portable ZIP or queue backup. If you
move the queue to another account or PC, enter credentials again. A different
download on the same server never inherits them.

Remote authenticated requests require HTTPS with normal certificate validation.
Plain HTTP is accepted only for literal loopback IP addresses. Credentials are
attached only to the configured origin; redirects to any other origin, including
a different port, are blocked. Use a direct URL for the intended server.

Completion, removal from the queue, switching to **None**, or accepting a link on
a different origin removes the obsolete credential entry. An opaque-ID cleanup
journal retries interrupted deletion on the next app start. Uninstall preserves
the queue and its credentials for later reinstallation: remove the downloads or
choose **None** first if you want their credentials removed. Older builds do not
understand credential bindings; remove authentication before downgrading.

For company servers, NTLM and Negotiate accept `DOMAIN\username` or `user@domain`
and a password. Negotiate uses Kerberos when available and may fall back to NTLM.
Only the explicit account is used; automatic Windows sign-in is disabled. These
methods require Windows and Direct connection or a custom
HTTP proxy. A custom HTTPS/SOCKS proxy cannot be combined with enterprise server
authentication. Kerberos domain availability and service names must be validated
in the organization's environment. Local fixtures do not prove domain sign-in.

This does not implement website sign-in forms, interactive OAuth login,
or Digest. Proxy authentication is configured separately in Preferences.
Automatic browser capture remains
anonymous and falls back to the browser for protected files; manual desktop
authentication does not grant the extension access to passwords or tokens.

## Queues and batches

Choose **New queue**, enter a name, then select it in the queue picker. New queues
wait for **Start** or a schedule. Single-link and automatic browser handoffs go
to Main queue; page-link batches use the queue chosen in their picker. An idle,
unscheduled Main queue starts when a new file is added or explicitly resumed.
Completed history and individually paused files do not block that automatic start;
custom queues, schedules and queues with existing waiting work retain their policy.
The tabs and table show the selected
queue; completed files stay in its history.

Use a download's right-click or three-dot menu → **Start immediately** to run that
existing file ahead of its queue without starting other waiting files. Saved bytes,
filename, destination and queue position are preserved. The override bypasses its
queue's pause, schedule and usual file limit, while retaining the global eight-file
cap and data quotas. Pause, Pause all, queue pause and app restart cancel it.

Use **Add batch** to paste one HTTP/HTTPS/FTP/FTPS URL per line (up to 1,000), select the
destination folder, and add the entire batch. Blank lines and duplicate links
within that batch are skipped. An invalid line prevents the whole batch from
being added and leaves the draft available to correct.

In **Manage queues**, **Download at once** controls simultaneous files in the selected queue.
Reducing it pauses excess transfers and returns them to the waiting list.
**Pause** stops new work and pauses its active files; **Start**
continues them in order. Files paused individually stay paused until you choose
Resume or Start again. **Pause all** clears one-time starts and suspends recurring schedules.

Choose a file, then **Move to** to reorder it or move it to another queue. Pause
an active file first and let cancellation finish. Order and membership are saved
separately from transfer progress. **More** lets you rename a queue or delete an
empty queue; the Main queue is kept.

**Schedule** offers **One time**, **Daily**, or **Selected days**:

- **One time** sets a future local start date/time, pausing the queue until then.
  If the app is closed at that time, the queue starts when reopened. Ambiguous or
  nonexistent daylight-saving times are rejected. Manual Start/Pause clears it.
- **Daily / Selected days** sets a start and pause time in the computer's local
  time zone. The days refer to the **start**: Mon 23:00–07:00 runs until Tuesday
  morning. Equal times are rejected. Saving/enabling inside a window starts it
  immediately; outside a window the queue waits. Missed windows are skipped.
- At a window's end, active downloads pause safely and keep their partial files.
  They return to the waiting list for the next window. Individually paused files
  stay paused. Recurring transfers also return to the waiting list after an app
  exit or interruption; they run again only during an enabled window.
- Manual **Start/Pause** suspends recurrence and retains the saved settings.
  **Enable schedule** (Scheduler or the queue's More menu) restores it.
  **Schedule** opens the saved settings for editing. **Clear schedule** removes
  either schedule type and keeps the current running/paused state.
- Schedules survive restarts. Keep Download Engine open (the tray is fine) and the PC awake
  for on-time transitions. On reopening or waking, it checks the current window
  instead of replaying missed starts. A recurring window with a nonexistent
  daylight-saving boundary is skipped; repeated times use the first occurrence
  for both start and pause. Schedules follow changes to the computer's time zone.

Waking the computer is not implemented. Optional shutdown after queue completion is available through the queue menu. In ordinary queues,
recurring windows control when unfinished transfers may run. Servers must support safe resume to reuse partial bytes;
non-resumable transfers restart when their queue next runs.

Existing histories migrate into Main queue in their original oldest-first order.
The updated database should be used with this version; older executables do not
understand all queue/schedule rules and could start files outside their window.

### Synchronization queues

Choose **New queue → Synchronization queue**, then add URLs or move existing
completed downloads into it. **Check now** downloads new entries and checks saved
files once. **Schedule** uses the same one-time, daily, and selected-day windows;
each recurring window starts one check, including after an app restart in that
window. The app must be open and the PC awake. Manual **Check now / Pause**
suspends the schedule; **Enable schedule** restores it.

- **Up to date** means a strong ETag and the saved SHA-256 still match, or a fresh
  download has the same bytes as the local copy. Weak ETags and modification
  dates alone are insufficient; those servers require downloading the file to
  compare it. Missing or edited local copies are downloaded again safely.
- **Updated** means the new file finished downloading and passed its integrity
  checks. It is published as a separate file, such as `file (1).zip`; previous
  copies are never overwritten or automatically removed. Plan for disk space
  for both versions. The row points to the newest copy. Its **⋮** menu shows the
  last check and the previous copy's folder.
- **Check failed** keeps the previous copy. Open the row for the error; existing
  retry, pause/resume, authentication, and link-recovery controls apply. A server
  changing again during a partial replacement can require **Restart**. Files
  paused individually remain paused until resumed individually.
- Browser size/ETag metadata may change for a new version, but its file-type
  guard is kept. User-entered expected SHA-256 checksums remain enforced. Saved
  login credentials still expire/are retired on completion; synchronization
  does not retain or silently renew browser sessions. Protected or expiring
  links may need fresh authentication or a replacement link.
- Updated copies use the normal optional antivirus scan and Open-file review.
  Unchanged checks do not repeat completion notifications or an existing scan.
  Malwarebytes results still require review in Malwarebytes. Clearing a row
  removes it from future checks and keeps its saved copies.

## Connect a browser

The extensions add **Download with Download Engine** to the right-click menu for file links,
and provide a popup for pasting URLs. The popup also has an **Automatic capture**
switch, off by default. Connect the desktop app first, then turn the switch on
and allow the browser's download permission. Reload an already-loaded extension
after updating this package.

**Capture rules**, under the switch, choose which downloads are captured: file types
(extensions separated by spaces; `*` matches any characters, so `r0*` covers split
archives and `*` alone captures every type), an optional minimum size, and sites
never captured from. A plain site name such as `example.com` also covers its
subdomains; `*.example.com` covers only subdomains. Sites are compared with the
download address, its final address after redirects and the page that started it.
The type comes from the browser's chosen filename, or from the address until the
browser reports one. Downloads outside the rules, or with an unknown size, stay in
the browser untouched. New installs start with common archives, installers, disk
images, audio, video and PDF; extensions that had capture on before 1.15.0 keep
capturing every type. **Download with Download Engine** and **Add a file URL** ignore
these rules.

When a new direct file download matches the rules, the extension briefly pauses it. Download Engine
checks that it can fetch the same file without browser cookies, starts the desktop
app, and saves the job in Main queue before the extension cancels the browser copy. Failed checks
return the download to the browser. Turning the switch off stops new captures;
files already queued in Download Engine continue there.
Capture setting changes are applied in order. Revoking download access during a
desktop connection check leaves capture off, even if access is granted again.

A recovery journal and a one-minute browser alarm resolve interrupted handoffs.
The popup's **Recover handoffs** button retries immediately. If the browser refuses
a resume request, the popup reports it; use Resume in the browser Downloads list.
The extension never deletes a downloaded file or clears browser history. If all
native replies are lost, it keeps the browser copy and warns that a second copy
may have reached Download Engine.

Capture is conservative: it skips private downloads, browser-flagged files,
downloads started by other extensions, HTML pages, generated blob/data files,
and files whose size or type remains unknown. When a browser has not yet supplied
that metadata, the extension rechecks for up to ten seconds while the browser
continues downloading. Firefox's paused/interrupted state is handled explicitly
for capture, fallback, and recovery. A file that finishes before takeover stays
in the browser. Login-only and POST downloads are not supported: a separate
unauthenticated GET must match the browser's size and type. This metadata check
cannot prove byte-for-byte identity; when available, a strong ETag is also checked
when Download Engine starts the transfer.

### Download links from the current page

In Chrome, Edge, or Firefox, open a normal HTTP/HTTPS page, click the Download Engine
extension, and choose **Choose files from this page**. **Download links from this
page** is also available when right-clicking the page background. A separate tab lets you:

1. Search filenames, link text, or URLs; filter by type, source, or reported size;
   and sort by name, size, type, or page order. Unknown sizes are shown as **—**
   and excluded by a minimum-size filter. The picker does not fetch files to measure them.
2. Select individual links or **Select matches**; nothing is preselected.
   Selections remain selected across pages and when hidden by filters, and the
   count says so. **Select matches** selects every match across the 100-row pages;
   **Shift+click** selects a range in the current filtered order.
3. Choose a saved **Queue** and **Save to** destination, then **Add selected files**.
   **Automatic folders** uses the app's category rules; **Default downloads** uses
   its root folder. Named destinations use the folders configured in Preferences.
   Use **Reload destinations** after changing queues or folders in the app.

The scan runs only when requested. It reads top-page links and assets, including
images/srcset, common lazy-image attributes, audio/video sources, posters,
background images on the first 500 elements, and open shadow roots. It resolves
relative URLs, removes fragments and duplicates, and rejects embedded credentials
and non-HTTP(S) links. Traversal and candidate counts are bounded at 10,000 each;
up to 1,000 unique links fit within a byte limit. A notice appears when truncated.
It does not crawl pages, inspect frames or closed shadow roots, fetch previews,
or transfer cookies. Private windows and browser-protected pages are unsupported.
Types use filenames and available media MIME metadata, not downloaded contents;
ordinary page links can also appear. HLS/DASH entries open a desktop preview.
A direct-file selection is an anonymous GET, so login-only and POST links can fail.

The native host validates and saves the whole batch atomically. A lost reply
keeps its original request ID: **Retry same batch** checks that receipt without
adding the files twice, even if their history was removed. A closed pending
picker can be reopened with the popup's **Recover handoffs** button. Once saved,
the selected queue controls when its files run; the extension cannot auto-open
or execute them.

Unsent scans and pending selections are held in memory for this browser session,
including across popup closure or a background-worker restart. Closing an unsent
or completed picker clears its scan. Restarting the browser or reloading/disabling
the extension clears session data, so resolve uncertain batches first, or check
the desktop queue before collecting them again. Saved desktop jobs persist.
Browser batch receipts keep IDs and a digest of the request, not another copy of
its URLs or filenames.

### Chrome, Edge and Brave

1. In Download Engine, open **Browser setup** and choose **Chrome**, **Edge** or **Brave**.
2. Choose **Copy extensions page**, paste it into that browser's address bar,
   and enable Developer mode.
3. Choose **Open extension folder** in Download Engine. On the browser page, use
   **Load unpacked** and select that folder.
4. Choose **Connect browser**. Download Engine fills in the 32-letter ID the browser gives
   that folder: Chromium derives an unpacked extension's ID from its folder path
   (SHA-256 of the path, first 16 bytes as the letters a–p), so the app computes it
   for its own `extensions` folders. If you loaded the extension from another folder,
   paste the ID shown on the browser's extensions page instead. A warning appears
   when the browser is connected to a different ID than this copy's folder.
5. In the extension popup, choose **Check connection**, then send a file link.

These unpacked extensions can only be used in Developer mode. Chrome, Edge and
Brave each have their own extension folder, so their extension IDs differ. Registration is scoped to the exact ID
you enter, under the current user's registry, with no admin access. The panel
shows **Registered for this copy** after saving the host mapping; only the
extension's Check connection verifies a live browser connection. Choose
**Disconnect** to remove this copy's mapping. A mapping owned by another Download Engine
copy is preserved: disconnect it in that copy before connecting a new one.
Moving a portable package requires disconnecting the old location first.

On Windows, Brave has no native-host registry key of its own. Like other
Chromium-based browsers without Google branding, it checks
`HKCU\Software\Chromium\NativeMessagingHosts` before Chrome's key, so Download Engine
registers Brave there with its own **native-brave.json**. Brave's connection is
therefore separate from Chrome's. Other browsers that read the Chromium key, such
as Chromium itself, find this registration first.

### Firefox

1. In Download Engine, open **Browser setup** and choose **Firefox**.
2. Copy the shown browser page into Firefox's address bar.
3. Choose **Load Temporary Add-on** and select **manifest.json** from the
   extension folder available through **Open extension folder**.
4. Choose **Connect browser** in Download Engine, then **Check connection** in the popup.

The Firefox ID is already included; there is no ID to paste. Firefox removes this
unsigned development add-on when it restarts. A permanent release needs Mozilla
signing. The Chrome/Edge development folders are not store installations either.
There is no browser-policy modification or signing bypass in the installer.

If automatic capture switches back off with **No such native application
com.xetro.downloadengine**, the extension is loaded but the desktop connector is not
registered. In the desktop app, choose **Tools → Browser setup → Firefox →
Connect browser**, then reopen the extension and try **Browser options → Check
connection**. Enable automatic capture again after it connects. Loading the
temporary add-on alone does not register the desktop connector.

When a checkbox asks for optional permission, accepting completes that setting
in the background even if Firefox closes the extension popup. Download Engine then
returns to the popup in the same focused window and active tab. Video-button
setup returns to the expanded **Browser options** section. Denying permission
leaves the feature off. Granted permissions are reused on later visits.
Automatic return requires Firefox 149 or newer (or Chrome/Edge 128 or newer);
older Firefox versions still save the setting but may require reopening the popup.

Turning either feature off preserves its browser permission. Turning it back on
reuses that access and checks it again in the background, without asking for it
again. If access was revoked in browser settings, enable the feature again to
request it. Connection errors and rejected changes keep the checkbox off.

For troubleshooting, open **Diagnostic log** directly on the extension popup,
below **Browser options**. You do not need to expand Browser options to find it.
It keeps the last 30 setting, connection-check and video-review results plus the latest popup error locally, across
popup closings. **Select log**, then **Ctrl+C**, copies the selected text; **Clear
log** removes these records. Action names and bounded error messages are logged;
web addresses and Windows paths in those messages are redacted. Nothing is sent
automatically.

**Check connection** displays its progress and result directly below the button.
Video-button failures appear in the popup status and diagnostic log, so the
error remains available after closing the page or popup.

### Optional scripted registration

The packaged **Register-Browsers.ps1** remains available for automation. It calls
the same validated Rust registration code used by the desktop. Examples:

    ./Register-Browsers.ps1 -ChromeId YOUR_32_LETTER_EXTENSION_ID
    ./Register-Browsers.ps1 -EdgeId YOUR_32_LETTER_EXTENSION_ID
    ./Register-Browsers.ps1 -BraveId YOUR_32_LETTER_EXTENSION_ID
    ./Register-Browsers.ps1 -Firefox

Add **-WhatIf** to preview or **-Unregister** to remove mappings owned by this
copy. Run the packaged script beside the executables. The script is optional.

## Checksums, queue completion and desktop controls

**Add download** accepts an optional expected SHA-256 (64 hexadecimal characters).
Download Engine calculates SHA-256 for every new download during file assembly. If an
expected value does not match, the download needs attention and the assembled
file is not published. Restart and replacement-link recovery retain the expected
checksum; duplicating a download copies the expectation, never the old result.
Completed files offer **row menu → SHA-256 checksum** to calculate or recheck a
saved file, enter a publisher checksum, and **Copy SHA-256**. This also works for
files downloaded by older versions. Manual mismatches preserve the saved file
and history, but the app's Open file action is blocked until a successful recheck
or an explicit calculation without an expected value. Results describe the bytes
when checked; integrity verification is not malware detection.

Use **queue More → When queue finishes → Notify / Exit / Shut down computer**. Actions start
**Off**, are one-shot, and apply only to this app session. They wait for every
remaining file to complete successfully; paused, failed, retrying or mismatched
files prevent completion. Removing or moving a watched file cancels the action.
Windows notifications follow the system's notification settings. Exit shows a
15-second countdown with **Cancel exit**, and cancels if any other download is
unfinished or a dialog is open. It exits only Download Engine, never the PC.

Shutdown uses a **60-second Cancel shutdown countdown inside the app**. It cancels
if another download, antivirus scan, script, control command or scheduled website
scan is unfinished, or a dialog is open. No OS power event is scheduled during
the countdown. At its end Windows is asked to shut down normally, with no forced
application termination; unsaved work can block shutdown. This release's shutdown
action was reviewed and compiled but deliberately not executed, at the user's request.

Drag an HTTP/HTTPS/FTP/FTPS link (or lines of URL text) into the Windows
app to open **Add batch** for review. Nothing downloads until submitted. Drops
received during another dialog wait for it to close; the current draft is kept.
The native OLE handler accepts Unicode URL text, rejects embedded credentials and
non-web schemes, and does not open or execute dropped files. Its bounded queue
rejects additional drops while full. Existing queued work resumes normally after
an app restart; completion actions must be enabled again.

The optional **floating drop box** (View → Floating drop box or the tray menu; off
by default) is a 64 px always-on-top, no-activate tool window with its own OLE
registration on a dedicated STA thread. It accepts the same drops while the main
window is minimized or in the tray: one link opens **Add download** filled in,
several open **Add batch**, and the main window comes forward. Its on/off state
and position (physical screen pixels) are saved in the interface preferences; the
position is kept fully on a monitor's work area and follows per-monitor DPI.

**Clipboard links** (Preferences → Integration & security, on by default) offers
HTTP/HTTPS file links you copy in other apps. The copied text must be exactly
one link whose last path part ends in an archive, installer, video, audio or
document extension (for example ZIP, ISO, EXE, MSI, MP4, MP3, PDF or DOCX).
Web pages, plain text, scripts, images, playlists, links with a query-only file
name, links with credentials and paragraphs containing a link are ignored. A
Windows notification from the tray icon and a card above the status bar show
the newest link; Download… or the notification opens **Add download** with the
link filled in, and nothing starts until you add it. While another dialog is
open, the card waits for it to close.

The app listens through a Windows clipboard-format listener (no polling) only
while the setting is on, and removes it when you turn the setting off or exit.
It reads Unicode text up to 8,192 characters, never logs or saves it, and
skips copies that password managers mark as private. Copies made while
Download Engine is the active app (Copy link, Copy SHA-256, text copied from its
fields) and a copy identical to the previous clipboard text are ignored.
Like download notifications, the notification's file name and host stay in the
Windows notification center until cleared.

Use **Preferences → Table columns** to show or hide row number, status, progress,
size, speed, time left and added date. Save applies changes; **×** preserves the
previous choices, and Reset restores visibility, order and widths. File name
and Actions remain visible. Narrow windows still hide extra columns to fit.

## Video links in the browser

Click **Find videos on this page** in the extension, or use its page/video
context-menu action. The scan collects recognizable direct video URLs (such as
MP4/WebM) from top-frame video elements, their source elements, and video links.
Known video MIME types allow sources with opaque URLs. Download Engine opens or comes
to the front with a native **Video downloads** dialog; no browser picker tab opens.
Nothing is preselected. Choose direct files, a queue and a saved destination, then
click **Add selected files**. HLS / DASH entries have **Choose quality** instead of
a checkbox. This opens the desktop quality picker and reads the available
qualities; downloading still requires **Add video to queue**. Close either dialog
with its top-right **×** to cancel. An empty scan explains why nothing can be added.
The ordinary **Choose files from this page** action keeps its browser file picker.

This is an explicit, local page scan with temporary activeTab access. It recognizes
`.m3u8` / `.mpd` links and HLS / DASH MIME types on video sources, plus manifest
URLs in the document's resource-timing entries (useful for some blob/MSE players).
Blob URLs themselves and players with attached DRM keys are excluded. It does not inspect iframes,
transfer cookies or bypass access controls. The desktop handles video previews
and segment downloads after you review them.
Open the actual video page and scan again after a source appears. Login-only
sources can fail because the desktop performs its own GET request.

For media requested during playback, turn on **Detect media while browsing** in
the popup and grant optional **webRequest** and HTTP/HTTPS website access. It is
off by default. Start playback or reload the page after enabling it. The popup
shows the active tab's detected audio, video, HLS and DASH entries; its toolbar
badge shows their count. Click an entry to review it in Download Engine before any download. **Find
videos** also includes these detected items in its review.

Detection observes successful GET response metadata (URL, MIME type and reported
size), including opaque media URLs. It does not read request headers, cookies or
response bodies, make preview requests, or queue downloads. Obvious stream
segments are excluded. URLs may still contain signed access tokens. Lists stay
in browser session storage, capped at 40 items per tab and 16 tabs; entries older
than 30 minutes are pruned when the detector next runs. Navigation, tab closure,
disabling detection and permission revocation clear the relevant records. **Clear**
empties the current list; later playback requests can add fresh entries. Turning
off detection preserves granted access shared by other features; revoke access
in browser extension settings if desired. Private windows are excluded.

Detection does not make DRM, live/encrypted streams, POST requests or login-only
media supported. The desktop's existing preview checks still apply. The separate
browser-login review remains the only path for explicitly approved cookie transfer.
YouTube, Bilibili (including Bangumi episodes), TikTok and Twitch video page links use
the desktop's built-in readers, even if browser media detection finds zero files.
**Download video** or **Find videos on this page** opens quality selection in
Download Engine. If a built-in reader cannot read the page (for example a region-locked
or members-only video), the review shows what the extension found on that page
instead, with the reader's error at the top; turning on **Detect media while
browsing** and starting the video first gives it the most to show. Pages covered by
one of your own site readers (a Lua script with the **Site reader** trigger, see
[the automation reference](automation.md#site-readers)) open with that reader the same
way. Other sites still need exposed direct files or supported playlists. Automatic capture handles browser file downloads;
it does not convert a streaming player into a downloadable file. Video buttons
are on by default wherever website access has already been granted, including
access granted for **Detect media while browsing**. On YouTube, the button
appears only beside the main player on video pages (watch, Shorts, embeds and
live), never on channel banners or hover previews; on Bilibili, only beside the
main player of a `/video/` or `/bangumi/play/` page, never on card previews; on
Twitch, only beside the player of a `/videos/…` page, never on live channels or
previews; on TikTok, only beside the video a `/@user/video/…` page is about, not
the next video it preloads.
Manual URL entry and browser connection/recovery controls are in the popup's
collapsible **Add a file URL** and **Browser options** sections.

**Browser options → Video button on this site** is always available on regular
web pages in Firefox and Chromium. It starts checked and remembers an explicit
off choice for that site's HTTP or HTTPS host, covering its ports. If access is
missing, **Allow this site** requests that optional permission after a click;
opening the popup never requests access. Existing pages and future visits with
access show buttons automatically unless disabled for that site. Buttons appear just above
visible top-frame video players and open the same desktop review; clicking a button
never queues a file. At the viewport edge they move below or beside the player.
If no outside space is available (including a full-screen player), the button
hides; the extension's **Find videos on this page** remains available. Buttons
avoid covering neighboring players. The setting persists across reloads and restarts.
Video buttons and media detection are independent: toggling detection does not
change a saved video-button choice. Explicit off choices survive permission
revocation and re-granting, as well as reloads and extension restarts.
Disabling does not revoke a site permission shared with browser-login support;
site permissions can also be removed in the browser's extension settings.
No cookie permission is requested for buttons. They are unavailable in private
windows, browser-internal pages and embedded frames. The button reads player
layout and reacts to DOM changes locally; URLs are collected only on a click.
Navigation within the same document (including YouTube watch-page changes) keeps
working: where supported, the browser's document identity binds the collector
result to the clicked page. A replacement document is rejected before handoff.
Closed shadow UI, trusted click checks, sender/frame checks and an
extension-controlled handoff keep page scripts from sending direct native download commands.

Files and streams added from a browser page, and downloads the extension hands
over, send that page's origin (for example `https://example.com/`) as the `Referer`,
as browsers do by default. The full page address, cookies and other browser data
are never sent, and an HTTPS page sends nothing to a plain-HTTP server. Some video
servers refuse requests without it.

Video handoffs save an expiring review, not a download. If the desktop reply is
lost, use **Browser options → Recover handoffs** to retry the same review ID.
Pending reviews survive an app restart and expire after 30 minutes. A dismissed
or handled review is not reopened by a retry; scan again to start another review.
Update both the desktop app and extension when using this handoff.

## YouTube video downloads

Use the extension's **Download video** button or **Find videos on this page**,
or paste a YouTube video link into **Add URL** / **Tools → Preview video**.
Choose a quality and audio track, preview the selection, then **Add video to queue**.
Loading qualities does not start a download. The filename comes from the video title.

This native Rust implementation ports yt-dlp's anonymous VISIONOS/HLS flow; it
does not run Python or a yt-dlp executable. Public, finished videos with a supported
HLS response are supported. Availability and qualities depend on YouTube's response;
this is not full yt-dlp feature parity. JavaScript challenges, proof-of-origin tokens,
sign-in/private/age-restricted videos, DRM, live recording, playlists, subtitles,
and direct adaptive-format extraction are not implemented. Unsupported responses
show an error instead of downloading the watch page or silently changing quality.

The download saves the video ID and selected video/audio format identifiers.
Starting or resuming refreshes expiring links, retaining partial segments only
when encoding versions and the segment layout still match. Packed AAC audio is
validated, its transport timestamps checked, and merged with the selected video
into MP4 using the existing bundled merger. Existing HLS/DASH jobs remain compatible;
new YouTube jobs require this updated application. Diagnostics record lookup
success/failure and timing without storing titles, URLs, visitor data or signed links.

See [the implementation and source map](youtube-port.md) for upstream references,
limits, and repeatable verification commands.

## Bilibili video downloads

Use the extension's **Download video** button on a Bilibili video or Bangumi episode
page, or paste a link such as `https://www.bilibili.com/video/BV…` (also `av…` numbers
and `?p=` for one part of a multi-part video) or
`https://www.bilibili.com/bangumi/play/ep…` into **Add URL** / **Tools → Preview
video**. Choose a quality and audio track, preview, then **Add video to queue**. The
filename comes from the video title, with the part number and name for multi-part
videos, or from the series title and the episode's name.

The app reads Bilibili's public web API the way its own player does: anonymous
visitor IDs from `/x/frontend/finger/spi`, then WBI-signed `/x/web-interface/wbi/view`
and `/x/player/wbi/playurl` requests (an MD5 signature over the sorted query with
keys Bilibili publishes at `/x/web-interface/nav`). The reply lists separate MP4
video and audio streams with their initialization and SIDX index ranges; they become
an indexed DASH presentation and use the existing segment downloader and bundled
merger. Streams from peer-assisted mirrors (PCDN) are avoided when Bilibili offers
its own CDN. Requests to Bilibili's video servers carry `https://www.bilibili.com/`
as the Referer, as its player sends; some mirrors refuse requests without it. No
account cookies are sent, so Bilibili offers up to 480p; sign-in qualities, paid,
region-locked and DRM videos, live pages and `b23.tv` short links are not supported.

Bangumi (anime, series and documentary) episodes use the PGC endpoints
`/pgc/view/web/season` and `/pgc/player/web/playurl`, which list the same DASH
streams. A season link (`/bangumi/play/ss…`) opens its first main episode; trailers
and extras listed in a season's sections work by their own `ep…` links. Episodes that
Bilibili shows anonymous visitors only as a short preview (members only), and DRM or
region-locked episodes, are refused with a message instead of saving a preview. The request flow follows
[bilibili-downloader-gui](https://github.com/j4rviscmd/bilibili-downloader-gui) (MIT);
no code was copied.

The download saves the video ID and part (or the Bangumi episode ID) and the chosen
stream keys, never a signed link.
Bilibili's links expire after about two hours, so starting or resuming reads fresh
links first and keeps partial segments only when the length, byte ranges and SIDX
index are unchanged. Live checks: `cargo test --lib bilibili::tests::live -- --ignored`.

## TikTok video downloads

Use the extension's **Download video** button on a TikTok video page, or paste a link
such as `https://www.tiktok.com/@user/video/…` into **Add URL** / **Tools → Preview
video**. Choose a quality, then **Add video to queue**. The filename comes from the
video's caption.

The app loads the video page the way a browser's first visit does and reads the video
data embedded in it (`__UNIVERSAL_DATA_FOR_REHYDRATION__`) without running any script.
Each quality is one ordinary MP4 file with sound; the app downloads it in resumable
16 MiB byte ranges and saves it unchanged, without FFmpeg. TikTok's video servers only
answer with the page's anonymous visitor cookies and a `https://www.tiktok.com/`
Referer, so the app sends both, and only to `tiktok.com` servers over HTTPS. The
cookies are never written to disk: the page is read again before every start or
resume, because links and cookies expire, and completed parts are kept only while the
file size and hash are unchanged.

TikTok sometimes answers automated page loads with a JavaScript browser check. The app
does not try to pass it; it reports it, and opening TikTok in a browser and trying
again a few minutes later usually helps. Private, sign-in-only and photo posts,
`vm.tiktok.com` short links and live videos are not supported. Live checks:
`cargo test --lib tiktok::tests::live -- --ignored`.

## Twitch video downloads

Use the extension's **Download video** button on a Twitch video page, or paste a link
such as `https://www.twitch.tv/videos/…` into **Add URL** / **Tools → Preview video**.
Past broadcasts, highlights and uploads are supported. Choose a quality, then **Add
video to queue**. The filename is the channel name and the video title.

The app asks Twitch's public GraphQL endpoint (`gql.twitch.tv`, with the public client
ID of Twitch's own web player) for the video's title and an anonymous playback token,
as the web player does, and opens the video's HLS master playlist on `usher.ttvnw.net`
with it. The qualities' own playlists and segments need no token, so the download is
an ordinary finished HLS download: newer broadcasts are fragmented MP4 and save as
`.mp4`, older uploads are MPEG-TS and save as `.ts`. The saved download keeps the
video page address, never the token, and pausing or resuming does not contact Twitch
again. Subscriber-only videos, live channels, clips and sign-in are not supported.
Live checks: `cargo test --lib twitch::tests::live -- --ignored`.

## HLS video downloads

Choose **Tools → Preview video**, paste an HTTP/HTTPS playlist URL, and click
**Load video qualities**. The first supported quality previews automatically,
with the site's default audio selected initially. Choose a different quality
or **Audio track** to update the preview immediately. The controls remain visible
during previewing; changing either cancels the previous request and clears its preview.
A direct media playlist opens its
original-quality preview. Review the duration, segment count, output format,
estimated size (when the playlist supplies bitrate), filename, queue, and folder,
then click **Add video to queue**. Your filename, queue, and folder choices are
kept when changing quality or audio; the filename extension follows the output format.
**Retry preview** retries a failed or cancelled
request; **Change URL** clears the preview and returns to URL entry.
Adding is disabled until the current selection has a ready preview.
Previewing fetches playlists only, with no queue entries
or media downloads. Playback inside the preview is not included.

Chrome, Edge, and Firefox use the same desktop preview. Right-click an exposed
`.m3u8` link and choose Download with Download Engine, paste it into the extension's URL
field, or use **Find videos on this page → Choose quality**. HLS links pasted in the
desktop Add download dialog also open the preview. Opaque playlist URLs can be
pasted directly into **Preview video**. Page scans read exposed URLs; optional
media detection can also find supported GET media responses during playback.
Blob/MSE-only players may not expose a usable link. Reload development
extensions after updating the app and extension files.

Ordinary file downloads reject HLS/DASH manifest responses at the initial probe,
fallback GET and range requests, before saving those response bodies. Use
**Preview video** for these links. HTTP authentication and retry errors retain
their normal handling even when an error response has a manifest content type.

- Supports finished, unencrypted HLS playlists (`EXT-X-ENDLIST`), muxed MPEG-TS
  segments saved as `.ts`, and fragmented MP4 with a single initialization
  segment saved as `.mp4`. Explicit and implicit byte ranges are supported.
  Separate `EXT-X-MEDIA:TYPE=AUDIO` tracks in either of these containers are
  downloaded with the chosen video quality and merged into one `.mp4`, without
  re-encoding. The preview shows the audio name/language and output format.
  A track labeled "included in video" retains the original media's audio.
  Codecs must be compatible with MP4; a missing track or merge error fails the
  download rather than publishing a silent video. Source timestamps determine
  synchronization, independently of segment counts and media sequence numbers.
  The bundled FFmpeg helper needs no system installation.
- Live recording, encryption (including AES-128 and SAMPLE-AES), DRM, alternate
  video rendition groups, subtitles, packed audio, changing initialization maps,
  discontinuities/gaps, nested master playlists, and variable substitution are
  not supported. Unsupported
  containers returned by a server fail during transfer instead of publishing a file.
- Downloads use the normal queues, retries, pause/resume, speed limits, payload
  quotas, SHA-256 verification, antivirus actions and safe file publication.
  Resume reuses whole completed segments after checking their saved hashes;
  an interrupted segment starts again. This covers both video and selected audio.
  Both playlists are rechecked before any media is fetched. The table shows
  **Merging audio and video** once all segments are downloaded. Pause cancels the
  owned merge process; Resume retries merging from verified complete segments.
  Merging needs temporary disk space for the assembled tracks as well as the
  output; temporary track copies are cleaned up when the merge stops.
  Only the successful, hashed output is published and passed to antivirus actions.
  A changed/expired playlist requires a
  fresh preview. Restart discards segments for the same reviewed playlist;
  Duplicate and Update download link open a new preview. Synchronization queues
  skip completed HLS videos rather than treating manifests as ordinary files.
- Requests do not inherit browser cookies or file-server credentials. App proxy
  settings apply. Each destination is DNS-checked and direct routes are pinned;
  a selected proxy owns final hostname resolution. Custom Lua reader results are
  an exception: their previews and saved downloads require Direct connection and
  public addresses at every request, including the root and redirects. CDN requests cannot cross
  into a different local/special-use origin. An explicitly entered local origin
  is allowed for local media. Redirects and playlist references cannot downgrade
  HTTPS to HTTP. No encryption key is fetched.
- The helper receives only locally assembled track files, forced TS/MP4 input
  formats, and fixed arguments. Network protocols and external MP4 references are
  disabled. It writes standard MP4 through an already-open, guarded output handle.
  The app verifies the pinned helper checksum, protects its file/folders while it
  runs, and bounds memory (1 GiB), individual allocations (256 MiB), runtime
  (30 minutes), and output size (1.5 times input bytes plus 8 MiB). Failure leaves
  complete segments available for retry; no incomplete output is published.
- Limits: 1 MiB per playlist, 64 qualities/servers, 32 audio choices per group,
  10,000 combined video/audio parts, seven days of
  duration, 256 MiB per segment, five redirects, and bounded network waits.
  Pending browser previews do not interrupt an open desktop dialog, are limited
  to ten, and expire after 24 hours. Playlist/resource URLs (including signed
  query strings) and progress are stored locally with queued jobs; preview-only
  requests clear their URL when opened. Use the current app for HLS job data.

Playlist handling follows the supported subset of [RFC 8216](https://www.rfc-editor.org/rfc/rfc8216).

Development builds need the pinned helper under `tools/ffmpeg`. Run
`./scripts/Prepare-MediaTools.ps1` to verify its binary, notices and source bundle.
Rebuild it using `tools/ffmpeg/source/README.md` when needed. Packaging and
installer builds verify and include the helper, corresponding source and license notices. Release
binaries load only their adjacent `tools/ffmpeg/ffmpeg.exe`; there is no PATH or
user-configurable command fallback. See [FFmpeg provenance](../tools/ffmpeg/PROVENANCE.md)
for the version, checksums, source archives, build recipe and license. Keep those
materials with any redistributed binary package.

## DASH video downloads

Use **Tools → Preview video**, a `.mpd` link in the extension, or **Choose quality**
on a DASH entry in the desktop's **Video downloads** dialog. Select the video resolution/bitrate and audio
language/bitrate, then review the duration, output, filename and queue before
adding. Loading qualities reads only the MPD. SegmentBase preview additionally
reads the chosen tracks' index byte ranges; initialization and audio/video
segments are downloaded only after queueing. Preview creates no download.
The same flow is used by the on-page video button.

- Supports static, single-period, unencrypted MP4 video/audio representations.
  SegmentTemplate supports inherited attributes, `$RepresentationID$`,
  `$Bandwidth$`, `$Number$` (including zero padding), `$Time$`, and bounded
  SegmentTimeline repeats. SegmentList supports explicit URLs and byte ranges.
- SegmentBase supports inherited `indexRange` and initialization ranges in the
  same MP4 file, flat SIDX versions 0/1, 32/64-bit box sizes and offsets, and
  presentation offsets. Each index is at most 1 MiB and must receive an exact
  HTTP 206 range response. Offsets, counts, timestamps and resource bounds are
  checked before ranges can enter the queue. Only the chosen quality/audio
  indexes are fetched. Index bytes, resource sizes and supplied strong ETag /
  Last-Modified validators are checked again before transfer/resume; range
  responses must retain those validators and resource size during transfer.
- Chosen tracks are merged into MP4 without re-encoding using the bundled helper.
  Per-track presentation offsets are applied before merging. A video-only MPD is
  explicitly labeled as having no audio. MP4-compatible codecs are required.
- Normal queues, retry, segment-level pause/resume, quotas, speed limits, hash
  verification, antivirus handling and safe publication apply. The MPD is checked
  again before transfer/resume; a changed selection requires a new preview.
  Completed segments are hash-checked before reuse. Old HLS downloads keep their
  saved format; existing DASH downloads retain version 3. Indexed DASH uses
  version 4, including the reviewed index identity, so older apps reject it safely.
- Requests are anonymous and use the same DNS-pinned destination checks as HLS.
  MPDs are limited to 1 MiB; XML depth, node counts, expanded URLs and segment
  counts are bounded. DTDs, external entities, XLink and ContentProtection are
  rejected. MP4 initialization metadata is checked for protected sample entries.
  FFmpeg receives only guarded local files and fixed arguments, never a manifest,
  remote URL, cookie, key or page-supplied command.
- Unsupported: live/dynamic or encrypted DASH, multiple periods, hierarchical or
  external representation indexes, separate-file SegmentBase initialization,
  WebM representations, dependent layers, multiple BaseURL alternatives, subtitles,
  timeline gaps/overlaps, or segments crossing the period's start. Unsupported
  manifests fail with an explanation; they are not silently saved as videos.
  Browser login for streams and playback inside the preview remain unavailable.

Timing behavior follows the [DASH-IF timing model](https://dashif.org/Guidelines-TimingModel/).
For local verification, run `cargo test --test hls --locked -- --test-threads=1` and
`node --test tests/video_button.test.cjs`. `scripts/video_button_smoke.cjs --firefox`
uses installed browsers with disposable profiles and fixture-only site permissions;
it does not test the browser permission prompt. `scripts/grabber_schedule_smoke.cjs --dash`
captures native quality/preview dialogs with a screenshot-feature build.

## Website grabber

The grabber has two modes: **Find downloadable files** retains the reviewed file
discovery workflow below; **Offline website** saves a browsable static copy.

In **Offline website**, enter a starting page, destination folder, link depth,
page limit and size limit. **Save offline copy** creates a unique folder;
**Open offline website** opens `index.html`. Keep the whole folder together.
HTML links, CSS imports and URLs, responsive images, icons and fonts are rewritten
to local paths, with shared resources using one local file. Missing pages lead to
`report.html`; `report.json` records the captured files and omissions. Stopping
keeps a partial copy when the starting page is available.

This is an anonymous static UTF-8 capture, not a JavaScript/browser-session
snapshot. Scripts, forms, SVG animations and embedded pages are disabled. Pages stay on the
starting origin; optional CDN assets must resolve to public addresses. Resources
use the configured proxy without direct fallback. Limits include depth 5,
200 pages, 2,000 assets, 500 MB total (100 MB by default), 20 MB of HTML/CSS,
2 MB per text resource, 16 MB per asset, 10,000 inspected URLs and ten minutes.
The operation runs separately from queues and their bandwidth/quota controls;
it does not persist or resume a capture after restart. See Help → Guide for details.

Choose **Tools → Website grabber** in the desktop app:

1. Enter the starting HTTP/HTTPS page and comma-separated file extensions, such
   as `pdf, zip, jpg` or `tar.gz`. Presets cover documents, archives, images,
   audio, and direct video files. Filtering is case-insensitive and recognizes
   URL filenames and HTML `download` filename hints; it does not verify contents.
2. Set **Scan depth** and **Page limit**. Depth **0** reads the starting page and
   its same-origin `include-fragment` HTML resources, including GitHub release
   asset lists. Fragments share the page/byte limits and never execute scripts.
   **1** also reads pages linked directly from it. Depth is limited to 5,
   with up to 200 pages including fragments (50 by default). GitHub `/blob/`
   file-view links become `/raw/` download links; duplicate raw links appear once.
3. Choose **Scan website**. **Stop scan** keeps partial results for review;
   closing the dialog cancels the scan and discards its preview.
4. Search the preview and select individual files or **Select visible**.
   Nothing is selected automatically. Filtering preserves hidden selections;
   **Clear selection** clears all of them. Choose a queue and a saved destination,
   then **Add selected to queue**. A running queue can start the selected files
   immediately, and an idle, unscheduled Main queue wakes automatically. Other
   paused queues wait. Folder rules, retries, antivirus settings,
   and the normal file-opening protections apply to these downloads.

### Saved grabber projects

In **Tools → Website grabber**, expand **Saved projects**, enter a **Project
name**, and choose **Save project**. A project remembers the starting URL, file
types, scan depth, page limit, queue, and destination folder choice. You can
choose the queue and folder before scanning, or save them from the preview.

- Pick a project from the saved-project list to restore its settings, then
  choose **Scan website**. **Scan again** reruns the current settings from a
  preview. Every run fetches fresh results with nothing selected; you must
  review and choose **Add selected to queue** each time.
- Use **Save changes** to update settings or rename the current project.
  Enter a different name and use **Save as new** to keep a separate copy.
  **New scan** in the list starts an unsaved scan with default settings.
- **Delete project** asks for confirmation and removes the saved project, its
  schedule, pending discoveries, and discovery history. Existing downloads are
  preserved. The currently displayed manual preview can still be used.
- Opening a project or choosing New scan replaces the current unsaved settings
  and preview. Project controls are disabled while a scan is running.
- If the saved queue was removed, choose another queue before saving or adding
  files. Folder choices use the current destination rules from Settings rather
  than a frozen folder path.

Projects persist locally in the app's database across restarts. Up to 100
projects can be saved with unique names of 1–80 characters. The starting URL,
including its query string, is saved; browser credentials and preview selections
are not. Manual scan results stay in memory. Invalid URLs or scan limits are rejected. Changes
to a newer saved version cannot be overwritten by a stale editor. Saving or
opening a project does not scan, create downloads, or start a queue.

### Scheduled grabber projects

Save a project, then choose **Schedule scans**, or open **Scheduler → Website
scans → Manage scan schedule**. Choose **One time** or **Repeating**, a first
scan date and local time, and, for repeating scans, an interval of 1–720 hours
(24 hours is the default). Save any edited project settings before choosing
**Save scan schedule**. **Clear scan schedule** cancels an active scheduled scan
and prevents future runs while keeping pending files.

- Download Engine must be open (including in the tray) and the PC awake. After a missed
  time, sleep, or restart, one scan runs when the app can resume. Missed repeats
  are coalesced; subsequent scans keep the original fixed hourly interval. This
  is an elapsed-time interval, so its local clock time can shift with daylight
  saving or a time-zone change. Ambiguous/nonexistent first local times are rejected.
- Scheduled scans run one at a time and use the saved filters and bounds. They
  collect links only: they never enqueue files or start download queues. The
  existing anonymous, same-origin crawl restrictions also apply here.
- The first scheduled scan treats all matching URLs as new. Later scans show
  URLs not previously found by this project's scheduled scans, ignoring URL
  fragments but preserving query strings. This does not compare file contents
  or deduplicate against unrelated/manual downloads. Manual reruns still show
  all matches and do not change scheduled discovery history.
- A banner shows new files awaiting review. Open **Scheduler → Website scans →
  Review new files**, or **Review new files** inside the saved project. Nothing
  is selected automatically. Select files and choose **Add selected to queue**;
  **Ignore selected** dismisses those URLs without downloading them. Both actions
  keep the URLs remembered. Unselected files and later discoveries stay pending.
- Closing a scheduled preview keeps its files across restarts; selections are
  intentionally cleared. **Refresh preview** loads discoveries that arrived
  while the dialog was open. Adding selected files and removing those pending
  entries commit atomically, so a failed add preserves the preview and a stale
  submission cannot enqueue duplicates. Choose another queue if the saved one
  was deleted. Normal folder rules and antivirus/open-file protections apply.
- Partial results and scan failures are shown with the last scan status. Failed
  scans keep older discoveries; repeating projects try again at the next interval.
  A cancelled/interrupted scan does not establish a discovery baseline. Project
  edits, rescheduling and deletion invalidate in-flight results from older settings.

Each project retains up to 1,000 pending files / 700,000 bytes of preview data and 10,000
URL fingerprints. Full previews wait for review or report that additional links
were left for another scan; those links are not marked seen. Reaching the history
limit stops the schedule. **Reset discovery history → Forget reviewed links**
allows previously reviewed URLs to appear again, preserves pending files, and
does not automatically enable a stopped schedule. A project copy starts with
no schedule, pending files, or history.

Pending URLs, suggested filenames, and their source pages are saved in the local
app database until added, ignored, or the project is deleted. They can include
query strings such as expiring links; review those links before downloading.
Discovery history retains SHA-256 URL fingerprints rather than a second copy of
the URLs. Browser cookies and download credentials are never used by these scans.

Discovery reads static UTF-8 HTML with a separate anonymous HTTP client. The
starting hostname is resolved once and pinned for the scan; mixed public/local
answers and unusable destinations are rejected. App proxy settings apply; a
selected proxy is trusted to resolve the final destination. Direct IP or localhost scans
remain supported, but one scan cannot cross between public and local networks. It
follows page links only on the starting **origin** (same scheme, host, and port),
including redirects. If a site redirects to another origin, enter that final
page URL and scan again. Matching file links on other origins, such as a CDN,
can appear in the preview but are not followed during discovery. File URLs are
deduplicated without fragments; query strings are preserved. Relative URLs,
HTML entities, `base` URLs, and direct image/audio/video sources are recognized.
Page links with no filename extension or a common HTML/server-page extension
can be followed. Matching HTML pages are both previewed and followed within the
depth limit; unknown binary responses are skipped without reading their bodies.
The scan does not execute scripts or programs, submit forms,
enter frames, or send browser cookies or saved download credentials.

Scanning is sequential and paced, with a 15-second timeout per page, a 3-minute
scan limit, 2 MiB per HTML page, 20 MiB of HTML per scan, and bounded preview
storage. Each page contributes at most 10,000 inspected links; the preview holds
up to 1,000 files. Limits, failed pages, and partial results are reported. Queue
insertion reuses the atomic batch receipt so repeating a submission cannot
duplicate that batch. Manual previews stay in memory; scheduled discoveries are
persisted for review. Neither creates jobs until you confirm your selection.

File discovery does not render JavaScript, scan login-only pages, discover
opaque file endpoints without a matching filename hint, or download HLS/DASH.
The separate **Offline website** mode saves bounded static copies as described
above. Help → Guide includes a GitHub example for each mode.

## Proxy settings

Open **Preferences → Connection → Proxy** and choose **System proxy** (the existing
default), **Direct connection**, or **Custom proxy**. Custom addresses support
`http://`, `https://`, `socks5://` (local DNS), and `socks5h://` (proxy DNS).
One custom endpoint handles all HTTP and HTTPS file destinations, including
browser automatic-capture checks, download retries/resume, synchronization,
replacement-link checks, ZIP previews, HLS/DASH previews and segments, YouTube
lookups/refreshes, and manual/scheduled grabber scans. System/environment bypass lists do
not override a custom route, and failures never fall back to direct access.
Pause and resume an active download to apply changed settings; a running transfer
retains its route snapshot. FTP/FTPS continue to connect directly. PAC editing,
custom bypass lists and separate proxies per protocol are not implemented.

For a company proxy, choose **NTLM** or **Negotiate (Kerberos / NTLM)** and supply
an explicit account. Enterprise proxy authentication requires Windows and an
`http://host:port` endpoint; HTTPS destinations use CONNECT tunnels with normal
certificate validation. Basic proxy authentication retains the HTTP, HTTPS and
SOCKS options. Enterprise requests require Direct or Custom, because automatic
System proxy discovery cannot guarantee an explicit proxy account. When no
file-server login is supplied, use a destination hostname rather than localhost
or an IP address through an enterprise proxy. Likewise, an unauthenticated proxy
for an enterprise file login must use a hostname. These restrictions prevent
WinHTTP's IP-address exception to its automatic-login policy from using the
current Windows account. Supplied accounts are set before the first handshake.
Standard requests retain System/environment routing.
Each enterprise request owns a bounded WinHTTP session and authentication
exchange. Cookies and automatic Windows credentials are disabled; redirects are
validated by the caller. Proxy credentials never become destination headers.
See Microsoft's [authentication contract](https://learn.microsoft.com/en-us/windows/win32/winhttp/authentication-in-winhttp)
and [handle concurrency contract](https://learn.microsoft.com/en-us/windows/win32/winhttp/concurrency-in-winhttp).

Choose **Enter login** to save a proxy username/password in **Windows Credential
Manager**. SQLite contains only the proxy endpoint, mode and an opaque credential
identifier. Proxy credentials are separate from download/server credentials.
Existing passwords are never loaded into the dialog; use **Keep saved login**,
enter a replacement, or **No login**. Changing the endpoint requires a new login
or removal. Saving System/Direct mode removes the custom endpoint and saved login.
Replacements/removals are journaled so interrupted cleanup retries at app startup.
Missing or mismatched credentials stop the connection rather than silently
downloading anonymously or directly.

HTTPS proxies validate their certificate and hostname. Basic HTTP and SOCKS5 proxy
logins are unencrypted even when the destination file uses HTTPS. NTLM/Negotiate
uses a challenge response, but does not encrypt the proxy connection itself.
Remote HTTP/SOCKS proxy logins require explicitly allowing that trusted connection.
Prefer HTTPS proxies or a trusted local proxy. Literal loopback proxies are
allowed for local VPN/proxy applications. An HTTPS destination still has its own
certificate validation, and proxy credentials are sent to the proxy rather than
inside its connection to the destination. OS/HTTP libraries may retain temporary
copies of credentials in memory; Windows vault protection is scoped to your user.
See the [HTTP client's proxy contract](https://docs.rs/reqwest/0.13.5/reqwest/struct.Proxy.html).

## ZIP contents preview

In **Add download**, paste an HTTP/HTTPS link and choose **Preview ZIP contents**.
You can also use **Preview ZIP contents** in a download's row menu. The preview
shows searchable file/folder names, expanded and packed sizes, password-protected
entry markers and unusual-path indicators. **Back to download** preserves the
URL, filename, folder and checksum draft; nothing is added to the queue until
you use the normal download action. Existing protected downloads reuse that
download's saved authentication. For a new protected link, save Authentication
with **Continue the download after saving** off, then preview from its row menu.

The preview reads at most a 65,633-byte tail, a 56-byte ZIP64 record if needed,
and 4 MiB of central-directory metadata. Small ZIP files may fit entirely in the
tail read. The server must honor HTTP ranges and return consistent ranges, size,
location and validators. A directory outside the tail requires a strong ETag or
valid Last-Modified value; changed/missing validators stop the preview. Servers
that ignore ranges do not trigger a whole-file download fallback. Closing or
cancelling stops the request, and the complete operation has a 60-second deadline.

Standard single-volume ZIP and ZIP64 version-1 directory records are supported,
with UTF-8, CP437 and CRC-checked Unicode path names. Limits are 10,000 entries,
4 MiB of directory data and 4,096 bytes per filename. Split archives, encrypted
or compressed directories, ZIP64 record extensions and unsupported layouts are
rejected. Encrypted entries can be listed when their directory is readable.
Nothing is decompressed, extracted, opened or scanned. Reported sizes and names
are archive metadata, not a safety verdict; antivirus checks still apply to the
normal downloaded file. This does not download individual archive members.
Format reference: [PKWARE ZIP specification](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT).

Verification uses loopback HTTP/HTTPS/SOCKS fixtures, a ZIP written by .NET,
malformed/range-change fixtures and native dialog interaction tests. Run
`cargo test --lib proxy:: --offline --locked -- --test-threads=1` and
`cargo test --test zip_preview --offline --locked -- --test-threads=1`.
No external proxy credentials, production servers or OS certificate changes are
used by these tests.

## First-version boundaries

- HTTP GET downloads, including the bounded HLS / DASH VOD workflows above. Explicit
  browser-login snapshots apply to direct files; POST downloads,
  encrypted/live streams and DRM players remain unsupported. Direct video files
  exposed in the current page can be collected on request.
- The extension has contextMenus, nativeMessaging, storage, alarms, activeTab,
  and scripting permissions. Temporary page access is used only for an explicit
  page-link or video scan. On-page buttons optionally register a persistent content
  script on individually enabled sites, or on HTTP/HTTPS pages while media
  detection and its granted access are active; there is no default broad host access.
  The optional downloads permission is requested only for automatic capture.
  Optional cookies are requested only by the explicit browser-login review.
  Browser login and standalone per-site video buttons request individual site access;
  optional media detection requests webRequest and broad HTTP/HTTPS host access
  to recognize playback served from other hosts such as CDNs.
- Temporary failures retry automatically; permanent failures and exhausted retries
  stay visible with an error. Resume starts a fresh retry budget and uses saved
  bytes only when the server supports safe resume. See automatic retries below.
- New unnamed downloads accept sanitized server-provided filenames when available;
  explicit names, browser-supplied names, and existing history retain their names.
  Folder selection stays fixed at add time.
- Signed download URLs are retained locally in the queue database, so treat the
  data folder as private and clear queue entries you no longer need.
- Portable and per-user release builds support Authenticode signing and timestamping,
  or explicitly unsigned GitHub releases. Browser-store publication is separate.

## Automatic retries

Temporary connection failures, timeouts, interrupted bodies, and HTTP 408, 429,
500, 502, 503, or 504 receive **up to three automatic retries**, after **5, 10,
and 20 seconds**. Progress does not reset the budget. A server's longer
[Retry-After delay](https://www.rfc-editor.org/rfc/rfc9110.html#name-retry-after)
is respected, including HTTP dates. Requests to wait more than 24 hours need
manual attention; invalid headers use the normal backoff. All range workers
share one attempt budget, and the longest server delay is retained.

The compact table shows **Retry in ...** and the details show the attempt count
and last error. Waiting retries release their download slot so other queued
files can continue. Queue limits, ordering, and scheduled starts still apply.
**Pause** cancels a file's pending retry; **Pause queue** holds it until the queue
starts again. **Remove** after pausing cancels it permanently. Waiting times and
attempt counts survive app restarts; interrupted active transfers remain paused.
**Resume**, **Restart**, changing authentication, or accepting a replacement link
starts a fresh budget. An older executable does not understand retry timing;
avoid downgrading while retries are pending.

Safe range retries recheck the final URL, strong ETag, and size before appending
missing bytes. Without a safe resume plan, a retry starts from byte zero.
Authentication errors, expired links, certificate/redirect failures, invalid or
changed ranges, disk/permission/database failures, and unknown errors do not
retry automatically. Saved credentials remain bound to the same download and
origin during retries and are removed on completion or deletion as before.

## FTP and FTPS files

Use **Add download** with a file link. Plain `ftp://server/path/file.zip` uses
anonymous login only and sends file contents without encryption. Saved usernames
and passwords require `ftps://`; embedded URL credentials are rejected.

- `ftps://server/path/file.zip` defaults to implicit TLS on port 990.
- `ftps://server:21/path/file.zip` defaults to explicit AUTH TLS on port 21.
- For another port, include it in the URL and choose **explicit** or **implicit**
  TLS in Add download. Under **Advanced**, check **Use login** and enter a
  username/password while the job is paused. Authentication is also available
  for paused or failed FTPS jobs. Bearer tokens and browser login do not apply.

Each saved login belongs to that download and server/port in Windows Credential
Manager. Completion or removal retires it; duplication requires a new login.
FTPS validates the hostname and trust chain using Windows TLS, requires TLS 1.2
or newer and encrypted data connections, and checks that control and data
connections use the same certificate. There is no certificate-bypass option or
fallback to plaintext. Private servers need a certificate trusted by Windows.
DNS answers that mix public and local or special-use addresses are rejected
before connecting. Hostnames resolving entirely to local addresses remain supported.

**Pause**, **Resume**, **Restart**, retries, ordinary queues/schedules, global and
per-file speed limits, data quotas, SHA-256 verification, filename collision
protection, Windows origin markers and antivirus processing also apply to FTP.
Each FTP job uses one data connection. Interrupted/transient transfers use the
existing three-retry policy; rejected logins, certificates and permanent server
errors require attention. Network waits are cancellable and bounded to 30 seconds;
intentional speed-limit waits are outside that deadline.

Resuming checks saved size/time metadata and compares the saved prefix against
the remote bytes before appending. **This re-downloads the prefix** and counts
against speed limits and enabled data quotas. FTP timestamps alone cannot
guarantee an unchanged file. Supplying the publisher's expected SHA-256 allows
REST offset resume when supported; publication still requires that exact hash.
Unsupported REST falls back to prefix verification. Changed metadata or saved
bytes stop the transfer and ask for Restart.

Paths use UTF-8, binary transfers, and per-directory CWD commands. A normal URL
path is relative to the login directory; encode the leading root slash as `%2F`
when needed, for example `ftps://server/%2Freleases/file.zip`. Encode a literal
semicolon as `%3B`; FTP `;type=` suffixes are unsupported. The implementation
follows [FTP URL paths](https://www.rfc-editor.org/rfc/rfc1738#section-3.2.2),
[passive connections](https://www.rfc-editor.org/rfc/rfc2428),
[FTP over TLS](https://www.rfc-editor.org/rfc/rfc4217) and
[file metadata/restart extensions](https://www.rfc-editor.org/rfc/rfc3659).

Only passive mode is supported. The data connection uses the actual control
peer address, ignoring PASV-advertised hosts; privileged data ports and the
control port are rejected. Replies, paths and commands are bounded, control
characters are rejected, and error messages omit server text that could echo a
password. FTP connections go directly to the server; FTP proxies, SFTP,
directory browsing and active mode are not implemented. Browser handoffs, video
previews and website grabbing remain HTTP/HTTPS-only. Desktop batch/drop accepts
FTP links; batches infer TLS mode from the port as above. Synchronization queues
skip completed FTP files; use Redownload to fetch a fresh copy.

`cargo test --lib ftp:: --offline --locked -- --test-threads=1` and
`cargo test --test ftp --offline --locked -- --test-threads=1` exercise loopback
FTP and explicit/implicit FTPS, synthetic vault logins and cleanup, TLS refusal,
untrusted/incorrect host certificates, control/data identity mismatch,
prefix/hash verification, pause/reopen, retries, queues, limits, quotas and
malformed replies. The public fixture certificates are trusted only inside test
connectors; tests never change Windows certificate stores. `node scripts/ftp_smoke.cjs --ui`
captures both themes using a screenshot-feature build. `node scripts/ftp_smoke.cjs`
checks the unsigned portable app with an isolated database, exact FTP bytes,
restart/REST recovery, Windows origin markers and untrusted FTPS rejection.
These tests do not
establish compatibility with every external FTP server.

## Security

New downloads and recovered publications on Windows receive **Mark of the Web**
before completion. This retains the Internet-origin information used by Windows
and applications such as Office. The app fails the download if that marker
cannot be written. The marker contains no source URL, referrer, or signed token.
Already completed files from older builds are not modified automatically.
See [Microsoft's description of Mark of the Web](https://learn.microsoft.com/en-us/microsoft-365-apps/security/internet-macros-blocked#mark-of-the-web-and-zones).

Temporary files are created without truncating existing link targets.
Windows partial-file handles reject reparse points and extra hard links,
and app-owned cache directories reject junctions. Open transfer handles deny
concurrent writes or deletion. Unicode direction-control characters are
neutralized in new filenames so they cannot disguise a file extension.
Opening the destination folder uses the Windows-owned Explorer executable.

HTTPS uses normal certificate validation; HTTPS-to-HTTP redirects and embedded URL
credentials are rejected. Browser handoffs, the website grabber and video previews
remain HTTP/HTTPS only. Desktop downloads also accept FTP/FTPS, with the protections
described below. Password/token fields are masked
and their undo history is cleared each frame. App-owned credential buffers are
cleared on drop, and vault API calls are serialized; GUI, HTTP, and OS components
may still make temporary memory
copies. Windows Credential Manager does not isolate secrets from other programs
running as your Windows user. Native messages have a 1 MiB
limit and a fixed action schema. Registered browser hosts allow only the
specified extension IDs, and extension background commands require the
extension's own sender identity and URL.

The browser extension loads local scripts/styles only and blocks extension-page web requests,
inline scripts, embedded objects, and form navigation through its content
security policy. It cannot execute downloaded files. Reload an already-loaded
extension after updating this package.

Keep the portable app and its data in a folder writable only by your account.
The queue database is not encrypted and can contain signed download URLs.
These controls do not sandbox other programs running as your Windows user or
guarantee that a downloaded file is trustworthy. Malware scanning is optional
and must be enabled separately in Scan settings with a working antivirus.
Windows warnings depend on file type and system policy. Registration writes validate
exact extension IDs, preserve other copies, and replace manifest files without
truncating linked targets. Packaging requires either verified code signing or an
explicit unsigned-release choice. Permanent browser-store distribution and
independent security review remain separate from publishing a GitHub release.

Before releases, run the regression suite and
[cargo-audit](https://github.com/RustSec/rustsec/tree/main/cargo-audit)
against the current RustSec database. A clean dependency audit only establishes
that no matching published advisories were found at that time.

## Remaining IDM feature gaps

Compared with the [official IDM feature list](https://www.internetdownloadmanager.com/features2.html),
the largest remaining gaps are signed installer/store-extension distribution,
live/encrypted streams, broader DASH addressing/container support, and dynamic or authenticated offline websites.
Unencrypted HLS / DASH VOD with quality/audio selection and separate-track merging is available. A bounded website grabber
with saved projects, scheduled discovery of new links, file-type filters, scan
depth, and selection previews is available. Static offline website capture with local pages and assets is also available. Adaptive segmentation is implemented with up to 32
active connections per file. We currently support
HTTP/HTTPS and passive FTP/FTPS file downloads, with no SFTP/MMS support or automatic updater. Optional Defender,
ClamAV, and configurable CLI antivirus scanning is available; Malwarebytes scan
requests can run automatically with results reviewed in its app.
Column visibility, resizing/reordering, server filenames, and URL drag-and-drop are available.
HTTP/HTTPS proxy settings with saved logins and bounded ZIP contents previews are also available.

Our queues now support ordering, independent limits, one-time starts, and
recurring daily/weekday start-stop windows. IDM also has [synchronization queues and start/stop schedules](https://www.internetdownloadmanager.com/support/idm-scheduler/idm_queues.html).
Synchronization queues, queue notifications, app-exit actions and optional computer shutdown are available.
Computer wake, restart and sleep actions remain future work.

For the file-download-first scope, the implementation order is:

1. **Installation and browser setup:** implemented locally with a per-user
   installer and in-app registration. Signing and store publication remain
   release steps; see [browser release preparation](browser-release.md).
2. **Expired-link recovery:** implemented with saved-byte verification and an
   explicit restart when safe resume is unavailable. Browser-assisted recovery can find a fresh link for that existing job.
3. **Authentication:** HTTP Basic and bearer tokens implemented with per-download
   Windows credential storage, plus opt-in Chrome/Edge/Firefox login snapshots
   protected by Windows user-scoped DPAPI.
4. **Automatic retries:** implemented with persisted bounded backoff.
5. **Speed limits:** persistent global and per-file controls, shared across range
   connections and applied during active transfers and replacement-link checks.
6. **System tray and notifications:** implemented with optional close-to-tray,
   cross-queue completion/failure notices, and safe explicit exit.
7. **Categories and folders:** implemented with filename-based filters and saved
   destination rules shared by app and browser downloads.
8. **Video detection and site grabbing:** direct video-file discovery, unencrypted
   HLS / DASH VOD previews/downloads, optional on-page video buttons, and a bounded HTML website grabber are available;
   live/encrypted streams, broader DASH compatibility, and JavaScript-rendered
   crawling remain future work. Saved projects support once-only and repeating
   scans with persistent new-file previews, plus manual reruns with fresh results.
9. **Antivirus:** optional scans, persisted results, and a fresh scan/confirmation
   before opening through Download Engine. Malwarebytes can receive automatic scan
   requests and requires manual result review in its app.
10. **Data quotas:** optional hourly and daily allowances with durable shared
    usage, resumable quota holds, automatic reset, and manual-pause preservation.

Category tests cover legacy history preservation, rule validation and persistence,
manual and batch overrides, native-message folder restrictions, prepared handoff
retries, resume after rule changes, safe publication, and desktop filter/dialog
interaction in both themes.

## Local diagnostic logs

The desktop app and native browser connector keep separate structured logs:
`desktop.log` and `native-host.log` in the active data folder's `logs` directory.
An installed copy normally uses `%LOCALAPPDATA%\Download Engine\logs`
(`%LOCALAPPDATA%\IDM_EX\logs` for a profile from before 2.0); a portable copy
uses `data\logs` beside its executables. `DOWNLOAD_ENGINE_HOME` overrides this with
`<DOWNLOAD_ENGINE_HOME>\data\logs`. **Logs**, beside Scripting in the top toolbar, opens the
in-app viewer. **Open log folder** inside that page opens this copy's log files
directly in File Explorer when needed.

The viewer combines desktop and browser connector events, newest first, with
local timestamps, severity/source filters, and search (Ctrl+F). Events appear as
plain, selectable text, colored red for errors, blue for info, and light red for
warnings (with darker shades on the light theme). Drag to select, use Ctrl+A/Ctrl+C
in the text area, or **Copy logs** to copy all displayed text. Escape clears the
focused log search. It shows up to 2,000 recent events from the retained files;
export includes older retained
events too. Reads run on a background thread. **Auto-refresh** checks every two
seconds while the Logs page is visible. It pauses while the text area is focused
or scrolled away from the top, keeping reading position and text selection stable.
Turn it off to hold a snapshot, or use **Refresh** when ready. Read failures retain
the previous snapshot and show an error until a later refresh succeeds. Download filters and selection are
preserved when switching to Logs and back.

Each component retains its current file plus three rotated backups, at up to
1 MiB each (8 MiB total, excluding tiny lock files). Logs include UTC timestamps,
app version, process/session identifiers, severity, startup/shutdown and panic
events, native request actions/outcomes, browser registration results, download
start/completion/pause/retry/quota/failure events, and UI/scheduler errors. Job
UUIDs and per-session request numbers support correlation. Available OS, SQLite,
and HTTP error codes are preserved. There is no automatic upload.
Writing runs on a dedicated thread with a bounded 1,024-event queue, so disk
contention does not block download workers or the UI. If that buffer fills,
an `entries_dropped` event records the count once writing recovers. Normal exit
and export drain pending records with a bounded wait; forced termination can
lose buffered events.

Privacy is enforced by the record schema: URLs, download names, local paths,
request IDs supplied by a browser, headers, cookies, passwords, request bodies,
and raw exception/panic text are never written. Unknown errors retain a category
and event context rather than arbitrary text. This also means a log may not
contain the full message visible in the UI. Native logs never use stdout, which
remains reserved for browser message frames. Logging failures do not stop
downloads or browser replies; Logs shows **Logging unavailable** when the logger
cannot write. Errors before the data directory can be resolved still use the
existing startup dialog/stderr path. Hard process termination or power loss may
leave a partial last record; these are not crash dumps.

**Logs → Export diagnostics…** asks for a destination folder, writes a uniquely
named `DownloadEngine-diagnostics-*.json` report on a background thread, and shows
its saved location in the Logs page. Cancel leaves everything untouched. Export
reads only the bounded, known log files, validates each record again, and reports skipped
malformed entries. It does not include the download database, settings, browser
profiles, or files you downloaded. Existing exports are never overwritten.
Browser extension settings diagnostics remain separate under the extension's
**Diagnostic log**; copy that log too for permission/popup bugs.

## Build and verify

Rust stable with the Windows MSVC target, Visual Studio C++ build tools, and the
Windows SDK (including `rc.exe`) is required. The supplied Cargo.lock pins the
resolved dependencies. `build.rs` embeds `assets/download-engine.ico` in the desktop
executable; the window and tray use PNG sizes derived from the same master artwork.

    cargo fmt --check
    cargo test --all-targets --all-features --locked
    cargo clippy --all-targets --all-features --locked -- -D warnings
    node --test tests/*.test.cjs
    $signing = @{
        SignTool = 'C:\absolute\path\to\signtool.exe'
        CertificateThumbprint = '<publisher certificate SHA-1 thumbprint>'
        TimestampUrl = 'https://<publisher-approved RFC3161 service>'
    }
    ./scripts/Package.ps1 @signing
    ./scripts/Build-Installer.ps1 -SkipBuild @signing

Signed packaging verifies all three executables, Setup, and the generated
uninstaller against the configured publisher certificate. For an unsigned public
GitHub release, explicitly use `-AllowUnsignedRelease` instead of the signing
parameters; see [release instructions](github-release.md). For isolated
local smoke artifacts only, use `-AllowUnsignedDevelopment`; never distribute
that output. Development archives and directories include `UnsignedDev` in the
name, and a development installer additionally requires `-TestBuild` so it cannot
be mistaken for a release. For example:

    ./scripts/Build-Installer.ps1 -TestBuild -AllowUnsignedDevelopment

HLS regressions use local HTTP fixtures for quality selection, encryption rejection,
playlist changes, exact TS/fMP4 assembly, byte ranges, pause/resume, retries,
quotas, cache tampering, scheduler integration, and the explicit review boundary.
Separate-track tests generate local H.264/AAC media with the pinned FFmpeg helper.
They verify all four TS/fMP4 combinations, unchanged encoded slices/audio packets,
B-frame timestamps and a shared audio/video clock, explicit audio selection,
pause/reopen/resume during audio, missing-track failure, collision-safe publishing,
and recovery after publication. Helper tests cover checksum rejection,
cancellation, timeout, and output limits; shared process-lifetime tests also
exercise antivirus child cleanup.
HLS, DASH and indexed-MP4 pause tests cancel once at an exact media request,
including its byte range. This keeps index/init requests intact and makes the
pause point independent of helper verification time or parallel test load.
Resume checks verify completed video/audio parts are reused and only the
interrupted part is requested twice.
With a screenshot-feature build, `node scripts/grabber_schedule_smoke.cjs --hls`
captures the quality picker and preview in both themes. After development
packaging, `node scripts/hls_packaged_smoke.cjs` exercises the actual executables
with isolated data: native preview receipts create no job or network traffic,
and a seeded reviewed fixture completes with exact bytes, SHA-256, and Windows
origin marking. Pass a package directory as its first argument to check another
build. For a packaged media check, build the test-only inspector using
`tools/ffmpeg/source/README.md`, copy `ffprobe.exe` to `artifacts/ffmpeg-tools/`,
and verify it with `./scripts/Prepare-MediaTools.ps1 -TestTools`, then run
`node scripts/hls_merge_packaged_smoke.cjs`. It generates TS/fMP4 tracks, exercises
the actual packaged queue/merger, and checks both streams, packet counts,
presentation/decode timestamps, successful decoding, SHA-256, and Windows origin
marking. The inspector is not shipped. These local checks do not establish
live-site or registered-browser compatibility.

Quota regressions cover persistent and concurrent accounting, independent reset
windows, clock rollback, live settings acknowledgements, safe resume, manual and
queue pauses, retry traffic, and link checks that preserve saved bytes. Native
previews use `python scripts/queue_ui_smoke.py --feature quotas`. A packaged,
isolated transfer check is available with `python scripts/desktop_smoke.py
--binary-dir dist/IDM-EX-UnsignedDev --quotas`.

Website-grabber regressions use local HTTP fixtures to check depth boundaries,
file-type matching, duplicate links, redirects and origin boundaries, malformed
or oversized responses, cancellation, preview selection, queue atomicity, and
the exact downloaded bytes. Desktop tests cover selection, retry after a removed
queue, cancellation on close, stale results, and minimum-size layouts in both
themes. Saved-project regressions cover database migration and reopening,
invalid settings, duplicate names, stale updates/deletions, concurrent project
limits, removed queues, UI save/rename/copy/delete, and reruns that fetch changed
page content without reusing a prior selection. Native previews can be captured
with a screenshot-feature build and `python scripts/queue_ui_smoke.py --feature
grabber` or `--feature grabber-project` for the saved-project controls, and
`--feature grabber-schedule` for the schedule form, `--feature grabber-discoveries`
for a persistent preview, or `--feature grabber-scheduler` for its overview. Scheduled-grabber regressions
cover actual local HTML scans, new-link tracking, restart/cancellation recovery,
stale results, bounded storage, and atomic preview-to-queue transfers.
`node scripts/grabber_schedule_smoke.cjs` captures the native schedule, preview,
and overview in isolated light/dark fixtures, including reopening a pending preview.
After packaging, `node scripts/grabber_schedule_packaged_smoke.cjs` checks the
actual portable app against loopback HTML across two launches. It verifies new
URL detection and persistent previews with zero automatic downloads, and stops
only the fixture desktop processes it starts.
It defaults to `dist/IDM-EX-UnsignedDev`; pass another binary directory as the first
argument to verify a different build.

The installer build uses the official [Inno Setup compiler](https://jrsoftware.org/isdl.php).
Use **-Compiler C:\path\to\ISCC.exe** if it is not in a default location.
**-TestBuild** creates a separate installer identity for isolated install/upgrade/
uninstall testing. Installers contain only an explicit list of executables,
extension files, the installed-mode marker, the optional registration script,
and README; no runtime queue or downloaded files are embedded.

The Rust integration tests use isolated temporary directories and local HTTP
fixtures, including real TCP transfers and native-host subprocesses. Range-pause
fixtures hold the response at 128 KiB until cancellation, so a delayed observer
cannot accidentally let the fixture finish first. The pause and link-completion
waits keep their eight-second watchdogs, read SQLite off the HTTP fixture's runtime
thread, and report unexpected retries or worker/scheduler failures immediately.
Regressions exercise both HTTP 503 and actual SQLite write failures, plus WAL
lifetime across store clones, concurrent reads, explicit checkpoints, final
handle release, and persistence after reopening.
Credential leak checks read the live database, WAL and other regular app files.
Only `queue.sqlite3-shm` is excluded: it contains SQLite coordination data, and
Windows enforces byte-range locks on it. A regression holds a real write
transaction and verifies the scan detects a synthetic marker in the WAL, in the
database after checkpointing, and in an unrelated similarly named file.
They cover range downloads, cancellation/resume, changed resources, truncated responses,
actual retry delays and exhaustion, persisted waits, slot release, queue/user pause,
server backoff, and permanent failures,
queue recovery, framing, and destination collisions. Replacement-link tests cover
all saved segments, cross-URL validator collisions, explicit restart, cancellation,
shutdown, stale state, login pages, and preservation of queue order. Queue tests also cover legacy
migration, atomic batches, ordering, limits, independent pauses, scheduled starts,
and concurrent native-host opens. Security regressions cover link-target preservation,
cache junctions, Windows origin markers, and misleading filename controls.
Windows authentication tests use only disposable, synthetic credentials and
isolated loopback servers. They cover Basic/bearer range transfers, pause/reopen,
origin and redirect restrictions, missing/incorrect credentials, cleanup after
completion/removal/interruption, concurrent credential writes, guarded restarts,
link recovery, and rejecting credentials from the anonymous browser protocol. UI tests
check masked fields, cleared undo history, cancellation, save acknowledgement,
and minimum-window layout. These tests do not exercise a real site's login or
prove all TLS/proxy environments work.
Browser-setup tests use disposable HKCU test keys to verify exact permissions,
registration repair, missing hosts, conflicting copies, and safe manifest replacement. JavaScript tests exercise
extension permissions, sender authentication, content policy, automatic handoff,
interrupted recovery, and fallback contracts. These do not by themselves prove installation in
each real browser.

Queue regressions hold a worker at the saved-state/cleanup boundary to verify
that terminal actions wait for ownership release and still publish notifications,
while controls on running downloads continue to require pausing. Link confirmation
uses a held HTTP response, and retry ordering uses server-recorded request times,
so these checks do not depend on the assertion thread winning a timing race.
Limiter regressions include a deterministic lost-wakeup case where file eligibility
changes between worker wakeups; the eligible owner is signaled explicitly.
Tray tests cover preference migration, failed saves, batching, stale events,
terminal scheduler events across queues, and quiet retry/pause transitions.
**python scripts/tray_smoke.py --binary-dir dist/IDM-EX-UnsignedDev** checks native icon
registration/removal, hidden downloads, second-launch restore, pause, exit, lock
release, and shutdown with an open tray menu. It sends synthetic menu/callback
messages only to its fixture windows and simulates TaskbarCreated only on the
owned helper window; it does not restart Explorer or prove visible banner delivery.
For actual Shell submission and synthetic notification-click routing, run
**cargo test --bin de native_notifications -- --ignored --test-threads=1**.
That opt-in test briefly creates its own icon and test notifications.
**python scripts/queue_ui_smoke.py --preferences** captures the Tray & notifications dialog
with the screenshot build. Screenshot fixtures do not create tray icons.
Use **--width 860 --height 600** for the minimum layout, **--dark** for dark
appearance, **--compact** for compact rows, and **--hide-details** for the full
list. **--view queues** (or scheduler/settings) captures an application page;
**--view browsers** and **--view help** open Browser integration and About.
The script saves both the original framebuffer and a PNG under a unique
artifacts folder, using paused fixture queues and no runnable network downloads.

For an isolated GUI snapshot, build with **cargo build --features screenshot**,
then set DOWNLOAD_ENGINE_HOME to an empty absolute test directory and
EFRAME_SCREENSHOT_TO to an absolute .png path before launching target\debug\de.exe.
This test-only feature captures the window and exits.

For the Classic Download Engine interface, run **node scripts/native_ui_smoke.cjs**
after that build. It captures the real native renderer in both themes at normal
and minimum sizes, plus About, Tray & notifications, Browser integration, queue management,
Settings, an empty queue, and a previously saved column layout. All fixture queues remain paused in separate
test directories; the report and PNGs are saved under **artifacts/download-engine-native-ui**.
It also covers Add download, validation errors, filtered empty results, and queue
controls at the minimum window size in both themes.

Use **python scripts/queue_ui_smoke.py --schedule --width 860 --height 600**
for the recurring schedule form, or **--view scheduler** for saved recurring
windows. These previews keep every sample download offline. Scheduler tests
cover start/stop boundaries, overnight weekdays, daylight-saving gaps/repeats,
manual overrides, migration/reopen, and safe worker ownership. The HTTP fixture
also checks exact resumed bytes across app exit and automatic cancellation.

Use **python scripts/queue_ui_smoke.py --retry** with that screenshot build for
an isolated compact-table retry preview. **python scripts/desktop_smoke.py
--binary-dir dist/IDM-EX-UnsignedDev --retry** checks real native handoff, a temporary HTTP 503,
the observed retry delay, and byte-identical completion in the packaged app.

For a settled dialog capture, set **IDM_EX_SCREENSHOT_TO** to an absolute **.ppm**
path instead, and launch the screenshot build with **--browser-setup**. This
captures the app's own framebuffer after layout settles and then closes normally;
no Windows screen-capture access is required.

Run **python scripts/installer_smoke.py** after building with **-TestBuild** to
check install, upgrade, uninstall, running-app guards, a missing cleanup helper,
and native download flows. It uses a fresh workspace folder, a separate uninstall
identity, and isolated queue data, and verifies existing browser mappings are
unchanged. The test build does not create shortcuts.

## Code layout

- src/browser_setup.rs: exact per-user browser registration and ownership checks.
- installer/DownloadEngine.iss: per-user installation, upgrade guards, and safe uninstall.
- src/engine.rs: HTTP validation, streaming, transfer dispatch, and safe publication.
- src/engine/segmented.rs: adaptive HTTP range workers and partial-file accounting.
- src/network_scope.rs: shared destination classification and rejection of DNS answers that mix public and local addresses.
- src/auth.rs and src/credential_vault.rs: per-download origin binding, credential lifecycle, and Windows vault access.
- src/download_file.rs: guarded Windows file handles, shared partial-file cleanup, cache protection, and Internet-origin metadata.
- src/antivirus/: scanner configuration, persisted reports, guarded file review, and bounded child-process lifetime.
- src/desktop/antivirus.rs: scan settings, results, and explicit opening confirmation.
- src/store.rs: download records, capture reservations, and durable handoff receipts.
- src/queue.rs and src/queue_store.rs: named queues, batch validation, ordering, migration, and schedules.
- src/capture.rs: browser metadata and anonymous HTTP preflight validation.
- src/scheduler.rs and src/scheduler/: scheduler API, worker settlement, event loop, queue policy, cancellation, and shutdown.
- src/retry.rs: transient-error classification, bounded backoff, and Retry-After parsing.
- src/preferences.rs and src/notifications.rs: persisted desktop options and terminal-attempt notification policy.
- src/tray.rs: Windows notification-area resources, callbacks, and recovery.
- src/main.rs: native desktop startup and single-instance ownership.
- src/desktop.rs: shared desktop state, background events, and frame coordination.
- src/desktop/: table, details, theme, and focused download, queue, authentication, browser, and settings dialogs.
- src/bin/de-host.rs and src/native.rs: framed browser handoff.
- extension/: shared JavaScript plus Chromium and Firefox manifests.
- scripts/: portable/installer builds, optional registration, and isolated smoke tests.
- analysis/: separate evidence from examining the user's installed IDM.
  No vendor implementation code is compiled or packaged into this application.

Native messaging follows the public browser interfaces:
[Chrome](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)
, [Edge](https://learn.microsoft.com/en-us/microsoft-edge/extensions/developer-guide/native-messaging),
and [Firefox](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging).


The Windows desktop smoke test is available as **python scripts/desktop_smoke.py --binary-dir dist/IDM-EX-UnsignedDev**. It uses an isolated queue and local HTTP server, checks exact output bytes, exercises native-host cold startup, and closes only its own application windows.

Antivirus integration can be checked without malware or antivirus configuration
changes. Compile the harmless protocol fixture (it is never packaged):
**rustc --edition 2024 scripts/fixtures/antivirus_scanner.rs -o artifacts/antivirus-scanner-fixture.exe**.
Run **python scripts/desktop_smoke.py --binary-dir dist/IDM-EX-UnsignedDev --antivirus-scanner artifacts/antivirus-scanner-fixture.exe**.
Use **--antivirus-mode detected --antivirus-expected detected**, **--antivirus-mode
error --antivirus-expected failed**, or **--antivirus-mode timeout
--antivirus-expected timed_out** to exercise failure paths. These tests verify the
integration contract, not a vendor's detection quality. **--antivirus-defender**
instead scans only the harmless downloaded fixture with installed Defender; its
result depends on that machine's antivirus configuration. The tests use isolated
settings/history and paths containing spaces and shell metacharacters.


The popup can be checked in an isolated headless Chrome session with
**node scripts/extension_popup_smoke.cjs** (add **--packaged-firefox** to check the
actual Firefox files in `dist`; requires Playwright as a development
dependency). It applies the extension content policy and checks that injected
inline scripts and web fetches are blocked. Its browser API adapter is simulated; this verifies the popup,
not installation or native messaging in a real browser profile.

Add **--automatic** to the desktop smoke command to verify prepare/commit, a real file transfer, and cold desktop startup through the automatic-capture protocol.


The page picker has a separate rendered UI check:
**node scripts/page_links_ui_smoke.cjs**. It verifies filtering, selection, saved
folder choices, text-only rendering of untrusted names, narrow/light/dark layouts,
and retry controls using a simulated extension adapter.

### Installed-browser integration checks

Run `python scripts/browser_smoke.py --browser chrome` (or `edge` / `firefox`)
after building a portable package. Chromium checks need Node and Playwright;
pass `--node <path>` if Node is not on PATH. Firefox checks need Mozilla's
`marionette_driver==3.7.0`, installed into `artifacts/browser-test-deps` (or pass
`--firefox-client-dir <directory>`). Use `--binary-dir target/debug` to check a
development build. These are opt-in Windows integration checks. Run one browser
at a time: native-host registration is serialized across all browsers.

The runner uses the installed browser executable in a fresh headless profile,
a disposable app copy with a path containing spaces, and local HTTP fixtures.
It refuses existing Download Engine native-host registrations, creates a temporary
registration, and removes it on exit. Normal browser profiles and the user's
queue are not used. The checks cover stock-manifest popup connection and manual
handoff, cold desktop startup, duplicate handoff, capture off/on, disconnected
host fallback, exact bytes and Internet-origin marking. The desktop must survive
browser shutdown and then exit normally when the fixture closes it.

Automatic-capture tests grant downloads access in a test-only manifest while
retaining the optional declaration required by Firefox's permission API. The
source/release manifests keep that permission optional. Actual permission
prompts, store installation/signing, authenticated websites, and right-click
context-menu interaction are not covered by these checks. Chromium uses its
[extension debugging protocol](https://chromedevtools.github.io/devtools-protocol/tot/Extensions/);
Firefox uses [Marionette temporary add-on installation](https://firefox-source-docs.mozilla.org/python/marionette_driver.html#marionette_driver.addons.Addons.install).
Reports and isolated test files remain under `artifacts/browser-smoke-*`.

Add **--page-links** to run the current-page batch flow in any of the three
browsers. This uses the unmodified release manifest and confirms that page access
is denied before invoking the real browser action. The driver triggers that
action through CDP/Marionette and invokes the real popup button from a background
extension tab, leaving the source page active. It checks link filtering and
selection, the native batch receipt, cold desktop startup, a chosen paused queue
and folder, duplicate prevention, completed bytes, and Internet-origin marking.
It does not automate physical toolbar clicks or right-click menu interaction.

Browser batch unit tests also exercise rollback on job/receipt write failures,
concurrent retries, changed-payload rejection, receipt persistence after removed
history, and session recovery after interrupted acknowledgements.

Server-filename regressions cover Unicode/legacy encodings, quoted parameters,
malformed/duplicate headers, redirects, range and single-response downloads,
empty files, error metadata, explicit/browser names, old database records,
resume/retry/restart, safe leaf paths, collisions, and Windows origin markers.
Column tests exercise migration, rejected writes, pointer resizing/reordering,
cancel/drop-outside behavior, file selection, Settings reset/save/cancel,
and responsive sizing. Use **python scripts/queue_ui_smoke.py --custom-columns**
to render an isolated customized layout, or **--feature columns --width 860
--height 600** to render its settings at the minimum size.

### Adaptive range verification

The regular Rust suite covers connection handoff, pause/resume, missing new part
files after a saved split, stale progress checkpoints, the four-connection cap
with many saved parts, validators, transient retries, checksum failures, failed
layout commits, unexpected part files, and replacement-link verification in
byte order. Fixed-range legacy records remain readable; versioned layouts and
invalid partitions are checked independently.

Run the optional 32 MiB loopback timing fixture separately from other tests:

    cargo test --release --locked --test adaptive uniform_and_uneven_benchmark -- --ignored --nocapture

It reports three complete-file timings for uniform connections and three for an
uneven connection, including assembly and SHA-256 verification. Every sample
checks exact output bytes and the observed connection cap. Timing is reported,
not used as a pass/fail threshold; these local fixtures are not an internet speed
guarantee.

The packaged desktop and native-host path can be checked with an isolated profile:

    python scripts/desktop_smoke.py --binary-dir dist/IDM-EX-UnsignedDev --adaptive

This verifies an adaptive transfer, exact bytes and SHA-256, Internet-origin
marking, normal shutdown, and native-host cold startup without personal queue data.

## Downloads using your browser login

In Chrome, Edge, Brave or Firefox, sign in to the website normally and keep that tab
open. Right-click a direct file link and choose **Download using browser login…**.
Alternatively, open the extension popup on that site, paste the file URL, select
**Use this site’s login (review first)**, and choose **Download with Download Engine**.

The review shows the exact file URL and server. Choose **Allow site login &
download**, then approve the browser's optional cookies and site permission.
The review itself reads no cookies. The link must use HTTPS and the same origin
(scheme, hostname and port) as the original site tab. Literal loopback HTTP
addresses are allowed for local testing. Browser host permissions cover a
hostname across ports, but Download Engine restricts the snapshot to the exact origin.

Only matching, unexpired, unpartitioned cookies are copied, including HttpOnly
cookies. Firefox uses the original tab's cookie store/container. Private tabs,
partitioned cookies and Firefox first-party-isolated sessions are unsupported;
the extension does not broaden access to find another session. Automatic capture,
page-link batches and video scanning remain anonymous. No browser passwords,
profile databases, OAuth storage, arbitrary headers or POST bodies are read.

Each download receives its own snapshot, encrypted with **Windows user-scoped
DPAPI** before it reaches SQLite. Job metadata contains only an opaque binding
and origin. Encryption and the job/receipt commit in one database transaction;
retries cannot overwrite a running download's login or create duplicate jobs.
Cookies are never written to extension storage or diagnostic messages. Managed
Rust secret buffers and native message frames are wiped on drop; the browser's
JavaScript runtime does not provide guaranteed memory erasure.

A snapshot can resume after pausing or restarting on the same Windows account.
It is usable for at most 24 hours, with each cookie also respecting its own
expiry. Signing out or closing the browser does **not** erase a snapshot already
sent to Download Engine. Completion, removing the download, **Authentication → None**, or
accepting a replacement link on another origin removes the saved ciphertext.
Removing the review's site permission stops future cookie reads, not existing
desktop snapshots. Encrypted remnants may remain in SQLite journals or backups;
these are not plaintext cookies. Backups of the data folder can contain
ciphertext, while distribution archives exclude all runtime data.

Every HTTP redirect and range request recomputes cookies using domain, path,
secure and expiry rules. A different origin or HTTPS downgrade is blocked before
contacting that destination. Response Set-Cookie headers do not modify the saved
snapshot. HTTP 401/403 and HTML login pages produce an actionable failure instead
of a completed file. Session expiry never triggers automatic login or refresh.

If login expires, sign in again and send a fresh download from the browser.
Refreshing a snapshot in place, cross-server login redirects, login-page HTML
downloads, device-bound sessions and sites requiring POST requests are not
supported. The extension limits each snapshot to 128 cookies and a 32 KiB Cookie
header. Password/token authentication remains available separately in the app.
DPAPI protects data at rest under your Windows account; it does not isolate
secrets from other programs already running as that same account.

After updating, reload the unpacked Chrome/Edge extension or temporary Firefox
add-on to load the new code. Signing and store publication remain separate work.

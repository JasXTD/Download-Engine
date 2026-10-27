# Download Engine guide

Open Help → Guide in Download Engine, or press F1 when no dialog is open. This guide
is included in the application and works offline. Choose a topic to get started.

## Getting started

### Preferences

Open Preferences from the toolbar, Tools → Preferences or the tray menu. The popup has six categories and leaves your current page in place. Appearance includes the theme, language and table layout; Downloads has status-window, folder and transfer-limit choices; Connection has proxy and per-site connection settings; Browser & AI has browser setup, clipboard capture and MCP; Antivirus selects a scanner; Startup & tray controls Windows startup and notifications.

Use Save to apply pending changes, or Cancel, × or Escape to discard them. Themes preview while you edit. Changing the language takes effect after Save. Reset restores defaults for the current category only and also waits for Save. Table-column edits return to the parent draft. Detailed folder, connection, speed, quota, browser, MCP and antivirus editors have their own explicit saves; closing them returns to Preferences without losing your draft. Cancelling Preferences does not undo a change already saved in one of those editors.

### Your first download

1. Choose Add URL, or press Ctrl+N. Paste a direct link to a file.

2. Choose the queue and destination. Leave the filename blank to use the name from the link or server. Automatic folders sort new downloads by file type.

3. Choose Start download. An idle, unscheduled Main queue starts automatically. For a paused or scheduled queue, choose Add to queue; you can then right-click the file and choose Start immediately, or start the whole queue from Manage queues.

4. Watch progress in All downloads. When the file is Completed, use Open folder or the three-dot menu to open it.

### Find your way around

The sidebar filters downloads by status, file type or queue. Manage queues controls groups of downloads. In the top toolbar, Scripting contains your rules, scripts, site readers and run history; Grabber beside it finds website files and saves offline websites. Logs contains application diagnostics and sits beside Preferences at the right end of the toolbar.

Tasks contains download actions. View changes the current page and list options. Tools opens settings, browser setup, video previews and the website grabber. Help contains this Guide, Changelog and About. Changelog shows release notes newest first; About shows the full installed version. Open Legal & licenses to read or copy the application’s MIT license and bundled third-party notices offline; Back to About or Escape returns to the information card. License texts retain their original language and wording.

### Useful shortcuts

Ctrl+N opens Add URL. Ctrl+F searches downloads (or logs when the Logs page is open). Ctrl+click selects multiple downloads; Shift+click selects a range. Escape closes dialogs or clears the active search. F1 opens this guide when no dialog or menu is open.

## Downloads & recovery

### Add one file or a batch

Add URL accepts direct HTTP, HTTPS, FTP and FTPS links. Tasks → Add batch accepts one link per line. Choose a queue and destination before adding the files.

For numbered files, such as img001.jpg to img100.jpg, choose From a pattern in Add batch. Enter the link with one * where the number goes, for example https://example.com/photos/img*.jpg, then From and To. Zero padding Auto pads every number to the longer of From and To, so 1 to 100 gives 001 to 100; choose Off for no zeros, or a fixed number of digits. From and To can also be single letters, a to z or A to Z. The preview shows how many links you will get and the first and last one. The * must be in the path or query of the link, and From must not be greater than To. Generated links get the same checks, queue and folder as a pasted list, and one batch holds up to 1,000 links. Link list keeps your pasted links while you use a pattern, and dropped links always open in Link list.

To add links while Download Engine is minimized or in the tray, turn on View → Floating drop box, or Floating drop box in the tray menu. A small box with the Download Engine icon then stays on top of other windows. Drag a link from your browser or another app onto it: one link opens Add download with the link filled in, and several open Add batch. Nothing starts until you add them. Drag the box to move it; it keeps its place and stays on screen. Double-click it to open Download Engine, or right-click it to open Download Engine or hide the box. Using the box never takes focus from the app you are working in.

Expand Advanced for an expected SHA-256 checksum, login and ZIP preview. Enable Use login to enter credentials directly. Supply credentials only when that server requires them. For a company file server, select NTLM or Negotiate and enter DOMAIN\username or user@domain plus the password. Negotiate uses Kerberos when available and may fall back to NTLM. The app does not automatically send your current Windows account. You can also pause an existing file and open Authentication to change its login; changing credentials after partial data was saved requires a confirmed restart. Remote file-server logins require HTTPS.

### Pause, stop, resume and recover

Select a download and use Pause or Resume. The idle, unscheduled Main queue starts when you add or resume a file. Custom or scheduled queues retain their chosen start times. A queue with existing waiting work stays paused until you start it or choose Start immediately for a file.

Stop on the toolbar stops the selected active or waiting files and closes their status windows. Stop is also in each download's right-click and three-dot menus; selecting several files offers Stop selected. Stop all on the toolbar or in Tasks stops current and waiting downloads across every queue, including files started immediately. Both keep saved progress and leave stopped files Paused until you resume them. Queue schedules and future downloads keep their existing settings. Completed files are untouched.

Tasks → Pause all pauses queues and their active downloads, including files started immediately. Starting those queues again continues their waiting work. Stop all marks the existing files individually Paused, so starting a queue does not resume them; choose Resume or Start immediately for those files.

Right-click a queued, paused or failed download, or open its three-dot menu, and choose Start immediately to run that file without opening Manage queues. The existing file keeps its saved progress and queue position. This one-run override bypasses its queue's pause, schedule and usual file limit, but still respects the app's overall 32-download limit and data quotas. Other queued files stay unchanged. Pausing or stopping the file, Pause all, Stop all, or pausing its queue cancels the override; restarting the app also clears it. Selecting several files offers Start selected immediately.

Download Engine saves partial progress. After an interruption, resume the download; reuse of partial bytes depends on the server supporting ranges and the file still matching its saved metadata. An expired link may need Update download link in the download menu. Read the reported error before retrying.

For an expired or changed link, Update download link checks whether saved data can be reused and offers Update and restart when needed. Recovery cannot guarantee that every server or expired URL will support resume.

### Download status window

Double-click a download, press Enter on a selected download, or choose Download status from its right-click or three-dot menu. A separate window shows progress, downloaded size, speed, time remaining, and what the worker is doing. It stays on that file even when you switch queues or pages. Show details includes resume support, transfer ranges or saved video segments, source, and destination.

Single downloads started manually, including browser handoffs and reviewed videos, open their status automatically once running. Turn this off in Preferences → Downloads → Download status window. Batch downloads, queue starts, CLI and automation stay in the background.

Pause keeps the window open. Resume or Start immediately starts only that file, respecting the app's active-download limit and data quotas. To stop a transfer and close its window, choose Pause, then the close button; saved progress is kept, although servers without resume support may need to start again. Hide, Escape and the close button alone leave downloads running. Completed files offer Open file and Open folder. Open file keeps the existing antivirus checks when enabled.

Unknown file sizes cannot show a percentage or time estimate. Video progress counts completed segments, so its percentage may advance in steps. Receiving every byte is followed by assembly, verification or merging; Completed appears only when the file is ready.

Show details displays a position map and per-connection progress for direct files. For YouTube, HLS and DASH, it shows the video and audio segments, completed count, active connections, and each active segment's received bytes. The video map follows playlist order; each segment has equal width because its final byte size may be unknown. Hover for details or scroll the segment list. Resume keeps every verified complete segment, including ones that finished out of order, and restarts unfinished segments.

Video downloads use up to four parallel connections after the first segment or initialization data has been verified. Preferences → Connection → Download connections also controls video downloads: a lower global or matching website limit reduces video concurrency, while values above four do not raise its safety cap. Pause and resume to apply changes to a running download. Direct HTTP/HTTPS files continue to use parallel ranges when the server supports them and provides a strong file validator. Video segments and simultaneous network connections are different counts. Server limits, disk speed, quotas and configured speed limits can still constrain throughput.

### Find a fresh link using your browser

Pause the failed or expired direct HTTP/HTTPS download, then open its menu → Update download link. Under Find a fresh link in the browser, enter the original webpage and choose Open page and wait for link. New links added through the extension remember the source page when the browser provides it; older downloads may need you to paste that page.

Use Chrome, Edge, Brave or Firefox with the updated Download Engine extension. If needed, open the extension popup and choose Allow recovery access. Automatic download capture can stay off. Click the same file's download link on the webpage. The extension looks for a new download with the same filename and known size; an unrelated download is not taken for recovery. Browser-added suffixes such as “ (1)” are accepted. A renamed file or a server with unknown size may require pasting its fresh URL manually.

The browser copy pauses while Download Engine checks the saved bytes. Review Update and resume, Update and start, or Update and restart in the app. Only confirming updates the existing download; its name, save folder and queue position are kept. Confirming wakes an idle, unscheduled Main queue. Other paused or scheduled queues keep their start policy; use Start immediately on the file to run it ahead of that queue. The browser copy is cancelled only after the app saves the update. Cancelling review or exiting the app releases that copy back to the browser. If the app disappears unexpectedly, recovery expires; the extension rechecks while it is running and on its recovery alarm. If browser permissions were removed, use Resume in the browser's Downloads list.

Recovery waits up to 30 minutes. It does not import fresh cookies or login details, handle video playlists or FTP, or bypass access restrictions. Sign-in-only links can still need the existing browser-login workflow. Checking saved data uses download bandwidth.

### Choose connections for each website

Open Preferences → Connection → Download connections. The default is 4 connections per HTTP/HTTPS file. Choose 1, 2, 4, 8, 16 or 32, then optionally add website overrides such as downloads.example.com. Enter a hostname only. Enable Include subdomains when the rule should also cover its child domains; the most specific matching rule wins. If both the original and redirected file hosts have rules, the lower limit applies. If only one has a rule, that rule applies; otherwise the default applies.

Use 1 for servers that reject parallel requests. More connections do not guarantee higher speed. Small files and servers without range support use one connection. The limit applies to each file, not to all downloads from the same server together; queue concurrency controls how many files run at once. Pause and resume an active file to apply changed settings. Existing partial files remain resumable with a lower limit. FTP and video streams retain their own connection behavior.

### Remove history or delete a file

Clear from history removes the record while keeping the downloaded file. Delete file removes the saved file as well; read the confirmation before accepting. Open folder shows where the file was saved.

### Export or import the download list

Tasks → Export list… saves your downloads to a file, to back them up or move them to another PC. Choose all downloads, the current queue or the selected downloads, and optionally leave out completed ones. A Download Engine list (.json) keeps each file's link, name, folder, queue, file type and expected SHA-256. Links only (.txt) writes one link per line, which Add batch and the command line batch command also read. Windows asks before replacing an existing file, and the file is written completely or not at all.

Passwords, cookies, browser logins, proxy settings, partial download data and resume information are never exported. Links are saved exactly as they are, and some sites put temporary access keys in their links, so keep exported lists private. Video streams added with Preview video are left out; preview them again on the other PC.

Tasks → Import list… reads a Download Engine list or a text file of links, up to 16 MiB and 10,000 entries. Before anything is added, a review shows how many downloads are new, how many are already in your list and how many are invalid; Show details lists each skipped line or entry and why. Links already in your list are skipped, so importing never replaces or duplicates a download. Choose the queue for the new downloads; when the list names a queue that exists here, those downloads can go to that queue instead. Queues are never created. Files go to your download folders unless you choose the list's folders, which is offered only for folders that exist on this PC. Imported downloads stay paused until you resume them, or choose Queue them to let their queue start them, like Add batch.

A Download Engine list is UTF-8 JSON. format and version identify it; a newer version is refused rather than guessed at. Each download needs a url; name, folder, queue, category and sha256 are optional, and other fields are ignored. category is informational, because Download Engine sorts files by their name.

```json
{
  "format": "xdownloader.download-list",
  "version": 1,
  "application": "Download Engine 1.16.0",
  "exported_utc": "2026-10-20T09:30:00Z",
  "downloads": [
    {
      "url": "https://example.com/files/archive.zip",
      "name": "archive.zip",
      "folder": "C:\\Users\\Me\\Downloads\\Compressed",
      "queue": "Main queue",
      "category": "compressed",
      "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
    }
  ]
}
```

## Offline websites

### Capture and open a website

Open Grabber in the top toolbar (or Tools → Website grabber) and choose Offline website. Enter the starting page, choose a folder (blank uses your Downloads folder), then choose Save offline copy. A new, uniquely named folder keeps each capture separate from existing files. Open offline website opens its index.html in your default browser. Keep the whole folder together when moving or sharing the copy.

Link depth 0 saves only the starting page and its assets. Depth 1 also saves pages linked from it; the maximum is 5. Page links stay on the same origin (the same scheme, host and port). The defaults are 50 pages and 100 MB; the maximum is 200 pages and 500 MB. Stylesheets, CSS imports, background images, responsive images, icons and fonts are saved and linked locally. Include styles and images from other websites allows public CDN assets; it never crawls another website's pages or reaches a different local network service.

Capture runs immediately while this dialog is open, separately from download queues. Stop capture, closing the dialog or exiting the app cancels network work and finishes a partial copy if the starting page was saved. A failed starting page leaves no copy. Capture does not resume after restarting; start another capture to retry. Each run uses your current Preferences → Connection → Proxy settings, without browser cookies or website logins. Transfer limits for download queues do not apply to website captures; use the capture's size limit.

After saving, View capture report opens report.html with saved files and omissions. Missing or out-of-scope page links lead to this report, rather than silently opening the internet. Original-site links in the report are clearly marked online. Up to 200 detailed notes are retained, with a total count. A report.json file contains the same capture details for inspection.

The copy is static: JavaScript, forms, SVG animations, embedded players and other embedded pages are disabled, so JavaScript-only apps, login-only content, searches, shopping carts and live streams will not work offline. HTML and CSS must be UTF-8. There are also limits of 2 MB per HTML/CSS file, 16 MB per image/font, 20 MB of HTML/CSS, 2,000 assets, 10,000 inspected URLs and ten minutes per capture. Cross-origin redirects, attachment downloads and unsupported file types are reported as omissions. An offline copy is not a guarantee that every page on a website was captured.

### Find files on a website

Choose Find downloadable files, enter a starting page and file types such as pdf, zip or png, then choose Scan website. Select the results you want and choose Add selected to queue. Scanning alone creates no downloads. Save a project to reuse its settings or schedule later scans for review.

Scan depth 0 reads the starting page and same-origin HTML fragments included in it. This includes GitHub's separately loaded release-asset lists. Fragments count toward the page limit. Depth 1 also follows ordinary page links; use a small depth and an appropriate page limit to keep the scan focused. The scanner does not run JavaScript or use browser logins.

### Try the GitHub example

For downloadable releases, enter https://github.com/uBlockOrigin/uBOL-home/releases, use zip, xpi, tar.gz as the file types, set depth 0 and page limit 50, then scan. Review and select only the files you need. To find README.md or images, start at https://github.com/uBlockOrigin/uBOL-home and use md, png. GitHub file-view links are converted to raw-file downloads.

For an offline copy, use the repository home URL in Offline website, set depth 0 and keep Include styles and images from other websites enabled. Open index.html after saving. The repository page, README, styles and images can be read offline; searches, sign-in, live commit details and JavaScript menus remain unavailable. Depth 0 does not copy linked repositories, release downloads or other pages. Uncaptured links open the capture report, which also lists missing source assets.

## Browser integration

### Connect a browser

Open Tools → Browser setup and follow the instructions for Chrome, Edge, Brave or Firefox. Load the matching extension from the app's extensions folder. For Chrome, Edge and Brave, Browser setup fills in the extension ID for the app's own extension folder, so choosing Connect browser is enough; paste the ID your browser shows only if you loaded the extension from another folder. These unpacked extensions can only be used in the browser's Developer mode.

Use Check connection in the extension. If you replace an unpacked extension build, reload the extension and refresh existing video tabs so they use its updated content scripts.

### Capture files and videos

Automatic download capture and Detect media while browsing are on by default after installation. Capture sends supported file downloads you click to Download Engine, following Capture rules. Detection lists media without downloading it; choose which video to send. Saved off choices are preserved. If the desktop app is unavailable, file downloads remain in the browser. Video button on this site controls the button beside supported players; site settings can override its default.

Click Download video beside the player to open the desktop video review. Select quality and audio; the selection is previewed automatically. Then choose Add video to queue.

Enable media detection before playback if the page has no discovered streams. Public, finished YouTube videos, Bilibili videos and Bangumi episodes (up to 480p without sign-in), public TikTok videos and Twitch videos (past broadcasts, highlights and uploads) use the app's extraction workflow. If the app cannot read such a page, it shows the media the browser found on it instead. Live, encrypted and browser-sign-in-only media are unsupported. A video page URL is not a direct file link for the CLI.

### Permission prompts

Version 3.0 uses the Firefox add-on ID download-engine@xetro and native host com.xetro.downloadengine. If you used the older developer add-on, finish any pending handoffs and remove it before installing the new add-on. Firefox treats the new ID as a separate extension, so review its fresh settings and permissions. Your desktop downloads, progress and queues are preserved. Open the updated desktop once to migrate existing browser connections, then use Check connection in the extension. Tools → Browser setup → Connect browser can repair the connection. Older extensions can still connect through the desktop's separate compatibility host; avoid running both Firefox add-ons together.

Allow download and HTTP/HTTPS website access during installation or when the browser asks after an update. Firefox requires version 142 or newer on Windows and discloses transfers to the local app in its data-consent prompt. Firefox for Android is not supported. Login-cookie access stays optional and requires approving a browser-login review. Remove browser login access stops future cookie reads on all sites while media detection keeps working. If the feature remains off, check the message in the popup and Browser options → Diagnostic log. The desktop Logs page records the app side of the connection.

## Queues & schedules

### Organize downloads

The Main queue starts automatically when adding a file to it while idle and unscheduled. Completed history does not block a new download. Individually paused files stay paused. If you have paused a queue with waiting work, adding another file keeps that queue paused.

Open Manage queues → New queue. Give it a name and choose Create queue. Custom queues start paused, so files wait until you start or schedule them, or choose Start immediately on a particular file.

Start and Pause affect that queue. The concurrency setting controls how many files run together. View downloads shows the queue's files. Moving a download to a paused queue leaves it waiting there.

### Schedule a queue

Use Schedule on the queue card, or open Scheduler. Choose a one-time start or recurring days and times, and an optional stop time. Check the displayed next run. The app must remain open, including in the tray, and the computer must be awake.

Manually starting or pausing a queue suspends its recurring schedule. Re-enable the schedule when you want automatic timing again.

### Shut down, sleep or hibernate after downloading

Open the queue's three-dot menu → When queue finishes → Shut down computer, Sleep computer or Hibernate computer. The When done tab of a download's status window offers the same choices for its queue. Add unfinished files first. This is off by default, runs once, and is forgotten when the app exits. Every file in that queue must complete successfully. Failed, paused, retrying or checksum-mismatched files prevent completion; removing or moving a watched file cancels the action.

When the queue finishes, the app opens a 60-second countdown with Cancel shutdown, Cancel sleep or Cancel hibernation. It cancels the countdown if any other download, antivirus scan, script, command or scheduled website scan is unfinished, or a dialog is open. Choose the action again when ready. Save any open work before allowing shutdown, sleep or hibernation. Download Engine asks Windows for an ordinary shutdown, sleep or hibernation without forcing other applications to close; Windows may block it or ask about unsaved work. Sleep and hibernation keep wake timers enabled. Choosing Off or Cancel stops the action while the countdown is visible. Nothing is scheduled in Windows during that countdown.

Sleep and Hibernate appear greyed out, with the reason, when Windows doesn't let apps request them. Modern Standby PCs can't be put to sleep by an app, and hibernation can be turned off or unsupported; an administrator can turn it on with powercfg /hibernate on. After the computer wakes from sleep or hibernation, Download Engine is still open and ready.

Notify and Exit remain separate choices. Exit uses a 15-second countdown and closes only Download Engine. Shutdown, sleep and hibernation require the app to remain open until completion; keeping it in the tray is sufficient. Test them manually when you are ready for this computer to turn off or sleep.

### Synchronization queues

A synchronization queue checks its source again on later runs. Previous copies are kept when a replacement is needed. Use it for files you want to check repeatedly, rather than for a one-time download.

Download at once in a queue's three-dot menu accepts a typed number from 1 to 32. Click Set or press Enter. The scheduler runs at most 32 files across all queues, including Start immediately. Higher limits use more CPU, memory and network connections; the default queue limits are unchanged. Clear finished removes completed entries from that queue's history and keeps their saved files.

## Scripting

### Route new downloads

Open Scripting in the top toolbar beside Scheduler, or Tools → Scripting. Choose New rule, enter a name and leave Enabled checked. Choose A download is added.

Site hostname is an exact host such as www.w3.org, without https:// or a path. File extension is a type such as pdf or zip. Both conditions must match when both are filled in. Blank conditions match every new download.

Choose Then send to queue. Optionally enable a per-download speed limit, in KiB/s. Save rule. Create the destination queue first if it does not exist.

Rules run before a new download starts. The first matching enabled rule wins, in creation order, even if Add URL or the CLI selected another queue. Existing downloads are not moved when a rule changes. Extension matching uses the initial filename, before the server may rename it.

### Test PDF routing

1. Create a queue named PDF test and keep it paused.

2. Create an enabled rule named Test PDF routing: A download is added; Site hostname www.w3.org; File extension pdf; Then send to queue PDF test. Temporarily disable any earlier rule that would also match this file.

3. Open PowerShell in the application folder, beside de-cli.exe, and run:

```powershell
.\de-cli.exe add "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf" --name "automation-test.pdf"
```

4. The file should be Queued in PDF test. Run history should contain a successful routing entry. Start PDF test; the PDF should become Completed. Disable or remove the test rule when you are finished.

### Start another queue after completion

Create a rule with A queue completes. Choose Source queue and Then start this queue. The source must contain downloads, and every remaining file must complete successfully. Failed, paused or unfinished files prevent completion. Empty queues do not trigger it.

The app remembers handled completions across restarts. Editing or re-enabling a completion rule watches from that point; it does not replay an old completion. Loops between queues are rejected.

### Check what happened

Run history shows the latest 500 runs: blue for information and red for errors. A missing destination queue is an error; edit the rule to choose an existing queue. Disabling a rule stops future actions and does not undo actions already completed.

Rules run only while Download Engine is open, including in the tray. They perform built-in app actions. For programmable workflows, use Scripting → Scripts. Use Scheduler for time-based queue starts.

### Write and run a script

Open Scripting → Scripts → New script. The editor starts with a working example. Give it a name, choose Save script, then Run script. Run logs appear below the editor; scroll down in a small window. The fixed Save, Run and Stop bar remains accessible. Information is blue, errors are red and warnings are light red. Copy log copies the output, and All scripts includes history from removed scripts.

New scripts use built-in Lua 5.4. No installation is needed. The Language picker also offers Windows PowerShell 5.1. Both support your own variables, functions, loops and conditions. Changing language keeps custom code; only an unchanged starter example is replaced. Existing PowerShell scripts keep their language.

Tab indents code inside the editor. Press Escape to leave the editor, then Tab to navigate controls.

For Lua, choose Lua guide above the editor or this Guide's Lua scripting topic. It includes a first-run walkthrough, download and completion-trigger examples, the app API and troubleshooting. Copy script copies only the code, ready to paste into the editor.

In PowerShell, Invoke-DownloadEngine returns objects from the CLI and throws on failure. For example, Invoke-DownloadEngine queues lists queues, and Write-Output $Event.trigger prints the trigger. $Event is the PowerShell equivalent of Lua's event.

```powershell
foreach ($queue in (Invoke-DownloadEngine queues)) {
    Write-Output ("{0}: running={1}" -f $queue.name, $queue.running)
}
Write-Output ("Trigger: {0}" -f $Event.trigger)
```

PowerShell runs with your Windows account and can access your files and programs; only run code you trust. Runs use the app's downloads folder and have no interactive prompts.

### Automatic script triggers

Choose a trigger, turn on Automatic trigger enabled and save. New scripts have automatic triggers off. Manual Run works regardless of this switch. Startup runs on the next app launch. Every interval supports 1 to 10,080 minutes and starts counting when you save. Download completed watches all queues or one queue. Queue completed waits until every remaining file in the selected queue is complete; an empty queue or a failed file does not count.

For Download completed, event.downloads holds up to 100 newly completed downloads per run, with id, queue, name and saved_path. Removed records can have missing name/path values. For Queue completed, event.queue identifies the queue and event.downloads is empty; use xd.downloads() to inspect its files. Manual, startup and interval events also have an empty downloads list. PowerShell uses $Event with the same fields.

```powershell
# Choose Download completed as this script's trigger.
foreach ($file in $Event.downloads) {
    Write-Output ("Finished: {0}" -f $file.name)
    if ($file.saved_path) { Write-Output $file.saved_path }
}
```

Triggers need the app open or in the tray. Saving or re-enabling starts watching from that point, without replaying earlier completions. A script never overlaps itself; extra completion events wait for the current run. Missed time intervals coalesce into one run, and intervals missed while the app was closed are skipped.

### Stop, limits and recovery

Stop script terminates its script process and child programs. Downloads and other app commands already submitted continue. The timeout is 1 to 3,600 seconds. Up to four scripts run at once; others wait. Editing a running script changes the next run, not the current saved revision. Reload / discard edits restores the latest saved version. Save your edits before Run.

The app keeps 50 script definitions, up to 64 KiB of source each, and the latest 100 finished runs plus active runs. Each run retains up to 128 KiB of output; excess output is discarded with a warning while execution continues. Avoid printing passwords or tokens because logs are saved in your local app data.

After a crash or app exit, unfinished runs are marked interrupted and are not automatically replayed. A new startup trigger is a separate run on each launch. Inspect run logs and download state before retrying actions. Failed scripts are not retried automatically. Syntax errors, thrown errors and nonzero exit codes appear as failures.

## Lua scripting

### Run your first Lua script

1. Open Scripting in the top toolbar → Scripts → + New script. Keep Language set to Lua and Trigger set to Manual only. Lua is built in; nothing needs installing.

2. Name it Hello Lua. Copy this script, replace the editor's contents, then choose Save script → Run script. This example only reads your queues.

```lua
-- Example: hello
print("Hello from Lua!")
print("Trigger:", event.trigger)
for _, queue in ipairs(xd.queues()) do
    print(queue.name, "running:", queue.running)
end
```

3. Scroll to Run logs below the editor. Expect Succeeded, Hello from Lua!, Trigger: manual, and your queue names. Print separates values with tabs. Copy log copies the displayed output.

To edit a script in another program, use Open file… and Save to file… in the editor's header. Open file… copies a .lua or .ps1 file into the editor and asks first if it would replace unsaved code; choose Save script to keep it. Save to file… writes the editor's code to a file.

Use Scripting in the toolbar to return from this guide to the editor. Your unsaved code is kept. The Language picker replaces only an untouched starter example; custom code is kept, so paste Lua code after switching from PowerShell. Save edits before running.

### Live syntax checks and autocomplete

Lua and PowerShell syntax checks run automatically after a short typing pause, without saving or running the code. The editor shows line numbers, syntax colours and error markers. Drag its bottom resize grip for more editing space. Click an error below the editor to move to its line. No syntax errors means the code parses; runtime errors still appear in Run logs.

Type xd.que to see xd.queues(), or press Ctrl+Space for suggestions. Up/Down selects an entry; Tab, Enter or a click inserts it. Escape closes the suggestions; press it again to leave the editor. Tab indents when suggestions are closed. Ctrl+Z undoes a completion. PowerShell offers Invoke-DownloadEngine and $Event helpers; site readers offer their own web-reading API. Suggestions also include identifiers from earlier in your code, but do not perform full type or scope analysis.

### Lua basics with your downloads

local creates a variable. Text goes in quotes; -- starts a comment. A table can hold a list or named fields. Lists start at index 1: jobs[1] is the first result, and job.name reads its name. ipairs walks a list. if chooses what to do, function defines reusable code, and end closes these blocks. The .. operator joins text.

This read-only example lists downloads and counts completed ones. With no downloads, the count is zero.

```lua
-- Example: inspect
local completed = 0
local function describe(job)
    return job.name .. " - " .. job.status
end
for _, job in ipairs(xd.downloads()) do
    print(describe(job))
    if job.status == "complete" then
        completed = completed + 1
    end
end
print("Completed downloads:", completed)
```

### Add a real download

Create another manual Lua script and paste the example below. It downloads W3C's sample PDF into a queue named Lua practice. It reuses that queue on later runs. Each run adds another download, so run it once for your first test.

```lua
-- Example: download
local url = "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf"
local queue
for _, item in ipairs(xd.queues()) do
    if item.name == "Lua practice" then queue = item end
end
queue = queue or xd.create_queue("Lua practice")
local jobs = xd.add(url, {
    queue = queue.id,
    name = "lua-example.pdf"
})
assert(jobs[1], "No download was added")
xd.start_queue(queue.id)
print("Added download:", jobs[1].id)
```

Save script → Run script, then open Lua practice in the sidebar. Wait for Completed and use Open folder. The script succeeds when its add/start commands are accepted; that is separate from the file finishing. Existing routing rules may choose a different queue. Replace url and name to download your own direct HTTP, HTTPS, FTP or FTPS file. A video watch-page URL needs the browser video workflow instead.

queue.id identifies the queue even if you later rename it. xd.add returns a list; jobs[1].id is the first download ID. A new queue starts paused, so xd.start_queue is necessary. nil means no value; queue or xd.create_queue(...) creates the queue only when it was not found.

### Run automatically when a download finishes

Create a Lua script named Completion log. Paste this example, set Trigger to Download completed, choose All queues or the queue to watch, tick Automatic trigger enabled, then Save script. Finish a new download after saving it, for example with the PDF script above.

```lua
-- Example: completion
if #event.downloads == 0 then
    print("No completed files in this run; use Download completed.")
end
for _, file in ipairs(event.downloads) do
    print("Finished:", file.name or file.id)
    if file.saved_path then print("Saved to:", file.saved_path) end
end
```

Check that script's Run logs for Finished and the saved path. Clicking Run script creates a manual event even when an automatic trigger is selected, so its downloads list is empty. The message above explains that case. #event.downloads gives the list length. Each automatic completion run can contain up to 100 files; removed files may have no name or saved_path.

App startup runs on the next launch. Every interval runs at the selected number of minutes (1 to 10,080). Queue completed waits until every remaining file in a nonempty selected queue is complete; it supplies event.queue and an empty event.downloads list. Use xd.downloads() and compare each job.queue with event.queue to inspect that queue. event.trigger is manual, startup, interval, download_completed, download_failed or queue_completed.

Keep the app open or in the tray for triggers to run. Saving or re-enabling starts watching from that point; earlier completions are not replayed. A script cannot overlap itself. Turn off Automatic trigger enabled and save to prevent future automatic runs; use Stop script to stop an already running one.

### Retry failed downloads and remember between runs

Create a Lua script named Retry failed. Paste this example, set Trigger to Download failed, choose All queues, tick Automatic trigger enabled, then Save script. It runs when a download ends up in Failed, after Download Engine's own automatic retries.

```lua
-- Example: retry
for _, file in ipairs(event.downloads) do
    if pcall(xd.retry, file.id) then
        print("Retrying:", file.name or file.id)
    else
        xd.notify("Download still failing", (file.name or file.id) .. ": " .. (file.error or "no details"))
    end
end
local handled = (xd.store.get("handled") or 0) + #event.downloads
xd.store.set("handled", handled)
print("Failed downloads handled:", handled)
```

Each entry in event.downloads has id, queue, name, error and attempts, the number of times Download Engine tried. xd.retry continues from where the download stopped; xd.restart starts it over. Scripts can retry or restart each download 5 times until it completes, so the sixth xd.retry fails and this example shows a Windows notification instead. pcall catches that failure. Clicking the notification opens the run's log.

xd.store keeps values between runs of this script, also after a restart. xd.store.get(key) returns a value or nil, xd.store.set(key, value) saves a number, text, true or false, or a table, and setting nil removes the key. xd.store.keys() lists the keys. The script settings show Stored values with Show and Clear.

To try the script without waiting for a failure, save it and choose Run with sample…. Tick the downloads that need attention and choose Run sample. The run gets the same event.downloads, event.sample is true, and Run logs label it Sample. Commands that would change something, such as xd.retry, xd.store.set and xd.notify, only write a line starting with Would; nothing changes.

### Write logs and handle errors

print and info are blue, warning is light red, and error is red. An error log is a message; it does not fail the run. error(...) or assert(false, ...) fails it. pcall returns true on success or false and the error on failure, so you can handle a problem and continue.

```lua
-- Example: logging
xd.log("info", "Started")
xd.log("warning", "This is a warning")
xd.log("error", "This is an error message")
local ok, message = pcall(function()
    error("Example failure")
end)
if not ok then xd.log("warning", "Handled: " .. tostring(message)) end
print("Still running")
```

This example should finish as Succeeded, with all three colours and Still running in its log. Remove pcall when an error should stop the script. Script output lives under Scripting → Scripts → Run logs; the toolbar's Logs page contains application diagnostics instead.

### Download Engine Lua API

xd.queues() returns queue tables with id, name, running and max_active. xd.create_queue(name) returns a new paused queue; duplicate names fail. xd.start_queue(id_or_name) and xd.pause_queue(id_or_name) control one queue.

xd.downloads() returns download summaries; xd.status(id) returns one summary. Fields include id, name, status, downloaded, total, queue, directory and error. total and error can be nil. Status strings include queued, downloading, paused, complete and failed. saved_path belongs to completed event entries, not download summaries.

xd.add(url, options) returns added download summaries. Optional queue, folder and name values must be strings. folder must be an absolute path inside your configured download folder or a category folder, including their subfolders; a Lua Windows path can use a long string such as [[C:\Downloads]]. Linked or redirected folders are refused. These limits also apply when Can change downloaded files is enabled. Omit folder to use the app's folder rules. Unknown options fail.

xd.pause(id), xd.resume(id) and xd.pause_all() submit controls. Resume wakes an idle, unscheduled Main queue. Other paused queues still need xd.start_queue. Commands can complete asynchronously; check xd.status for the resulting download state.

xd.retry(id) tries a download that needs attention again and xd.restart(id) starts a download over. xd.notify(title, text) shows a Windows notification, at most 5 per run; Preferences → Startup & tray → Show notifications from scripts turns them off, and the run log always keeps them. xd.rename(id, name), xd.move(id, folder) and xd.remove(id, {delete_file = true}) change a finished download's file. They work only when Can change downloaded files is ticked in the script's settings. Moves stay inside your download folder and category folders, and nothing is ever overwritten.

xd.sleep(milliseconds) waits from 0 to 60,000 milliseconds per call. It counts toward the script timeout. Use a delay when polling status rather than a busy loop. xd.command passes CLI arguments as separate strings; for example xd.command("queue", "limit", "main", "2") changes Main queue's concurrency. Supported command roots are list, status, add, pause, resume, pause-all, queues, queue, request, cancel, automation, retry, restart, rename, move, remove, notify and state. See the packaged docs/automation.md for CLI argument details.

### Stop, limits and troubleshooting

Timeout is 1 to 3,600 seconds. Stop script, timeout and app exit terminate the script worker, including an infinite loop. App commands and downloads already submitted continue; pause those separately if needed. Each run uses the saved revision; editing does not change a running script.

Lua includes table, string, math, UTF-8 and coroutine libraries. It has no io, os, require, package, debug, load, loadfile or dofile. Use optional PowerShell for general Windows file or program access. Each Lua VM is limited to 64 MiB; scripts to 64 KiB; retained output to 128 KiB per run. The latest 100 finished runs are retained. Variables reset for every run; use xd.store to keep values between runs.

Run disabled: save changed code first and wait for the current run to finish. No output: inspect the selected script's Run logs and scroll down. No automatic run: enable the trigger, save, keep the app running and cause a new event. A manual run has no completion files. A red message with Succeeded: logging an error does not throw it. Syntax or command failure: read the failed run's error and line number; check quotes, matching end statements, IDs and option names. After a crash, check the interrupted run and download state before running again.

### Site readers: add your own video sites

A site reader is a Lua script that teaches Download Engine a video site it does not know.

1. Open Scripting → Scripts → + New script. Keep Language set to Lua and choose Trigger → Site reader. The editor switches to a starter reader.
2. Under Pages, add the pages it reads, such as `example.com/watch/*`, and choose Add page. `*` matches anything; `*.example.com` covers every subdomain. Leave Automatic trigger enabled checked.
3. Write the reader. `event.url` is the page. Read it, or the site's own API, with `xd.fetch`, and return what you found:

```lua
local page = xd.fetch(event.url)
local stream = page.body:match('"(https://[^"]-%.m3u8[^"]*)"')
if not stream then error("No video stream found on this page") end
return {
    title = page.body:match("<title>(.-)</title>"),
    items = { { kind = "hls", url = stream, label = "All qualities" } },
}
```

4. Choose Save script, enter a page under Test on URL and choose Test. The result lists what the reader found; nothing is downloaded. Errors appear in Run logs.

From then on, Download video, Find videos on this page, Add URL and Preview video open matching pages with your reader. One stream goes straight to the quality choice; files or several streams open the Video downloads list. If the reader fails on a page opened from the browser, Download Engine shows what the browser found instead. YouTube, Bilibili, TikTok and Twitch always use the built-in readers.

Site readers can read web pages but cannot see or change your downloads. Their requests and returned downloads reach the public internet only: home-network and local addresses are refused and browser cookies and logins are never sent by the reader. Choose Preferences → Connection → Proxy → Direct connection to use a reader or download its results; System and custom proxies are refused because they cannot guarantee the same address checks. Redirects to another origin drop script-supplied headers and cannot forward a POST body. The full reference is in docs/automation.md.

## Command line

### Open PowerShell in the app folder

Find the folder containing de.exe and de-cli.exe. Open PowerShell there. Run the following read-only commands to see the available commands and current queues:

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

### Download a test PDF

This example creates a paused queue, adds W3C's sample PDF, then starts the queue. Use a new queue name if CLI test already exists. Enabled automation rules can override the requested queue.

```powershell
.\de-cli.exe queue create "CLI test"
.\de-cli.exe add "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf" --queue "CLI test"
.\de-cli.exe queue start "CLI test"
.\de-cli.exe list
```

### Read a download ID and status

Commands return JSON. Read commands use result; commands that change state use receipt.result. A successful add returns an array of added downloads. For an existing download, copy its id from list and run status followed by that ID.

```powershell
$downloads = (.\de-cli.exe list | ConvertFrom-Json).result
$downloads | Format-Table id, name, status, queue
```

### Add a batch or control a queue

Create a UTF-8 file with one direct URL per line, then pass its absolute path to batch. Add --queue followed by a queue name or ID. Blank lines are ignored. A batch accepts up to 1,000 links and rejects the whole batch if one link is invalid.

export followed by a new file name ending in .json or .txt saves the download list, or one queue with --queue. It reads the list without starting the desktop and never replaces an existing file. Import the result with Tasks → Import list…, or pass a .txt list to batch.

```powershell
.\de-cli.exe export "C:\Backup\downloads.json"
.\de-cli.exe export "C:\Backup\work-links.txt" --queue "Work"
```

queue pause followed by a queue name pauses it; queue start starts it. pause or resume followed by a download ID controls one download. Resume wakes an idle, unscheduled Main queue. Other paused queues must be started before their files can resume.

### Script behavior

Commands that change state launch the desktop if needed. Read commands do not. Both use the same local data as that copy of the app. The CLI is not a separate download service.

Exit code 0 means success, 1 means failure or rejection, and 2 means the request is still running or has an uncertain outcome. Check the exit code and command receipt before retrying. Reuse --request-id with the same UUID and command to avoid duplicate submissions while the receipt is retained.

Direct HTTP, HTTPS, FTP and FTPS file URLs are supported. Use the browser or desktop video review for YouTube, Bilibili, TikTok, Twitch and HLS/DASH quality selection. Keep passwords and private tokens out of command lines and shell history. The packaged docs/automation.md contains the full command and receipt reference.

## MCP integration

### Connect a local AI client

MCP lets a compatible AI client inspect and control Download Engine through named tools. It uses localhost HTTP and keeps running when clients disconnect. Open the MCP popup on the top toolbar, Tools → MCP control, or Preferences → Browser & AI → AI connection (MCP). Close it with × or Escape to return to the same page or Preferences draft. Closing the popup keeps the server running and retains pending MCP edits for the next time you open it; use Save settings or Start server to apply them.

Keep port 17842 or choose an unused port. Select View only for inspection, or Allow changes for download, queue and saved-script controls. Click Start server; this also saves your settings. Wait for Running, then click Check connection to verify authenticated access. This checks the local server; your AI client still needs to connect separately.

Once running, click Copy client config and paste into your AI client's MCP settings. For manual setup, choose Streamable HTTP, use the copy button beside Server URL, add a header named Authorization, and use Copy access header for its value. The URL can be copied before starting; configuration and access-header copying and connection checks appear when running. The configuration contains a private token; keep it out of chat and public files. The AI client must run on this computer and support HTTP headers. Client formats vary; the app does not modify client settings for you. Click the suggested first prompt to copy it. Scroll inside the popup at smaller window sizes to reach the lower controls. Setup guide and Open logs stay below the scrolling area when opened from the toolbar or Tools menu.

MCP is off by default with View only selected. Start with Download Engine enables automatic startup after you save. It runs while this copy is open, including in the tray, and stops when the app exits. Stop server disconnects AI clients; accepted downloads and scripts keep running. Stop before changing port or access, then restart and reconnect the client. To disable future automatic starts, uncheck Start with Download Engine and save.

### Optional terminal setup

The terminal server is independent of the page. Open PowerShell beside de.exe and de-cli.exe:

```powershell
.\de-cli.exe mcp
```

Leave that terminal running. The address is http://127.0.0.1:17842/mcp. Open a second terminal in the same folder and print the connection settings:

```powershell
.\de-cli.exe mcp-config
```

Add the printed download-engine entry to your client's MCP settings. CLI flags do not read or overwrite the MCP popup settings. The terminal defaults to port 17842 and allows changes. Stop the desktop-owned server before starting the terminal server on that same port.

Start mcp --read-only to allow inspection only, or mcp --no-start if you want to open the desktop manually. Otherwise mutations launch the desktop when needed; reads do not. Use --port 17843 on both mcp and mcp-config if the default port is occupied. Press Ctrl+C in the server terminal to stop it. Disconnecting a client does not stop the server or accepted downloads.

### Try it and give the AI its guide

Ask your AI: Read Download Engine's get_usage_guide, then show my queues and paused downloads. The built-in tool returns the AI guide that ships with this version. A copy is in docs/mcp-ai-guide.md, ready to give to another AI. Download summaries omit stored URLs and authentication fields; names, script source and logs can still contain private or untrusted text.

With Allow changes enabled, ask: Create a paused queue named MCP practice and add this direct file URL to it. Replace the URL with one you want to download. Review the proposed tool calls in your client. Ask it to start MCP practice when ready, then check until the file is complete. A successful command receipt means the command was accepted; it does not mean the file has finished downloading.

For Lua: save a script under Scripting → Scripts first. Ask the client to list your scripts and show the script you want to run. After reviewing its source, ask it to run that saved script once and read its run logs. Manual runs have no completed-download event files. PowerShell scripts can access files and execute programs, so authorize those calls with that behavior in mind. MCP does not provide a tool to upload or edit scripts, delete downloads, or open downloaded programs.

### Profiles, retries and cancellation

Use the CLI beside the app whose history you want to control. It follows portable.json or the installed profile. An absolute DOWNLOAD_ENGINE_HOME selects a test profile; set it in both terminals before generating configuration and starting the server. Use a separate port for each simultaneous profile. Tokens are encrypted for your Windows account in the profile's mcp-token.dpapi file.

Changing tools accept request_id, a UUID your client can reuse for the same operation after a lost reply. The MCP JSON-RPC ID is a separate transport identifier. get_request checks a saved command receipt. Do not blindly submit a second add with a new UUID: it can create another download. Receipts retain the latest 1,000 commands; safe retry depends on that receipt still being retained.

Cancelling a call can cancel a command still waiting; work already claimed by the desktop is not undone. Use pause_download or pause_queue to stop transfers, and stop_script for a running script. Inspect the returned receipt and actual state before retrying. Modern clients cancel by closing the response stream; older clients send a cancellation notification. List tools support offset and limit, up to 100 items per page.

### Connection troubleshooting

Connection refused: start the server and check the address. 401: copy the current profile's full Authorization header. 403: use the exact local address; remote origins are rejected. 405 in a browser is expected because this is an MCP endpoint, not a webpage. 429: wait for active calls and retry with the same request ID. An expired older-client session needs to initialize again.

If the port is occupied, stop the other server or choose another port on the MCP popup. Download Engine never takes over another listener. Copy the new configuration and reconnect your AI client. If tools only allow inspection, stop the server, select Allow changes, start it again, and reconnect the client.

### Recover an unreadable token

An unreadable token may be damaged or belong to a different Windows account. Use the original Windows account if possible. To reset it, open Logs → Open log folder and go one folder up to the profile's data folder. Choose Tasks → Exit (or Exit in the tray menu) and stop every terminal MCP server for that profile. Remove only mcp-token.dpapi; keep the database and downloads. Open Download Engine, start MCP, and use Copy client config to update your AI client with the new token. Existing client tokens will no longer work. Do not copy the private token into chat or logs.

Stop the MCP server and close Download Engine before replacing executables during an update. The complete connection and protocol guide is in docs/mcp.md.

## Antivirus scanning

### Choose a scanner

Open Preferences → Antivirus → Antivirus scan. Choose Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV, or a compatible custom command-line scanner. Only the selected scanner's help and app actions appear. Check setup validates the saved-path/configuration requirements without starting a scan or changing your protection settings. Integration found does not prove that the antivirus engine, signatures, or license are ready; a scan checks availability.

Malwarebytes, Kaspersky and Bitdefender consumer integrations use the installed English Windows Explorer scan action in the standard Program Files installation. Explorer must be running. Kaspersky uses Scan for viruses; the others use Scan with Malwarebytes or Scan with Bitdefender. The action must be exposed as an individual Explorer command. A submenu-only, localized, missing or ambiguous action is reported as unavailable; use the antivirus app to scan manually in that case. Download Engine never falls back to opening the downloaded file. These adapters do not automate Bitdefender BEST or Kaspersky Endpoint policy management.

Automatic requests are optional and off by default. Try Tools → Antivirus Scan → Single with one harmless completed text file first. Review its result in the antivirus app. For Batch, wait for each scan to finish before choosing Scan next file. A request is never shown as No threats found. To test Stop batch, stop during a review pause and confirm no next request starts. Stop any scan already running in the antivirus app itself.

### Scan one download

Open Tools → Antivirus Scan → Single. Choose one completed download and select Start scan. If a completed download is selected in the main list, it is preselected here. Cancel scan stops this request; it does not cancel unrelated scans or open the file. For Malwarebytes, Kaspersky or Bitdefender, review the result in its app and select Finish review; a request is not a verified clean result.

Single and Batch share the same dialog. You can switch modes before scanning or after the current run finishes; switching to Single keeps at most one selected file. Tabs are disabled during scanning, cancellation and external antivirus review. New scan refreshes the completed-download list after a run. Saved results remain available through each download’s Antivirus scan / result action.

### Scan several downloads

Open Tools → Antivirus Scan → Batch to scan completed downloads from any queue. Tick the files you want, or use Select all. Clear selection unchecks every file. Start scan uses the scanner configured in Preferences → Antivirus → Antivirus scan; it does not enable automatic scanning or open any files.

The progress bar shows how many selected files have been processed. Files run one at a time. Each row shows its result; hover over a shortened message for details. Missing files and scanner errors never count as a clean result. Downloads removed, restarted or changed after selection are skipped. A file already queued for another scan is also skipped.

Stop batch cancels the active scan and prevents the remaining files from starting. Closing the dialog or pressing Escape also stops the batch. Other automatic scans are unaffected. Finished reports remain with each download: right-click it and choose Antivirus scan / result. Opening the batch dialog again starts a fresh selection.

Malwarebytes, Kaspersky and Bitdefender use external scan requests: Download Engine shows Scan requested, not No threats found. The Explorer action returns before the scan finishes and does not tell Download Engine if the antivirus rejected a request because another scan was running. To avoid overlapping requests, the batch pauses after each file. Open the selected antivirus, wait for the scan to finish and review its result, then choose Scan next file. If the antivirus said another scan is in progress, wait until it finishes and choose Retry this file instead. Finish batch confirms review of the last file. A failed request also pauses for retry or review.

Stop batch cancels the remaining requests and ends the review pause; it cannot stop a scan already handed to the antivirus. Closing the dialog has the same effect. Defender, ClamAV and configured command-line scanners wait for their scanner process and continue automatically.

## Preferences & logs

### Display language

Choose Preferences → Appearance → Language to use English, Simplified Chinese,
Traditional Chinese, Malay, Japanese, Korean, Vietnamese, Thai, Polish, Russian,
Ukrainian, Arabic, German, French, Swedish, Norwegian Bokmål, Swiss standard German, Italian or Romansh. Changes apply after Save and persist across
restarts. System language follows Windows, with English fallback. The browser
extensions have their own language selector under Browser options. Setup offers
the same nineteen language choices and seeds fresh installed profiles while preserving an
existing app choice. Files, credentials, custom names, scripts and API identifiers
are unaffected. See docs/localization.md for font support and translation scope.

### Appearance and destinations

Preferences controls theme, row density, the details panel, sidebar labels and table columns. Use Save to apply these choices. Download folders sorts newly added files by type; changing these settings does not move existing files.

On every launch, an installed copy checks the current Downloads location registered with Windows, including a folder moved to another drive. New profiles use its Download Engine subfolder; older profiles keep the IDM EX name. Restart the app after changing the Windows Downloads location. Portable copies resolve their default downloads folder beside the app each time they start. Explicitly chosen folders and existing downloads retain their saved paths, so check custom paths when moving to another PC. Folder lookup errors are shown instead of silently using a different location.

Speed limits cap transfers. Data quotas limit how much data may be transferred in an hour or day. Connection configures the proxy used for HTTP and HTTPS downloads, HLS/DASH previews and segments, YouTube lookups, and manual or scheduled website scans. Browser integration connects extensions to this copy of the app.

### Tray behavior

Startup & tray controls background operation and completion messages. Closing the window can leave Download Engine running in the tray. Use Exit from its tray menu to stop the app completely. Downloads, schedules and automation require the app to be running.

Play a sound when downloads finish is off by default. When it is on, Download Engine plays one Windows sound when nothing is left downloading or waiting to start, including automatic retries, rather than one sound per file. If a file in that batch needs attention, it plays a warning sound instead, but only while Notify when a download needs attention is on. The sounds follow your Windows Sound settings, so an event set to None stays silent, and they stay quiet over full-screen apps and presentations.

### Read diagnostics

Open Logs in the top toolbar beside Preferences. Messages are selectable text with level badges: errors are red, warnings are amber and information is blue. Use search and filters to find an event. Open the log folder or export diagnostics from this page.

Scripting → Run history records rule actions; Logs records application and connection events. Extension popup → Browser options → Diagnostic log contains browser-side events. Include the error and approximate time when reporting a problem.

### Company logins and proxies

Open Preferences → Connection → Proxy. Choose Direct, System proxy, or Custom proxy. A custom proxy applies to all HTTP/HTTPS destinations and never falls back to a direct connection after a failure. Start a new preview or scan, or pause and resume an active download, after changing its route. FTP/FTPS remains direct.

For a company HTTP proxy, enter its http://host:port address, choose NTLM or Negotiate (Kerberos / NTLM), choose Enter login and supply DOMAIN\username or user@domain and a password. For a remote proxy without transport encryption, explicitly allow that trusted proxy connection. NTLM/Negotiate uses challenge responses; an HTTPS file stays encrypted inside the proxy tunnel. Passwords are saved separately from website logins in Windows Credential Manager.

Enterprise authentication requires Windows and Direct connection or a Custom http:// proxy. Choose one of those in Preferences → Connection → Proxy before using a company file-server login. Automatic System proxy discovery and custom HTTPS/SOCKS proxies are not supported on the same enterprise-authenticated request. Basic authentication keeps the existing System, HTTP, HTTPS and SOCKS5/5h options. Through an enterprise proxy, a destination without its own file-server login must use a hostname, not localhost or an IP address. An unauthenticated proxy used with an enterprise file-server login must also use a hostname. This prevents Windows from selecting the current account implicitly for IP-based endpoints.

Use only a trusted proxy: it can resolve destination hostnames itself. The app still rejects disallowed local video destinations and off-site scan redirects, but a proxy owns the final network route. Website scans and video streams do not inherit website passwords or browser sessions; the new login support is for direct files and the proxy connection. Kerberos needs a reachable domain and a correctly registered server name; use its hostname, not an IP address. A 401 concerns the file server and a 407 concerns the proxy. Full domain sign-in must be checked in your organization; local tests cannot prove its configuration.

## Troubleshooting

### A download is queued but does not start

An idle, unscheduled Main queue starts automatically when adding or resuming a file. Otherwise, check that its queue is running, a concurrency slot is free, and no quota or schedule is holding it. An automation rule may have routed it into a different queue. Right-click the file and choose Start immediately to run it ahead of its queue; the overall 32-download limit and data quotas still apply.

### A rule did not run

Make sure it was enabled before the download was added. Check the exact hostname and initial file extension. An earlier matching routing rule takes priority. Read Run history for a missing queue or other error. Rules do not apply retroactively.

### A browser cannot connect

Check Browser setup and the extension ID for the correct browser. Reload the extension after an update, refresh the page, and use Check connection. Look at both the extension Diagnostic log and desktop Logs for the specific error.

### A CLI command timed out

Check its receipt state. A still-pending request is cancelled when the wait ends. A running request cannot be undone by closing PowerShell. Use request followed by its request ID to inspect the outcome before retrying. --wait accepts 1 to 120 seconds; the default is 15.

### The CLI shows different downloads

Run the CLI beside the copy of Download Engine that you use. Installed and portable copies may have different data folders. A DOWNLOAD_ENGINE_HOME environment variable selects a separate profile; remove it from your shell when you are finished with an isolated test.

### Downloads fail or a link expires

Read the download error and Logs. Check your connection, proxy, free disk space and server access. Retry only after fixing the cause. Use Update download link for an expired URL when offered. Some servers do not support resuming and require a restart.

# Changelog

Release notes are available offline in Help → Changelog. Newest releases appear first.

## 3.0.1 — 2026-10-27

### Browser extension setup guidance

The README now makes it clear that browser-store approval is pending. Chrome, Edge and Brave currently require Developer mode and Load unpacked; Firefox uses Load Temporary Add-on and must be loaded again after each browser restart. Browser-specific loading and connection steps are linked directly from the setup section.

## 3.0.0 — 2026-10-27

### Download Engine browser identities

Browser integration now uses native host com.xetro.downloadengine and Firefox add-on ID download-engine@xetro. Firefox treats the new ID as a separate extension: finish pending handoffs, remove the old developer add-on, then install the new one and review its fresh settings and permissions. Desktop downloads, saved progress and queues are preserved.

The desktop migrates its own existing browser connections and keeps separate compatibility hosts for older extensions. Registration changes roll back on failure; disconnect and uninstall remove only this copy’s mappings. Other installed or portable copies are left alone.

### Legal and licenses in About

About now includes a Legal & licenses reader for the application’s MIT license and bundled third-party notices. Read and copy the original texts offline, switch between documents, and return to About without changing your downloads. Controls follow the selected interface language and fit both themes.

### Documentation

Expanded the README with a larger logo, feature overview, supported video sources and setup/automation guidance. Security documentation now explains browser permissions, local data, MCP access and vulnerability reporting. Automatic capture defaults and Firefox installation guidance match the shipped extension.

Source packages omit obsolete mockups. Git checkout rules preserve generated catalogs and bundled license/source bytes on Windows.

## 2.1.2 — 2026-10-27

### Firefox submission compatibility

Firefox extension packages now require Firefox 142 or newer, resolving Mozilla's data-consent compatibility warning. Firefox 140/141, including ESR 140, no longer meet the minimum. The extension still requires the Windows desktop app and does not advertise Android support. Release checks guard the minimum version and desktop-only manifest.

## 2.1.1 — 2026-10-26

Updated third-party components with upstream safety fixes for asynchronous I/O and HTML clipboard handling.

### Browser integration

Firefox submission packages now include the required data-transmission declaration. Firefox 140 or newer shows consent for URLs and website content sent to the local desktop app; browser-login authentication remains optional and requires approval. Release checks reject missing or misleading declarations.

File-download capture and passive video detection now default on across Firefox, Chrome, Edge and Brave after installation permissions are accepted. Saved off choices and capture rules are preserved. Video downloads still require selection; a missing desktop connection leaves file downloads in the browser. Website and download access are requested during installation, so an update may require browser approval. Removing browser-login access revokes cookie access on all sites while keeping video detection available.

## 2.1.0 — 2026-10-26

### Live scripting editor

Lua and PowerShell now check syntax automatically after a short typing pause, without saving or executing the draft. Errors mark their line and can be clicked to move the cursor. The resizable editor adds line numbers and syntax colours.

Autocomplete suggests app helpers, event fields, common language functions and identifiers from your buffer. Site-reader suggestions follow their separate API. Type a prefix or press Ctrl+Space, use Up/Down to choose and Tab, Enter or a click to insert; Escape dismisses suggestions. Completions support Undo/Redo and preserve surrounding code. Syntax checks are cancellable and discard stale results; runtime errors remain in Run logs.

## 2.0.3 — 2026-10-26

### Refresh the default download folder

Installed copies now ask Windows for the current Downloads folder on every launch, including folders redirected to another drive. New downloads follow that location; existing downloads, saved progress and explicitly chosen folders keep their locations. Portable defaults are resolved relative to the app each time it starts. Older profiles retain the IDM EX subfolder name. If Windows cannot provide the folder, the app reports the error instead of guessing a different destination.

## 2.0.2 — 2026-10-26

### MCP control popup

MCP now opens in the shared popup from the toolbar, Tools menu or Preferences, keeping the current page in place. Close it with × or Escape; a running server stays available and pending settings remain in the editor until you reopen it. Use Stop server to disconnect AI clients. Smaller windows scroll the controls. When opened from the toolbar or Tools menu, Setup guide and Open logs remain accessible at the bottom. Opening MCP from Preferences returns to the same Preferences draft when closed.

## 2.0.1 — 2026-10-26

### Preferences popup

Settings is now Preferences. Open it from the toolbar, the first item in Tools, or the tray menu. The compact popup keeps your current page behind it and groups controls into Appearance, Downloads, Connection, Browser & AI, Antivirus, and Startup & tray.

Appearance, language, table columns, download-status and clipboard choices, startup, and notifications now wait for Save. Preview a theme, move between categories, and use Cancel, × or Escape to discard pending edits. Reset restores only the current category's draft. Detailed connection, folder, speed, quota, browser, MCP and antivirus editors retain their own saves and return to Preferences when closed.

## 2.0.0 — 2026-10-22

### XDownloader is now Download Engine

The app has a new name in its windows, menus, Setup, browser extension and guides. The programs are now de.exe, de-cli.exe and de-host.exe, and release files start with DownloadEngine. The icon is unchanged.

### Upgrading keeps your setup

Setup upgrades your installation in its current folder and keeps queues, history, unfinished downloads, saved site sign-ins and settings. It replaces the old shortcuts and moves Start with Windows and connected browsers to the new programs, so the browser extension does not need to be reinstalled. A portable copy updated in place moves its browser connections the next time it opens. New installations use Download Engine folders for data and downloads; existing profiles keep their folders.

### Update scripts and AI clients

Replace idm-ex-cli.exe with de-cli.exe in scripts and MCP client settings. MCP setup now names the server download-engine. Use DOWNLOAD_ENGINE_HOME to select a separate profile; IDM_EX_HOME still works. Downloads identify themselves to websites as DownloadEngine/2.0.0.

## 1.22.4 — 2026-10-22

Updated the bundled media helper to fix a memory disclosure in HDR10+ metadata handling. Normal HLS/DASH merging remains unchanged.

## 1.22.3 — 2026-10-22

The Scripting page now shows the messages that confirm Open file… and Save to file… in the script editor. Previously they stayed hidden and appeared later on another page.

## 1.22.2 — 2026-10-22

Fixed security issues in scripting. Scripts can now add downloads only to your configured download folder, category folders, or their subfolders, even when Can change downloaded files is enabled. Paths that escape these folders or use linked destinations are refused. Script add and move commands also distinguish different Windows folder names that Unicode lowercasing can confuse. Running a script with a sample event can no longer cancel real pending app commands. Normal command-line downloads and cancellation by live scripts keep their existing behavior.

Hardened the bundled media helper against an MP4 parsing denial of service. Unsupported DRM protection metadata is rejected before allocation, and encryption metadata cleanup no longer uses recursion. Clear HLS, DASH and MP4 media remain supported.

## 1.22.1 — 2026-10-22

The script editor can now open and save script files, so you can write scripts in another editor. Open file… in the editor's header copies a .lua or .ps1 file into the editor and sets the language from the file, and a new script also takes the file's name. Nothing is saved until you choose Save script, and if the editor holds unsaved code, XDownloader asks before replacing it. Save to file… saves the editor's code as a .lua or .ps1 file. Files up to 64 KiB in UTF-8 or UTF-16 work.

## 1.22.0 — 2026-10-22

### Scripts that react to failed downloads

A new trigger, Download failed, runs a Lua or PowerShell script when a download ends up in Needs attention after XDownloader's own automatic retries. The script receives each download's name, queue, error and number of attempts, and can try it again with xd.retry or start it over with xd.restart. Scripts can retry each download five times until it completes, so a script cannot keep retrying forever.

### Try a script on a sample event

Run with sample… runs a Download completed, Download failed or Queue completed script on downloads you choose, with the same event it would receive for real. Sample runs change nothing: commands that would retry, rename, notify or store something are written to the log as lines starting with Would. Run logs label these runs Sample. PowerShell's own commands still run, and the picker says so.

### Notifications, stored values and file actions

Scripts can show a Windows notification with xd.notify, up to five per run. Clicking it opens the run's log, and Settings → Tray & notifications → Show notifications from scripts turns them off. xd.store keeps values between runs, also after a restart; the script settings show Stored values with Show and Clear. With the new Can change downloaded files switch on, a script can rename, move and remove its downloads with xd.rename, xd.move and xd.remove. Moves stay inside your download folder and category folders, and an existing file is never replaced. The switch is off for every script until you turn it on.

The command line adds retry, restart, rename, move, remove and scripts sample, which PowerShell scripts can also use through Invoke-XDownloader. The Trigger and Queue lists in the script settings are now wide enough for their names in every language.

## 1.21.2 — 2026-10-22

Fixed three security issues in custom Lua site readers. Videos and files returned by a reader now stay on the public internet during preview and download, including after a restart; a reader cannot make XDownloader contact this computer or the home network through a returned video address. Redirecting a reader request to another site no longer forwards its authorization or custom API-key headers, and redirects that would forward a POST body to another site are refused. Readers and their downloads now require Direct connection in Settings → Connection, because proxy routes cannot guarantee the same address checks. Ordinary downloads and built-in video sites keep their existing proxy support.

Re-add downloads queued by a custom reader before 1.21.2 to apply the saved restriction; older records did not retain their reader origin.

## 1.21.1 — 2026-10-22

Buttons, text boxes, drop-down lists and number boxes now share one height across XDownloader, so controls in a row line up and forms take a little less room. The main action at the bottom of each window and buttons such as + New queue stay larger. Checkboxes are now small squares, so they no longer look like the round options beside them. In a small window, the toolbar uses one text size for every label, and labels that wrap onto two lines start level with the others. The strip kept beside the download list for its scroll bar now matches the list header instead of looking like a gap, and the connection rows in a download's status window end evenly.

## 1.21.0 — 2026-10-21

### Add your own video sites

You can now teach XDownloader a video site it does not know with a site reader: a short Lua script that reads the page and returns its videos. In Scripting → Scripts, create a Lua script, choose Trigger → Site reader and add the pages it reads, such as example.com/watch/*. Pages it covers then open with your script from Download video, Find videos on this page, Add URL and Preview video. One stream goes straight to the quality choice; files or several streams open the Video downloads list. Test on URL tries a reader on a page without downloading anything, and its output appears in Run logs. If a reader fails on a page opened from the browser, XDownloader shows what the browser found instead. YouTube, Bilibili, TikTok and Twitch keep using the built-in readers.

### Site readers stay away from your downloads

A site reader can read web pages, but it cannot see or change your downloads, and other scripts cannot read the web. A reader reaches the public internet only: addresses on your home network or this computer are refused, your browser cookies and logins are never sent, and every run has limits on the number of requests, their size and their time. Site readers never run on a schedule or from Run script.

### Automation is now Scripting

The Automation page in the toolbar and the Tools menu is now called Scripting, and so is its guide topic. Rules, scripts and run history work as before, and the command line keeps its automation commands. The How rules work button now opens the right guide topic in every language.

### Tidier layouts

Buttons, boxes and switches now line up with each other and with the edges of their panels, also in longer languages such as German and Japanese. In Settings, every switch, arrow, language list and theme choice ends at the same edge. Queues and Scheduler keep each queue's buttons inside its card, so the page no longer runs past a small window. Add download lines up Paste, the queue list and Browse…, and every folder box leaves room for Browse… in any language. Speed limits and Data quotas put their boxes in one column, Table columns lines up its Auto buttons and puts Save on the right, and Video downloads lines up its file names. In the browser extension, found media keep their names in one column and Find videos on this page now looks like the button below it. In Arabic, page and link addresses sit under their titles and long file names show their beginning. Reload the XDownloader extension in your browser after updating.

## 1.20.0 — 2026-10-21

### Twitch videos

XDownloader can now download Twitch videos: past broadcasts, highlights and uploads. Click Download video on a twitch.tv/videos/… page, or paste such a link into Add URL, then choose a quality. Newer broadcasts are saved as MP4 and some older uploads as MPEG-TS (.ts) files, exactly as Twitch streams them. Pausing and resuming works without visiting Twitch again. Subscriber-only videos, live channels and clips are not supported yet.

### Bilibili anime and series (Bangumi)

Bilibili anime, series and documentary episodes now download like other Bilibili videos. Click Download video on an episode page, or paste a link such as bilibili.com/bangumi/play/ep… into Add URL, then choose the quality and audio track; a season link (ss…) opens its first episode, and a series' trailers and extras work too. The filename is the series title with the episode's name. Without a Bilibili sign-in, Bilibili offers up to 480p. Episodes for members only, where Bilibili shows visitors just a short preview, and region-locked or DRM-protected episodes are not supported; XDownloader tells you instead of saving the preview.

### When a video page cannot be read

When Download video or Find videos on this page opens a YouTube, Bilibili, TikTok or Twitch page that XDownloader cannot read itself, for example a region-locked video, it now shows the media your browser found on that page instead of only an error, with the reason at the top. Turning on Detect media while browsing in the extension and starting the video first gives it the most to show.

### Downloads from browser pages

Files and streams you add from a browser page now tell the server which site they came from, as your browser does: only the site's address, such as https://example.com/, never the full page address or any cookies. Some video servers refuse downloads without it. This covers files and streams chosen in the Video downloads window and downloads the extension hands over to XDownloader. Bilibili downloads now send Bilibili's address too, which fixes "This video stream requires login or its link expired" on some of Bilibili's servers, also for downloads saved by earlier versions. Reload the XDownloader extension in your browser after updating.

## 1.19.0 — 2026-10-21

XDownloader can now download TikTok videos. Click Download video on a TikTok video page, or paste a link such as tiktok.com/@user/video/… into Add URL, then choose a quality. Each quality is a single MP4 file with sound, saved exactly as TikTok serves it. TikTok's video servers need a fresh visit to the video page, so XDownloader loads the page itself before every start or resume; nothing from your browser is used, and pausing and resuming works even after TikTok's links expire. The Download video button on TikTok appears only beside the video the page is about. If TikTok asks for a browser check, XDownloader tells you instead of trying to get past it; opening TikTok in your browser and trying again a few minutes later usually helps. Private, sign-in-only and photo posts, short vm.tiktok.com links and live videos are not supported yet. Reload the XDownloader extension in your browser after updating.

## 1.18.1 — 2026-10-21

Browser setup now fills in the extension ID for Chrome, Edge and Brave by itself, so connecting a browser is one click: load the extension from the folder that Open extension folder shows, then click Connect browser. The ID is worked out from the app's own extension folder, the same way the browser does. If you load the extension from another folder, paste the ID your browser shows instead. A warning now appears when a browser is connected to a different extension ID than this copy's folder, for example after switching Brave to its own extension folder.

## 1.18.0 — 2026-10-21

XDownloader can now download Bilibili videos. Click Download video on a Bilibili video page, or paste a link such as bilibili.com/video/BV… into Add URL, then choose the quality and audio track as you do for YouTube. One part of a multi-part video can be chosen with ?p= in the link, and older av links work too. Bilibili sends video and audio separately; XDownloader downloads both and joins them into one MP4 file. Without a Bilibili sign-in, Bilibili offers up to 480p. Paid, region-locked and DRM videos, anime (bangumi) and live pages are not supported yet. Pausing and resuming later works even after Bilibili's download links expire. Reload the XDownloader extension in your browser after updating.

## 1.17.2 — 2026-10-21

The Download video button next to videos on web pages has a cleaner look: one flat blue button that is smaller and covers less of the page. It is as wide as its text, so the label is no longer cut off in languages such as German, French, Japanese and Ukrainian, and the icon sits right beside it. While XDownloader opens the video, the button shows a spinner. It then turns green with a check mark when the video is open in the app, or red with a retry arrow when you need to try again, without changing its size or place. The text is easier to read in every state. Reload the XDownloader extension in your browser after updating.

## 1.17.1 — 2026-10-21

Fixed the Download video button appearing on YouTube's channel and home page banners, such as the large preview at the top of the Music page. On YouTube the button now appears only beside the main player on video pages: watch, Shorts, embedded and live videos. Reload the XDownloader extension in your browser after updating.

## 1.17.0 — 2026-10-21

Browser integration has a new Brave tab with the brave://extensions page and its own extension folder. Load the extension in Brave from the folder that Open extension folder shows, paste its ID and choose Connect browser. Brave connects separately from Chrome, so both browsers can be used at the same time. The Chrome, Edge and Brave tabs now say that the extension can only be used in Developer mode. Settings, the guides and Register-Browsers.ps1 (with -BraveId) include Brave, in every supported language.

## 1.16.1 — 2026-10-21

Drop target is now called Floating drop box, a name that says what it is. The new name appears in the View menu, the tray menu, the box's own right-click menu and its messages, and the translations now use matching names. Your on/off choice and the box's position are kept.

## 1.16.0 — 2026-10-21

### Drop target

Turn on View → Drop target, or Drop target in the tray menu, to keep a small box with the XDownloader icon on top of your other windows. Drag a link from a browser or another app onto it, also while XDownloader is minimized or in the tray: one link opens Add download with the link filled in, several links open Add batch, and the XDownloader window comes forward. Nothing starts until you add the files. Drag the box to move it; it remembers its place and stays on screen when monitors change. Double-click it to open XDownloader, or right-click it to open XDownloader or hide it. The box never takes focus from the app you are using and has no taskbar button. Drop target is off by default.

### Numbered batch downloads

Tasks → Add batch can now create numbered links from one pattern. Choose From a pattern, enter a link with one * where the number goes, such as https://example.com/photos/img*.jpg, and set From and To. Zero padding Auto turns 1 to 100 into img001…img100; choose Off or a fixed number of digits instead. From and To can also be single letters, a to z. A live preview shows how many links you will get and the first and last one, and explains problems such as a missing or repeated *, a * in the site name, a reversed range or more than 1,000 links. Generated links get the same checks, queue and folder as a pasted list. Link list works as before and keeps your pasted links while you use a pattern.

### Export and import the download list

Tasks → Export list… saves your downloads to a file, so you can back them up or move them to another PC. Export all downloads, the current queue or the selected downloads, optionally without completed ones. An XDownloader list keeps each file's link, name, folder, queue, file type and expected SHA-256; Links only writes a plain text file that Add batch also reads. Passwords, cookies, browser logins, proxy settings and partial download data are never exported, and Windows asks before an existing file is replaced.

Tasks → Import list… reads an XDownloader list or a text file of links and shows what is new, already in your list or invalid before anything is added, with the reason for each skipped entry. Links already in your list are skipped and nothing is replaced. Imported downloads stay paused until you resume them, or you can choose to queue them. Queue names and folders from the list are used only when they exist on this PC. The command line can export too, with idm-ex-cli export FILE.

### Sleep or hibernate when downloads finish

When queue finishes, in a queue's ⋯ menu, and the When done tab of the download status window now offer Sleep computer and Hibernate computer next to Shut down computer. They work like Shut down: you choose them once for the current session, a 60-second countdown with Cancel sleep or Cancel hibernation appears when the queue finishes, and the countdown stops by itself if a download, scan, script or command is still unfinished or a dialog is open. XDownloader asks Windows without forcing other apps to close, keeps wake timers enabled and stays open, so it's ready when the computer wakes. If Windows doesn't let apps sleep or hibernate the computer, for example on Modern Standby PCs or when hibernation is turned off, the choice is greyed out and the reason is shown.

### Completion sound

Settings → Tray & notifications can now play a Windows sound when downloads finish. It is off by default. The sound plays once when nothing is left downloading or waiting to start, rather than once per file. If a file needs attention, the warning sound plays instead while Notify when a download needs attention is on. The sounds follow your Windows Sound settings and stay quiet over full-screen apps and presentations.

### Arabic text with English words and numbers

In Arabic, English words, commands, queue names and numbers inside a sentence now keep their reading order, for example "Windows", "powercfg /hibernate on" and "60" instead of reversed letters and digits. Labels that contain Arabic read right to left, Arabic text that starts with an English word or a number is no longer drawn letter by letter, and long Arabic text that wraps reads from the top line down. Other languages look exactly as before.

### Faster helper launches

Antivirus scans, audio and video merging and automation scripts now start faster, especially on a busy PC. XDownloader still starts each helper inside its own protected process group, so the helper and anything it opens end when the task stops or XDownloader closes, but it no longer inspects every thread on the computer to do so. This saves from a few hundred milliseconds up to about two seconds per launch.

All new options are translated into every supported language.

## 1.15.1 — 2026-10-20

Fixed the window title bar keeping its old color after changing the theme. On Windows 10, switching between Light and Dark (or following a Windows theme change) now repaints the title bar right away instead of after the window was minimized or lost focus. Window focus and typing are not affected.

The GitHub release guide again lists the current version's files.

## 1.15.0 — 2026-10-20

### Capture rules for automatic capture

Automatic download capture can now be limited. In the browser extension, open Capture rules under the Automatic download capture switch to choose which file types XDownloader takes over, set an optional minimum file size, and list sites it should never capture from. Use * as a wildcard, for example r0* for split archives or *.example.com for every subdomain. A plain site name also covers its subdomains. Downloads outside the rules stay in the browser untouched.

The default list covers common archives, installers, disk images, audio, video and PDF files; images, Office documents and text files stay in the browser. If automatic capture was already on before this update, it keeps capturing every file type until you change the rules. Right-click → Download with XDownloader and Add a file URL always send the link, whatever the rules say. The new options are translated into every supported language.

### Start with Windows

Settings → Startup can now open XDownloader when you sign in to Windows, so queue schedules, browser downloads and automation keep working without opening the app yourself. Start in the tray, on by default, keeps it hidden in the notification area until you open it; starting XDownloader yourself always shows the window. Only one copy starts with Windows: turning the option on in another portable or installed copy switches to that copy, and a copy that was moved updates its own entry the next time it starts. If Task Manager has turned XDownloader off, Settings says so, and turning the option on allows it again. Uninstalling removes the entry when it belongs to that installation. The new options are translated into every supported language.

### Clipboard links

When you copy a link to a file in another app, such as a ZIP, an installer, a video or a PDF, XDownloader now offers to download it. A Windows notification and a small card in the main window show the file name and site. Choose Download… or click the notification to open Add download with the link filled in; nothing starts until you add it. While another dialog is open, the card waits for it to close.

Clipboard links is on by default; turn it off in Settings → Integration & security. XDownloader reads only copied text, never logs or saves it, and skips copies that password managers mark as private. Links you copy from XDownloader itself and repeated copies of the same text are ignored. New wording is translated into every supported language.

## 1.14.7 — 2026-10-20

Long translated labels in the main window now fit cleanly, for example in German and Russian. Sidebar entries that are too long end with … before their counts, which always stay fully visible, and show the full name on hover. Toolbar labels wrap only between words: a button with a long word gets a little room from buttons that have space to spare, and a word that still cannot fit ends with … and shows in full on hover. Icons, order and wording are unchanged, and English labels look the same as before.

## 1.14.6 — 2026-10-20

Help → Guide now lists its topics in one column beside the article instead of wrapping rows of buttons above it. Topics stay in one aligned list at every window size and in every language; long translated titles end with … and show in full on hover. The article keeps the full height of the page, and the guide text is unchanged.

## 1.14.5 — 2026-10-20

### Clearer download windows

The download status window now uses plain wording. Its tabs are Status, Speed limit and When done. Show details and Hide details have an arrow instead of << and >>. The connections list is titled Connections and shows states such as Connecting and Downloading. Size, Speed and Resume replace File size, Transfer rate and Resume capability. The window shows the full file name, shortened with … only when space runs out.

The Download complete window no longer repeats "Download complete". It shows the file name and size, labels the fields Source and Saved to, and makes Open the main button.

Main green buttons use a slightly darker green so their white text is easier to read. About and the details panel now use the same outlined X as other dialogs, and About reads "Created by Xetro". New wording is translated into every supported language.

## 1.14.4 — 2026-10-20

Dialogs no longer repeat their top-right X with a Cancel or Close button at the bottom. Use the X or Esc to dismiss a dialog; unsaved changes are discarded as before. Footers keep only each dialog's own actions, such as Save, Add to queue or Delete file. ZIP contents opened from Add download keeps Back to download.

The download status window no longer has a Cancel button. Select Pause to stop a transfer, then the X to close the window; the X alone hides the window while the download continues. The Download complete window closes with its X.

## 1.14.3 — 2026-10-19

Added a security policy covering private vulnerability reports, supported releases and security boundaries for downloads, automation and local MCP access. The README links to it, and the policy ships with the portable app, installer and source archive. The GitHub release guide now includes private reporting setup.

## 1.14.2 — 2026-10-19

Enlarged the X in shared dialog close buttons and centered the visible symbol inside its square outline. Hover, keyboard focus, and disabled controls keep their existing behavior.

## 1.14.1 — 2026-10-19

Settings now appears first in the Tools menu for quicker access. Other menu items keep their existing order.

Browser integration now shows the extensions-page address in a larger, high-contrast field. Select and copy the address directly, or use Copy extensions page, for Chrome, Edge, and Firefox.

Shared dialog headers now use a clearly outlined square X button with hover feedback. Browser integration no longer has a duplicate Close button at the bottom; use the X or Esc to dismiss it.

## 1.14.0 — 2026-10-19

### Antivirus settings and additional scanners

Antivirus settings now follow the selected scanner, with provider-specific help, an explicit setup check, and app shortcuts only for the relevant external scanner. Microsoft Defender, ClamAV, and custom command-line scanners retain their existing result handling.

Added Kaspersky and Bitdefender consumer scan requests through their English Windows Explorer scan actions. These integrations require a standard installation and an individual scan command exposed by Explorer; missing, ambiguous, localized, or unsupported menu actions fail without opening the downloaded file. Live scan results must be reviewed in the antivirus app. A successful handoff never means No threats found.

Single and Batch share the same review controls. External batches pause after every request, including failures, for retry or confirmation before continuing. Stop batch cancels remaining requests; scans already handed to the antivirus must be stopped in that app. Opening after review still rechecks the file and scanner settings.

## 1.13.4 — 2026-10-19

### Single and batch antivirus scanning

Tools → Antivirus Scan now opens one dialog with Single and Batch tabs. Choose one completed download in Single, or select several files in Batch. Both modes use the configured scanner, show results and support cancellation. Scan results remain attached to each download, and Malwarebytes requests still wait for manual review. Switching tabs keeps the dialog and backdrop steady.

## 1.13.3 — 2026-10-19

### Smooth website grabber tabs

Switching between Find downloadable files and Offline website now keeps the same dialog open. The backdrop no longer disappears or fades again, preventing the whole application from flashing when changing tabs. Each tab keeps its existing settings and results.

## 1.13.2 — 2026-10-19

### Safer release packaging

Source packages now use an exact reviewed file inventory and reject unexpected files, sensitive filenames, recognizable credential content and linked paths. The public localhost test certificates and required FFmpeg build logs are included only when their reviewed checksums match.

Public downloads and their checksum manifest now go into a dedicated version folder, separate from development installers and private portable profiles. Packaging and checksum generation reject unexpected files in that folder. Existing downloads, settings and browser registrations are preserved.

Corrected a download test that could check its destination before temporary recovery-file cleanup finished.

Shortened the README into a quick start. The detailed reference remains available in the documentation and is included with the application packages.

## 1.13.1 — 2026-10-19

### Open-source release licensing

XDownloader's original application, browser extension and documentation code is now available under the MIT license. Included third-party software and fonts retain their original licenses and attribution.

The Windows packages now include dependency notices and source archives for MPL-covered components. The bundled FFmpeg helper is built from pinned FFmpeg and x264 sources; its complete source archives, build recipe, license texts and build records are included with both the portable app and installer.

Added an explicit unsigned GitHub release packaging option, separate from local test builds. Public artifacts identify their unsigned status and include SHA-256 checksums. Video downloads continue to merge local tracks without re-encoding.

Corrected malformed filename placeholders in the Ukrainian signature warning and Swedish uninstall confirmation.

## 1.13.0 — 2026-10-19

### Italian and Romansh

Added Italiano and Rumantsch Grischun to the desktop app, offline Help guide, Windows installer and Chrome, Edge and Firefox extensions. Together with the existing German, Swiss standard German and French choices, this covers the four Swiss languages requested.

Automatic Windows and browser language selection now recognizes Italian (including it-CH) and Romansh (rm-CH). The language picker offers nineteen choices plus System language; unsupported languages continue to use English. Existing language preferences, downloads and browser registrations are preserved.

## 1.12.0 — 2026-10-18

### German, French, Swedish, Norwegian and Swiss German

Added Deutsch, Français, Svenska, Norsk bokmål and Deutsch (Schweiz) to the desktop app, offline quick guide, Windows installer and Chrome, Edge and Firefox extensions. The Swiss German option uses written Swiss standard German (ss instead of ß). Swiss French locales use French.

Windows and browser language detection recognizes German, French, Swedish and Norwegian Bokmål, including de-CH, fr-CH, nb-NO and no-NO. Unsupported languages continue to fall back to English. Select a language in Settings → Appearance → Language or the extension's Browser options.

## 1.11.0 — 2026-10-18

### Eight additional languages

Added Japanese, Korean, Vietnamese, Thai, Polish, Russian, Ukrainian and Arabic to the desktop app, offline quick guide, Windows Setup and browser extensions. Choose a language in Settings → Appearance → Language or the extension's Browser options. System/browser language detection recognizes the new languages and continues to use English for unsupported languages.

Added installed Windows font fallbacks for Japanese, Korean, Thai and Arabic. The browser extension and Setup use right-to-left layout for Arabic; the native desktop retains its existing layout. Filenames, URLs, commands, saved data, script output and raw technical errors keep their original text. Missing translations fall back to English.

## 1.10.0 — 2026-10-18

### Four interface languages

Choose English, Simplified Chinese, Traditional Chinese or Malay in Settings → Appearance → Language. The desktop interface and offline quick guide follow your selection immediately and remember it across restarts. Windows language detection and English fallback are included; existing downloads and preferences are preserved.

The Chrome, Edge and Firefox extensions include the same languages, with a separate selector under Browser options. Popups, link and login reviews, context menus and video buttons are translated. Windows Setup and its uninstaller also offer all four languages; a fresh installed profile inherits the setup language while upgrades preserve your saved choice.

Chinese text uses available Windows fonts. CLI/API identifiers, user content, script output, raw technical errors and historical release notes retain their original text. See docs/localization.md for language selection and translation maintenance.

## 1.9.1 — 2026-10-18

### Prevent overlapping Malwarebytes scan requests

Malwarebytes batches now pause after each request so you can check its result before choosing Scan next file. If Malwarebytes reports that another scan is in progress, wait until it finishes and use Retry this file. The last file waits for Finish batch, and Stop batch still cancels the remaining requests. This prevents sending the entire selection while Malwarebytes is busy; its Explorer action acknowledges requests without reporting when scans finish. Defender and command-line scanners continue automatically.

## 1.9.0 — 2026-10-18

### Batch antivirus scanning

Tools → Batch antivirus scan lets you choose completed downloads, select all, follow progress and review each result. Stop batch or close the dialog to cancel its remaining work. Scans run one at a time, and saved results remain available with each download.

### Queue controls and menus

Download at once now accepts a typed value from 1 to 32. The app enforces a total limit of 32 active downloads across queues and preserves existing queue settings during the upgrade. Automation has moved from View to Tools, and the extra explanatory text has been removed from the queue completion menu. The Guide explains batch scanning and Clear finished.

About now has a centered close icon with subtle hover feedback and a clear keyboard-focus outline.

## 1.8.6 — 2026-10-18

### Release unused scan results from memory

The download details panel now keeps only the selected file's antivirus report in memory. Browsing scan results and clearing download history no longer retain old reports until the app closes. Saved scan history remains available when a download is selected again.

## 1.8.5 — 2026-10-18

### More reliable website file discovery

Grabber now uses GitHub's raw-file links for repository files and reads same-origin HTML fragments, including the ZIP assets on GitHub release pages. Fragments count toward the existing page and size limits, and duplicate file links appear once. The Guide includes a GitHub example for both file discovery and static offline capture.

Downloads now fall back to a full transfer when a server's range probe reports an unknown file size, instead of failing with “invalid digit found in string.” Such responses do not enable unsafe partial-file resuming.

## 1.8.4 — 2026-10-18

### Wider download status window

The Download status window now opens 30% wider, giving filenames, transfer details and segment rows more horizontal space.

## 1.8.3 — 2026-10-18

### Clearer download segments and toolbar navigation

Download status and Download complete now use neutral action buttons. YouTube, HLS and DASH status details show their actual video and audio segments, completed count and live bytes for the current segment, instead of a single generic file range. The segment map follows playlist order and the scrollable list shows completed, active and waiting parts. These video downloads still fetch one segment at a time; the status window and Guide explain that direct-file connection settings do not enable parallel video transfers.

Added Grabber beside Automation in the main toolbar and moved Logs beside Settings. All toolbar actions remain available in compact windows.

## 1.8.2 — 2026-10-17

### Complete Download Status window, unified Guide menu, toolbar & sidebar refinements, and updated Logs page

Upgraded the Download Status window with instant tab switching across `Download status`, `Speed limiter`, and `Options on completion`, an IDM-style summary card (`Status`, `File size`, `Downloaded`, `Transfer rate`, `Time left`, `Resume capability`), an action row directly beneath the overall progress bar (`<< Hide details` on the left, `Pause`/`Resume` and `Cancel` on the right), and an expandable connection details section with a visual multi-segment start-position bar and live per-connection table (`N.`, `Downloaded`, `Info`). When a download completes, the window switches to a dedicated `Download complete` dialog showing the exact byte count, source `Address`, `The file saved as`, `Open`, `Open with…`, `Open folder`, `Close`, and `Don't show this dialog again`.

Consolidated the Help menu so all documentation topics—including Lua scripting and MCP integration—open from Help → Guide, while keeping quick-access guide buttons on the Scripts and MCP pages. Renamed Exit XDownloader to Exit across the Tasks menu, tray menu, and queue completion options.

Added a Queues button to the main toolbar, matched the MCP toolbar button width to adjacent navigation controls, and moved Queued and Failed directly into the sidebar Downloads filter list while removing duplicate status filter and appearance items from the View menu. Redesigned the Logs page with a structured header, severity summary badges, recessed log console, and color-coded level badges (blue Info, amber Warn, and red Error).

## 1.8.1 — 2026-10-17

### Stop downloads from the main window

Added Stop for selected downloads and Stop all to the toolbar, with matching Tasks and download-menu actions. Stop keeps saved progress and closes the stopped files' status windows. Stop all covers every queue, including files started immediately, without deleting completed files or changing queue schedules. Resume stopped files when ready.

Pause all is now in Tasks so the toolbar stays compact. The Guide explains the difference between pausing queues and stopping individual downloads.

## 1.8.0 — 2026-10-17

### Download status windows

Follow a download in its own compact window with progress, downloaded size, speed, estimated time remaining, and current activity. The window uses the same cards, colors, and controls as the rest of XDownloader. Connecting, checking saved progress, merging video, verifying, and saving are reported by the running download worker.

Double-click a download or choose Download status from its menu. Single downloads started manually open their status automatically; turn this off in Settings → Downloads. Batch downloads, queue starts, CLI commands, and automation stay in the background. Windows continue to track their own files when you change the main queue or view.

Pause and resume from the window, or use Stop to pause and hide it while keeping saved progress. Hide, Escape, and the close button leave the download running. Completed downloads offer Open file and Open folder. Expand details for resume support, transfer ranges or media segments, source, and destination. Unknown sizes and unavailable estimates are shown honestly.

## 1.7.1 — 2026-10-17

### More reliable offline copies

Fixed images and stylesheets being skipped when the same URL also appears as a clickable page link. Successfully captured resources no longer produce false omission notices for another use of the same URL.

Offline SVG styles now handle text split across XML sections and character references before removing online dependencies. CSS namespace names are preserved so matching styles still apply. SVG marker references are rewritten, and inline SVG animations are disabled so they cannot restore online links after capture.

## 1.7.0 — 2026-10-17

### Browse saved websites offline

Website grabber now offers an Offline website mode. Save linked HTML pages, stylesheets, images, icons and fonts into a separate folder, then open index.html to browse without the source website. Page links, CSS imports, background images and responsive images are rewritten to local files. Shared resources are saved once, and distinct query URLs remain separate.

Choose the link depth, page and size limits, and whether to include assets from other websites. Stop capture keeps the pages already received as a partial copy. The capture report explains skipped links, missing resources and limits; uncaptured page links lead to that report. Existing files, download history and saved discovery projects are preserved. The configured HTTP/HTTPS proxy is used throughout.

This captures static UTF-8 pages. Scripts, forms, embedded pages, sign-in and live services are disabled in the offline copy. JavaScript-only sites are not reproduced as working applications. Help → Guide explains the workflow and limits.

## 1.6.1 — 2026-10-16

### Safer portable packaging

Portable releases now come only from a fresh, versioned archive. Updating the stable local test copy requires an explicit packaging option and keeps its queue history, diagnostics, browser registration and downloads clearly separate from distributable files.

## 1.6.0 — 2026-10-16

### Enterprise logins and consistent proxy routing

Added NTLM and Negotiate (Kerberos / NTLM) choices for direct-file logins and HTTP proxy logins. Enter a domain username and password; Windows performs the handshake using only those credentials. Website logins stay tied to their server, proxy logins stay separate, and passwords remain in Windows Credential Manager. Negotiate selects Kerberos when the domain and server support it, with NTLM fallback.

Connection settings now also apply to HLS/DASH previews, media segments, YouTube lookups and refreshes, and manual or scheduled website scans. New operations take a fixed route snapshot. Failed custom proxies never switch to direct connections. FTP/FTPS remains direct. Enterprise authentication requires Direct or a custom HTTP proxy; automatic System proxy discovery is not supported for enterprise logins. Existing Basic-auth HTTPS and SOCKS proxy support is unchanged.

This build also includes the idle Main queue auto-start, Start immediately download actions and the redesigned MCP controls from 1.5.0 and 1.5.1. Help → Guide explains the new login and proxy choices.

## 1.5.1 — 2026-10-16

### Clearer MCP controls

Refined the AI connection page with a clear server status and start/stop action, aligned settings, explanations beside both access choices, and a dedicated connection card. Copy the server URL directly from its field, even before starting, or copy the suggested first prompt for your AI. Authenticated client configuration, access-header copying and connection checks remain available while the server is running. Compact windows scroll to keep the controls readable.

## 1.5.0 — 2026-10-16

### Start downloads without managing the queue

Adding a file or reviewed video now starts an idle Main queue automatically when it has no schedule. Completed history no longer leaves the next download stuck behind an old queue pause. Existing waiting work, custom queues, schedules and individually paused files retain their controls.

Download right-click and three-dot menus now include Start immediately, including for selected files. It starts the existing file ahead of its queue, even when the queue is paused, scheduled or at its usual file limit. Saved progress, file identity and queue order are preserved; other files and schedules are unchanged. The overall eight-download limit and data quotas still apply. Pause, Pause all or pausing its queue cancels the override, and it is not restored after restarting the app.

Waiting downloads now show Queue paused when that is what is blocking them. Help → Guide describes automatic and immediate starts.

## 1.4.0 — 2026-10-16

### Recover expired links using the browser

Update download link can open the original webpage and wait for the same file to be downloaded again in Chrome, Edge or Firefox. The extension pauses the matching browser copy, sends the fresh URL to the existing download and brings the saved-data check into the app. Confirm before resuming or restarting. Cancellation releases the browser copy; successful updates preserve the download's identity, filename, folder and queue position. Automatic capture can remain off. Reload the updated extension to use recovery.

### Connection settings for each website

Settings → Download connections now provides a default per-file limit and website overrides, including optional subdomains. Choose up to 32 connections, or 1 for sites that reject parallel requests. Redirects respect the stricter matching rule. Pause and resume active files to apply changes, including to downloads with existing partial files. FTP and video streams keep their existing transfer behavior.

### Shut down after queue completion

Added Shut down computer to the queue menu's When queue finishes choices. This optional, one-time action waits for successful completion and shows a cancellable 60-second countdown. Other unfinished downloads, scans, scripts, commands or open dialogs cancel it. Windows is asked to shut down without forcing other programs to close. The option starts off and is not restored after exiting the app.

Help → Guide includes instructions and limits for all three features.

## 1.3.1 — 2026-10-16

### Cleaner Logs workspace

Grouped search, severity and source filters, auto-refresh and the log console into one bordered card. The event count now sits beside the Logs heading and shows matching versus total events when filtered. Removed repeated descriptions and update timestamps, while keeping read errors and notices about older or skipped entries visible. Refresh and reading indicators stay in a fixed slot so the console does not shift while viewing logs.

## 1.3.0 — 2026-10-16

### MCP controls in the app

Added an MCP page on the toolbar, in Tools → MCP control and in Settings. Start or stop the localhost server, choose its port, limit AI access to viewing or allow changes, and optionally start it with XDownloader. Settings are saved for this copy of the app. MCP is off by default and new desktop settings start with view-only access.

Copy the client configuration, server URL or private access header directly into your AI client. Check connection verifies authenticated access over HTTP. The page shows startup, shutdown and port errors, links to the setup guide and logs, and keeps connection secrets out of the visible page and diagnostic logs. The app owns this server: it keeps running in the tray and stops when XDownloader exits. Stopping MCP does not pause downloads already started. The existing terminal server remains available.

## 1.2.0 — 2026-10-16

### Local MCP integration

Added a localhost HTTP MCP server through idm-ex-cli mcp. Compatible AI clients can inspect downloads, manage queues, add and pause/resume files, inspect automation rules, and run or stop saved Lua/PowerShell scripts with retained logs. Commands use the app's existing scheduler and durable receipts. A read-only mode is available. The server stays running when clients disconnect; pending commands support cancellation, and claimed commands can be checked through their receipts.

Help → MCP setup explains connection and usage. The mcp-config command generates the localhost URL and private connection token for the current profile. Current MCP discovery and older initialization-based clients are supported over Streamable HTTP, with loopback-only binding, host/origin checks and Windows-protected token storage. The bundled AI guide is also available through get_usage_guide, with examples for downloads, scripts, retries and completion checks.

Refined Create new queue, Rename queue, Schedule queue and Add batch dialogs with unified bordered card layouts, dedicated modal sizing and compact footer spacing across all dialogs.

## 1.1.4 — 2026-10-16

### Automation alignment, selection fixes and streamlined dialogs

Aligned script setting labels, text fields and dropdowns, with matching Name and Language columns. Labels stay with their controls when the window is narrow, and long script and queue names stay within their pickers. Script names now use the same 64-character limit as saved scripts.

Choosing the current trigger again preserves its interval and queue. A removed trigger queue is identified correctly instead of appearing as All queues. Long rule names no longer overlap timestamps in Run history; hover shows the full name.

Streamlined Browser integration, Download folders, Speed limits, Data quotas, Preview video and Website grabber dialogs with bordered card layouts, concise non-redundant descriptions, inline status badges and compact destination controls.

## 1.1.3 — 2026-10-16

### Automation workspace and desktop UI polish

Redesigned the Automation page with a segmented sub-navigation bar for Rules, Scripts and Run history, an active-rules status badge and a right-aligned contextual guide button.

Rule cards now display evaluation priority numbers (`#1`, `#2`), workflow type badges (`ROUTING` and `QUEUE CHAIN`) and structured `WHEN → THEN` condition and action pills. Run history entries now render as structured status cards with severity badges, rule names and timestamps. The Automation rule dialog groups trigger and action settings into numbered cards and uses XDownloader's emerald accent palette across modal headers and primary actions.

Refined Automation → Scripts into a structured workbench card with a unified action and script-selector header, compact two-row trigger and queue scope controls, a language-badged code editor and a docked Run logs console card with run status badges. Enhanced the Downloads header with a live queue status indicator and concurrency summary.

## 1.1.2 — 2026-10-16

### Dedicated Lua guide

Added Help → Lua guide and a Lua scripting topic in the offline Guide. The Lua editor's guide button opens it directly. The walkthrough covers creating, saving and running scripts, Lua basics, a sample PDF download, automatic completion triggers, coloured logs, errors and the XDownloader Lua API. Each example has Copy script. Opening the guide keeps unsaved editor text, and the packaged documentation includes the same instructions.

## 1.1.1 — 2026-10-16

### Toolbar navigation

Moved Automation and Logs beside Scheduler in the top toolbar. The sidebar now contains download filters, file types and queues. The toolbar stays visible on every page, with matching hover effects and a highlight for the open Automation, Logs or Settings page. Buttons fit the minimum window size. Help → Guide and the packaged documentation describe the new locations.

## 1.1.0 — 2026-10-16

### Script automation

Automation → Scripts now has an editor with Lua selected by default and optional PowerShell, Save, Run and Stop controls, configurable timeouts and saved, colour-coded text logs. Lua is built in; no separate installation is needed. The editor includes working examples and keeps unsaved changes during live updates. Existing PowerShell scripts retain their language.

Scripts can run manually, at app startup, on a recurring interval, when downloads finish or when a queue completes. Automatic triggers are off for new scripts. The app must be open or running in the tray.

Lua's `xd` API can add downloads, inspect status and control queues; `event` provides trigger details. PowerShell uses `Invoke-XDownloader` and `$Event`. Lua has a 64 MiB VM memory limit and no shell/file/module loaders; PowerShell remains available for general Windows automation. The CLI also supports saving scripts, enabling triggers, starting and stopping runs, and reading run history and output.

Runs use a saved revision, never overlap another run of the same script, and are limited to four concurrent scripts. Stop, timeouts and app exit terminate script child processes. Interrupted runs are recorded without replaying them after a crash. Logs retain the latest 100 finished runs and up to 128 KiB output per run.

Help → Guide and the packaged automation reference include script examples, trigger behaviour, cancellation limits and troubleshooting. The app, CLI, installer and all browser extension packages now share version 1.1.0.

## 1.0.1 — 2026-10-16

### Release history and version visibility

Added Help → Changelog alongside Guide and About. The same release notes ship with the portable app and installer.

About, CLI help, CLI --version and Windows executable properties now show the full build version. Chrome, Edge and Firefox extension packages use the same release number.

Packaging checks the application version, lockfile, browser manifests, latest changelog entry and executable version metadata before producing a release. The project now documents a version bump and release note for each delivered update.

### Recent improvements included

The CLI can add and inspect downloads, manage queues, pause or resume jobs and configure automation rules. Commands return JSON for scripts and can start the desktop when needed.

The Automation page can route new downloads by site or file type, apply speed limits and start another queue after completion. Rules have enable switches and a persistent run history.

Help → Guide provides offline instructions, topic navigation and copyable PowerShell examples. It covers downloads and recovery, browser integration, queues, schedules, automation, CLI usage, settings and diagnostics.

### Release history note

This is the first release with a maintained changelog. Earlier development updates reused 1.0.0, including the recent CLI, Automation and Guide work described above. Those earlier builds do not have separately recorded release numbers.

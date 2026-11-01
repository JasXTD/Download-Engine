# Native preview guide

These articles replace the Windows-specific startup and command-line sections in
the macOS/Linux Guide. Other topics describe the shared download engine.

## Getting started

### Open Download Engine

On Linux, extract the complete archive to a permanent folder and run ./de. Run bash ./install-desktop.sh from that folder to add it to your application menu. Add --set-defaults only if you want this copy to open torrent files and magnet links by default.

On macOS, keep Download Engine.app in its final location, then open it. The CLI and helper programs stay inside its app bundle. Preview bundles are not Developer ID signed or notarized; public macOS distribution still needs those steps and native desktop validation.

### Add your first download

Choose Add URL, paste a direct file link, check the name, queue and destination, then choose Start download. Add batch accepts multiple links. Video pages use the video review so you can choose a quality before adding a download. Download status shows progress, speed and time remaining. Pause, Resume and Stop keep their existing download controls.

Completed files remain in their folders when you clear completed history. Delete file is a separate, confirmed action. Existing transfers keep their recorded paths if you change the default download folder.

### Connect browsers and open torrents

Open Preferences → Browser & AI → Browser integration, install the matching extension, and connect its ID. Keep the packaged helper programs with the application. Browsers installed through a sandboxed package may need their package's native-messaging support before they can reach an external app.

Torrent files, magnet links and dropped local torrent files open a review before downloading. Torrents stay off until you enable them. Seeding and incoming connections remain separate choices in Preferences.

### Passwords and desktop services

Saved credentials use macOS Keychain or an unlocked Linux Secret Service keyring. A missing or locked keyring produces an error; passwords are never saved as plain text. Windows-protected credentials must be entered again on another operating system.

Linux file and folder dialogs need your desktop's XDG portal service and backend. Language coverage also depends on installed fonts. The bundled docs/cross-platform.md lists the system requirements and profile locations.

### What differs in this preview

The shared interface includes queues, schedules, torrents, browser integration, website grabbing, video merging, Lua extensions and providers, localhost MCP, themes and language preferences. The remaining Guide topics describe these shared features.

Windows tray controls, automatic clipboard offers, the floating drop box, Windows sign-in startup, enterprise NTLM/Kerberos login, and completion power actions are Windows features. Their controls are not offered in the native previews. Native antivirus settings support ClamAV or a configured command-line scanner. PowerShell scripts need an existing PowerShell 7 installation; Lua is included.

## Command line

### Open Terminal in the application folder

On Linux, open Terminal in the extracted folder containing de and de-cli. On macOS, change to the app's Contents/MacOS folder, for example /Applications/Download Engine.app/Contents/MacOS. These read-only commands show help and the current queues:

```sh
./de-cli --help
./de-cli queues
./de-cli list
```

### Download a test PDF

This creates a paused queue, adds W3C's sample PDF, then starts the queue. Choose a different queue name if CLI test already exists. Enabled automation rules can override the requested queue.

```sh
./de-cli queue create "CLI test"
./de-cli add "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf" --queue "CLI test"
./de-cli queue start "CLI test"
./de-cli list
```

### Read status and control a download

Commands return JSON. Read commands use result; state-changing commands use receipt.result. Copy a download's id from list and replace JOB_ID in these examples:

```sh
./de-cli status "JOB_ID"
./de-cli pause "JOB_ID"
./de-cli resume "JOB_ID"
```

### Batch and export

batch accepts an absolute path to a UTF-8 file with one direct URL per line, up to 1,000 links. Blank lines are ignored; an invalid link rejects the whole batch. Add --queue and a queue name or ID to choose its destination.

export saves the list to a new .json or .txt file without starting the desktop or replacing an existing file. For example, ./de-cli export "$PWD/downloads.json" writes a new JSON list in the current folder. Import it through Tasks → Import list. Exported links can contain temporary access tokens, so keep the list private.

### Script behavior

Commands that change state launch the desktop if needed; read commands do not. Both use the same local profile as that application copy. DOWNLOAD_ENGINE_HOME can select a separate profile. The CLI is not a separate download service.

Exit code 0 means success, 1 means failure or rejection, and 2 means the request is still running or its outcome is uncertain. Check the receipt before retrying. Reuse --request-id with the same UUID and command to avoid duplicate submissions. Keep passwords and private tokens out of command lines and shell history.

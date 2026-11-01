#!/usr/bin/env bash
# Register this extracted copy for the current user; never run as root.
set -euo pipefail
[[ "$(uname -s)" == Linux && "$(id -u)" != 0 ]] || { echo 'Run this as your normal Linux desktop user.' >&2; exit 1; }
[[ $# == 0 || ( $# == 1 && "$1" == --set-defaults ) ]] || { echo 'Usage: bash install-desktop.sh [--set-defaults]' >&2; exit 1; }
root=$(cd "$(dirname "$0")" && pwd)
data=${XDG_DATA_HOME:-"$HOME/.local/share"}
[[ "$data" == /* && "$root" != *'='* && ! "$root" =~ [[:cntrl:]] ]] || { echo 'Use an absolute data directory and an extraction path without = or control characters.' >&2; exit 1; }
test -x "$root/de" && test -f "$root/download-engine.png"
test -x /usr/bin/env || { echo '/usr/bin/env is required for desktop registration.' >&2; exit 1; }
if [[ $# == 1 ]]; then command -v xdg-mime >/dev/null || { echo 'xdg-mime is required to set default handlers.' >&2; exit 1; }; fi
# Desktop Exec values are parsed by the desktop, not a shell. Escape quoted
# arguments and double literal field-code markers before adding our own %U.
escaped=${root//\\/\\\\}
escaped=${escaped//\"/\\\"}
escaped=${escaped//\$/\\\$}
escaped=${escaped//\`/\\\`}
escaped=${escaped//%/%%}
# Desktop string escaping is decoded before Exec quoting. Double those quoting
# backslashes once more (four backslashes in the file represent one in the path).
escaped=${escaped//\\/\\\\}
mkdir -p "$data/applications" "$data/icons/hicolor/256x256/apps"
install -m 644 "$root/download-engine.png" "$data/icons/hicolor/256x256/apps/download-engine.png"
entry=$(mktemp "$data/applications/.download-engine.XXXXXX")
trap 'rm -f -- "$entry"' EXIT
cat > "$entry" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Download Engine
Comment=Manage downloads, videos and torrents
# GIO checks argv[0] before expanding %% field escapes. A fixed env executable
# also lets extraction folders contain literal percent signs. No shell is used.
Exec=/usr/bin/env -- "$escaped/de" %U
Icon=download-engine
Terminal=false
Categories=Network;FileTransfer;
MimeType=application/x-bittorrent;x-scheme-handler/magnet;
StartupNotify=true
DESKTOP
chmod 644 "$entry"
mv -f "$entry" "$data/applications/com.xetro.downloadengine.desktop"
if command -v update-desktop-database >/dev/null; then update-desktop-database "$data/applications"; fi
if [[ $# == 1 ]]; then
  xdg-mime default com.xetro.downloadengine.desktop application/x-bittorrent
  xdg-mime default com.xetro.downloadengine.desktop x-scheme-handler/magnet
fi
printf 'Registered this copy: %s\n' "$root/de"

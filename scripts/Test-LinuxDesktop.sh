#!/usr/bin/env bash
# Run in an isolated Linux account/container with GCC and gio-unix-2.0 headers.
# Only temporary launcher files are written; default associations are untouched.
set -euo pipefail
[[ "$(uname -s)" == Linux && "$(id -u)" != 0 ]] || { echo 'Run this as a non-root Linux user.' >&2; exit 1; }
root=$(cd "$(dirname "$0")/.." && pwd)
test_root=$(mktemp -d /tmp/de-desktop-entry.XXXXXX)
cleanup() {
  local resolved
  resolved=$(realpath "$test_root")
  case "$resolved" in /tmp/de-desktop-entry.*) rm -rf -- "$resolved" ;; *) return 90 ;; esac
}
trap cleanup EXIT
gcc "$root/tests/native/desktop_entry.c" -o "$test_root/check" $(pkg-config --cflags --libs gio-unix-2.0)
export XDG_DATA_HOME="$test_root/data"
export XDG_CONFIG_HOME="$test_root/config"
export XDG_CACHE_HOME="$test_root/cache"
export DE_TEST_LAUNCH_LOG="$test_root/launch-log"
package="$test_root/Download Engine % test \$ back\`slash\\ 日本語"
mkdir -p "$package"
cp "$root/scripts/Install-LinuxDesktop.sh" "$package/install-desktop.sh"
cp "$root/assets/download-engine.png" "$package/download-engine.png"
cat > "$package/de" <<'STUB'
#!/bin/sh
printf '%s\0' "$0" "$@" > "$DE_TEST_LAUNCH_LOG"
STUB
chmod +x "$package/de"
bash "$package/install-desktop.sh"
"$test_root/check" "$XDG_DATA_HOME/applications/com.xetro.downloadengine.desktop" "$package/de" "$DE_TEST_LAUNCH_LOG"
test ! -e "$XDG_CONFIG_HOME/mimeapps.list"
# env interprets NAME=VALUE before its command, including after --. Refuse such
# extraction paths before writing an entry instead of launching the wrong thing.
mkdir "$test_root/rejected=path"
cp "$root/scripts/Install-LinuxDesktop.sh" "$test_root/rejected=path/"
if bash "$test_root/rejected=path/Install-LinuxDesktop.sh" > "$test_root/rejected.log" 2>&1; then
  echo 'An unsupported extraction path was accepted.' >&2; exit 1
fi
grep -Fq 'without = or control characters' "$test_root/rejected.log"
echo 'No default file associations changed; temporary launcher files removed on exit.'

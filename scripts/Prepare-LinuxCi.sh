#!/usr/bin/env bash
# Fetch only the previously reviewed CI archives. No packages are installed here.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
output=${1:?Usage: Prepare-LinuxCi.sh NEW_INPUT_DIRECTORY}
[[ ! -e "$output" ]] || { echo 'Use a fresh CI input directory' >&2; exit 1; }
mkdir -p "$output"
output=$(cd "$output" && pwd)
while read -r expected size url extra; do
  [[ -z "$expected" || "$expected" == \#* ]] && continue
  [[ -z "$extra" && "$expected" =~ ^[a-f0-9]{64}$ && "$size" =~ ^[0-9]+$ && "$size" -le 67108864 ]] || { echo 'Invalid CI archive record' >&2; exit 1; }
  case "$url" in
    https://deb.debian.org/debian/pool/main/*.deb|https://nodejs.org/dist/v24.22.2/node-v24.22.2-linux-x64.tar.xz) ;;
    *) echo 'Unreviewed CI archive origin' >&2; exit 1 ;;
  esac
  name=${url##*/}
  [[ "$name" =~ ^[A-Za-z0-9_+.~-]+$ && ! -e "$output/$name" ]] || { echo 'Invalid or duplicate CI archive name' >&2; exit 1; }
  curl --fail --silent --show-error --proto '=https' --max-redirs 0 --connect-timeout 20 --max-time 180 --max-filesize "$size" "$url" -o "$output/$name"
  [[ $(stat -c %s "$output/$name") == "$size" ]] || { echo 'CI archive size mismatch' >&2; exit 1; }
  printf '%s  %s\n' "$expected" "$output/$name" | sha256sum --check --status
  printf 'Verified %s\n' "$name"
done < "$root/scripts/linux-ci-inputs.lock"

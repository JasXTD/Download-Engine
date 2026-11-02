#!/usr/bin/env bash
# Exercise the package's real source-copy routine without rebuilding binaries.
set -euo pipefail
project=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d "${TMPDIR:-/tmp}/de-package-test.XXXXXX")
trap 'rm -rf -- "$fixture"' EXIT
awk '/^copy_source_tree\(\) \{/ {copy=1} copy {print} copy && /^\}/ {exit}' \
  "$project/scripts/Package-Unix.sh" > "$fixture/copy-source.sh"
source "$fixture/copy-source.sh"
root="$fixture/source"
work="$fixture/work"
mkdir -p "$root/scripts" "$root/examples/nested" "$work"
inventory="$root/scripts/release-source-inventory.json"
printf '%s\n' '{"files":["examples/nested/reviewed.lua"]}' > "$inventory"
printf '%s\n' 'return 42' > "$root/examples/nested/reviewed.lua"
printf '%s\n' 'private test fixture' > "$root/examples/unlisted.data"
copy_source_tree examples "$fixture/output"
cmp "$root/examples/nested/reviewed.lua" "$fixture/output/nested/reviewed.lua"
test ! -e "$fixture/output/unlisted.data"

# A source file or any of its parent folders must not escape the checkout.
mv "$root/examples/nested/reviewed.lua" "$fixture/original.lua"
ln -s "$fixture/original.lua" "$root/examples/nested/reviewed.lua"
if copy_source_tree examples "$fixture/file-symlink" > "$fixture/error" 2>&1; then
  echo 'Source file symlink accepted' >&2; exit 1
fi
grep -Fq 'Source symlink refused' "$fixture/error"
test ! -e "$fixture/file-symlink/nested/reviewed.lua"
mv "$root/examples/nested" "$fixture/nested"
ln -s "$fixture/nested" "$root/examples/nested"
if copy_source_tree examples "$fixture/folder-symlink" > "$fixture/error" 2>&1; then
  echo 'Source folder symlink accepted' >&2; exit 1
fi
grep -Fq 'Source symlink refused' "$fixture/error"

printf '%s\n' '{"files":["examples/../../original.lua"]}' > "$inventory"
if copy_source_tree examples "$fixture/traversal" > "$fixture/error" 2>&1; then
  echo 'Source traversal accepted' >&2; exit 1
fi
grep -Fq 'Invalid source inventory path' "$fixture/error"
printf '%s\n' '{"files":[]}' > "$inventory"
if copy_source_tree examples "$fixture/empty" > "$fixture/error" 2>&1; then
  echo 'Empty source inventory accepted' >&2; exit 1
fi
grep -Fq 'No reviewed source files' "$fixture/error"
printf 'Reviewed files copied; unlisted files, symlinks, traversal and empty inventories rejected.\n'

awk '/^copy_media_build_record\(\) \{/ {copy=1} copy {print} copy && /^\}/ {exit}' \
  "$project/scripts/Package-Unix.sh" > "$fixture/copy-record.sh"
source "$fixture/copy-record.sh"
record_source="$fixture/record-source"
mkdir "$record_source"
for record in binaries.sha256 compiler-version.log config_components.h config.h config.mak ffmpeg-version.log hdr-metadata-test.log x264-config.mak; do
  printf 'reviewed record\n' > "$record_source/$record"
done
printf 'CI_SAMPLE_SECRET=synthetic-value-never-export\n' > "$record_source/config.log"
printf 'unreviewed file\n' > "$record_source/extra.data"
copy_media_build_record "$record_source" "$fixture/records"
test "$(find "$fixture/records" -type f | wc -l | tr -d ' ')" -eq 8
test ! -e "$fixture/records/config.log"
test ! -e "$fixture/records/extra.data"
mv "$record_source/config.h" "$fixture/header"
ln -s "$fixture/header" "$record_source/config.h"
if copy_media_build_record "$record_source" "$fixture/linked-records" > "$fixture/error" 2>&1; then
  echo 'Build record symlink accepted' >&2; exit 1
fi
grep -Fq 'Missing or linked native build record' "$fixture/error"
printf 'Native build records copied; environment dumps, extras and symlinks excluded.\n'

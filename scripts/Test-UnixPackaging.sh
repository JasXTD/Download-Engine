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

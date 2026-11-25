#!/usr/bin/env bash
# Build the standalone encoder without network access or a system installation.
set -euo pipefail
output=${1:?Supply the destination executable path}
output=$(cd "$(dirname "$output")" && pwd)/$(basename "$output")
source_dir=$(cd "$(dirname "$0")" && pwd)
archive="$source_dir/lame-4.0.tar.gz"
if command -v sha256sum >/dev/null; then
    digest=$(sha256sum "$archive")
else
    digest=$(shasum -a 256 "$archive")
fi
[[ ${digest%% *} == 3df5124d5ad3a98312ffd7ba6a9b36230e4f8a3e66d3ce0f425e336c32d216eb ]] || { echo 'LAME source checksum mismatch' >&2; exit 1; }
work=$(mktemp -d "${TMPDIR:-/tmp}/de-lame.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
tar -xzf "$archive" -C "$work"
cd "$work/lame-4.0"
# LAME calls setlocale even when iconv is unavailable on macOS. Move its
# standard header beside ctype.h, outside the optional iconv block.
awk '/^#include <locale\.h>$/ { next }
     { print }
     /^#include <ctype\.h>$/ { print "#include <locale.h>" }' frontend/parse.c > frontend/parse.c.fixed
mv frontend/parse.c.fixed frontend/parse.c
ac_cv_lib_termcap_initscr=no ac_cv_lib_curses_initscr=no ac_cv_lib_ncurses_initscr=no \
    ./configure --disable-shared --enable-static --disable-decoder --disable-nasm \
        --disable-mp3x --disable-mp3rtp --disable-dynamic-frontends --with-fileio=lame
make -j"${BUILD_JOBS:-2}"
install -m 755 frontend/lame "$output"

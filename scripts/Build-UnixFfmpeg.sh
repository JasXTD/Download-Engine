#!/usr/bin/env bash
# Native Linux/macOS helper built exclusively from this repository's pinned source.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
output=${1:?Usage: Build-UnixFfmpeg.sh OUTPUT_DIRECTORY}
mkdir -p "$output"
output=$(cd "$output" && pwd)
jobs=${JOBS:-4}
[[ "$jobs" =~ ^[1-9][0-9]?$ ]] || { echo 'JOBS must be 1 through 99' >&2; exit 1; }
build=$(mktemp -d "${TMPDIR:-/tmp}/download-engine-ffmpeg.XXXXXXXX")
trap 'rm -rf -- "$build"' EXIT
export LC_ALL=C TZ=UTC SOURCE_DATE_EPOCH=1789687190
if ! command -v sha256sum >/dev/null; then
    sha256sum() { shasum -a 256 "$@"; }
    export -f sha256sum
fi
if [[ $(uname -s) == Darwin ]]; then
    # The verified downstream patch uses GNU sed's in-place argument syntax.
    sed() { if [[ ${1:-} == -i ]]; then shift; command sed -i '' "$@"; else command sed "$@"; fi; }
    export -f sed
fi
cd "$repo/tools/ffmpeg/source"
sha256sum --check SHA256SUMS
tar -xzf ffmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8.tar.gz -C "$build"
tar -xzf x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee.tar.gz -C "$build"
mkdir -p "$output/licenses" "$output/build-record" "$build/prefix"
cd "$build/x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee"
./configure --prefix="$build/prefix" --enable-static --enable-pic --disable-cli --disable-opencl --disable-asm
make -j"$jobs"
make install
cp COPYING "$output/licenses/x264-COPYING"
cp config.mak "$output/build-record/x264-config.mak"
cd "$build/FFmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8"
tr -d '\015' < "$repo/tools/ffmpeg/source/apply-security-fixes.sh" > "$build/apply-security-fixes.sh"
bash "$build/apply-security-fixes.sh"
export PKG_CONFIG_PATH="$build/prefix/lib/pkgconfig"
./configure --prefix="$build/prefix" --pkg-config-flags=--static \
    --disable-autodetect --disable-network --disable-debug --disable-doc \
    --disable-ffplay --disable-x86asm --enable-gpl --enable-version3 --enable-libx264 \
    --extra-cflags="-I$build/prefix/include" --extra-ldflags="-L$build/prefix/lib" \
    --extra-version=download-engine-security2
make -j"$jobs" ffmpeg ffprobe
cp ffmpeg ffprobe "$output/"
if [[ $(uname -s) == Darwin ]]; then
    # Sign before recording the digest embedded in the app. Signing afterwards
    # would change the helper bytes and make runtime verification reject it.
    /usr/bin/codesign --force --sign - "$output/ffmpeg"
    /usr/bin/codesign --force --sign - "$output/ffprobe"
fi
cc -std=c17 -O2 -Wall -Wextra -Werror -I. "$repo/tools/ffmpeg/source/test-hdr-metadata.c" \
    libavutil/libavutil.a -lm -pthread -o "$build/hdr-metadata-test"
"$build/hdr-metadata-test" > "$output/build-record/hdr-metadata-test.log"
cp config.h config_components.h ffbuild/config.mak ffbuild/config.log "$output/build-record/"
cp COPYING.GPLv3 "$output/LICENSE"
cp COPYING.* LICENSE.md "$output/licenses/"
cc --version > "$output/build-record/compiler-version.log"
"$output/ffmpeg" -version > "$output/build-record/ffmpeg-version.log"
cd "$output"
sha256sum ffmpeg ffprobe > build-record/binaries.sha256
printf 'Verified native helper: %s\n' "$output/ffmpeg"

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
stage=setup
cleanup() {
    result=$?
    if [[ "$result" -ne 0 ]]; then
        # Emit only a fixed stage name and exit code, never command output,
        # environment values or runner paths. CI exposes this as an annotation.
        printf '::error title=Native FFmpeg build::Stage %s exited with code %d.\n' "$stage" "$result" >&2
        # The CI wrapper redacts this output before forwarding or saving it.
        if [[ ${DOWNLOAD_ENGINE_CI_CAPTURE:-} == 1 ]]; then
            case "$stage" in
                x264-configure) diagnostic="$build/x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee/config.log" ;;
                ffmpeg-configure) diagnostic="$build/FFmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8/ffbuild/config.log" ;;
                *) diagnostic= ;;
            esac
            if [[ -n "$diagnostic" && -f "$diagnostic" ]]; then tail -n 80 "$diagnostic" >&2; fi
        fi
    fi
    rm -rf -- "$build"
    exit "$result"
}
trap cleanup EXIT
export LC_ALL=C TZ=UTC SOURCE_DATE_EPOCH=1789687190
if [[ $(uname -s) == Darwin ]] || ! command -v sha256sum >/dev/null; then
    # macOS can provide a sha256sum compatibility utility which rejects a check
    # without a filename. Use its installed shasum and make stdin explicit for
    # the canonical patch's before/after checksum here-documents.
    sha256sum() {
        if [[ $# -eq 1 && $1 == --check ]]; then
            shasum -a 256 --check -
        else
            shasum -a 256 "$@"
        fi
    }
fi
if [[ $(uname -s) == Darwin ]] && ! command sed --version >/dev/null 2>&1; then
    # The canonical patch uses GNU sed's c command. Apple's sed needs both the
    # empty backup suffix and a final newline; its legacy parser also strips
    # unescaped leading whitespace from replacement lines. Preserve the exact
    # patched bytes so the canonical output checksums remain authoritative.
    sed() {
        if [[ ${1:-} == -i ]]; then
            shift
            local program=$1
            shift
            program=$(printf '%s\n' "$program" | /usr/bin/sed '2,$s/^[[:space:]]/\\&/')
            /usr/bin/sed -i '' "$program"$'\n' "$@"
        else
            /usr/bin/sed "$@"
        fi
    }
fi
cd "$repo/tools/ffmpeg/source"
stage=source-checksums
sha256sum --check SHA256SUMS
stage=source-extraction
tar -xzf ffmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8.tar.gz -C "$build"
tar -xzf x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee.tar.gz -C "$build"
mkdir -p "$output/licenses" "$output/build-record" "$build/prefix"
# Patch before compiling x264 so a source verification failure is immediate.
cd "$build/FFmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8"
stage=security-patch
tr -d '\015' < "$repo/tools/ffmpeg/source/apply-security-fixes.sh" > "$build/apply-security-fixes.sh"
(
    # Keep the compatibility functions in this Bash process; do not depend on
    # exported-function imports across different installed Bash versions.
    trap - EXIT
    . "$build/apply-security-fixes.sh"
)
cd "$build/x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee"
stage=x264-configure
./configure --prefix="$build/prefix" --enable-static --enable-pic --disable-cli --disable-opencl --disable-asm
stage=x264-build
make -j"$jobs"
stage=x264-install
make install
cp COPYING "$output/licenses/x264-COPYING"
cp config.mak "$output/build-record/x264-config.mak"
cd "$build/FFmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8"
export PKG_CONFIG_PATH="$build/prefix/lib/pkgconfig"
stage=ffmpeg-configure
./configure --prefix="$build/prefix" --pkg-config-flags=--static \
    --disable-autodetect --disable-network --disable-debug --disable-doc \
    --disable-ffplay --disable-x86asm --enable-gpl --enable-version3 --enable-libx264 \
    --extra-cflags="-I$build/prefix/include" --extra-ldflags="-L$build/prefix/lib" \
    --extra-version=download-engine-security2
stage=ffmpeg-build
make -j"$jobs" ffmpeg ffprobe
cp ffmpeg ffprobe "$output/"
if [[ $(uname -s) == Darwin ]]; then
    stage=helper-signing
    # Sign before recording the digest embedded in the app. Signing afterwards
    # would change the helper bytes and make runtime verification reject it.
    /usr/bin/codesign --force --sign - "$output/ffmpeg"
    /usr/bin/codesign --force --sign - "$output/ffprobe"
fi
if [[ "${DOWNLOAD_ENGINE_SKIP_FFMPEG_TESTS:-0}" == 1 ]]; then
    printf 'Not rerun: FFmpeg functional tests explicitly skipped for this app release.\n' > "$output/build-record/hdr-metadata-test.log"
else
    stage=hdr-test-build
    cc -std=c17 -O2 -Wall -Wextra -Werror -I. "$repo/tools/ffmpeg/source/test-hdr-metadata.c" \
        libavutil/libavutil.a -lm -pthread -o "$build/hdr-metadata-test"
    stage=hdr-test-run
    "$build/hdr-metadata-test" > "$output/build-record/hdr-metadata-test.log"
fi
stage=build-record
# configure's config.log includes `set` output (the complete environment).
# Keep it private to the temporary build; failures use the redacted CI report.
cp config.h config_components.h ffbuild/config.mak "$output/build-record/"
cp COPYING.GPLv3 "$output/LICENSE"
cp COPYING.* LICENSE.md "$output/licenses/"
cc --version > "$output/build-record/compiler-version.log"
"$output/ffmpeg" -version > "$output/build-record/ffmpeg-version.log"
cd "$output"
sha256sum ffmpeg ffprobe > build-record/binaries.sha256
printf 'Built native helper: %s\n' "$output/ffmpeg"

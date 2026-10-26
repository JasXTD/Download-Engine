#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C TZ=UTC SOURCE_DATE_EPOCH=1789687190
export PATH=/usr/bin:/bin
export PKG_CONFIG_LIBDIR=/build/prefix/lib/pkgconfig
export PKG_CONFIG_PATH=
jobs="${JOBS:-8}"
[[ "$jobs" =~ ^[1-9][0-9]?$ ]] || { echo 'JOBS must be 1 through 99' >&2; exit 1; }
cd /input
sha256sum --check SHA256SUMS
mkdir -p /build /output/build-record /output/licenses
cd /build
tar -xzf /input/ffmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8.tar.gz
tar -xzf /input/x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee.tar.gz
cd x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee
./configure --host=x86_64-w64-mingw32 --cross-prefix=x86_64-w64-mingw32- \
    --prefix=/build/prefix --enable-static --disable-cli --disable-opencl \
    --extra-ldflags=-static
make -j"$jobs"
make install
cp config.mak /output/build-record/x264-config.mak
cp config.h /output/build-record/x264-config.h
cp COPYING /output/licenses/x264-COPYING
cd /build/FFmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8
bash /input/apply-security-fixes.sh
./configure --arch=x86_64 --target-os=mingw32 --cross-prefix=x86_64-w64-mingw32- \
    --prefix=/build/prefix --pkg-config=pkg-config --pkg-config-flags=--static \
    --disable-autodetect --disable-network --disable-debug --disable-doc \
    --disable-ffplay --disable-pthreads --enable-w32threads \
    --enable-gpl --enable-version3 --enable-libx264 \
    --extra-cflags=-I/build/prefix/include --extra-ldflags='-L/build/prefix/lib -static' \
    --extra-version=xdownloader-security2
make -j"$jobs" ffmpeg.exe ffprobe.exe
cp ffmpeg.exe ffprobe.exe /output/
# Test the same Windows library that is linked into the helper. Run this
# test-only executable on Windows before redistributing a rebuilt bundle.
x86_64-w64-mingw32-gcc -std=c17 -O2 -Wall -Wextra -Werror -static -I. \
    /input/test-hdr-metadata.c libavutil/libavutil.a -lm -luser32 -lbcrypt -latomic \
    -o /output/hdr-metadata-test.exe
cp config.h config_components.h ffbuild/config.mak ffbuild/config.log /output/build-record/
cp COPYING.GPLv3 /output/LICENSE
cp COPYING.* LICENSE.md /output/licenses/
dpkg-query -W -f='${Package}\t${Version}\n' | sort > /output/build-record/toolchain-packages.tsv
x86_64-w64-mingw32-gcc --version > /output/build-record/compiler-version.log
x86_64-w64-mingw32-objdump -p /output/ffmpeg.exe > /output/build-record/ffmpeg-pe.log
x86_64-w64-mingw32-objdump -p /output/ffprobe.exe > /output/build-record/ffprobe-pe.log
cp /usr/share/doc/mingw-w64-common/copyright /output/licenses/mingw-w64-COPYRIGHT
cp /usr/share/doc/gcc-mingw-w64-x86-64-win32/copyright /output/licenses/gcc-runtime-COPYRIGHT
cd /output
sha256sum ffmpeg.exe ffprobe.exe hdr-metadata-test.exe > build-record/binaries.sha256

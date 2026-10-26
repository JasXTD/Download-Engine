# Corresponding source for the bundled Windows media helper

This directory ships with the executable, including in the installer. It contains
the exact upstream archives and downstream security edits used to build `../ffmpeg.exe`:

- FFmpeg revision `946fcce07b6dcd0331c8cc609192aeff5e1924f8` (9.0.2):
  `ffmpeg-946fcce07b6dcd0331c8cc609192aeff5e1924f8.tar.gz`.
  Origin: https://github.com/FFmpeg/FFmpeg/commit/946fcce07b6dcd0331c8cc609192aeff5e1924f8
- x264 revision `0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee`:
  `x264-0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee.tar.gz`.
  Origin: https://code.videolan.org/videolan/x264/-/commit/0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee

`SHA256SUMS` records both unmodified source-archive checksums.
`apply-security-fixes.sh` applies three downstream changes to FFmpeg: iterative
encryption-initialization cleanup for CVE-2026-107678 and immediate PSSH rejection
in the MOV parser, plus the upstream HDR10+ serialization fix for CVE-2026-107676
([commit 2c2f6e96](https://github.com/FFmpeg/FFmpeg/commit/2c2f6e96e31795ae95a8f8323a493ffbc493111f)).
The HDR10+ fix writes saturation fields independently of tone mapping, so the
complete advertised payload is initialized. This dedicated helper supports clear media only. Both input
and output source-file hashes are checked; an unexpected revision or partial
edit stops the build. x264 is unchanged.
`build.sh` applies these edits and contains the full configure and build commands. Network access and
automatic third-party library discovery are disabled in FFmpeg. x264 is the only
separately sourced media library; GCC/MinGW are the general-purpose compiler and
Windows runtime. The FFmpeg executable uses GPLv3; see `../LICENSE` and
`../licenses/`. x264's license permits GPL version 2 or later.

## Rebuild with Docker Desktop (Linux containers)

Run these PowerShell commands from this directory. The image setup downloads
Ubuntu toolchain packages; the actual source build has no network access. No
system PATH, proxy, antivirus, or compiler installation is changed on Windows.
Choose unused container/image names if running two builds at once.

```powershell
docker build --pull=false -t xdownloader-ffmpeg-builder .
docker create --name xdownloader-ffmpeg-build --network none --cap-drop ALL --security-opt no-new-privileges --pids-limit 512 --memory 8g --cpus 8 xdownloader-ffmpeg-builder
docker cp . xdownloader-ffmpeg-build:/input
docker start --attach xdownloader-ffmpeg-build
docker inspect xdownloader-ffmpeg-build --format '{{.State.ExitCode}}'
New-Item -ItemType Directory -Path rebuilt
docker cp xdownloader-ffmpeg-build:/output/. rebuilt
.\rebuilt\hdr-metadata-test.exe
if ($LASTEXITCODE -ne 0) { throw 'HDR10+ regression check failed.' }
docker rm xdownloader-ffmpeg-build
docker image rm xdownloader-ffmpeg-builder
```

The exit code must be zero before using the output. `rebuilt/ffmpeg.exe` is the
runtime helper; `ffprobe.exe` and `hdr-metadata-test.exe` are test-only tools and
are not included in the runtime package. The HDR10+ test checks all 84 combinations
of tone-mapping and saturation flags across one to three windows. Differently
prefilled buffers must produce identical, fully initialized payloads; caller-owned
and allocated outputs must agree, preserve guard bytes, and round-trip metadata.
It links the same Windows libavutil archive as the helper. Preserve the build
records and notices alongside any redistributed result. `build-record/` in this
source directory records the compiler/packages and configuration of the shipped
binary. Compiler and system package updates can change output hashes.

XDownloader pins the helper's hash to prevent executing a substituted program.
Developers rebuilding it must deliberately update `media-manifest.json` and the
hash in `src/hls/merge.rs`, then rerun the merge tests. The MIT application source
is available to make that change. This does not restrict rights granted by GPL.

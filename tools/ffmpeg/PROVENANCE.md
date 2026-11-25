# Bundled media merger

FFmpeg 9.0.2, Windows x64, built by the Download Engine project on
2026-11-25 from the upstream archives and downstream security edits in `source/`.
It is a separate executable used to remux downloaded, local HLS/DASH tracks
without re-encoding. The application never passes it a network URL. Network
support and automatic external-library detection are disabled in this build.

- FFmpeg revision: `946fcce07b6dcd0331c8cc609192aeff5e1924f8`
  ([upstream](https://github.com/FFmpeg/FFmpeg/commit/946fcce07b6dcd0331c8cc609192aeff5e1924f8)).
- x264 revision: `0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee`
  ([upstream](https://code.videolan.org/videolan/x264/-/commit/0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee)).
- `ffmpeg.exe` SHA-256:
  `4ac14da35b445022c0a339f1bf39f56d26b69d1501d7b7317de24c179d349854`.
- Test-only `ffprobe.exe` SHA-256:
  `7a824a8ed76735944217f0acbbd690ae96f038ada223aa0afe446ecf8aabccc2`.
- Combined helper license: **GPL-3.0-or-later**, configured with `--enable-gpl`
  and `--enable-version3`. x264 permits GPL version 2 or later. Full license and
  copyright notices, including compiler/runtime notices, are in `LICENSE` and
  `licenses/`. Download Engine's MIT license does not replace these terms.

`source/SHA256SUMS` pins both source archives. `source/Dockerfile` and
`source/build.sh` provide the complete build recipe. `source/build-record/`
contains the actual configuration, compiler/package versions, Windows imports
and output hashes. `source/apply-security-fixes.sh` replaces recursive
encryption-initialization cleanup with iteration (CVE-2026-107678) and rejects
PSSH before allocation in the MOV parser, matching the clear-media-only policy.
It also backports the upstream HDR10+ serialization fix for CVE-2026-107676
([commit 2c2f6e96](https://github.com/FFmpeg/FFmpeg/commit/2c2f6e96e31795ae95a8f8323a493ffbc493111f)),
writing saturation fields independently of tone mapping. The Vorbis parser
backport for CVE-2026-125224 records a header initialization attempt even if it
fails ([upstream fix](https://github.com/FFmpeg/FFmpeg/commit/90f5e61a1da2dcc0050169af8ca593530f26071b)).
The script verifies each patched source file before and after the edits. See
`source/README.md` for rebuilding and updating the application's helper pin.

The portable ZIP and installer include these sources, recipes, records and
notices with the binary. Keep the complete bundle together when redistributing
it; an upstream website link alone is not the supplied corresponding source.
The source ZIP also includes this material, excluding executable binaries.

The Windows runtime uses `ffmpeg.exe`.
FFprobe and the HDR10+ and Vorbis regression programs are test tools. Their C tests
are included under `source/`. The app checks the helper's pinned hash and
holds it against replacement while it runs. `media-manifest.json` records every
bundle file; packaging verifies those hashes and rejects missing/unlisted files.

## Native macOS/Linux previews

Native packages build the same pinned sources and safety fixes with the included
`scripts/Build-UnixFfmpeg.sh`. Their runtime helper is named `ffmpeg`. Their native
compiler, configuration, regression result and binary hashes are recorded in
`tools/ffmpeg/build-record/`; the Windows binary hashes and toolchain records
above do not describe a native binary. The native helper hash is compiled into
the matching app and verified again when packaging and running it. Native builds
also use ffprobe for tests, but do not ship that test tool in the application.

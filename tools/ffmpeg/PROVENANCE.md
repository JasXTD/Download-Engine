# Bundled media merger

FFmpeg 9.0.2, Windows x64, built by the Download Engine project on
2026-10-22 from the upstream archives and downstream security edits in `source/`.
It is a separate executable used to remux downloaded, local HLS/DASH tracks
without re-encoding. The application never passes it a network URL. Network
support and automatic external-library detection are disabled in this build.

- FFmpeg revision: `946fcce07b6dcd0331c8cc609192aeff5e1924f8`
  ([upstream](https://github.com/FFmpeg/FFmpeg/commit/946fcce07b6dcd0331c8cc609192aeff5e1924f8)).
- x264 revision: `0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee`
  ([upstream](https://code.videolan.org/videolan/x264/-/commit/0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee)).
- `ffmpeg.exe` SHA-256:
  `f1f9097330cc07678bebe983e1e5854bacce5f36cd851e234cdcf074ea54d577`.
- Test-only `ffprobe.exe` SHA-256:
  `f4faba94399610c63c39998e35479b84fd0515bad753016689dd026027b697c9`.
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
writing saturation fields independently of tone mapping. The script verifies
source hashes before and after all three changes. x264 is unchanged. See
`source/README.md` for rebuilding and updating the application's helper pin.

The portable ZIP and installer include these sources, recipes, records and
notices with the binary. Keep the complete bundle together when redistributing
it; an upstream website link alone is not the supplied corresponding source.
The source ZIP also includes this material, excluding executable binaries.

Only `ffmpeg.exe` is a runtime helper; ffprobe and the HDR10+ regression executable
are used for isolated tests and are not shipped in the application. The regression
source is included in `source/test-hdr-metadata.c`. The app checks the helper's pinned hash and
holds it against replacement while it runs. `media-manifest.json` records every
bundle file; packaging verifies those hashes and rejects missing/unlisted files.

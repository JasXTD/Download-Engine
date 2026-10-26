FFmpeg 9.0.2 - Windows x64

Built by the Download Engine project with the documented FFmpeg security changes.
PSSH metadata is rejected before allocation; initialization lists are freed
iteratively. The upstream HDR10+ metadata memory-disclosure fix is backported.
x264 sources are unchanged. See source/apply-security-fixes.sh.
License: GNU GPL version 3 or later. This helper is not MIT licensed.

LICENSE and licenses/ contain the license and copyright notices.
source/ contains the exact corresponding sources, checksums, complete build
recipe and build records. Read source/README.md to rebuild this helper.
PROVENANCE.md records revisions and output hashes.

This executable is used separately from Download Engine to merge local downloaded
media tracks without re-encoding. Network support and automatic discovery of
third-party libraries are disabled. x264 is the only external media library.

Keep the complete source and license bundle with redistributions.
FFmpeg project: https://ffmpeg.org/

# FFmpeg 9.0.2 for Windows x64

Download Engine builds this helper under the GNU GPL version 3 or later. It
rejects PSSH metadata before allocation and frees initialization lists
iteratively. The build includes the upstream HDR10+ metadata and Vorbis parser
fixes and uses unmodified x264 sources. See the
[security patch script](source/apply-security-fixes.sh).

[LICENSE](LICENSE) and [licenses/](licenses/) contain the license and copyright
notices. The [rebuild instructions](source/README.md) accompany the exact
sources, checksums, build recipe and records. [PROVENANCE.md](PROVENANCE.md)
records the revisions and output hashes.

FFmpeg runs as a separate program. This build can merge downloaded tracks
without re-encoding and decode local audio. Network support and automatic
library discovery are disabled. x264 is the only external media library.

Keep the complete source and license bundle with redistributions.
[FFmpeg project](https://ffmpeg.org/)

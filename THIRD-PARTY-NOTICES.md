# Third-party software and sources

Download Engine's original code is Copyright (c) 2026 Xetro and licensed under
[MIT](LICENSE). That license does not replace the licenses of the components
below. Their copyright notices, licenses and source terms remain in effect.

## FFmpeg and x264

The separate `tools/ffmpeg/ffmpeg.exe` helper is distributed under GPL version 3.
It is built from unmodified FFmpeg and x264 sources. Download Engine invokes it as a
separate command-line program to merge local tracks without re-encoding.

- License: [GNU GPLv3](tools/ffmpeg/LICENSE).
- Complete corresponding source: the two original upstream source archives in
  [tools/ffmpeg/source](tools/ffmpeg/source/README.md), included in this package.
- Build instructions: `tools/ffmpeg/source/README.md`, `Dockerfile`, and `build.sh`.
- Exact revisions, source checksums and build records:
  [FFmpeg provenance](tools/ffmpeg/PROVENANCE.md).
- Additional license texts, including x264 and the compiler runtime notices:
  `tools/ffmpeg/licenses/`.

No separate download or source-code request is required to obtain these sources.
If redistributing a binary package, keep these materials with it. Publishing
Download Engine's MIT source alone does not replace this source distribution.

## Rust dependencies and fonts

See the [complete dependency index](docs/licenses/RUST-DEPENDENCIES.md) and
`docs/licenses/rust-dependencies.json` for versions, license texts and source
checksums. The index includes build/test dependencies as well as runtime crates.
The dependencies are unmodified except epaint 0.36.2, which Download Engine builds from
a modified copy in `vendor/epaint` that adds bidirectional text layout. It remains
Apache-2.0 licensed; `vendor/epaint/patch.md` describes the changes, and
the index includes the original crate. Where `self_cell` offers Apache-2.0 or
GPL-2.0, the Apache-2.0 option is used.

The MPL-2.0 sources for cssparser, cssparser-macros, dtoa-short and selectors are
included as their original `.crate` archives in `docs/licenses/sources/`.
A `.crate` is a gzip-compressed tar archive; extract it with `tar -xzf file.crate`.
These components remain MPL-2.0; Download Engine's own code remains MIT.

The default egui fonts retain their MIT, Bitstream Vera, SIL Open Font and Ubuntu
Font licenses; all supplied font notices are in the epaint_default_fonts entry
of the index. Windows fallback fonts are read from the user's system and are
not redistributed. Rust's standard-library notices are included in
[Rust-standard-library.html](docs/licenses/Rust-standard-library.html).

## Other included components

- The adapted yt-dlp extraction code retains the
  [Unlicense notice](docs/licenses/yt-dlp-LICENSE); see
  [source attribution](docs/youtube-port.md).
- Lua and mlua retain their [MIT notices](docs/licenses/lua-automation-LICENSES).
- Inno Setup and its translations retain the
  [Inno Setup license](docs/licenses/Inno-Setup-LICENSE). Translation provenance
  is recorded in the source repository's `installer/languages/provenance.json`.

Unsigned packages do not have an Authenticode publisher signature. Signing
status does not alter any of these licenses.

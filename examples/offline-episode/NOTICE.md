# Source and license

`extension/lib/upstream.lua` adapts these helpers from
[CuteTenshii/crunchyroll-downloader](https://github.com/CuteTenshii/crunchyroll-downloader/tree/734421c9c1268bdd3716ba65d429ec54ae5f2358):

- `download.go`: `requestsAll`, `expandLangs`, `buildUrl`.
- `mpd.go`: `expandTimeline`.

Pinned commit: `734421c9c1268bdd3716ba65d429ec54ae5f2358`.
The Go code was read as source, not compiled or executed. No upstream dependencies,
credentials, DRM devices, licenses or media were imported.

Changes: translated the helpers to Lua, copied the available-language list before
sorting, bounded the demo's timeline expansion and rejected unsupported template
tokens. `$Number$` retains upstream's five-digit padding. The provider adds locale
deduplication and fails for unavailable locales. The catalog, provider integration,
sample generator and server are Download Engine example code under the project's
MIT license. Generated audio/video/subtitles contain no upstream service content.

SHA-256 of the inspected upstream UTF-8 files:

| File | SHA-256 |
| --- | --- |
| `download.go` | `28b98c51addee412d6ea5e4701db837389cba68e453743d6fff91c71bcbdb767` |
| `mpd.go` | `9c4173e57affb393718c90f16129c66c9586c50c6631013114cf7d0ce1e96f3d` |
| `LICENSE.txt` | `08f469694c5a7d6c3e21afb56171ff1fe64ef18cb253007dab1b0cad39e585c7` |

The MIT notice below was verified at that commit. It is also included inside the
adapted Lua module so that sharing the extension JSON preserves the notice.

Copyright 2025-2026 CuteTenshii

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

# Installer language provenance

The `.isl` files come from the official Inno Setup source repository at the
commit pinned in `provenance.json`. Its SHA-256 values identify the original
upstream bytes, before any changes described here. Original translator credits
and comments are retained inside the files.

ChineseSimplified.isl and ChineseTraditional.isl are unchanged upstream files.
Malaysian.isl originally targeted Inno Setup 5.1. It has been updated locally for
the bundled Inno Setup 6.5 compiler: obsolete messages were removed, 85 missing
nonempty messages were translated and the language name is Bahasa Melayu.
Its current file hash therefore differs from the upstream provenance hash.

`tests/localization.test.cjs` checks all compiler message keys and substitution
tokens against the installed compiler's Default.isl. Setup-specific messages
are maintained in `installer/DownloadEngine.iss`. The tiny JSON files contain only locale
identifiers and seed a fresh installed app profile; they never execute code.

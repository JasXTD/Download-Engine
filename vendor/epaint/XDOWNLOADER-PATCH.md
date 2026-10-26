# epaint 0.36.2, modified for XDownloader

This directory is the published crate [epaint 0.36.2](https://crates.io/crates/epaint/0.36.2)
(crates.io SHA-256 `a0ec0308dc23130fc1a7d4623a8534c2ed069c6dc84e258ee4f3de99615aa1a3`,
upstream commit in `.cargo_vcs_info.json`), used through `[patch.crates-io]` in the
XDownloader `Cargo.toml`. epaint is licensed under MIT OR Apache-2.0; XDownloader uses it
under Apache-2.0 (`docs/licenses/common/Apache-2.0`). The changes below are XDownloader's
and are released under the same terms.

## Why

Upstream epaint 0.36.2 splits text into runs by font only and lets the shaper guess each
run's direction from its first letter. In an Arabic run, Latin words, digits and
punctuation were therefore drawn reversed ("swodniW" for "Windows", "06" for "60"), and
wrapped right-to-left paragraphs showed their rows in reverse order. Upstream has no
bidirectional layout yet; drop this patch when an egui release provides it.

## What changed

- `src/text/bidi.rs` (new): embedding levels from a subset of the Unicode Bidirectional
  Algorithm (UAX #9): weak types W1–W7, paired brackets N0, neutrals N1–N2, implicit
  levels I1–I2 and separators in L1. Character properties come from `icu_properties`
  2.3.0, which the application already uses through `url`. Explicit embeddings,
  overrides and isolates are not supported and stay invisible, as before.
- `src/text/text_layout.rs`: a paragraph that contains right-to-left script characters
  is laid out with bidi levels. It reads right to left when it contains a right-to-left
  letter. Runs split where the font or the level changes and are shaped in their own
  direction. Glyphs are kept in logical order for line breaking, and each row is then
  put into visual order (L1 trailing whitespace, L2), returning glyphs to their shaped
  positions. Clusters that were neighbours when shaped move by the same whole-pixel
  offset, so joined Arabic letters are unchanged. In right-to-left runs, zero-width
  continuation glyphs are now added to the cluster they belong to, which keeps one
  glyph per character. Text without right-to-left characters takes the upstream path
  unchanged.
- `src/text/mod.rs`: declares the `bidi` module.
- `Cargo.toml`: adds the `icu_properties` dependency.

Not changed: sections of a layout job are each resolved on their own (with the
paragraph's direction); the caret, selection and cursor movement in text edits are
not direction-aware; the native interface is not mirrored.

## Checking or updating the copy

Extract the upstream crate from the local Cargo cache and compare:

```powershell
tar -xzf $env:USERPROFILE\.cargo\registry\cache\index.crates.io-1949cf8c6b5b557f\epaint-0.36.2.crate -C $env:TEMP
git diff --no-index $env:TEMP\epaint-0.36.2 vendor\epaint
```

Only the files listed above and this file differ (`.cargo-ok` is not copied). When
egui is upgraded, re-apply the change to the new epaint version or remove the patch,
and update `patchedCrates` in `scripts/Build-LicenseBundle.cjs`.

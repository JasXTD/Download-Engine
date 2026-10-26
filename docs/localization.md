# Languages

Download Engine 1.13.0 includes English, Simplified Chinese, Traditional Chinese, Malay, Japanese, Korean, Vietnamese, Thai, Polish, Russian, Ukrainian, Arabic, German, French, Swedish, Norwegian Bokmål, Swiss standard German, Italian and Romansh (Rumantsch Grischun) for the desktop interface, its offline quick guide, the Windows installer and the Chrome, Edge and Firefox extensions.

## Choose a language

Desktop: open Preferences → Appearance → Language. Choose System language or one of the nineteen language choices. Changes apply after Save and persist across restarts. System language follows the Windows display language, including regional variants such as ja-JP, ko-KR, vi-VN, th-TH, pl-PL, ru-RU, uk-UA, ar-SA, ar-EG, de-DE, de-CH, fr-CH, sv-SE, nb-NO, no-NO, it-IT, it-CH and rm-CH. Unsupported languages use English. Existing downloads, custom names, folders, passwords and scripts are unchanged.

Installer: select a language when Setup opens. Setup remembers the previous installation language. A fresh installed profile starts in that language; upgrades preserve an explicit app choice, including System language. A portable copy follows Windows until you select a language in Preferences.

Extension: open Browser options → Extension language in the popup. It follows the browser by default, independently of the app. The popup reloads after selection. Reload an already open link picker or login review to refresh its static labels; save or resolve pending work first. Existing video buttons update without interrupting playback. Chrome, Edge, Brave and Firefox use the same catalogs. Extension names and descriptions in browser management follow the browser locale.

## What remains unchanged

CLI commands, JSON/API fields, MCP tool names, script language keywords, persisted identifiers and status codes stay in English for compatibility. User content, filenames, paths, URLs, script output and raw operating-system/server/scanner errors retain their original text. Translated context explains the operation around these technical messages. Missing translations fall back to English.

Help → Guide includes translated quick steps for all twelve topics and executable Lua/CLI examples. The longer developer/API references and historical release notes remain in English. Translation does not change authentication, proxy routing, scheduling or automation behavior.

The desktop uses installed Windows fonts, including Microsoft YaHei, Microsoft JhengHei, SimSun, Yu Gothic or Meiryo, Malgun Gothic, Leelawadee UI and Tahoma where available. If characters appear as boxes, install the appropriate Windows language/font support and restart. These Microsoft font files are not redistributed with Download Engine.

Arabic extension pages and Setup use right-to-left layout. The native desktop keeps its left-to-right layout, but its text follows the Unicode Bidirectional Algorithm: a label that contains Arabic letters reads right to left, and Latin words, numbers and paths inside it keep their reading order. For example, "Windows", "powercfg /hibernate on" and queue names read normally, a percentage reads "62.4" then "%", and wrapped Arabic lines read from top to bottom. Full interface mirroring and caret movement in mixed-direction text fields are not supported. Language changes do not modify the web page hosting an extension video button.

The bidirectional layout is a patch to egui's text renderer in `vendor/epaint` (see its `patch.md`). Text without right-to-left characters is laid out exactly as before. `src/desktop/theme/tests.rs` checks rendered Arabic layouts.

## Maintain translations

`locales/languages.json` records language codes, native names, Rust variants and installer message files. English is first, followed by the original three languages and the fifteen additions. `locales/desktop.json` maps an English display string to Simplified Chinese, Traditional Chinese and Malay. `locales/additional.json` supplies Japanese, Korean, Vietnamese, Thai, Polish, Russian, Ukrainian, Arabic, German, French, Swedish, Norwegian Bokmål, Swiss standard German, Italian and Romansh (Rumantsch Grischun), in that order, for both desktop and extension source keys. Keep the ordering aligned with the registry. `scripts/localization-build.rs` generates lookup code and `tf!` formatting arms during the Rust build. Keep format placeholders and their types unchanged; rustc validates every used translated format. Use `tr` for app-owned labels only, never for arbitrary user content or stored identifiers. Language scopes are thread-local; workers retain raw records and UI results are rendered under the current language.

`locales/extension.json` uses numbered placeholders, such as `{0}`. Run `node scripts/Build-Locales.cjs` after editing either catalog, or `node scripts/Build-Locales.cjs --check` to verify generated assets. It creates `_locales` metadata and the shared `i18n-catalog.js` data file. `i18n.js` resolves browser locale and the saved override. Translate authored DOM once before filling user data; never automatically translate arbitrary dynamic page content. Functions injected with `scripting.executeScript` must be self-contained.

The installer uses Inno Setup language files in `installer/languages`. `provenance.json` records pinned upstream sources or the existing reviewed compiler's bundled files and their original hashes. The Malay file has been updated locally for the bundled compiler's complete message set. App-specific setup messages live in `installer/DownloadEngine.iss`; language marker files seed only profiles without an explicit preference.

## Validation

Run `node --test tests/localization.test.cjs` and the existing extension suites; run Rust tests, formatting and Clippy as described in `docs/releases.md`. `scripts/localization_ui_smoke.cjs` renders extension UI in isolated Chrome and Edge contexts; browser APIs are simulated. `scripts/native_ui_smoke.cjs --localization` captures the real native renderer using isolated paused profiles. Installer tests must use a temporary installation, test AppId, no shortcuts and no browser registration changes. Keep human linguistic review and live Firefox verification separate from automated results.

## Regional language choices

Deutsch (Schweiz) is written Swiss standard German, using ss instead of ß; it is not a dialect transcription. de-CH selects it automatically. fr-CH selects French. Norwegian Bokmål uses no as the saved choice and also recognizes nb-NO. Italian (it-IT and it-CH) selects Italiano; rm-CH selects Rumantsch Grischun, the written standard used by this translation. Nynorsk (nn) is not included and falls back to English.

Extension metadata includes both no and the nb alias. The alias does not add a duplicate picker option. Chromium's supported locale list uses no, while regional extension catalogs such as de_CH are supported by [locale fallback](https://developer.chrome.com/docs/extensions/reference/api/i18n).

Setup's German Swiss file derives from the existing reviewed compiler's German translation with Swiss spelling and LanguageID $0807. Setup first matches the full Windows language ID, then the primary language, then English (the first entry); see [Inno Setup language selection](https://jrsoftware.org/ishelp/topic_languagessection.htm). Its language dialog remains available, and existing installation language preferences take precedence.

Italian Setup messages use the existing reviewed compiler’s bundled Italian translation. Romansh Setup messages are a Download Engine-authored translation of its complete English message schema, using [Windows language ID 0417](https://learn.microsoft.com/it-it/openspecs/windows_protocols/ms-adts/a29e5c28-9fb9-4c49-8e43-4b9b8e733a05). The provenance file identifies the authoring source and hashes; it does not claim an upstream Romansh translation.

The extension’s manual language picker supports all nineteen choices regardless of the browser’s installed UI languages. Browser-managed extension names and descriptions depend on that browser’s supported locale set; an unsupported browser locale uses the manifest’s English default. The app’s own Italian and Romansh labels do not depend on browser metadata support.

For 1.13.0, runtime suites, UI/browser checks and installer execution were skipped at the user’s request. Catalog generation, placeholder checks, release compilation and package inspection are recorded separately. Native-speaker linguistic review of the new translations is still pending.

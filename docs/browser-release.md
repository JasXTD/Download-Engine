# Browser release preparation

The local installer and in-app host registration are implemented. Local unsigned
builds create `download-engine-{chrome,edge,firefox}-<version>-UnsignedDev.zip` in `dist`
for developer loading and testing. These are not signed or published browser
releases and must not be submitted. Version-only extension ZIPs are generated
in `dist/releases/<version>` by signed packaging or the explicit
`-AllowUnsignedRelease` workflow; they are submission candidates, not
proof of browser-store approval or Firefox signing.

## Public release steps

1. Prepare the version-only packages with the public packaging workflow, then
   upload `download-engine-chrome-<version>.zip` to the Chrome Web Store developer dashboard
   and record the assigned public extension ID. Publish only after reviewing
   the listing, privacy declarations, and requested permissions. Brave installs
   from the Chrome Web Store with the same ID, so this listing also serves Brave;
   Download Engine registers Brave's connection separately.
2. Upload `download-engine-edge-<version>.zip` to Microsoft Edge Add-ons and record its ID.
   The Edge ID may differ from Chrome. Users can already paste either ID into
   the corresponding browser's connection panel.
3. Submit `download-engine-firefox-<version>.zip` from the public release folder
   to Mozilla for signing (not a `.7z` archive), keeping the existing
   `download-engine@xetro` add-on ID for the first public listing. Keep this ID
   for later updates. Choose a listed release on AMO or
   a signed unlisted XPI for direct distribution. Verify the current submission
   validation before uploading. The package requires Firefox 142 or newer;
   select Windows as the supported platform, not Android. The release preflight
   checks the data declaration and minimum version.
4. Supply the publisher name, support/contact URL, privacy policy URL, store
   artwork, and final signing identity. These have not been invented or published.
5. Once listings are approved, replace developer-loading instructions with their
   actual store links and verify permanent installation, update, native messaging,
   disconnect, and uninstall in all three real browsers.
6. Run `Package.ps1` and `Build-Installer.ps1` with `-SignTool`,
   `-CertificateThumbprint`, and `-TimestampUrl`. The scripts fail closed unless
   both desktop executables, Setup, and the generated uninstaller are signed with
   the configured publisher certificate. This repository does not contain that
   certificate. `-AllowUnsignedDevelopment` is only for local test artifacts;
   those archives use `UnsignedDev` names and require `-TestBuild` for installers.
   Alternatively, `-AllowUnsignedRelease` produces explicitly unsigned public
   Windows packages for GitHub; see `docs/github-release.md`. Desktop Authenticode
   signing does not sign a Firefox extension or publish any browser-store listing.

## Data handling for the publisher's privacy declaration

The published [privacy policy](../PRIVACY.md) covers the extension's data handling,
local desktop integration, retention and user controls. Use
`https://github.com/JasXTD/Download-Engine/blob/main/PRIVACY.md` for the browser
store's privacy-policy URL. Keep store declarations consistent with that policy
and the released extension's behaviour.

The extension sends URLs and optional filenames to the local native host when
requested. File capture and passive media detection default on; explicit saved
off choices are preserved. Downloads, webRequest and HTTP/HTTPS website access
are requested at install time. Browser restrictions and withheld access are still
enforced. Capture rules continue to filter types, sizes and excluded sites. If
the desktop is unavailable, the download stays in the browser. It stores capture settings and handoff-recovery
records in local extension storage. Download Engine stores URLs, filenames, progress,
and queue records locally. Download URLs may contain access tokens.

Page-link scanning uses `activeTab` and `scripting`: only a user-requested scan
injects the packaged, read-only collector into the isolated top frame. It reads
HTTP/HTTPS links, suggested filenames, link text, page title and URL. The asset
picker also reads existing image/srcset and common lazy-image attributes,
audio/video sources and posters, bounded CSS background images, and open shadow
roots. It does not traverse closed shadow roots. Direct-video
scans also read top-frame video/source URLs, declared video MIME types, and HLS/DASH
manifest URLs in that document's resource-timing entries. Blob URLs and players
with attached DRM keys are excluded. Stream manifests open a desktop preview
instead of being sent as direct file batches. There is no automatic cookie access,
frame scanning, crawling, remote scripts, or browser network previews. Page data stays in
`storage.session` memory, with at most five open/pending pickers. Closing an
unsent or completed picker clears it; pending selections survive worker restarts
but expire when the browser session ends or the extension is reloaded/disabled.

**Detect media while browsing** defaults on and can be disabled independently.
It uses `webRequest` plus HTTP/HTTPS host access, including media CDN hosts. Successful
GET response URLs, MIME types and reported file sizes are read passively; request
headers, cookies and response bodies are never read by this feature. Obvious
media segments are filtered out. No preview fetch or download starts from an
observed response. Choosing an item opens the existing review, with streams
requiring desktop preview before queueing. DRM and live/encrypted stream support
is unchanged. Broad host access does not grant cookie access.

The detector stores its setting locally and a bounded media list in
`storage.session`: up to 16 tabs and 40 entries each. Entries older than 30 minutes
are pruned on subsequent detector activity. URLs can include sensitive signed
tokens. Navigation, tab closure, Clear, turning detection off and permission
revocation clear applicable lists. New playback can repopulate a cleared list.
Turning detection off does not revoke permissions shared with browser-login or
on-page buttons; users can revoke them in browser settings. Private windows are
excluded. The popup and toolbar badge show the active tab's media count;
existing handoff errors take badge priority. This access must be declared
accurately in each store listing. See [Chrome webRequest](https://developer.chrome.com/docs/extensions/reference/api/webRequest)
and [Firefox webRequest](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/webRequest).

**Video button on this site** remains visible in Browser options in both Firefox
and Chromium, independently of **Detect media while browsing**. It defaults on
within existing HTTP/HTTPS website access. With no grant, it stays checked and
shows **Allow this site**; a user click requests access where the browser supports it.
The shipped manifests request HTTP/HTTPS host access during installation. Grants update already-open
pages and future visits, except sites explicitly turned off. Changing media
detection does not override a saved video-button preference.

The packaged isolated-world content script observes video elements/layout locally and shows a button
outside the player: above, or below/beside it when space is limited. It hides
when no outside space is available and does not cover neighboring video players.
On YouTube it is shown only beside the main player of a watch, Shorts, embed or
live page; carousel banners and hover previews are muted page decoration. On
Bilibili it is shown only beside the main player of a `/video/` or `/bangumi/play/`
page, on Twitch only beside the player of a `/videos/…` page, and on TikTok
only beside the video a `/@user/video/…` page is about. For TikTok the desktop loads
that public page itself and keeps TikTok's anonymous visitor cookies in memory only,
sending them just to TikTok's video servers; no browser cookies are read.
An actual click sends a review to the desktop after sender, top-frame, origin
and permission checks. When available, `MessageSender.documentId` must match the
active top frame's `scripting.executeScript` result before native handoff. This
accepts same-document navigation (such as YouTube's `history.pushState`) without
accepting a replacement document. Older browsers without document IDs retain the
strict URL check. See the [Chrome scripting API](https://developer.chrome.com/docs/extensions/reference/api/scripting)
and [Firefox document identity guide](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Work_with_documentId).
Content scripts cannot send native requests or queue
downloads. Explicit off choices are saved in local extension storage (up to 1,000).
Disabling removes the current UI and excludes that site from future registration.
Permission revocation removes access and the UI, while preserving the choice
across re-grants, reloads and restarts. Cookie permissions are independent; this feature never
reads cookies or authentication headers. It does not run in private windows.

Connection checks show progress and success/failure beside the popup's button.
Connection and video-review outcomes join setting changes in the bounded local
diagnostic log, with addresses and Windows paths redacted from messages.

For the ordinary page-file picker, only selected URLs/filenames plus a queue ID,
saved-folder ID and batch ID go to the native host. The picker reads queue names and saved folder paths from the
local app. Jobs retain their URLs in the local database; replay receipts retain
job IDs and a SHA-256 intent digest without duplicating signed URLs or names.
An uncertain batch must be resolved before a browser restart, or checked in the
desktop queue before resubmission.

Video actions instead send the page URL/title and up to 1,000 candidate
URLs, filenames, labels, file/stream kinds and reported sizes to the local desktop
for review. No cookies, headers or media bodies accompany this transfer. The
native host validates and bounds the payload (850 KB), saves at most ten pending
reviews and does not create jobs or fetch media. The desktop's **Video downloads**
dialog requires file selection or a stream quality preview before queuing.
Closing it queues nothing. Pending payloads expire after 30 minutes and are
pruned on subsequent review activity; dismissed or handled reviews retain only
a SHA-256 digest and any queued job IDs for retry receipts, for up to one day.
URLs may contain signed tokens and must not be described as nonsensitive.
An uncertain browser handoff retains one request in session storage; **Recover
handoffs** resends the same `video:` ID and requires an exact `preview_id` receipt.
Native diagnostic logs record the action and outcome without these URLs or titles.
The `browser_video_review_v1` capability identifies compatible desktop versions.

The desktop contacts requested download servers. There is no analytics,
telemetry endpoint, advertising or account sync. A privacy statement should
explain this local transfer and storage without describing URLs as nonsensitive.
Permission behavior follows [Chrome activeTab](https://developer.chrome.com/docs/extensions/develop/concepts/activeTab)
and [Firefox scripting](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/scripting/executeScript);
[session storage](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/storage/session)
is available within the manifests' minimum supported browser versions.

### Firefox data declaration

The canonical native host is `com.xetro.downloadengine` and the Firefox add-on ID
is `download-engine@xetro`. Releases before 3.0 used `com.idmex.downloader` and
`idm-ex@local`. When upgrading an existing connection, the desktop registers
separate manifests for the current and old host identities so older extensions
can keep connecting. Fresh installations authorize only the new identity.
Migration only adopts registrations owned by that installed or portable copy;
another copy's registration is preserved. Disconnect and uninstall remove both
identities only where owned by that copy.

The Firefox ID change is a fresh extension installation, not an update of the old
ID. Finish pending extension handoffs, remove the old developer add-on, install
the new add-on, and review its settings and permissions. Firefox extension settings
do not transfer automatically. Desktop downloads, saved progress and queues remain.
Start the updated desktop once to migrate its browser connections, or use
Tools → Browser setup → Firefox → Connect browser, then Check connection in the
extension. Do not run both Firefox identities together: both can capture downloads.

The package sets `gecko.strict_min_version` to `142.0`. Built-in consent arrived
in desktop Firefox 140 and Firefox for Android 142, but AMO checks the inherited
minimum against both platforms. Using 142 avoids the Android compatibility warning;
Firefox 140/141, including ESR 140, no longer meet the package minimum. Do not add
`gecko_android` just to suppress that warning: that key advertises Android support,
while this extension requires the Windows desktop app and its native host.

The Firefox manifest declares `browsingActivity` and `websiteContent` as required:
URLs, page titles, selected links and media metadata are transferred to the local
desktop app for downloading or review. Native messaging counts as transmission
under [Mozilla policy](https://extensionworkshop.com/documentation/publish/add-on-policies/#data-collection-and-transmission-disclosure-and-control), even without a vendor server.
`authenticationInfo` is optional and requested only when approving browser login.
Do not replace these declarations with `none`. No analytics or technical telemetry
is collected. See [Firefox built-in consent](https://extensionworkshop.com/documentation/develop/firefox-builtin-data-consent/).

The listing should explain the separate Windows desktop installation and native
host connection, default capture/detection, manual video selection, and optional
login transfer. New permissions may require users to approve an extension update.
The upload ZIP has manifest.json at its root. Localization assets are generated:
if AMO requests original source, provide the matching Source ZIP with
`locales/`, `extension/`, and `scripts/Build-Locales.cjs`; run
`node scripts/Build-Locales.cjs` to reproduce the catalogs, then copy the Firefox
manifest as `manifest.json` alongside the extension files.

## Browser requirements

- [Chrome distribution rules](https://developer.chrome.com/docs/extensions/how-to/distribute/install-extensions):
  ordinary Windows distribution requires a Web Store release; a local CRX is not
  a substitute for store publication.
- [Firefox signing and distribution](https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/):
  release Firefox requires Mozilla signing for permanent installation, including
  self-distributed add-ons.
- [Microsoft Edge publication](https://learn.microsoft.com/en-us/microsoft-edge/extensions/publish/publish-extension):
  submit through Partner Center and complete its listing and policy review.

No store listing was submitted, no account was created, and no signing or
enterprise browser policy was changed by the local installer work.

## Optional browser login

The separate browser-login review asks for optional **cookies** permission and,
on Firefox, optional `authenticationInfo` data consent. Website access alone does
not grant login access. Both are checked before reading cookies and immediately
before handoff. Review requests expire after ten minutes; their URL/tab
metadata stays in storage.session, bounded to 30 requests and cleared when the
browser session ends. Cookie values never enter extension storage.

Only an explicit approval reads matching first-party, unpartitioned cookies from
the original regular tab's store, including Firefox containers and HttpOnly
cookies. It sends them once through native messaging using browser_login_v1.
Automatic capture and page batches do not acquire login access. The desktop
encrypts the snapshot with Windows user-scoped DPAPI and atomically commits it
with the job. Snapshots are limited to one origin/download, usable for at most
24 hours, and deleted on completion or removal. Browser logout does not revoke
a snapshot already transferred. The review can remove browser-login access for
all sites without disabling media detection;
desktop Authentication → None removes the saved snapshot.

The privacy declaration must disclose the optional local transfer of session
cookies and protected local persistence. No cookie data is sent to a vendor
service. Sites requiring private/partitioned sessions, cross-origin login
redirects, or POST downloads are unsupported. URL metadata can still contain
sensitive signed query strings.

API references: [Chrome cookies](https://developer.chrome.com/docs/extensions/reference/api/cookies),
[Firefox cookies.getAll](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/cookies/getAll),
and [Windows DPAPI](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata).

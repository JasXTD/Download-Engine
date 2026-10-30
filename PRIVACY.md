# Privacy policy

Last updated: 2026-10-28

This policy covers the official Download Engine project website, browser extensions
and their integration with the Download Engine desktop application for Windows. The project
is maintained at [JasXTD/Download-Engine](https://github.com/JasXTD/Download-Engine).

Download Engine uses the information below to find, review, transfer and manage
downloads. The project does not operate an analytics, advertising, telemetry or
account-sync service that receives your download activity. Data does leave your
device when the desktop app contacts the websites and download servers needed to
fulfil your requests, or when you use integrations you configure.

## Project website

The [project website](https://jasxtd.github.io/Download-Engine/) is a static site
hosted on GitHub Pages. Its code does not set cookies, use browser storage, include
analytics or advertising scripts, or collect form submissions. Fonts and images
are served with the site. Download and documentation links take you to GitHub.

GitHub processes normal hosting requests and logs visitor IP addresses for security
purposes. See [GitHub Pages data collection](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages#data-collection)
and [GitHub's privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement)
for the hosting provider's practices.

## Information used and why

| Feature | Information handled | Purpose |
| --- | --- | --- |
| File capture and link recovery | Download and source-page URLs, filenames, browser download identifiers, file types, reported sizes and handoff status | Send eligible downloads to the desktop app, recover interrupted handoffs and avoid duplicate downloads |
| Page-link and video selection | Page URLs and titles, link text, filenames, image/audio/video resource addresses, media types and reported sizes | Show downloadable items, let you choose files and prepare video reviews |
| Media detection | Media response URLs, content types, reported sizes and the associated tab | List detected media so you can decide what to download |
| Preferences and diagnostics | Language, capture filters, site choices, feature settings, timestamps, operation outcomes and diagnostic messages | Remember your choices and help diagnose connection or handoff failures |
| Desktop downloads | URLs, filenames, save paths, queue details, progress and completion/error state | Store your downloads and resume or manage them later |
| Optional browser login | Matching website authentication cookies, after permission and explicit approval | Allow the desktop app to make the approved authenticated file download |

Download URLs, filenames and page titles can contain personal information or
private access tokens. They should be treated as sensitive. The extension does
not read the browser's saved-password database.

File capture and media detection are enabled by default after installation
permissions are granted. You can turn them off independently in the extension.
Capture rules can exclude file types, sizes and sites. On-page video buttons also
have a separate site setting. Detection alone does not start a video download;
you choose an item for review first. These discovery, capture and login features
exclude private/incognito windows.

Media detection reads response metadata, not request headers, cookies or response
bodies. User-requested page scans inspect links and media resources already
present on that page. The on-page video control observes video elements and their
layout. The extension does not record general keystrokes or build advertising
profiles from browsing activity.

## Transfers to the desktop app and websites

The extension uses the browser's native messaging interface to send download
requests to the companion application on the same computer. It also reads queue
names and saved-folder information from that application to display download
destinations. A video review can include the page title and candidate media links;
an ordinary file batch sends the links you selected.

The desktop app contacts the source websites, download servers and media services
needed for the downloads or previews you request. Those services receive the
normal connection and request information, such as your IP address, requested
URL, request headers and any authentication you explicitly configure. A proxy
you configure may also process the connection. Those services have their own
privacy policies. Plain HTTP and FTP do not encrypt network traffic; HTTPS and
FTPS should be used when available.

Built-in video-site readers may use temporary visitor cookies obtained directly
from the requested site while resolving its media. This does not copy your
browser's login cookies.

No automatic copy of your download list, detected media or login cookies is sent
to a project-operated collection server. The project does not sell this data or
use it for advertising, credit scoring or unrelated profiling. Maintainers do not
have remote access to your local profile through the extension.

## Optional update checks

Download Engine contacts the public GitHub API once when the program opens by default.
Disable Check for updates on launch in Preferences → Startup & tray to opt out.
Help → Check for updates also lets you request a check manually.
The request includes a Download Engine version user-agent and normal connection
information such as your IP address. It uses your configured proxy and does not
send your download history, filenames, browser cookies or GitHub account token.
Opening the release page or the GitHub link in About opens GitHub in your browser.
There is no automatic update download or installation.

## Optional browser login

Browser-login access is separate from normal file capture and media detection.
It requires the optional cookies permission and your approval in a dedicated
review. Only matching first-party, unpartitioned cookies for the approved site
are read, including HTTP-only cookies where applicable. The extension does not
save cookie values in extension storage.

The approved cookies are sent to the local desktop app. It stores the snapshot
encrypted with Windows protection bound to your user account (DPAPI), restricts
it to the relevant origin/download, and uses it for at most 24 hours. The saved
snapshot is removed when the download completes, when its job is removed, or
when you set that download's authentication to None. An expired snapshot is no
longer usable; expiration alone does not imply immediate physical deletion.

Revoking the extension's cookies permission or signing out of the website does
not delete a snapshot already transferred to the desktop app. To remove that
copy, use the desktop download's authentication controls or remove its job.

## Storage and retention

- Extension preferences and pending file-handoff recovery records use local
  browser extension storage. Recovery records are removed as handoffs are
  resolved and the corresponding browser download reaches a terminal state.
  Unresolved records can persist across browser restarts.
- Detected media and page-selection state use browser session storage. The media
  list is bounded; entries older than 30 minutes are pruned during subsequent
  detector activity. Navigation, tab closure, Clear, disabling detection or
  permission revocation clear the applicable detected-media records. New
  playback can repopulate a cleared list while detection remains enabled.
- Unsent or completed page pickers are cleared when closed. Pending selections
  can remain for recovery during that browser session. Browser-login review
  requests expire after ten minutes; their URL/tab metadata is temporary session
  data, separate from cookie values.
- Desktop video-review payloads expire after 30 minutes and are pruned during
  subsequent review activity. Processed reviews retain a digest and any queued
  job identifiers for retry handling with a one-day expiry. Expired records may
  remain on disk until subsequent activity runs cleanup.
- Local diagnostics keep bounded recent operation records. Extension diagnostic
  messages redact addresses and Windows paths, but you should still review any
  logs or screenshots before sharing them.
- Desktop jobs, settings, saved history and downloaded files remain in your
  configured local profile or download folders until removed. Ordinary metadata
  such as URLs, titles and paths is not generally encrypted by the application.
  The protection described for browser-login snapshots does not apply to every
  local file. Copies in your backups are under your control.

## Your controls

Use the extension's capture, media-detection and per-site video settings to control
discovery. Use Clear in the media list and Clear log in Diagnostic log to remove
those records. Browser settings let you revoke site/cookie permissions or remove
the extension. Turning a feature off does not necessarily revoke a permission
shared with another feature.

Use the desktop app to remove download records and authentication snapshots.
Removing a record or using Clear finished does not delete the downloaded file;
delete files separately if you want them removed. Removing the browser extension
does not remove the desktop profile or downloaded files. For a complete local
reset, first stop the desktop app and any local MCP server, then remove the
appropriate profile data after preserving anything you want to keep.

The project has no central copy of this local download data to retrieve or erase
on your behalf.

## Integrations and information you choose to share

If you enable desktop scripting, install an extension, connect an MCP/AI client,
or configure an antivirus scanner or proxy, the selected tools can process
information or files within the access you give them. In particular, an AI client
may send information obtained from the local MCP server to its own provider.
Custom scripts can have their own network behaviour, and an extension can contact
the hosts it declares. Review those tools and their privacy settings; this policy
does not control third-party processing or user-modified builds.

If you choose to submit a GitHub issue, attachment or support report, the project
receives what you share. Public issues are visible to others and GitHub processes
them under its own privacy policy. Do not include passwords, cookies, private
URLs, access tokens or your profile database in a public report.

## Chrome Web Store Limited Use

Download Engine's handling of information obtained through Chrome APIs follows
the [Chrome Web Store User Data Policy](https://developer.chrome.com/docs/webstore/program-policies/user-data-faq)
and its [Limited Use requirements](https://developer.chrome.com/docs/webstore/program-policies/limited-use).
That information is used for the disclosed download features and related
operation and security. It is not sold or used for advertising or lending
decisions. Transfers are limited to delivering these features and the exceptions
permitted by that policy. The project does not give people access to your local
data; maintainers can review information you explicitly choose to share for
support, subject to the policy's permitted uses.

## Changes and contact

Changes to this policy will be published on this page with an updated date.
Relevant changes to application behaviour are documented in the
[changelog](https://github.com/JasXTD/Download-Engine/blob/main/CHANGELOG.md).

For privacy questions, use the project's
[GitHub issue tracker](https://github.com/JasXTD/Download-Engine/issues) and include
only non-sensitive information. For a suspected security vulnerability, follow
the [private reporting instructions](https://github.com/JasXTD/Download-Engine/blob/main/SECURITY.md#report-a-vulnerability).

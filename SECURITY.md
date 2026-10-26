# Security policy

This policy explains how to report vulnerabilities in **Download Engine**, what
the application trusts, and where its protections end. It covers the Windows
desktop app, download engine, CLI, local MCP server, browser extensions, native
messaging host, installer, packaging, and bundled integrations.

**Report suspected vulnerabilities privately.** Do not publish exploit details,
credentials, private URLs, or profile data in an issue or pull request.

- [Supported versions](#supported-versions)
- [Report a vulnerability](#report-a-vulnerability)
- [Handling and disclosure](#handling-and-disclosure)
- [Security model](#security-model)
- [Browser permissions and native messaging](#browser-permissions-and-native-messaging)
- [Automation and custom site readers](#automation-and-custom-site-readers)
- [Local MCP access](#local-mcp-access)
- [Local data and credentials](#local-data-and-credentials)
- [Packages and dependencies](#packages-and-dependencies)
- [Testing and report scope](#testing-and-report-scope)

## Supported versions

| Version | Security maintenance |
| --- | --- |
| Latest released version | Target for security fixes. |
| Older releases | No separate security-update branch; upgrade to the latest release. |
| Development and locally modified builds | Useful for reproductions, but not a separately maintained release line. |

Check **Help → About** or `de-cli.exe --version` for the app version, and the
browser's extension details for its version. Keep the app and extension aligned;
reload developer-loaded extensions after updating their files. A changed extension
identity, such as Firefox's 3.0 migration, requires a fresh installation rather
than just a reload. See the [changelog](CHANGELOG.md) and
[upgrade guidance](README.md#upgrade-an-existing-copy).

## Report a vulnerability

On the project's GitHub repository, open **Security and quality** (or **Security**)
and choose **Report a vulnerability**, when available. This requires the repository
owner to enable private vulnerability reporting; this policy does not enable it.
See [GitHub's private reporting guide](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/report-privately).

If that option is unavailable, open an issue asking **only for a private security
contact**, without describing the vulnerability or attaching files. Wait for a
private channel before sharing details. Ordinary bugs and feature requests can
use public issues.

### What to include

Include enough information to understand the problem and reproduce it safely.
A complete exploit is not required:

- **Environment:** app and extension versions, Windows/browser versions, installed
  or portable mode, and the affected component.
- **Preconditions:** enabled features, permissions, proxy mode, script access, or
  user interaction needed to trigger the issue.
- **Reproduction:** minimal steps and a harmless fixture or synthetic input, with
  expected and actual results.
- **Impact:** what an attacker controls, what they can gain, and which security
  boundary is crossed.
- **Evidence:** redacted errors, logs, screenshots, or a small proof of concept.
  State whether the latest release is affected if you can check safely.

For a malicious media file, archive, script, or manifest, first describe the input
and its effect in the private report. Arrange safe sharing with the maintainer
instead of posting it publicly or sending someone an unlabelled executable.

### What to remove

Remove passwords, cookies, authentication headers, MCP access tokens, signed
download URLs, account identifiers, and personal file paths. Query strings and
referrers can contain secrets even when no password is visible.

Do not upload your real queue database, portable profile, browser profile, exported
download list, or private downloaded files. Use a separate test profile and
synthetic data. Automatic log redaction is useful but is not a substitute for
reviewing everything you share, especially script output and screenshots.

## Handling and disclosure

The intended process is to review the affected version and impact, reproduce the
issue, agree on a mitigation or fix, and validate it before coordinating public
disclosure. Maintainers may ask for a smaller fixture or more environment details.
Security fixes target the latest release, with relevant user-facing changes in
the changelog and an advisory when appropriate.

Response and fix times depend on availability and severity; there is no guaranteed
response time. Please coordinate disclosure through the private report so affected
users can receive a fix or mitigation first. If you plan to publish findings, tell
the maintainer your proposed timeline in that channel.

## Security model

Download Engine runs under your Windows account. It treats remote pages, server
responses, filenames, manifests, archives, and browser handoffs as untrusted input.
Its validation is intended to prevent that input from gaining unintended access
to local files, credentials, or app control.

These are application controls, not operating-system isolation. A program already
running as your Windows user can have access to the same files and Windows
credential facilities. The app does not protect a compromised Windows account.

### Downloaded files and offline websites

- HTTPS uses certificate validation. Plain HTTP and FTP do not provide encrypted
  transport; prefer HTTPS or FTPS when the source supports it.
- New Windows downloads and recovered publications receive **Mark of the Web**
  before completion. Older completed files are not retroactively marked. The
  marker supports Windows handling of internet files; it is not a safety verdict.
- File handling rejects unsafe filenames and linked destinations in protected
  write paths. Temporary-file and publication checks aim to prevent unintended
  overwrites and writes through links.
- A checksum verifies bytes against an expected digest. It is useful only when
  that expected value comes from a source you trust.
- Antivirus scanning is optional and requires a configured, working scanner.
  A clean result is not a guarantee; some integrations require manual result
  review in the scanner's own app.
- Treat downloaded files and offline websites as untrusted even after a successful
  transfer. Opening or running them gives their content to another application.

Read the [security reference](docs/reference.md#security) and
[antivirus guide](docs/user-guide.md#antivirus-scanning) for implementation details
and scanner behavior.

## Browser permissions and native messaging

The extension passes supported download requests to the local native messaging
host, which hands them to the desktop app. Registered hosts restrict extension
identities, and native messages have a fixed action schema and a 1 MiB size limit.
Extension pages use packaged scripts/styles with a restrictive content security
policy; the extension does not execute downloaded files.

The permissions correspond to these features:

| Capability | What it is used for |
| --- | --- |
| Native messaging | Connect the extension to the installed or portable desktop copy. |
| Download access | Capture eligible file downloads and manage the browser side of a handoff. |
| HTTP/HTTPS site access and `webRequest` | Discover supported media response URLs and metadata across pages and media CDNs. |
| Active tab and scripting | Inspect the selected page for supported links/media and provide on-page controls. |
| Local/session storage and alarms | Retain settings and bounded pending handoff or discovery state. |
| Optional cookies | Copy matching cookies only through the browser-login workflow after permission and review. |

File capture and passive media detection default to on after installation
permissions are accepted. Saved off choices are preserved within the same extension
identity; Firefox's new 3.0 identity starts with fresh settings. Passive detection
reads response URLs, MIME types, and reported sizes, not request headers, cookies,
or response bodies. It does not automatically preview or download a detected video.
Its bounded discovery list is held in browser session storage and can include
signed URLs. Turning a feature off does not necessarily revoke permissions shared
with another feature; review permissions in the browser's extension settings.

Browser-login handoffs are separate. For supported direct downloads, they copy
matching cookies from the reviewed site into an encrypted per-download snapshot.
They do not read browser passwords or profile databases. Ordinary capture, page
link batches, and media scanning remain anonymous. Private browsing is unsupported
for these capture/login workflows, and built-in video readers do not inherit your
browser login. See [browser-login details](docs/reference.md#downloads-using-your-browser-login)
and [browser integration](docs/browser-release.md).

## Automation and custom site readers

Only enable scripts and automatic triggers you have reviewed and trust.

| Mechanism | Access and limits |
| --- | --- |
| Routing and completion rules | Supported app actions such as assigning queues or starting another queue; no shell execution. |
| Lua automation | Restricted app API for reading state and permitted download/queue actions. It has application-level limits, not an OS sandbox. |
| PowerShell scripts | Normal file, network, and program access under your Windows account. App API restrictions do not confine the PowerShell process. |
| Custom scanner commands | External programs with your Windows account's permissions; trust both the executable and its arguments. |
| Lua site readers | A separate API for fetching public pages and returning supported files/streams; they cannot inspect or change existing downloads through the automation API. |

Custom readers' network requests and returned downloads are restricted to public
destinations, including redirects and stream segments. The reader path validates
and pins resolved addresses and requires **Direct connection**; system and custom
proxies are refused rather than used as an unchecked fallback. Browser cookies
and saved logins are not supplied to readers. On cross-origin redirects,
script-supplied headers are removed and forwarding a POST body is refused.

These public-only rules apply specifically to custom-reader traffic; ordinary
user-added downloads retain their documented network/proxy behavior. Reader jobs
created before 1.21.2 did not retain this distinction and must be re-added to gain
the persisted restriction. See [site-reader limits](docs/automation.md#site-readers)
and [script permissions](docs/automation.md#custom-scripts-lua-and-powershell).

## Local MCP access

MCP is an optional authenticated **Streamable HTTP** server on loopback, normally
`http://127.0.0.1:17842/mcp`. The listener checks the local Host/Origin and requires
the profile's bearer token. It is intended for trusted clients on the same computer,
not public hosting or remote access through a tunnel or port forward.

- The desktop server starts **disabled**; new desktop settings use **View only**.
- **Allow changes** lets a client perform exposed mutations, including running
  saved PowerShell scripts. Treat this as authority to run those scripts under
  your account, not just permission to view downloads.
- The terminal command `de-cli.exe mcp` allows changes by default. Use
  `de-cli.exe mcp --read-only` for a read-only terminal server; it hides mutation
  tools and rejects attempts to call them.
- View only still exposes information available through read tools. Only connect
  clients you trust with that information, and consider where an AI client sends
  the responses it receives.
- Copied client configuration and access headers contain the token. Do not post
  them in chat, logs, issues, screenshots, or shared configuration files.
- Closing the MCP popup or disconnecting an AI client does not stop the server.
  Use **Stop server** for the desktop server or stop the separate CLI server.
  Work already accepted by the app can continue after the server stops.

If a token is exposed, stop all MCP servers for that profile and follow the
[token rotation instructions](docs/mcp.md#options-and-profiles). Rotate only the
token; do not delete your database or downloads. See the [MCP guide](docs/mcp.md)
for profiles, access modes, and connection troubleshooting.

## Local data and credentials

Installed profiles normally use `%LOCALAPPDATA%\Download Engine`. Older profiles
may keep `%LOCALAPPDATA%\IDM_EX`; portable copies use their configured `data`
directory. `DOWNLOAD_ENGINE_HOME` can select a separate profile, with
`IDM_EX_HOME` retained for compatibility.

| Data | Protection and sharing considerations |
| --- | --- |
| Queue database and command records | Not encrypted; may contain URLs, signed query parameters, filenames, local paths, and download history. |
| Supported saved site/proxy credentials | Stored using Windows Credential Manager; other programs under the same Windows user remain outside the isolation boundary. |
| Browser-login snapshots | Encrypted using Windows user-scoped DPAPI before database storage and scoped to the associated download/origin. |
| MCP access token | Encrypted using user-scoped DPAPI in the profile's `mcp-token.dpapi`; copied client configuration contains the usable token. |
| Saved scripts and script output | Stored locally; source and output can contain anything the script author includes. Review before sharing. |
| Diagnostics and exported lists | Can expose operational details or private URLs. Redact them manually; an export is not a safe anonymized profile. |

Keep the application, portable folder, and profile writable only by accounts you
trust. Windows credential protection does not encrypt the whole profile, prevent
access by another program running as you, or guarantee portable secrets will work
under another Windows account. Secure backups of profiles and downloads separately.

## Packages and dependencies

Current Windows release packages use explicit **Unsigned** / **UnsignedSetup**
filenames and have no verified Authenticode publisher signature. Release checksums
help detect changed bytes when compared with a trusted copy of the checksum list;
they are not a substitute for code signing or an independent security review.

Use the project's published release assets rather than repackaged binaries. Keep
the app, extension, and bundled media helper from the matching release together.
The supplied browser ZIPs are not proof of store approval; permanent Firefox
installation requires Mozilla signing.

Bundled FFmpeg processes untrusted media through a controlled local-file workflow.
It remains third-party native code that needs security maintenance. The original
app code and bundled dependencies have separate licenses and source obligations;
see [third-party notices](THIRD-PARTY-NOTICES.md).

Maintainers should follow the [release workflow](docs/releases.md), verify the
source inventory and package contents, run relevant regressions, and check
dependencies against current advisories. A clean dependency audit means no matching
published advisory was found at that time; it does not establish that the app has
no vulnerabilities. Publishing this policy does not certify an audit or enable
GitHub's private reporting setting.

## Testing and report scope

Test only on systems, accounts, and data you own or are authorized to assess.
Prefer isolated profiles, loopback fixtures, and synthetic files. Do not access
other users' data, use stolen credentials, disrupt services, or test third-party
video platforms on the project's behalf.

Useful security reports include unexpected file writes or path escapes, credential
leaks, unsafe cross-origin forwarding, unintended private-network access from a
restricted reader, unauthorized native/MCP actions, and exploitable media or
archive parsing failures. Include the attacker-controlled input and any permissions
or user actions required.

A trusted PowerShell script intentionally using its documented account access,
or a website refusing an unsupported video, is not by itself evidence of a boundary
bypass. Unexpected access outside documented permissions is still worth reporting.
Ordinary compatibility failures, unavailable qualities, and feature requests can
use public issues with private details removed.

These boundaries do not exclude reports of bypasses or unexpected access caused
by untrusted input. If you are unsure whether a finding is security-sensitive,
start with a private report.

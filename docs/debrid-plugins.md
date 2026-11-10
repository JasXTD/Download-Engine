# All Debrid, Real Debrid and Premiumize.me

These are three separate optional **Lua application plugins**, bundled in Download
Engine 3.20.0. They generate direct file links using your own service account.
They are not Chrome/Firefox extensions. No subscription or account is included.

## Install and connect

1. Open **Extensions → Installed → From plugins folder** and choose **Refresh**.
2. Review and install **All Debrid**, **Real Debrid**, or **Premiumize.me**. Check the
   declared hosts and protected account setting. These three plugins are enabled
   by default after installation. Existing disabled choices are preserved.
3. Open the plugin's **Overview**. No account action starts automatically.
4. For **All Debrid** or **Real Debrid**, click **Connect account**. The action log
   shows an approval code. Choose **Open service website**, sign in there, and enter
   that code. Keep the action running while you approve it. Codes are displayed as
   selectable text; passwords stay on the service website.
5. For **Premiumize.me**, choose **Open service website**, copy your account's API
   key, then open this plugin's **Settings**. Paste it into **Premiumize API key**,
   click **Save settings**, and return to Overview → **Check account**.

All Debrid and Real Debrid stop waiting after about three minutes. If authorization
expires, choose Connect account again for a new code. **Stop** cancels the worker;
a stopped or superseded action cannot replace your saved account credentials.

Premiumize's OAuth requires an application registration. This plugin uses its
documented API-key authentication and sends the key in an HTTPS Authorization
header, never in the URL. It does not borrow another application's OAuth identity.

## Generate and download

Paste one supported HTTP(S) file-host link into **Source URL**, then choose
**Generate links → Review generated links**. Select the files, queue and destination
in the review before adding downloads. Generating a link alone does not enqueue
anything. Results retain file names and sizes when the service supplies them.

Downloads use one connection per file to respect restrictive generated-link
connection limits. Pause/resume uses the normal engine and depends on the server
supporting ranges and the generated link remaining valid. If a signed link expires,
generate it again. Do not share generated links: they can grant account-backed
download access.

All Debrid handles delayed file preparation. Real Debrid handles multi-link replies
and refreshes expired access tokens once, saving rotated credentials securely.
Premiumize maps its direct-download file list into selectable files, flattens source
folders into safe file names, and distinguishes repeated names. Folder layout is not
recreated. Premiumize cloud transfers, magnet submission, account purchases, and
AllDebrid stream-quality selection are outside these file-link plugins.

## Account limits and privacy

Supported hosts, premium requirements, traffic allowances, IP/proxy restrictions
and server availability belong to the service. Generating links may consume its
allowance or create a record in its account history even if you do not download.
Use content you are allowed to access and follow the service's terms.

Credentials are protected by Windows DPAPI or the OS-backed encrypted credential
store on macOS/Linux. Each plugin has its own setting. API credentials are never
attached to the generated file downloads. API sessions are bound to one HTTPS
origin and refuse cross-origin redirects. Results are limited to the declared
service/CDN domains, and the download engine rejects private network destinations.
Unexpected hosts fail explicitly and require a reviewed plugin update.
Installing an account-action plugin upgrades that profile's format. Older app
versions refuse to open it, preventing accidental loss of permissions or transfer
limits. Back up your profile before downgrading the application.

**Disconnect** removes the local saved credential. It does not revoke authorization
on the provider's website or cancel downloads already queued. For full revocation,
remove the authorized application or regenerate the API key on that website.
Existing action logs can contain short-lived approval codes; clear run history if
you do not want to retain those. Tokens, account email and raw service replies are
not logged by these plugins. A proxy configured in Preferences also applies to
plugin requests; the service may restrict which proxy/VPN addresses it accepts.

## For Lua plugin authors

The service implementations live in `assets/plugins/`, not in the Rust downloader.
`node scripts/pack_debrid_plugins.cjs` produces the three self-contained
`.de-extension.json` files. The shared helper is copied into each package, so none
requires another installed plugin. The `bundled.*` IDs are reserved for these exact
reviewed files; use your own ID when modifying the code.

A site-reader job can declare `"action": "none"` (button) or `"action": "url"`
(URL input and button). Such a job is explicit-only: it never intercepts page URLs.
The URL arrives in `event.url`. For an action with no input, that field is a
placeholder on the job's declared page host; it must not be treated as user input.

An action can declare `"secret_writes": ["account"]`, where `account` is an existing
`secret` setting. After permission review, the worker exposes
`xd.save_secret("account", value)`; pass `nil` to clear it. Values are limited to
4096 bytes and 16 writes per run. `xd.setting` is a snapshot of the settings at
launch, so keep a refreshed value in your Lua variable after saving it.
`xd.supports("xd.save_secret")` reports availability. Automation jobs, processors,
ordinary readers and undeclared settings cannot use this write capability.

The core checks the live run, cancellation, current plugin version, enabled state,
job revision and original secret before writing. Account actions from one plugin
are serialized. Changed action/write declarations appear in update review.
Return the normal reader shape, `{title="Connected", items={}}` for a status, or
file items for review. `single_connection=true` preserves a one-connection limit
through the review, queue and profile reload.

## Validation and service references

Automated tests execute the shipped Lua against synthetic API responses, exercise
protected storage in isolated profiles, and check the real CLI worker and review
handoff. No customer credentials or paid downloads are used by these tests.
Live paid-account/host compatibility still needs a user account check; offline
fixtures cannot prove current service availability.
The public All Debrid PIN and Real Debrid device endpoints were also checked
without signing in, including Real Debrid's pending-approval response.

- [AllDebrid API](https://docs.alldebrid.com/)
- [Real-Debrid API and open-source device authorization](https://api.real-debrid.com/)
- [Premiumize API](https://www.premiumize.me/api)

Report failures on [GitHub Issues](https://github.com/JasXTD/Download-Engine/issues)
with the app version, plugin name and error text. Remove approval codes, API keys,
tokens, account identifiers and private generated URLs before sharing any log.

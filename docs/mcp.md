# Connect an AI client to Download Engine

Download Engine provides a local **Streamable HTTP** MCP server, controlled from the
MCP popup or the existing `de-cli.exe`. No extra runtime is needed. It uses the app's profile, scheduler
and durable command inbox. The server runs independently of AI clients; reconnecting
a client does not restart it. No stdio transport is implemented.

## Set up in the app

1. Open **MCP** on the toolbar (also **Tools → MCP control** or **Preferences → Browser & AI → AI connection (MCP)**).
2. The status, **Start server** and server URL are shown at the top. Open **Advanced settings** to keep port **17842** or choose an unused port. Choose **View only** to inspect
   downloads, queues and scripts. Under **Allow changes**, select **Download controls**,
   **Queue controls**, **Approved Lua scripts**, and **Approved PowerShell scripts**
   independently. Script execution starts disabled, including on existing profiles.
   Each script also needs local approval for its exact saved revision (see below).
3. Click **Start server**. This saves your settings and starts the server in the app.
   Wait for **Running**. Click **Check connection** to verify authenticated HTTP
   access and see how many tools are available. This verifies the server; it does
   not mean a particular AI client is connected.
4. Click **Copy client config** and paste it into your AI client's MCP settings.
   For a form, choose **Streamable HTTP**, use the copy button beside **Server URL**, add a header named
   **Authorization**, and paste the value from **Copy access header**.
5. Click **Read get_usage_guide, then show my queues.** to copy the suggested first
   prompt, then paste it into your connected AI client.

The URL can be copied before starting the server. Configuration and access-header
copying, and the connection check, appear once it is running. At smaller window
sizes, scroll inside the popup to reach the connection tools. Setup guide and Open logs
stay below the scrolling area when opened from the toolbar or Tools menu.

The popup keeps your current page behind it. Close it with × or Escape to return
to that page, or to the same Preferences draft if opened there. Closing MCP does
not stop its server or discard pending MCP settings; reopen it to continue editing.
Save settings or Start server applies those settings.

MCP starts disabled. New desktop settings use **View only**. **Start with
Download Engine** is optional and takes effect after **Save settings** or **Start
server**. It starts MCP when this app/profile opens, including a launch from the
browser extension; it does not register a Windows service or start the app at login.
Stopping the server ends the current run; uncheck and save the startup option if
you also want it disabled on the next app launch. Port and access settings can be
changed after stopping the server. Restart/reconnect your AI client after changing
access so it refreshes its tool list.

The server keeps running while Download Engine is open or hidden in its tray. **Stop
server** disconnects AI clients; downloads and scripts already accepted by the
app continue. **Tasks → Exit** (or **Exit** in the tray menu) also stops the server. Use **Open logs** on
the MCP popup for startup, shutdown and failure events. Errors appear directly on
the popup; connection tokens are never displayed there or recorded in diagnostics.
Do not post the copied configuration or header in chat: they include your private token.

## Optional terminal server

Open PowerShell in the folder containing `de.exe` and `de-cli.exe`:

```powershell
.\de-cli.exe mcp
```

Leave this terminal running. It prints the address `http://127.0.0.1:17842/mcp`.
Press Ctrl+C in that terminal to stop the server. The desktop app and downloads
already accepted by it can continue. Starting MCP is opt-in; it does not register
a Windows service or start at login.

Open a second PowerShell terminal in the same folder:

```powershell
.\de-cli.exe mcp-config
```

This prints the connection settings for the current profile. Copy the
`download-engine` entry into your AI client's MCP configuration. For clients with a
form, choose **Streamable HTTP** / **HTTP**, enter the URL, and add the
`Authorization` header exactly as printed. Keep the token private: this output
allows control of this profile while its MCP server runs. Do not paste it into
chat, source control, logs, or a public configuration example.

Example shape with a placeholder token:

```json
{
  "mcpServers": {
    "download-engine": {
      "url": "http://127.0.0.1:17842/mcp",
      "headers": {"Authorization": "Bearer <token from mcp-config>"}
    }
  }
}
```

Client settings formats vary; some also require a `type` or `transport` field.
Use that client's HTTP option. Download Engine does not change AI-client settings.
The client must run on this computer and support custom HTTP headers. A cloud
service's localhost points at its own machine, not this PC. The endpoint is for
MCP clients, not a webpage to open in a browser.

## Options and profiles

```powershell
.\de-cli.exe mcp --read-only
.\de-cli.exe mcp --port 17843 --no-start
.\de-cli.exe mcp --no-queue-control
.\de-cli.exe mcp --allow-lua
.\de-cli.exe mcp-config --port 17843
```

The terminal server is independent of the desktop controls. CLI flags do not read
or overwrite the MCP popup's settings: `mcp` defaults to port 17842 with changes
allowed for downloads and queues but script execution disabled. `mcp-config`
defaults to port 17842. Use explicit `--port` values for a
custom port, or copy the running app's configuration from its MCP popup. Stop the
desktop-owned server before using the terminal server on the same port. If a
terminal server launches an app whose MCP startup option is enabled on that port,
the popup reports the conflict; it does not take over or stop the terminal server.

Run one server at a time per port. `--read-only` hides mutation tools and
rejects direct attempts to call them. Restart the server without that flag when
you want to allow changes; it is a server setting, not a client setting.
`--no-start` requires the desktop to be opened manually before commands can be
processed. Otherwise mutations launch it when needed; reading state does not.
New profiles can initialize their database even in read-only mode.

`--no-download-control` and `--no-queue-control` remove their respective mutation
tools. `--allow-lua` and `--allow-powershell` enable each script language; both are
off unless explicitly selected. These are server restrictions, enforced even for
direct calls to hidden tools. Cancelling a request also requires permission for
the underlying command. Inspection tools remain available.

Use the CLI beside the intended app. Portable copies follow `portable.json`;
installed copies use the installed data folder. Set an absolute `DOWNLOAD_ENGINE_HOME`
before both server and configuration commands to choose another profile:

```powershell
$env:DOWNLOAD_ENGINE_HOME = 'C:\DownloadEngine-MCP-Test'
.\de-cli.exe mcp --port 17843
```

Set the same override in the second terminal before running `mcp-config --port
17843`. Use separate ports for separate simultaneous profiles. No profile path or
executable command is needed in the AI client: it connects to the server's URL.

The profile's token is encrypted with Windows user-scoped DPAPI in
`data\mcp-token.dpapi` (relative to `DOWNLOAD_ENGINE_HOME` or the portable root; installed
profiles use their configured data directory). `mcp-config` returns the same token
on later calls. A token from another profile or Windows account will not work.
To rotate it, stop all MCP servers for that profile, delete only its
`mcp-token.dpapi`, run `mcp-config` to create a new token, update client headers,
then restart the server. Keep the database and downloads.

## Approve a script locally

Review the script's source, language, file access, and revision in the desktop or
with `scripts show SCRIPT_ID`. Then, in a local terminal beside this app, use the
actual ID and reviewed revision:

```powershell
.\de-cli.exe scripts show SCRIPT_ID
.\de-cli.exe mcp-approve-script SCRIPT_ID REVISION
.\de-cli.exe mcp-revoke-script SCRIPT_ID
```

Approve only scripts whose effects you intend to allow. The language's server
permission and this approval are both required. MCP has no approval tool, and
automation workers cannot use these local approval commands. Editing or resaving
a script invalidates its approval. Approval checks cover its source and settings,
and are repeated when the queued command snapshots the script for execution.
Revoking approval prevents new snapshots; already accepted script runs keep their
snapshot and can be stopped with **Stop script**.

The download and queue switches govern direct MCP tools. An approved Lua script
retains its existing automation API and file-change setting; an approved PowerShell
script can run programs and access files with your Windows account's permissions.
Those language permissions therefore grant the reviewed script's broader effects.

## Give the AI its guide

Ask your client:

> Read Download Engine's get_usage_guide, then show my queues and paused downloads.

The server advertises up to 21 tools with input schemas and read/write annotations
(19 with the default download/queue controls; script tools require a language permission).
Its initialization/discovery instructions tell clients to call `get_usage_guide`
first. That tool returns the bundled [AI usage guide](mcp-ai-guide.md), including
exact tool arguments, download and script workflows, receipts, cancellation and
retry rules. You can also provide the Markdown guide directly to another AI.

A safe first mutation is: “Create a paused queue named MCP practice and add this
direct file URL to it.” Supply a real file URL, review the client's proposed
actions, then ask it to start the queue and verify completion. A successful
command receipt is not a completed download. For scripts, first save one in
Scripting → Scripts, review and approve its saved revision locally, and enable its
language's server permission before asking the AI to run it.

## Connection failures

| Symptom | Check |
| --- | --- |
| Connection refused | Click Start server on the MCP popup, or start the terminal server. Check its address and port. |
| Port already in use | Stop the other server, or select another port and update your client. The app never takes over another listener or binds a public interface. |
| 401 Unauthorized | Generate settings from the same profile and copy the complete Authorization header. |
| 403 Forbidden | Use the exact loopback address. Remote origins, proxy hosts and cross-site browser requests are rejected. |
| 405 on browser visit | Expected: MCP uses POST, not a webpage. Choose Streamable HTTP in the client. |
| 400 header mismatch | A modern client must send the protocol/method/name headers matching its JSON body. See below. |
| 404 session expired | Reinitialize the older client, then check outstanding command receipts. |
| 429 busy | Four tool calls are active. Wait and retry with the same durable request ID. |
| Only inspection tools | View only is selected on the MCP popup, or the terminal uses `--read-only`. Stop, change access, restart, and reconnect your client. |
| Desktop did not start | Open this copy manually; check `--no-start`, then retry using the original request ID. |

Before updating the app, exit the desktop and stop any terminal MCP server.
Disconnecting the AI client alone leaves the server running and its executable in use.

## Protocol and implementation limits

The endpoint implements the official [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http).
The listener binds IPv4 `127.0.0.1` only. Requests require the bearer token, a Host
of `127.0.0.1:<port>` or `localhost:<port>`, and either no Origin or that same local
HTTP origin. It does not enable cross-origin browser access, OAuth, remote binding,
TLS termination, or an unauthenticated mode.

Modern `2026-07-28` requests are stateless: send protocol version and client
capabilities in `params._meta`, with matching `MCP-Protocol-Version`, `Mcp-Method`,
and (for tool calls) `Mcp-Name` headers. Discovery uses `server/discover`.
`Mcp-Name` supports the specified Base64 sentinel encoding. Unsupported versions
and missing/mismatched headers return the protocol's structured errors. Tool
results include `resultType: complete`, server information, text and structured
content. Closing a modern SSE response cancels unclaimed work.

Legacy `2025-11-25` and `2025-06-18` clients send `initialize` first, retain the
returned `Mcp-Session-Id`, send `notifications/initialized`, and include that
session ID and negotiated protocol version on subsequent requests. Sessions
expire after 30 idle minutes or server restart; DELETE ends a session and cancels
its waiting work. Legacy cancellation uses `notifications/cancelled`. A lost
connection alone does not cancel a legacy call. No replay/resumable event stream
is advertised; use durable `request_id` receipts to resolve uncertain outcomes.

All POST requests use `Content-Type: application/json` and Accept containing both
`application/json` and `text/event-stream`. Discovery/listing can return JSON;
tool execution returns a short SSE response ending with one JSON-RPC result.
Notifications receive 202 without a body. GET returns 405. There is no background
GET event stream, sampling, elicitation, tasks, or script-upload capability.

Limits: 256 KiB request body, 16 KiB HTTP header buffer, 32 headers, 32 concurrent
connections, 32 legacy sessions, four active tools. Headers have a 5-second read
deadline and bodies an 8-second deadline; each connection has a 60-second total
deadline, including response delivery. A tool waits at most 30 seconds for its
command, plus bounded desktop/database work. Replies use bounded tool content;
list pages have at most 100 items. Persistent script logs and command receipts
use the app's existing retention. One request per HTTP connection bounds idle
socket use; clients open fresh connections without restarting the server.

## Maintainer verification

```powershell
cargo test --offline --locked --lib mcp::
cargo build --offline --locked --bins
node scripts/mcp_smoke.cjs target/debug
node scripts/mcp_desktop_smoke.cjs target/debug
```

The smoke test creates a separate `DOWNLOAD_ENGINE_HOME`, starts owned loopback servers,
checks modern/legacy HTTP, authentication, host/origin/header validation, reconnects,
pending cancellation, read-only mode, receipts and resource bounds. It also starts
an isolated desktop, pauses/resumes a local fixture download, checks its SHA-256,
and runs/stops saved Lua scripts with retained logs. It does not touch the normal
profile, configure an AI client, or test a live external download site.

The desktop smoke test launches the actual app with isolated saved MCP settings,
checks opt-in startup, read-only enforcement, queue/download control, clean exit,
restart, persistent credentials and a conflicting listener. Native UI tests also
exercise clicks, clipboard output, settings persistence, error recovery and layout.

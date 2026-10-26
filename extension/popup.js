(async () => {
"use strict";
await globalThis.IDMI18n?.ready;
globalThis.IDMI18n?.translateDocument();
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
const api = globalThis.browser || globalThis.chrome;
const $ = id => document.getElementById(id);
const browserLogin = $("browser-login"), status = $("status"), autoCapture = $("auto-capture"), videoButton = $("video-button");
let videoSite = null, videoRefresh = 0, pending = null, busy = 0, mediaTab = null, mediaReady = false, mediaRefresh = 0, popupTarget = null, captureDefaults = null;
const mediaRows = new Map();
const permissionAccess = { capture: null, media: null, video: null };
const MEDIA_GRANT = { permissions: ["webRequest"], origins: ["https://*/*", "http://*/*"] };
async function refreshBrowserRecovery() {
  const reply=await api.runtime.sendMessage({action:"link_recovery_status"});
  $("link-recovery").hidden=!reply?.ok || !reply.recovery;
  if(reply?.recovery){$("link-recovery-hint").textContent=reply.allowed?__xdL.f("Waiting for {0}. Click its download link on the original page, then review it in Download Engine.", reply.recovery.name):__xdL.f("Allow download access to find a fresh link for {0}. Automatic capture can stay off.", reply.recovery.name);$("link-recovery-allow").hidden=reply.allowed;}
}
$("link-recovery-allow").addEventListener("click",async()=>{
  if(busy)return;
  await request({action:"request_setting_permission",setting:"recovery",returnTo:popupTarget});
  await refreshBrowserRecovery().catch(()=>{});
});
function show(ok, message) { status.textContent = message || ""; status.dataset.error = String(!ok); }
function controls() {
  for (const id of ["send","check","recover","page-videos","page-links","link-recovery-allow"]) $(id).disabled = busy > 0;
  for (const id of ["capture-rules-save","capture-rules-defaults"]) $(id).disabled = busy > 0 || !captureDefaults;
  autoCapture.disabled = busy > 0 || !popupTarget || permissionAccess.capture === null;
  videoButton.disabled = busy > 0 || !videoSite || !popupTarget;
  $("video-site-access").disabled = videoButton.disabled;
  $("media-detection").disabled = busy > 0 || !mediaReady || !popupTarget;
  $("clear-media").disabled = busy > 0 || mediaRows.size === 0;
  for (const row of mediaRows.values()) row.disabled = busy > 0;
}
async function request(message) {
  busy++; controls();
  try {
    // Disabling a feature does not revoke its optional permission. Re-enabling
    // uses the normal setting path; the background still checks current access.
    if (message.action === "request_setting_permission" && permissionAccess[message.setting] === true) {
      message = { action: { capture: "configure_capture", media: "configure_media", video: "configure_video_button" }[message.setting], enabled: true, pattern: message.pattern };
    }
    // Keep Firefox's permission prompt attached to this visible document while
    // its background owns completion and observes grants after the popup closes.
    const background = message.action === "request_setting_permission" && api.extension?.getBackgroundPage?.();
    if (background && typeof background.requestPopupPermission !== "function") throw Error(__xdL.t("Reload the Download Engine extension to finish updating it."));
    const reply = await (background ? background.requestPopupPermission(message, grant => api.permissions.request(grant)) : api.runtime.sendMessage(message));
    if (!reply) throw Error(__xdL.t("The extension background did not respond."));
    show(reply.ok, reply.message);
    return reply;
  } catch (error) {
    show(false, error.message);
    const setting = message.setting || { configure_capture: "capture", configure_media: "media", configure_video_button: "video", ping: "connection", scan_videos: "review", review_media: "review" }[message.action];
    if (setting) {
      const detail = String(error.message || error).replace(/(?:https?|moz-extension|chrome-extension):\/\/[^\s"'<>]+/gi, "[address]").replace(/[A-Z]:[\\/][^\r\n"'<>]+/gi, "[path]").replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 400);
      await api.storage.local.set({ lastPopupError: { time: Date.now(), setting, ...(["connection","review"].includes(setting)?{}:{enabled:message.enabled !== false}), ok: false, message: detail } }).catch(console.warn);
    }
    return null;
  }
  finally { busy--; controls(); }
}
async function refreshToggle() {
  const { autoCapture: enabled } = await api.storage.local.get("autoCapture");
  permissionAccess.capture = await api.permissions.contains({ permissions: ["downloads"] });
  autoCapture.checked = (enabled === undefined || enabled === true) && permissionAccess.capture;
  controls();
}
$("download-form").addEventListener("submit", async event => {
  event.preventDefault();
  if (busy) return;
  const url = $("url").value.trim();
  if (browserLogin.checked) {
    const reply = await request({ action: "browser_login_open", url });
    if (reply?.ok) browserLogin.checked = false;
    return;
  }
  if (!pending || pending.url !== url) pending = { action: "add", url, request_id: crypto.randomUUID() };
  const reply = await request(pending);
  if (reply?.ok) { pending = null; $("url").value = ""; }
});
autoCapture.addEventListener("change", async () => {
  const enabled = autoCapture.checked;
  busy++; controls();
  try {
    await request(enabled ? { action: "request_setting_permission", setting: "capture", returnTo: popupTarget } : { action: "configure_capture", enabled });
  } catch (error) { show(false, error.message); }
  finally { await refreshToggle().catch(error => show(false, error.message)); busy--; controls(); }
});
function fillCaptureRules(rules) {
  const megabytes = rules.minSizeKB % 1024 === 0;
  $("capture-types").value = rules.types.join(" ");
  $("capture-min-size").value = rules.minSizeKB > 0 ? String(megabytes ? rules.minSizeKB / 1024 : rules.minSizeKB) : "";
  $("capture-size-unit").value = megabytes ? "MB" : "KB";
  $("capture-sites").value = rules.sites.join("\n");
}
function showCaptureRules(ok, message) {
  const output = $("capture-rules-status");
  output.hidden = !message; output.textContent = message || ""; output.dataset.error = String(!ok);
}
async function refreshCaptureRules() {
  const reply = await api.runtime.sendMessage({ action: "capture_rules_get" });
  if (!reply?.ok || !reply.rules || !Array.isArray(reply.defaults)) throw Error(reply?.message || __xdL.t("Capture rules are unavailable. Reload the extension."));
  fillCaptureRules(reply.rules);
  captureDefaults = reply.defaults;
  controls();
}
$("capture-rules-form").addEventListener("submit", async event => {
  event.preventDefault();
  if (busy || !captureDefaults) return;
  const size = $("capture-min-size");
  // A number field reports unparsable text as an empty value.
  if (size.validity.badInput) { showCaptureRules(false, __xdL.t("Enter a minimum size between 0 and 1 TB.")); return; }
  const reply = await request({ action: "capture_rules_save", rules: { types: $("capture-types").value,
    minSize: size.value, unit: $("capture-size-unit").value, sites: $("capture-sites").value } });
  if (reply?.ok && reply.rules) fillCaptureRules(reply.rules);
  showCaptureRules(reply?.ok === true, reply?.message || status.textContent);
});
$("capture-rules-defaults").addEventListener("click", () => {
  if (busy || !captureDefaults) return;
  fillCaptureRules({ types: captureDefaults, minSizeKB: 0, sites: [] });
  showCaptureRules(true, __xdL.t("Defaults restored. Select Save rules to keep them."));
});
$("check").addEventListener("click", async () => {
  if(busy)return;
  const button=$("check"), feedback=$("connection-status");
  button.textContent=__xdL.t("Checking…");
  feedback.hidden=false; feedback.dataset.error="false"; feedback.textContent=__xdL.t("Connecting to Download Engine…");
  const reply=await request({action:"ping"});
  button.textContent=__xdL.t("Check connection");
  feedback.dataset.error=String(!reply?.ok);
  feedback.textContent=reply?.ok ? "✓ " + __xdL.t("Download Engine is connected.") : reply?.message || status.textContent;
  feedback.scrollIntoView({block:"nearest"});
});
$("recover").addEventListener("click", () => request({ action: "recover_capture" }));
$("page-links").addEventListener("click", () => request({ action: "scan_page" }));
$("page-videos").addEventListener("click", () => request({ action: "scan_videos" }));
async function refreshVideoButton() {
  const revision = ++videoRefresh;
  const reply = await api.runtime.sendMessage({ action: "video_button_status" });
  if (revision !== videoRefresh) return;
  videoSite = reply?.ok ? reply.pattern : null;
  permissionAccess.video = !!videoSite && reply.permitted === true;
  // No site means the default applies; it is not a saved choice to turn buttons off.
  videoButton.checked = !videoSite || reply.enabled !== false;
  $("video-site-access").hidden = !videoSite || !videoButton.checked || permissionAccess.video;
  $("video-button-hint").textContent = !videoSite ? __xdL.t("On by default. Open a website to change its setting.") :
    !videoButton.checked ? __xdL.t("Off for this site. Your choice is saved.") :
    permissionAccess.video ? __xdL.t("On by default. Turn off to hide video buttons on this site.") :
    __xdL.t("On by default. Allow this site to show video buttons.");
  controls();
}
async function setVideoButton(enabled) {
  if (busy || !videoSite || !popupTarget) return;
  const pattern = videoSite;
  busy++; controls();
  try {
    await request(enabled ? { action: "request_setting_permission", setting: "video", pattern, returnTo: popupTarget } : { action: "configure_video_button", enabled, pattern });
  } catch (error) { show(false, error.message); }
  finally { await refreshVideoButton().catch(() => { videoSite = null; }); busy--; controls(); }
}
videoButton.addEventListener("change", () => setVideoButton(videoButton.checked));
$("video-site-access").addEventListener("click", () => setVideoButton(true));
function sizeLabel(bytes) {
  if (!Number.isSafeInteger(bytes) || bytes <= 0) return __xdL.t("Size unknown");
  const units = ["B","KB","MB","GB","TB"]; let n = bytes, i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return (i ? n.toFixed(1) : n) + " " + units[i];
}
function renderMedia(items) {
  const ids = new Set(items.map(item => item.id));
  for (const [id, row] of mediaRows) if (!ids.has(id)) { row.remove(); mediaRows.delete(id); }
  items.forEach((item, index) => {
    let row = mediaRows.get(item.id);
    if (!row) {
      row = document.createElement("button"); row.type = "button"; row.className = "media-item";
      const format = document.createElement("span"), info = document.createElement("span"), name = document.createElement("span"), meta = document.createElement("span"), review = document.createElement("span");
      format.className = "media-format"; info.className = "media-info"; name.className = "media-name"; meta.className = "media-meta"; review.className = "media-review";
      review.textContent = __xdL.t("Review ↗"); info.append(name, meta); row.append(format, info, review);
      const tabId = mediaTab;
      row.addEventListener("click", () => request({ action: "review_media", tabId, id: item.id }));
      mediaRows.set(item.id, row);
    }
    const format = item.hls ? "HLS" : item.dash ? "DASH" : item.kind === "audio" ? "AUDIO" : "VIDEO";
    row.querySelector(".media-format").textContent = format;
    row.querySelector(".media-name").textContent = item.filename;
    let host = ""; try { host = new URL(item.url).hostname; } catch { /* Malformed state gets a text-only row. */ }
    // Isolate each part so an Arabic row keeps "46.0 MB" together instead of mixing it with the host.
    const isolated = text => { const part = document.createElement("bdi"); part.textContent = text; return part; };
    row.querySelector(".media-meta").replaceChildren(isolated(item.hls || item.dash ? __xdL.t("Choose quality in app") : sizeLabel(item.size)), " · ", isolated(host));
    row.title = item.filename + " — " + host;
    row.setAttribute("aria-label", __xdL.t("Review ") + item.filename);
    // Keep existing focused buttons attached when only their metadata changed.
    const at = $("media-list").children[index];
    if (at !== row) $("media-list").insertBefore(row, at || null);
  });
  $("media-count").textContent = String(items.length);
  $("media-empty").hidden = items.length > 0; controls();
}
async function refreshMedia() {
  const revision = ++mediaRefresh;
  const [reply, allowed] = await Promise.all([api.runtime.sendMessage({ action: "media_status" }), api.permissions.contains(MEDIA_GRANT)]);
  if (revision !== mediaRefresh) return;
  permissionAccess.media = allowed;
  mediaReady = reply?.ok === true;
  if (mediaTab !== reply?.tabId) { for (const row of mediaRows.values()) row.remove(); mediaRows.clear(); }
  mediaTab = mediaReady && Number.isInteger(reply.tabId) ? reply.tabId : null;
  $("media-detection").checked = reply?.enabled === true;
  $("media-hint").textContent = reply?.error || (reply?.enabled ? __xdL.t("Detection is on. Play media to update this list.") : __xdL.t("Enable detection with website access. Does not use your login."));
  $("media-empty").textContent = mediaTab !== null ? __xdL.t("Start playback with detection on, or use Find videos to scan this page.") : __xdL.t("Open a regular web page to find its media.");
  renderMedia(mediaTab !== null && Array.isArray(reply.items) ? reply.items : []);
}
$("media-detection").addEventListener("change", async () => {
  const enabled = $("media-detection").checked;
  busy++; controls();
  try {
    await request(enabled ? { action: "request_setting_permission", setting: "media", returnTo: popupTarget } : { action: "configure_media", enabled });
  } catch (error) { show(false, error.message); }
  finally { await Promise.all([refreshMedia(),refreshVideoButton()]).catch(error => show(false, error.message)); busy--; controls(); }
});
$("clear-media").addEventListener("click", async () => { await request({ action: "clear_media", tabId: mediaTab }); await refreshMedia().catch(error => show(false, error.message)); });
api.storage.onChanged?.addListener((changes, area) => {
  if ((area === "session" && Object.keys(changes).some(key => key.startsWith("media-tab:"))) ||
      (area === "local" && changes.mediaDetection)) void refreshMedia().catch(error => show(false, error.message));
  if (area === "local" && (changes.videoButtonDisabledSites || changes.videoButtonPermissions)) void refreshVideoButton().catch(error => show(false,error.message));
  if (area === "local" && (changes.settingDiagnostics || changes.lastPopupError) && $("diagnostics").open) void refreshDiagnostics();
});
api.permissions.onRemoved?.addListener(() => {
  permissionAccess.capture = permissionAccess.media = permissionAccess.video = false;
  void Promise.all([refreshToggle(), refreshMedia(), refreshVideoButton(), refreshBrowserRecovery()]).catch(error => show(false, error.message));
});
api.permissions.onAdded?.addListener(() => {
  void Promise.all([refreshToggle(), refreshMedia(), refreshVideoButton(), refreshBrowserRecovery()]).catch(error => show(false, error.message));
});
async function refreshDiagnostics(action = "diagnostics_get") {
  try {
    const reply = await api.runtime.sendMessage({ action });
    if (!reply?.ok || !Array.isArray(reply.entries)) throw Error(reply?.message || __xdL.t("Diagnostic log is unavailable. Reload the extension."));
    const labels = { capture: __xdL.t("Automatic capture"), media: __xdL.t("Media detection"), video: __xdL.t("Video button"), connection: __xdL.t("Connection check"), review: __xdL.t("Video review"), recovery: __xdL.t("Link recovery") };
    $("diagnostic-log").value = reply.entries.length ? reply.entries.map(entry =>
      new Date(entry.time).toISOString() + " | " + (labels[entry.setting] || __xdL.t("Setting")) + (typeof entry.enabled === "boolean" ? " → " + (entry.enabled ? "on" : "off") : "") + " | " + (entry.ok ? "OK" : "ERROR") + "\n" + entry.message).join("\n\n") : __xdL.t("No activity recorded yet.");
  } catch (error) { $("diagnostic-log").value = error.message; }
}
$("diagnostics").addEventListener("toggle", () => { if ($("diagnostics").open) void refreshDiagnostics(); });
$("refresh-log").addEventListener("click", () => { void refreshDiagnostics(); });
$("clear-log").addEventListener("click", () => { void refreshDiagnostics("diagnostics_clear"); });
$("select-log").addEventListener("click", () => { $("diagnostic-log").focus(); $("diagnostic-log").select(); });
api.storage.local.get("lastResult").then(({ lastResult }) => { if (lastResult && !busy) show(lastResult.ok, lastResult.message); }).catch(error => show(false, error.message));
async function initialize() {
  controls();
  const window = await api.windows.getCurrent();
  const [tab] = await api.tabs.query({ active: true, windowId: window.id });
  if (!Number.isInteger(window.id) || !Number.isInteger(tab?.id)) throw Error(__xdL.t("Open a browser tab and reopen Download Engine."));
  popupTarget = { windowId: window.id, tabId: tab.id };
  await api.runtime.sendMessage({ action: "permission_popup_opened", returnTo: popupTarget });
  await Promise.all([
    refreshToggle(), refreshMedia(), refreshBrowserRecovery().catch(()=>{}),
    refreshCaptureRules().catch(error => showCaptureRules(false, error.message)),
    refreshVideoButton().catch(() => { videoSite = null; controls(); })
  ]);
  const key = "permission-popup-return:" + window.id;
  const saved = (await api.storage.session.get(key))[key];
  if (saved) {
    await api.storage.session.remove(key);
    if (saved.tabId === tab.id) {
      const control = { capture: autoCapture, media: $("media-detection"), video: videoButton }[saved.setting];
      if (control === videoButton) $("browser-options").open = true;
      control?.focus();
    }
  }
  controls();
}
void initialize().catch(error => { controls(); show(false, error.message); });

const languagePicker = document.getElementById("display-language");
const languageSaved = await api.storage.local.get("displayLanguage");
languagePicker.value = globalThis.IDMI18n.choices.includes(languageSaved.displayLanguage) ? languageSaved.displayLanguage : "system";
languagePicker.addEventListener("change", async () => {
  try { await IDMI18n.setLanguage(languagePicker.value); location.reload(); }
  catch(error) { show(false,error.message); }
});

})().catch(error => { console.error(error); const output=document.getElementById("status"); if(output){output.textContent=String(error.message || error);output.dataset.error="true";} });

"use strict";
if (typeof importScripts === "function") importScripts("i18n-catalog.js", "i18n.js", "core.js", "capture.js", "link-recovery.js", "page-links.js", "browser-login.js", "video-button-manager.js", "media.js", "permission-flow.js");
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
const api = typeof browser !== "undefined" ? browser : chrome;
const actionApi = api.action || api.browserAction;
const ALARM = "idm-ex-capture-recovery";
let captureSettingsTail = Promise.resolve();
let downloadPermissionRevision = 0;
let diagnosticTail = Promise.resolve();
const DIAGNOSTICS_KEY = "settingDiagnostics";
function diagnosticEntry(entry) {
  const setting=["capture", "media", "video"].includes(entry.setting);
  if (setting ? typeof entry.enabled !== "boolean" : !["connection", "review", "recovery"].includes(entry.setting)) throw Error(__xdL.t("Invalid diagnostic setting."));
  return { time: Date.now(), setting: entry.setting, ...(setting ? {enabled:entry.enabled} : {}), ok: entry.ok === true,
    message: String(entry.message || __xdL.t("No response from the extension background.")).replace(/(?:https?|moz-extension|chrome-extension):\/\/[^\s"'<>]+/gi, "[address]").replace(/[A-Z]:[\\/][^\r\n"'<>]+/gi, "[path]").replace(/[\u0000-\u001f\u007f]/g, " ").slice(0, 400) };
}
function recordDiagnostic(entry) {
  const record = diagnosticEntry(entry);
  const task = diagnosticTail.then(async () => {
    const saved = (await api.storage.local.get(DIAGNOSTICS_KEY))[DIAGNOSTICS_KEY];
    const entries = Array.isArray(saved) ? saved.slice(-29) : [];
    await api.storage.local.set({ [DIAGNOSTICS_KEY]: [...entries, record] });
  });
  diagnosticTail = task.catch(console.warn);
  return diagnosticTail;
}
function trackSetting(setting, enabled, run, persistStatus = false) {
  // Invoke synchronously: permission requests must stay on the user's click stack.
  let task;
  try { task = run(); } catch (error) { task = Promise.reject(error); }
  if (!["capture", "media", "video"].includes(setting) || typeof enabled !== "boolean") return Promise.resolve(task);
  return Promise.resolve(task).then(async reply => {
    if (persistStatus) await saveStatus(reply.ok, reply.message).catch(console.warn);
    await recordDiagnostic({ setting, enabled, ok: reply?.ok, message: reply?.message });
    return reply;
  }, async error => {
    if (persistStatus) await saveStatus(false, error.message).catch(console.warn);
    await recordDiagnostic({ setting, enabled, ok: false, message: error.message });
    throw error;
  });
}
async function trackOperation(setting, run) {
  let reply;
  try { reply=await run(); }
  catch(error) { reply={ok:false,message:error.message}; }
  await saveStatus(reply.ok,reply.message).catch(console.warn);
  await recordDiagnostic({setting,ok:reply.ok,message:reply.message});
  return reply;
}
async function settingDiagnostics() {
  await diagnosticTail;
  const saved = await api.storage.local.get([DIAGNOSTICS_KEY, "lastPopupError"]);
  const entries = Array.isArray(saved[DIAGNOSTICS_KEY]) ? saved[DIAGNOSTICS_KEY].slice(-30) : [];
  if (saved.lastPopupError) entries.push({ ...diagnosticEntry(saved.lastPopupError), time: saved.lastPopupError.time });
  return { ok: true, entries: entries.sort((a, b) => a.time - b.time) };
}
function clearSettingDiagnostics() {
  const task = diagnosticTail.then(() => api.storage.local.remove([DIAGNOSTICS_KEY, "lastPopupError"]));
  diagnosticTail = task.catch(console.warn);
  return task.then(() => ({ ok: true, entries: [] }));
}

function serializeCaptureSettings(update) {
  const result = captureSettingsTail.then(update);
  // Return the failure to its caller without blocking later setting changes.
  captureSettingsTail = result.catch(() => {});
  return result;
}
async function saveStatus(ok, message) {
  await api.storage.local.set({ lastResult: { ok, message, time: Date.now() } });
  await actionApi.setBadgeText({ text: ok ? "" : "!" });
  if (!ok) await actionApi.setBadgeBackgroundColor({ color: "#ba4736" });
  await media.setAttention(!ok);
}
function native(request) {
  // A timed-out native request can still commit. The capture manager resolves
  // that uncertainty through its durable receipt before cancelling anything.
  return IDMCore.handoff((host, payload) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(__xdL.t("The desktop app did not respond in time."))), 15000);
    Promise.resolve().then(() => api.runtime.sendNativeMessage(host, payload)).then(resolve, reject).finally(() => clearTimeout(timer));
  }), request);
}
async function send(request) {
  try {
    const reply = await native(request);
    await saveStatus(true, reply.message).catch(console.warn);
    return reply;
  } catch (error) {
    const message = error.message + " Open Browser setup in Download Engine if the desktop app is not connected.";
    await saveStatus(false, message).catch(console.warn);
    return { ok: false, version: 1, message };
  }
}
async function enabled() {
  const { autoCapture } = await api.storage.local.get("autoCapture");
  return (autoCapture === undefined || autoCapture === true) && await api.permissions.contains({ permissions: ["downloads"] });
}
const RULES = IDMCapture.rules;
async function captureRules() {
  const saved = await api.storage.local.get([RULES.KEY, "autoCapture"]);
  return RULES.normalize(saved[RULES.KEY], saved.autoCapture === true);
}
// Users who turned capture on before rules existed keep capturing every type.
async function migrateCaptureRules() {
  const saved = await api.storage.local.get([RULES.KEY, "autoCapture"]);
  if (saved[RULES.KEY] === undefined && saved.autoCapture === true) await api.storage.local.set({ [RULES.KEY]: RULES.LEGACY });
}
function saveCaptureRules(input) {
  const { rules, error } = RULES.parse(input);
  if (error) return Promise.resolve({ ok: false, message: error });
  return serializeCaptureSettings(async () => {
    await api.storage.local.set({ [RULES.KEY]: rules });
    return { ok: true, message: __xdL.t("Capture rules saved."), rules };
  });
}
const capture = new IDMCapture.Manager({
  downloads: Object.fromEntries(["search", "pause", "resume", "cancel"]
    .map(method => [method, (...args) => api.downloads[method](...args)])),
  storage: api.storage.local, enabled, rules: captureRules, native, report: saveStatus,
  uuid: () => crypto.randomUUID()
});
void serializeCaptureSettings(migrateCaptureRules).catch(console.warn);
const media = new IDMMedia.Manager({ api });
const linkRecovery = new IDMLinkRecovery.Manager({api,native,report:async(ok,message)=>{
  await saveStatus(ok,message);await recordDiagnostic({setting:"recovery",ok,message});
}});
void media.start().catch(() => { media.error = __xdL.t("Media detection is unavailable. Reload the extension to retry."); });
const pageLinks = new IDMPageLinks.Manager({ api, native, detected: tab => media.list(tab) });
const browserLogin = new IDMBrowserLogin.Manager({ api, native });
const videoButtons = new IDMVideoButtons.Manager({ api, pageLinks });
async function configureMedia(enabled) {
  const reply=await media.configure(enabled);
  if(api.scripting?.getRegisteredContentScripts) await videoButtons.sync(enabled);
  return reply;
}
const permissionFlow = new IDMPermissionFlow.Manager({ api, report: (ok, message) => saveStatus(ok, message).catch(console.warn), settings: {
  recovery: {grant:()=>({permissions:["downloads"]}),configure:()=>{installDownloadListeners();return Promise.resolve({ok:true,message:__xdL.t("Recovery access enabled. Click the same file on its webpage, then review it in Download Engine.")});},denied:__xdL.t("Download access was not granted. You can still paste a fresh link in Download Engine.")},
  capture: { grant: () => ({ permissions: ["downloads"] }), configure: () => configureCapture(true),
    get denied() { return __xdL.t("Download access was not granted. Automatic capture stays off."); } },
  media: { grant: () => ({ permissions: ["webRequest"], origins: ["https://*/*", "http://*/*"] }), configure: () => configureMedia(true),
    get denied() { return __xdL.t("Website access was not granted. Media detection stays off."); } },
  video: { grant: ({ pattern }) => {
    if (typeof pattern !== "string" || pattern !== IDMVideoButtons.site(pattern)) throw Error(__xdL.t("Invalid video button site."));
    return { origins: [pattern] };
  }, configure: ({ pattern }) => videoButtons.configure(true, pattern),
  get denied() { return __xdL.t("Site access was not granted. Allow this site to show video buttons."); } }
} });
// Firefox passes a synchronous requester from its visible popup; the background
// watches approval even if Firefox destroys that popup before resolving its API call.
function requestPopupPermission(message, requestInPopup) {
  if(message.setting==="recovery")return trackOperation("recovery",()=>permissionFlow.request(message,requestInPopup));
  return trackSetting(message.setting, true, () => permissionFlow.request(message, requestInPopup));
}
api.tabs.onRemoved.addListener(tabId => { permissionFlow.tabLeft(tabId); media.closed(tabId); videoButtons.last.delete(tabId); void pageLinks.closed(tabId).catch(console.warn); });
api.tabs.onActivated?.addListener(({ tabId, windowId }) => {
  const target = permissionFlow.pending?.target;
  if (target?.windowId === windowId && target.tabId !== tabId) permissionFlow.tabLeft(target.tabId);
});
api.tabs.onUpdated?.addListener((tabId, changes) => media.navigated(tabId, changes));
if (api.scripting?.getRegisteredContentScripts) {
  void videoButtons.sync(true).catch(console.warn);
  api.permissions.onAdded.addListener(() => { void videoButtons.sync(true).catch(console.warn); });
  api.permissions.onRemoved.addListener(() => { void videoButtons.sync().catch(console.warn); });
}
let listening = false;
function installDownloadListeners() {
  if (listening || !api.downloads) return;
  listening = true;
  api.downloads.onCreated.addListener(item => {
    void ready().then(async () => {if(!await linkRecovery.consider(item.id))await capture.consider(item.id);}).catch(console.warn);
  });
  api.downloads.onChanged.addListener(delta => {
    if (delta.state?.current === "complete" || delta.state?.current === "interrupted") {
      void capture.recover(delta.id);
      void linkRecovery.recover(delta.id);
    } else if (delta.filename || delta.mime || delta.totalBytes || delta.finalUrl) {
      void ready().then(async () => {if(!await linkRecovery.consider(delta.id))await capture.consider(delta.id);}).catch(console.warn);
    }
  });
}
async function ready() {
  await globalThis.IDMI18n?.ready;
  // Recreating an existing alarm would postpone recovery on every download event.
  if (!await api.alarms.get(ALARM)) await api.alarms.create(ALARM, { periodInMinutes: 1 });
}
async function recover() {
  if (await api.permissions.contains({ permissions: ["downloads"] })) {await linkRecovery.recoverAll();await capture.recoverAll();}
}
installDownloadListeners();
api.alarms.onAlarm.addListener(alarm => { if (alarm.name === ALARM) void recover().catch(console.warn); });
api.permissions.onAdded.addListener(() => { void media.start().catch(console.warn); installDownloadListeners(); void recover().catch(console.warn); });
api.permissions.onRemoved.addListener(removed => {
  if (removed.permissions?.includes("downloads")) {permissionFlow.cancel("capture");permissionFlow.cancel("recovery");}
  if (removed.permissions?.includes("webRequest") || removed.origins?.length) permissionFlow.cancel("media");
  if (removed.origins?.length) permissionFlow.cancel("video");
  if (removed.permissions?.includes("webRequest") || removed.origins?.length) media.revoked();
  if (removed.permissions?.includes("downloads")) {
    downloadPermissionRevision++;
    void serializeCaptureSettings(async () => {
      await api.storage.local.set({ autoCapture: false });
      await saveStatus(false, __xdL.t("Automatic capture is off. If a download is paused, use Resume in the browser Downloads list."));
    }).catch(console.warn);
  }
});
api.runtime.onStartup.addListener(() => { void ready().then(recover).catch(console.warn); });
void ready().then(recover).catch(console.warn);

let menuTail = Promise.resolve();
function refreshLanguageMenus() {
  menuTail = menuTail.catch(() => {}).then(async () => {
    await globalThis.IDMI18n?.ready;
    await api.contextMenus.removeAll();
    api.contextMenus.create({
      id: "idm-ex-download", title: __xdL.t("Download with Download Engine"), contexts: ["link"],
      targetUrlPatterns: ["http://*/*", "https://*/*"]
    });
    api.contextMenus.create({ id: "idm-ex-browser-login", title: __xdL.t("Download using browser login…"), contexts: ["link"], targetUrlPatterns: ["https://*/*", "http://127.0.0.1/*", "http://[::1]/*"] });
    api.contextMenus.create({ id: "idm-ex-videos", title: __xdL.t("Find videos with Download Engine"), contexts: ["page", "video"], documentUrlPatterns: ["http://*/*", "https://*/*"] });
    api.contextMenus.create({ id: "idm-ex-page-links", title: __xdL.t("Download links from this page"), contexts: ["page"], documentUrlPatterns: ["http://*/*", "https://*/*"] });
  }).catch(error => saveStatus(false, error.message));
  return menuTail;
}
api.runtime.onInstalled.addListener(refreshLanguageMenus);
api.runtime.onStartup.addListener(refreshLanguageMenus);
api.storage.onChanged?.addListener((changes, area) => { if(area === "local" && changes.displayLanguage) void refreshLanguageMenus(); });
api.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId === "idm-ex-browser-login") {
    void browserLogin.open(info.linkUrl, tab).catch(error => saveStatus(false, error.message));
    return;
  }
  if (["idm-ex-page-links", "idm-ex-videos"].includes(info.menuItemId)) {
    void pageLinks.scan(tab, info.menuItemId === "idm-ex-videos" ? "videos" : "links").then(reply => saveStatus(true, reply.message), error => saveStatus(false, error.message));
    return;
  }
  if (info.menuItemId !== "idm-ex-download" || !info.linkUrl) return;
  try { void send(IDMCore.makeRequest(info.linkUrl, crypto.randomUUID(),info.pageUrl || tab?.url)); }
  catch (error) { void saveStatus(false, error.message); }
});
function configureCapture(value) {
  const permissionRevision = downloadPermissionRevision;
  return serializeCaptureSettings(() => configureCaptureNow(value, permissionRevision));
}
async function configureCaptureNow(value, permissionRevision) {
  if (typeof value !== "boolean") throw new Error(__xdL.t("Invalid automatic capture setting."));
  if (value) {
    if (!await api.permissions.contains({ permissions: ["downloads"] })) throw new Error(__xdL.t("Allow download access to enable automatic capture."));
    let reply;
    try { reply = await native({ action: "ping" }); }
    catch (error) {
      const message = __xdL.t("Automatic capture stays off. Open Download Engine → Tools → Browser setup, select your browser, and choose Connect browser. Then retry here. ") + error.message;
      await saveStatus(false, message).catch(console.warn);
      throw new Error(message, { cause: error });
    }
    if (!reply.capabilities?.includes("automatic_capture_v1")) throw new Error(__xdL.t("Update the Download Engine desktop app before enabling automatic capture."));
    // Access may be revoked while the host is starting. Even a revoke/regrant
    // must invalidate that older enable request until the user enables again.
    if (permissionRevision !== downloadPermissionRevision ||
        !await api.permissions.contains({ permissions: ["downloads"] })) {
      throw new Error(__xdL.t("Download access changed. Enable automatic capture again when ready."));
    }
  }
  // The first enable stores the default rules; existing rules are kept.
  const saved = value ? await api.storage.local.get(RULES.KEY) : {};
  await api.storage.local.set({ autoCapture: value, ...(value && saved[RULES.KEY] === undefined ? { [RULES.KEY]: RULES.DEFAULTS } : {}) });
  installDownloadListeners();
  await ready();
  await recover();
  const message = value ? __xdL.t("Automatic capture is on for direct file downloads.") : __xdL.t("Automatic capture is off.");
  await saveStatus(true, message).catch(console.warn);
  return { ok: true, message };
}
api.runtime.onMessage.addListener((message, sender, respond) => {
  if (sender.id !== api.runtime.id) return false;
  if (message?.action === "video_button_state" && /^https?:\/\//.test(sender.url || "")) {
    videoButtons.pageStatus(sender).then(respond, error => respond({ok:false,message:error.message}));
    return true;
  }
  if (message?.action === "on_page_videos" && /^https?:\/\//.test(sender.url || "")) {
    trackOperation("review",()=>videoButtons.open(sender)).then(respond, error => respond({ok:false,message:error.message}));
    return true;
  }
  if (!sender.url?.startsWith(api.runtime.getURL(""))) return false;
  let task;
  try {
    const page = sender.url.split(/[?#]/)[0];
    if (message?.action === "diagnostics_get" && page === api.runtime.getURL("popup.html")) task = settingDiagnostics();
    else if (message?.action === "link_recovery_status" && page === api.runtime.getURL("popup.html")) task = linkRecovery.status();
    else if (message?.action === "diagnostics_clear" && page === api.runtime.getURL("popup.html")) task = clearSettingDiagnostics();
    else if (message?.action === "permission_popup_opened" && page === api.runtime.getURL("popup.html")) task = Promise.resolve(permissionFlow.popupOpened(message.returnTo || {}));
    else if (message?.action === "request_setting_permission" && page === api.runtime.getURL("popup.html")) task = requestPopupPermission(message);
    else if (message?.action === "media_status" && page === api.runtime.getURL("popup.html")) task = media.status();
    else if (message?.action === "configure_media" && page === api.runtime.getURL("popup.html")) { permissionFlow.cancel("media"); task = trackSetting("media", message.enabled, () => configureMedia(message.enabled), true); }
    else if (message?.action === "clear_media" && page === api.runtime.getURL("popup.html")) task = media.clear(message.tabId);
    else if (message?.action === "review_media" && page === api.runtime.getURL("popup.html")) task = trackOperation("review",()=>media.review(message.tabId, message.id, pageLinks));
    else if (message?.action === "video_button_status" && page === api.runtime.getURL("popup.html")) task = videoButtons.status();
    else if (message?.action === "configure_video_button" && page === api.runtime.getURL("popup.html")) { permissionFlow.cancel("video"); task = trackSetting("video", message.enabled, () => videoButtons.configure(message.enabled,message.pattern), true); }
    else if (message?.action === "browser_login_open" && page === api.runtime.getURL("popup.html")) task = browserLogin.open(message.url);
    else if (["browser_login_get", "browser_login_submit"].includes(message?.action) &&
        page === api.runtime.getURL("login.html") && new URL(sender.url).searchParams.get("id") === message.id) {
      task = message.action === "browser_login_get" ? browserLogin.get(message.id) : browserLogin.submit(message.id);
    }
    else if (message?.action === "scan_videos" && page === api.runtime.getURL("popup.html")) task = trackOperation("review",()=>pageLinks.scan(undefined, "videos"));
    else if (message?.action === "scan_page" && page === api.runtime.getURL("popup.html")) task = pageLinks.scan();
    else if (message?.action === "page_links_get" && page === api.runtime.getURL("links.html")) task = pageLinks.get(message.id);
    else if (message?.action === "page_links_hls" && page === api.runtime.getURL("links.html") && new URL(sender.url).hash.slice(1) === message.id) task = pageLinks.previewHls(message.id, message.index);
    else if (message?.action === "page_links_submit" && page === api.runtime.getURL("links.html")) task = pageLinks.submit(message.id, message);
    else if (message?.action === "capture_rules_get" && page === api.runtime.getURL("popup.html")) task = captureRules().then(rules => ({ ok: true, rules, defaults: RULES.DEFAULT_TYPES }));
    else if (message?.action === "capture_rules_save" && page === api.runtime.getURL("popup.html")) task = saveCaptureRules(message.rules);
    else if (message?.action === "configure_capture") { permissionFlow.cancel("capture"); task = trackSetting("capture", message.enabled, () => configureCapture(message.enabled)); }
    else if (message?.action === "recover_capture") task = recover().then(() => pageLinks.recover()).then(count => ({ ok: true, message: count ? __xdL.t("Pending reviews and downloads recovered. Browser handoffs checked.") : __xdL.t("Recovery checked. See the browser Downloads list for any paused files.") }));
    else if (message?.action === "ping") task = trackOperation("connection",()=>send({ action: "ping" }));
    else if (message?.action === "add") task = api.tabs.query({active:true,currentWindow:true}).then(tabs=>send(IDMCore.makeRequest(message.url, message.request_id,tabs[0]?.url)));
    else return false;
  } catch (error) { respond({ ok: false, message: error.message }); return false; }
  task.then(respond, error => respond({ ok: false, message: error.message }));
  return true;
});


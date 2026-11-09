/* Response-metadata discovery. Never reads cookies, request headers or
   response bodies, and never starts a download. Records live only in session storage. */
(function (root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const PREFIX = "media-tab:", TTL = 30 * 60 * 1000, TAB_LIMIT = 16, ITEM_LIMIT = 40;
  const GRANT = { permissions: ["webRequest"], origins: ["https://*/*", "http://*/*"] };
  const FILTER = { urls: GRANT.origins, types: ["main_frame", "sub_frame", "media", "xmlhttprequest", "other"] };
  function pageUrl(value) { return IDMCore.normalizeUrl(value); }
  function dailymotionMedia(value) {
    let url; try { url=new URL(value); } catch { return null; }
    if(url.protocol!=="https:" || url.username || url.password || url.port)return null;
    if(/(^|\.)dmcdn\.net$/.test(url.hostname)) {
      const stream=/\/(video|cloud)\/fmp4\/(\d+)\/([a-zA-Z0-9_-]+)\/(\d+)\/(manifest\.m3u8|init\.mp4)$/.exec(url.pathname);
      if(stream)return {kind:stream[5]==="init.mp4"?"init":"stream",
        key:`${stream[1]}/${stream[2]}/${stream[3]}/${stream[4]}${url.search}`,format:stream[3]};
      if(/\/preview\/fmp4\/\d+\/[a-zA-Z0-9_-]+\/\d+\.pmp4$/.test(url.pathname))return {kind:"preview"};
    }
    if(["dmxleo.dailymotion.com","cdndirector.dailymotion.com"].includes(url.hostname)) {
      if(/^\/cdn\/manifest\/video\/x[a-z0-9]+\.m3u8$/.test(url.pathname))
        return {kind:"entry",player:url.hostname==="dmxleo.dailymotion.com"};
    }
    return null;
  }
  function compactMedia(items) {
    const result=[],positions=new Map();
    for(const item of items) {
      const info=dailymotionMedia(item.url);
      if(info?.kind==="init" || info?.kind==="preview")continue;
      // Response metadata cannot prove which playlist is replayable outside the
      // player. Keep alternative entry points even after a CDN rendition appears.
      // CDN host and signed path prefixes can change for the same asset. Keep
      // the latest observed URL and stable row ID, but retain distinct renditions.
      const key=info?.kind==="stream"?`dm:${info.key}`:item.url;
      if(positions.has(key)) {
        const index=positions.get(key),previous=result[index];
        if(item.seen>=previous.seen)result[index]={...item,id:previous.id};
      } else { positions.set(key,result.length);result.push(item); }
    }
    return result;
  }
  function describe(details) {
    if (details.method !== "GET" || ![200, 206].includes(details.statusCode)) return null;
    let url;
    try { url = new URL(pageUrl(details.url)); } catch { return null; }
    if (url.href.length > 4096 || /\.(ts|m4s|cmfv|cmfa|key)$/i.test(url.pathname)) return null;
    const dm=dailymotionMedia(url.href);
    if(dm?.kind==="init" || dm?.kind==="preview")return null;
    const headers = details.responseHeaders || [];
    const header = name => String(headers.find(h => h.name.toLowerCase() === name)?.value || "");
    const mime = header("content-type").split(";")[0].trim().toLowerCase();
    if (/html|json|javascript/.test(mime) || (mime === "text/plain" && !/\.(m3u8|mpd)$/i.test(url.pathname))) return null;
    const hls = /^(application\/(vnd\.apple\.mpegurl|x-mpegurl|mpegurl)|audio\/(x-)?mpegurl)$/.test(mime) || /\.m3u8$/i.test(url.pathname);
    const dash = mime === "application/dash+xml" || /\.mpd$/i.test(url.pathname);
    const video = /^video\//.test(mime) && mime !== "video/mp2t" || /\.(mp4|webm|mkv|mov|m4v|ogv)$/i.test(url.pathname);
    const audio = /^audio\//.test(mime) || /\.(mp3|m4a|flac|ogg|opus|wav)$/i.test(url.pathname);
    if (!hls && !dash && !video && !audio) return null;
    const kind = hls ? "hls" : dash ? "dash" : video ? "video" : "audio";
    const suffix = {"video/mp4":"mp4","video/webm":"webm","audio/mp4":"m4a","audio/mpeg":"mp3","audio/ogg":"ogg"}[mime];
    let filename = url.pathname.split("/").pop() || (video ? "video" : "audio");
    try { filename = decodeURIComponent(filename); } catch { /* Keep a literal malformed escape. */ }
    if (suffix && !/\.[a-z0-9]{2,5}$/i.test(filename)) filename += "." + suffix;
    filename = Array.from(filename.toWellFormed().replace(/[\u0000-\u001f\u007f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/g, " ")).slice(0, 160).join("");
    const label=dm?.kind==="stream" ? "HLS · "+(dm.format.match(/_(ld|sd|hq|hd|fhd|uhd)$/i)?.[1].toUpperCase() || dm.format)
      : dm?.kind==="entry" ? __xdL.t(dm.player?"HLS · player playlist":"HLS · alternate playlist") : filename;
    const sizeText = details.statusCode === 206 ? /^bytes \d+-\d+\/(\d+)$/i.exec(header("content-range"))?.[1] : header("content-length");
    const size = /^\d+$/.test(sizeText || "") ? Number(sizeText) : 0;
    return { url: url.href, filename, text: label, kind, mime: mime.slice(0, 100), hls, dash,
      size: Number.isSafeInteger(size) && size > 0 && !hls && !dash ? size : null, source: "network" };
  }
  class Manager {
    constructor({ api, now = () => Date.now(), uuid = () => crypto.randomUUID(), trace = () => {} }) {
      Object.assign(this, { api, now, uuid, trace });
      this.action = api.action || api.browserAction;
      this.enabled = false; this.revision = 0; this.attention = false;
      this.tabs = new Map(); this.requests = new Map(); this.epochs = new Map();
      this.tail = Promise.resolve(); this.listeners = false;
    }
    serial(run) { const task = this.tail.then(run); this.tail = task.catch(() => {}); return task; }
    event(run) { void this.serial(run).catch(() => { this.error = __xdL.t("Media discovery could not update its list. Turn detection off and on to retry."); this.trace("discovery_error"); }); }
    start() {
      // Register during worker evaluation so a later network event can wake MV3.
      this.install();
      const revision = this.revision;
      return this.serial(async () => {
        const settings = await this.api.storage.local.get(["mediaDetection", "lastResult"]);
        this.attention = settings.lastResult?.ok === false;
        const allowed = await this.api.permissions.contains(GRANT);
        this.enabled = revision === this.revision && (settings.mediaDetection === undefined || settings.mediaDetection === true) && allowed;
        const saved = await this.api.storage.session.get(null);
        for (const [key, record] of Object.entries(saved)) {
          if (!key.startsWith(PREFIX)) continue;
          const tabId = Number(key.slice(PREFIX.length));
          if (Number.isInteger(tabId) && record && Array.isArray(record.items)) this.tabs.set(tabId, record);
        }
        if (!this.enabled) await this.clearAll();
        else await this.prune();
        this.trace("ready", { enabled: this.enabled, allowed, count: this.tabs.size });
      });
    }
    install() {
      if (this.listeners || !this.api.webRequest) return;
      this.api.webRequest.onBeforeRequest.addListener(d => this.event(() => this.before(d)), FILTER);
      this.api.webRequest.onHeadersReceived.addListener(d => this.event(() => this.received(d)), FILTER, ["responseHeaders"]);
      const finish = d => this.event(() => this.requests.delete(d.requestId));
      this.api.webRequest.onCompleted.addListener(finish, FILTER);
      this.api.webRequest.onErrorOccurred.addListener(finish, FILTER);
      this.listeners = true;
    }
    async configure(value) {
      if (typeof value !== "boolean") throw Error(__xdL.t("Invalid media detection setting."));
      const revision = ++this.revision;
      if (!value) this.enabled = false;
      return this.serial(async () => {
        if (value && !await this.api.permissions.contains(GRANT)) throw Error(__xdL.t("Allow media detection and website access to watch media requests."));
        if (revision !== this.revision) throw Error(__xdL.t("Media detection access changed. Enable it again when ready."));
        await this.api.storage.local.set({ mediaDetection: value });
        this.enabled = value; this.error = null; this.install();
        if (!value) await this.clearAll();
        return { ok: true, message: value ? __xdL.t("Media detection is on. Start playback or reload the page.") : __xdL.t("Media detection is off. Its list has been cleared.") };
      });
    }
    revoked() {
      this.enabled = false; ++this.revision;
      this.event(async () => { await this.api.storage.local.set({ mediaDetection: false }); await this.clearAll(); });
    }
    async badge(tabId, count = 0) {
      // Null restores global status inheritance. An empty per-tab string would
      // hide later handoff errors, and a per-tab "!" would outlive a cleared list.
      await this.action.setBadgeText({ tabId, text: count ? this.attention ? "!" : String(count) : null });
      // The global badge is only ever an error or blank; keep its error color
      // when a tab no longer has a media count (Chromium has no color reset).
      await this.action.setBadgeBackgroundColor({ tabId, color: !count || this.attention ? "#ba4736" : "#237d62" });
    }
    setAttention(value) {
      return this.serial(async () => { this.attention = value; for (const [id, record] of this.tabs) await this.badge(id, record.items.length); });
    }
    async remove(tabId) {
      this.tabs.delete(tabId);
      await this.api.storage.session.remove(PREFIX + tabId);
      await this.badge(tabId).catch(() => {}); // A tab may have closed already.
    }
    clearAll() {
      this.requests.clear(); this.epochs.clear();
      return Promise.all([...this.tabs.keys()].map(id => this.remove(id)));
    }
    async prune() {
      const cutoff = this.now() - TTL;
      for (const [id, record] of this.tabs) {
        if (!Number.isFinite(record.updated) || record.updated < cutoff || record.updated > this.now()) { await this.remove(id); continue; }
        const items = compactMedia(record.items.filter(item => item.seen >= cutoff && item.seen <= this.now())).slice(-ITEM_LIMIT);
        if (!items.length) await this.remove(id);
        else if (items.length !== record.items.length) {
          const updated = { ...record, items };
          await this.api.storage.session.set({ [PREFIX + id]: updated });
          this.tabs.set(id, updated); await this.badge(id, items.length);
        }
      }
      while (this.tabs.size > TAB_LIMIT) await this.remove(this.tabs.keys().next().value);
      for (const [id, req] of this.requests) if (req.time < cutoff) this.requests.delete(id);
      while (this.requests.size > 1000) this.requests.delete(this.requests.keys().next().value);
    }
    async before(d) {
      if (!this.enabled || d.tabId < 0 || d.incognito) return;
      if (d.type === "main_frame") {
        this.epochs.set(d.tabId, (this.epochs.get(d.tabId) || 0) + 1);
        await this.remove(d.tabId);
      }
      if (d.method !== "GET") return;
      this.requests.set(d.requestId, { tabId: d.tabId, epoch: this.epochs.get(d.tabId) || 0, time: this.now() });
      while (this.requests.size > 1000) this.requests.delete(this.requests.keys().next().value);
    }
    async received(d) {
      const request = this.requests.get(d.requestId);
      if (d.statusCode >= 300 && d.statusCode < 400) return;
      this.requests.delete(d.requestId);
      if (d.incognito || d.tabId < 0) return;
      const item = describe(d);
      const dm = dailymotionMedia(d.url);
      const candidate = item || dm || /\.(m3u8|mpd)(?:[?#]|$)/i.test(d.url || "");
      const reason = !this.enabled ? "disabled" : !request ? "no_request" :
        request.tabId !== d.tabId || request.epoch !== (this.epochs.get(d.tabId) || 0) ? "stale_navigation" :
        !item ? dm?.kind === "init" ? "init" : dm?.kind === "preview" ? "preview" : "unsupported" : null;
      if (reason) {
        if (candidate) this.trace("network_ignored", { tab: d.tabId, reason, status: d.statusCode, cached: d.fromCache === true });
        return;
      }
      const tab = await this.api.tabs.get(d.tabId).catch(() => null);
      if (!tab || tab.incognito || !/^https?:\/\//.test(tab.url || "")) return;
      const revision = this.revision;
      if (!this.enabled || !await this.api.permissions.contains(GRANT) || revision !== this.revision || !this.enabled) return;
      await this.prune();
      const url = pageUrl(tab.url), previous = this.tabs.get(tab.id);
      const items = previous?.url === url ? [...previous.items] : [];
      const existing = previous?.url === url && previous.items.find(entry => entry.url === item.url);
      const next = { ...item, id: existing?.id || this.uuid(), seen: this.now() };
      if (existing) items[items.indexOf(existing)] = next;
      else items.push(next);
      const record = { url, title: (tab.title || __xdL.t("Media on this page")).slice(0, 200), updated: this.now(), items: compactMedia(items).slice(-ITEM_LIMIT) };
      while (this.tabs.size >= TAB_LIMIT && !this.tabs.has(tab.id)) await this.remove(this.tabs.keys().next().value);
      await this.api.storage.session.set({ [PREFIX + tab.id]: record });
      this.tabs.delete(tab.id); this.tabs.set(tab.id, record);
      await this.badge(tab.id, record.items.length);
      this.trace("network_saved", { tab: tab.id, count: record.items.length, format: item.kind, status: d.statusCode, cached: d.fromCache === true });
    }
    closed(tabId) {
      this.event(async () => {
        this.epochs.delete(tabId);
        for (const [id, req] of this.requests) if (req.tabId === tabId) this.requests.delete(id);
        await this.remove(tabId);
      });
    }
    navigated(tabId, changes) {
      if (!changes.url) return;
      this.event(async () => {
        // History/SPA navigation has no main-frame network request. Invalidate
        // older responses as well as already displayed entries in that case.
        this.epochs.set(tabId, (this.epochs.get(tabId) || 0) + 1);
        let url; try { url = pageUrl(changes.url); } catch { url = null; }
        if (this.tabs.get(tabId)?.url !== url) await this.remove(tabId);
      });
    }
    async current() {
      const tab = (await this.api.tabs.query({ active: true, currentWindow: true }))[0];
      if (!tab || tab.incognito || !Number.isInteger(tab.id) || !/^https?:\/\//.test(tab.url || "")) throw Error(__xdL.t("Open a regular web page to review its media."));
      return tab;
    }
    list(tab) {
      return this.serial(async () => {
        tab ||= await this.current();
        await this.prune();
        if (!this.enabled || tab.incognito || !await this.api.permissions.contains(GRANT)) return [];
        const record = this.tabs.get(tab.id);
        if (record && record.url !== pageUrl(tab.url)) { await this.remove(tab.id); return []; }
        return record ? structuredClone(record.items) : [];
      });
    }
    async status() {
      const tab = (await this.api.tabs.query({ active: true, currentWindow: true }))[0];
      const available = tab && !tab.incognito && Number.isInteger(tab.id) && /^https?:\/\//.test(tab.url || "");
      const items = available ? await this.list(tab) : await this.serial(async () => { await this.prune(); return []; });
      // The global toggle remains usable on browser settings and new-tab pages.
      return { ok: true, enabled: this.enabled, tabId: available ? tab.id : null, items, error: this.error || null };
    }
    async clear(tabId) {
      const tab = await this.current();
      if (tab.id !== tabId) throw Error(__xdL.t("The page changed. Reopen the extension."));
      return this.serial(async () => {
        this.epochs.set(tab.id, (this.epochs.get(tab.id) || 0) + 1);
        await this.remove(tab.id);
        return { ok: true, message: __xdL.t("Media list cleared. New playback requests can appear again.") };
      });
    }
    async review(tabId, id, pageLinks) {
      const tab = await this.current();
      if (tab.id !== tabId) throw Error(__xdL.t("The page changed. Reopen the extension."));
      const item = (await this.list(tab)).find(entry => entry.id === id);
      if (!item) throw Error(__xdL.t("This media item expired. Play it again to detect it."));
      return pageLinks.openScan({ url: tab.url, title: tab.title, mode: "videos", links: [item] });
    }
  }
  root.IDMMedia = { Manager, describe, GRANT, TTL, TAB_LIMIT, ITEM_LIMIT };
})(globalThis);

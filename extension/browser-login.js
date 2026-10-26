/* Explicit site-login handoff. Cookies live only in this operation's memory. */
(function (root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const PREFIX = "browser-login:";
  const TTL = 10 * 60 * 1000;
  function target(value) {
    const url = new URL(IDMCore.normalizeUrl(value));
    if (url.protocol !== "https:" && !(url.protocol === "http:" && ["127.0.0.1", "[::1]"].includes(url.hostname))) {
      throw Error(__xdL.t("Browser login requires HTTPS. Open a secure direct file link."));
    }
    return url;
  }
  function permissionOrigin(url) { return url.protocol + "//" + url.hostname + "/*"; }
  function loginPermission(api) {
    const grant = { permissions: ["cookies"] };
    // Chrome has no data_collection key. Firefox declares this optional consent
    // separately from website access, which is already needed for detection.
    if (api.runtime.getManifest().browser_specific_settings?.gecko?.data_collection_permissions?.optional?.includes("authenticationInfo")) {
      grant.data_collection = ["authenticationInfo"];
    }
    return grant;
  }
  function selectCookies(all, url, storeId, now = Date.now() / 1000) {
    const selected = [];
    for (const cookie of all) {
      // Do not guess a storage partition or read first-party-isolated cookies.
      if (cookie.storeId !== storeId || cookie.partitionKey || cookie.firstPartyDomain ||
          (cookie.expirationDate !== undefined && cookie.expirationDate <= now)) continue;
      const domain = cookie.domain?.replace(/^\./, "").toLowerCase();
      const host = url.hostname.toLowerCase();
      const matchesDomain = host === domain || (!cookie.hostOnly && domain && host.endsWith("." + domain)
        && !/^[\d.]+$/.test(host) && !host.includes(":"));
      const matchesPath = typeof cookie.path === "string" && (url.pathname === cookie.path ||
        (url.pathname.startsWith(cookie.path) && (cookie.path.endsWith("/") || url.pathname[cookie.path.length] === "/")));
      if (!matchesDomain || !matchesPath || (cookie.secure && url.protocol !== "https:")) continue;
      if (typeof cookie.name !== "string" || !/^[!#$%&'*+\-.^_\x60|~0-9A-Za-z]+$/.test(cookie.name) || cookie.name.length > 256 ||
          typeof cookie.value !== "string" || !/^[\x21\x23-\x2b\x2d-\x3a\x3c-\x5b\x5d-\x7e]*$/.test(cookie.value) || cookie.value.length > 16384 ||
          typeof cookie.hostOnly !== "boolean" || typeof cookie.secure !== "boolean" ||
          !domain || domain.length > 255 || !/^[a-z0-9.:[\]-]+$/.test(domain) ||
          !cookie.path.startsWith("/") || cookie.path.length > 4096 || !/^[\x21-\x7e]+$/.test(cookie.path) ||
          (cookie.expirationDate !== undefined && !Number.isFinite(cookie.expirationDate))) {
        throw Error(__xdL.t("This site's cookies cannot be transferred safely. Download this file in the browser."));
      }
      selected.push({ name: cookie.name, value: cookie.value, domain, path: cookie.path,
        host_only: cookie.hostOnly, secure: cookie.secure,
        expires: cookie.expirationDate === undefined ? null : Math.floor(cookie.expirationDate) });
    }
    selected.sort((a, b) => b.path.length - a.path.length);
    if (!selected.length) throw Error(__xdL.t("No supported login cookies match this file. Sign in on this site first. Private, partitioned and first-party-isolated sessions are not supported."));
    if (selected.length > 128 || selected.reduce((n, c) => n + c.name.length + c.value.length + 3, 0) > 32768 ||
        JSON.stringify(selected).length > 120000) throw Error(__xdL.t("This site's browser login is too large. Download this file in the browser."));
    return selected;
  }
  class Manager {
    constructor({ api, native, now = () => Date.now(), uuid = () => crypto.randomUUID() }) {
      this.api = api; this.native = native; this.now = now; this.uuid = uuid;
      this.inflight = new Map();
    }
    async context(record) {
      let tab;
      try { tab = await this.api.tabs.get(record.tabId); }
      catch { throw Error(__xdL.t("The original site tab was closed. Open the site and try again.")); }
      if (tab.incognito || target(tab.url).origin !== record.origin) {
        throw Error(__xdL.t("Keep the original signed-in site tab open. Private browsing is not supported."));
      }
      return tab;
    }
    async open(value, suppliedTab) {
      const url = target(value);
      const tab = suppliedTab || (await this.api.tabs.query({ active: true, currentWindow: true }))[0];
      if (!tab || !Number.isInteger(tab.id) || tab.incognito || target(tab.url).origin !== url.origin) {
        throw Error(__xdL.t("Open the signed-in site in a regular tab, then choose a file on that same server."));
      }
      const id = this.uuid();
      const record = { tabId: tab.id, origin: url.origin, url: url.href, created: this.now(), requestId: "login:" + id };
      await this.context(record);
      const old = await this.api.storage.session.get(null);
      const keys = Object.keys(old).filter(k => k.startsWith(PREFIX)).sort((a, b) => old[a].created - old[b].created);
      for (const key of keys) {
        if (this.now() - old[key].created >= TTL || keys.indexOf(key) < keys.length - 29) await this.api.storage.session.remove(key);
      }
      await this.api.storage.session.set({ [PREFIX + id]: record });
      await this.api.tabs.create({ url: this.api.runtime.getURL("login.html?id=" + id) });
      return { ok: true, message: __xdL.t("Review this site's browser login in the new tab.") };
    }
    async record(id) {
      if (typeof id !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id)) throw Error(__xdL.t("Invalid browser login request."));
      const record = (await this.api.storage.session.get(PREFIX + id))[PREFIX + id];
      if (!record || this.now() - record.created >= TTL || this.now() < record.created) {
        throw Error(__xdL.t("This login request expired. Check the Download Engine queue before sending the file again."));
      }
      await this.context(record);
      return record;
    }
    async get(id) {
      const record = await this.record(id);
      return { ok: true, url: record.url, origin: record.origin, permissionOrigin: permissionOrigin(target(record.url)), grant: loginPermission(this.api) };
    }
    async submit(id) {
      if (this.inflight.has(id)) return this.inflight.get(id);
      const task = this.transfer(id);
      this.inflight.set(id, task);
      try { return await task; } finally { this.inflight.delete(id); }
    }
    async transfer(id) {
      const record = await this.record(id);
      if (record.jobId) return { ok: true, message: __xdL.t("Added with browser login"), job_id: record.jobId };
      const url = target(record.url);
      const grant = { ...loginPermission(this.api), origins: [permissionOrigin(url)] };
      if (!await this.api.permissions.contains(grant)) throw Error(__xdL.t("Allow cookie access for this site to use its login."));
      const connection = await this.native({ action: "ping" });
      if (!connection.capabilities?.includes("browser_login_v1")) throw Error(__xdL.t("Update the Download Engine desktop app before using browser login."));
      let all = [], cookies = [];
      try {
        const tab = await this.context(record);
        const stores = await this.api.cookies.getAllCookieStores();
        const store = stores.find(s => s.tabIds.includes(tab.id));
        if (!store || (tab.cookieStoreId && tab.cookieStoreId !== store.id)) throw Error(__xdL.t("Cookie store mismatch"));
        // Omitting partitionKey requests only unpartitioned cookies. In Firefox
        // with first-party isolation enabled this may fail; never broaden it.
        all = await this.api.cookies.getAll({ url: url.href, storeId: store.id });
        cookies = selectCookies(all, url, store.id);
        await this.context(record);
        if (!await this.api.permissions.contains(grant)) throw Error(__xdL.t("Permission revoked"));
      } catch {
        for (const cookie of [...all, ...cookies]) { cookie.value = ""; cookie.name = ""; }
        throw Error(__xdL.t("No supported browser login could be read. Sign in on this site and try again. Private or partitioned sessions are not supported."));
      }
      let reply;
      try {
        reply = await this.native({ action: "add_browser_login", request_id: record.requestId, url: record.url, cookies });
      } catch {
        throw Error(__xdL.t("The browser login handoff was not confirmed. Check the desktop connection, then retry this same request."));
      } finally {
        for (const cookie of [...all, ...cookies]) { cookie.value = ""; cookie.name = ""; }
        all.length = cookies.length = 0;
      }
      // A failed UI-state write must not change the durable native acknowledgement.
      await this.api.storage.session.set({ [PREFIX + id]: { ...record, jobId: reply.job_id } }).catch(() => {});
      return { ok: true, message: __xdL.t("Added with browser login"), job_id: reply.job_id };
    }
  }
  root.IDMBrowserLogin = { Manager, target, permissionOrigin, loginPermission, selectCookies };
})(globalThis);

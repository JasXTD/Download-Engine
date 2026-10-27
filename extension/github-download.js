/* GitHub's raw button otherwise creates an opaque blob download. Keep it a
   normal browser HTTP download so capture can verify and recover it as usual. */
(function (root) {
  "use strict";
  const ID = "download-engine-github-download";
  const SITE = "https://github.com/*";
  const ACCESS = "githubDownloadAccess";
  function rawUrl(value) {
    if (typeof value !== "string" || value.length > 16384) return null;
    let url;
    try { url = new URL(value); } catch { return null; }
    if (url.origin !== "https://github.com" || url.username || url.password) return null;
    const parts = url.pathname.slice(1).split("/");
    if (parts.length < 5 || parts[2] !== "blob" || parts.some(part => !part)) return null;
    parts[2] = "raw";
    url.pathname = "/" + parts.join("/");
    url.hash = "";
    return url.href;
  }
  function bind(document, location, enabled) {
    const listener = event => {
      if (!enabled() || !event.isTrusted || event.defaultPrevented || event.button !== 0 ||
          event.ctrlKey || event.shiftKey || event.altKey || event.metaKey) return;
      const button = event.target?.closest?.('button[data-testid="download-raw-button"]');
      if (!button || button.disabled) return;
      const url = rawUrl(location.href);
      if (!url) return;
      const link = document.createElement("a");
      link.href = url;
      link.download = "";
      link.hidden = true;
      document.documentElement.appendChild(link);
      event.preventDefault();
      event.stopImmediatePropagation();
      try { link.click(); } finally { link.remove(); }
    };
    document.addEventListener("click", listener, true);
    return () => document.removeEventListener("click", listener, true);
  }
  class Manager {
    constructor({ api, enabled }) { this.api = api; this.enabled = enabled; this.tail = Promise.resolve(); }
    async permitted() {
      return await this.enabled() && await this.api.permissions.contains({ origins: [SITE] });
    }
    async status(sender) {
      let origin; try { origin = new URL(sender.url).origin; } catch { return { ok: true, enabled: false }; }
      return { ok: true, enabled: sender.frameId === 0 && Number.isInteger(sender.tab?.id) &&
        !sender.tab.incognito && origin === "https://github.com" && await this.permitted() };
    }
    sync(inject = false) {
      const task = this.tail.then(() => this.syncNow(inject));
      this.tail = task.catch(() => {});
      return task;
    }
    async syncNow(inject) {
      const active = await this.permitted();
      // Existing content scripts need a revocation signal too; unregistering
      // only prevents future injection. The state request rechecks permissions.
      await this.api.storage.local.set({ [ACCESS]: active });
      const scripts = await this.api.scripting.getRegisteredContentScripts({ ids: [ID] });
      if (scripts.length) await this.api.scripting.unregisterContentScripts({ ids: [ID] });
      if (!active) return;
      await this.api.scripting.registerContentScripts([{ id: ID, matches: [SITE], js: ["github-download.js"],
        runAt: "document_start", allFrames: false, persistAcrossSessions: true }]);
      if (inject) {
        const tabs = (await this.api.tabs.query({ url: SITE })).filter(tab => !tab.incognito && Number.isInteger(tab.id));
        const results = await Promise.allSettled(tabs.map(tab => this.api.scripting.executeScript({
          target: { tabId: tab.id, frameIds: [0] }, files: ["github-download.js"]
        })));
        for (const result of results) if (result.status === "rejected") console.warn("GitHub download button could not update an open tab", result.reason);
      }
    }
  }
  root.IDMGitHubDownload = { rawUrl, bind, Manager };
  if (typeof document === "undefined" || root !== root.top || root.location.origin !== "https://github.com" || root.__downloadEngineGitHub) return;
  root.__downloadEngineGitHub = true;
  const api = typeof browser !== "undefined" ? browser : chrome;
  let active = false, revision = 0;
  async function refresh() {
    const current = ++revision;
    active = false;
    try {
      const reply = await api.runtime.sendMessage({ action: "github_download_state" });
      if (current === revision) active = reply?.ok === true && reply.enabled === true;
    } catch (error) { console.debug("GitHub download capture is unavailable", error); }
  }
  bind(document, location, () => active && !!api.runtime.id);
  api.storage.onChanged.addListener((changes, area) => {
    if (area === "local" && (changes.autoCapture || changes[ACCESS])) void refresh();
  });
  void refresh();
})(globalThis);

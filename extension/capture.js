/* Recoverable browser handoff. All browser effects are injected for contract tests. */
(function (root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const PREFIX = "capture.";
  const safeDanger = value => value === undefined || value === "safe";
  const activeCandidate = item => item?.state === "in_progress" && !item.paused &&
    !item.incognito && !item.byExtensionId && safeDanger(item.danger);
  // Firefox reports a resumable pause as interrupted. Chromium may retain its
  // paused flag after cancellation, when canResume is false and the copy is done.
  const pausedDownload = item => item?.paused === true &&
    (item.state === "in_progress" || (item.state === "interrupted" && item.canResume === true));
  function candidate(item) {
    if (!activeCandidate(item) ||
        !Number.isSafeInteger(item.totalBytes) || item.totalBytes <= 0) return null;
    const mime = (item.mime || "").split(";")[0].trim().toLowerCase();
    if (/mpegurl|dash\+xml/.test(mime)) return null;
    if (!mime.includes("/") || mime.length > 200 || ["text/html", "application/xhtml+xml"].includes(mime)) return null;
    try {
      if (/\.(m3u8|mpd)$/i.test(new URL(item.finalUrl || item.url).pathname)) return null;
      let source_page; try {if(item.referrer)source_page=IDMCore.normalizeUrl(item.referrer);}catch{}
      return {
        url: IDMCore.normalizeUrl(item.finalUrl || item.url),
        filename: (item.filename || "").split(/[\\/]/).pop() || undefined,
        size: item.totalBytes, mime, ...(source_page?{source_page}:{})
      };
    } catch { return null; }
  }
  // Capture rules choose which downloads automatic capture may take over.
  // Downloads outside them are never paused, journaled or reported.
  const RULES_KEY = "captureFilters";
  const DEFAULT_TYPES = Object.freeze(("3gp 7z aac ace aif aiff apk arj asf avi bin bz2 cab deb dmg exe flac flv gz img " +
    "iso lzh m4a m4v mkv mov mp3 mp4 mpa mpeg mpg msi msix msu ogg ogv opus pdf pkg qt r0* r1* ra rar rm rmvb rpm " +
    "sit sitx tar tgz wav webm wma wmv xz z zip zst").split(" "));
  const LIMITS = Object.freeze({ types: 200, sites: 100, input: 32768, minSizeKB: 1024 ** 3 });
  const rulesOf = (types, minSizeKB = 0, sites = []) => Object.freeze({ version: 1, types: Object.freeze(types), minSizeKB, sites: Object.freeze(sites) });
  const DEFAULT_RULES = rulesOf(DEFAULT_TYPES);
  // Before capture rules existed, automatic capture took every file type.
  const LEGACY_RULES = rulesOf(["*"]);
  const tokens = value => String(value).split(/[\s,;]+/).filter(Boolean);
  function normalizeType(token) {
    if (typeof token !== "string") return null;
    const value = token.trim().toLowerCase().replace(/^\*?\./, "").replace(/\*+/g, "*");
    return /^[a-z0-9*]{1,16}$/.test(value) ? value : null;
  }
  function normalizeSite(token) {
    if (typeof token !== "string") return null;
    let value = token.trim().toLowerCase();
    if (!value || value.length > 2048) return null;
    try {
      // Accept pasted addresses but keep only the host. Chromium escapes * in hosts.
      value = new URL(/^[a-z][a-z0-9+.-]*:\/\//.test(value) ? value : "http://" + value).hostname
        .replace(/%2a/gi, "*").replace(/\*+/g, "*").replace(/\.$/, "");
    } catch { return null; }
    return value.length <= 253 && /[a-z0-9]/.test(value) &&
      /^(?:\[[0-9a-f:.]+\]|[a-z0-9*_-]+(?:\.[a-z0-9*_-]+)*)$/.test(value) ? value : null;
  }
  function unique(values, normalize) {
    const valid = new Set(), invalid = [];
    for (const value of values) {
      const normalized = normalize(value);
      if (normalized) valid.add(normalized); else invalid.push(String(value).slice(0, 40));
    }
    return { valid: [...valid], invalid };
  }
  // Stored rules were validated when saved; damaged fields fall back individually.
  function normalizeRules(saved, legacy = false) {
    if (saved === undefined) return legacy ? LEGACY_RULES : DEFAULT_RULES;
    if (!saved || typeof saved !== "object") return DEFAULT_RULES;
    const types = Array.isArray(saved.types) ? unique(saved.types, normalizeType).valid.slice(0, LIMITS.types) : [];
    const sites = Array.isArray(saved.sites) ? unique(saved.sites, normalizeSite).valid.slice(0, LIMITS.sites) : [];
    const minSizeKB = Number.isSafeInteger(saved.minSizeKB) && saved.minSizeKB >= 0 && saved.minSizeKB <= LIMITS.minSizeKB ? saved.minSizeKB : 0;
    return rulesOf(types.length ? types : DEFAULT_TYPES, minSizeKB, sites);
  }
  // Parses popup input. Nothing is saved unless every entry is valid.
  function parseRules(input) {
    const fields = ["types", "minSize", "unit", "sites"];
    if (!input || typeof input !== "object" || fields.some(name => typeof input[name] !== "string" || input[name].length > LIMITS.input) ||
        !["KB", "MB"].includes(input.unit)) return { error: __xdL.t("Invalid capture rules.") };
    const types = unique(tokens(input.types), normalizeType);
    if (types.invalid.length) return { error: __xdL.f("Check these file types: {0}", types.invalid.slice(0, 5).join(", ")) };
    if (!types.valid.length) return { error: __xdL.t("Add at least one file type, or enter * to capture every type.") };
    if (types.valid.length > LIMITS.types) return { error: __xdL.f("Use up to {0} file types.", LIMITS.types) };
    const size = input.minSize.trim() === "" ? 0 : Number(input.minSize.trim());
    const minSizeKB = Math.round(size * (input.unit === "MB" ? 1024 : 1));
    if (!Number.isFinite(size) || size < 0 || minSizeKB > LIMITS.minSizeKB) return { error: __xdL.t("Enter a minimum size between 0 and 1 TB.") };
    const sites = unique(tokens(input.sites), normalizeSite);
    if (sites.invalid.length) return { error: __xdL.f("Check these sites: {0}", sites.invalid.slice(0, 5).join(", ")) };
    if (sites.valid.length > LIMITS.sites) return { error: __xdL.f("Use up to {0} sites.", LIMITS.sites) };
    return { rules: rulesOf(types.valid, minSizeKB, sites.valid) };
  }
  const wildcard = pattern => new RegExp("^" + pattern.split("*").map(part => part.replace(/[.^$+?()[\]{}|\\-]/g, "\\$&")).join(".*") + "$");
  function extension(item) {
    let name = (item.filename || "").split(/[\\/]/).pop();
    // Chromium may report the chosen filename after the download starts.
    if (!name) try { name = decodeURIComponent(new URL(item.finalUrl || item.url).pathname.split("/").pop()); } catch { return ""; }
    const dot = name.lastIndexOf(".");
    return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
  }
  const hostOf = url => { try { return new URL(url).hostname; } catch { return ""; } };
  function matchesRules(rules, item) {
    if (!(item.totalBytes >= rules.minSizeKB * 1024)) return false;
    const type = extension(item);
    if (!rules.types.some(pattern => pattern === "*" || (type && wildcard(pattern).test(type)))) return false;
    const hosts = [item.url, item.finalUrl, item.referrer].map(hostOf).filter(Boolean);
    return !rules.sites.some(site => hosts.some(host => site.includes("*") ? wildcard(site).test(host) : host === site || host.endsWith("." + site)));
  }
  class Manager {
    constructor({ downloads, storage, enabled, rules, native, report, uuid,
      wait = ms => new Promise(resolve => setTimeout(resolve, ms)), now = () => performance.now() }) {
      Object.assign(this, { downloads, storage, enabled, rules, native, report, uuid, wait, now });
      this.running = new Set();
    }
    async file(id) {
      const [first] = await this.downloads.search({ id });
      let item = first;
      const deadline = this.now() + 10000;
      while (activeCandidate(item)) {
        const file = candidate(item);
        if (file) return { item, file };
        // Firefox can fill totalBytes without emitting a metadata onChanged.
        // Leave the browser running while briefly rechecking missing metadata.
        const mime = (item.mime || "").split(";")[0].trim().toLowerCase();
        if ((mime && (!mime.includes("/") || mime.length > 200 ||
            ["text/html", "application/xhtml+xml"].includes(mime))) ||
            (mime && ![undefined, -1, 0].includes(item.totalBytes)) || this.now() >= deadline) return null;
        try { IDMCore.normalizeUrl(item.finalUrl || item.url); } catch { return null; }
        await this.wait(200);
        if (!await this.enabled()) return null;
        [item] = await this.downloads.search({ id });
        if (!item || item.url !== first.url || item.startTime !== first.startTime) return null;
      }
      return null;
    }
    key(id) { return PREFIX + id; }
    async read(id) { return (await this.storage.get(this.key(id)))[this.key(id)]; }
    async save(record) { await this.storage.set({ [this.key(record.id)]: record }); }
    async item(record) {
      const [item] = await this.downloads.search({ id: record.id });
      // Never act on a reused browser ID or an unrelated history entry.
      return item && item.url === record.browserUrl && item.startTime === record.startTime ? item : null;
    }
    async note(ok, message) {
      try { await this.report(ok, message); } catch (error) { console.warn(error); }
    }
    async terminal(record, phase) {
      record.phase = phase;
      const item = await this.item(record);
      if (!item || (item.state !== "in_progress" && !pausedDownload(item))) await this.storage.remove(this.key(record.id));
      else await this.save(record); // Suppress recapture of our resumed fallback.
    }
    async resume(record) {
      // After a successful fallback, later pauses belong to the user.
      if (record.browserResumed) return;
      const item = await this.item(record);
      if (pausedDownload(item)) await this.downloads.resume(record.id);
      record.browserResumed = true;
    }
    async committed(record) {
      // An acknowledgement is durable even if saving the local journal fails.
      record.phase = "committed";
      await this.save(record).catch(error => console.warn(error));
      const item = await this.item(record);
      if (pausedDownload(item) && safeDanger(item.danger)) {
        try {
          await this.downloads.cancel(record.id);
          const afterCancel = await this.item(record);
          if (afterCancel?.state === "complete" || afterCancel?.state === "in_progress" || pausedDownload(afterCancel)) {
            throw new Error(__xdL.t("The browser copy finished or continued during cancellation"));
          }
          await this.terminal(record, "captured");
          await this.note(true, __xdL.t("Download sent to Download Engine."));
          return;
        } catch {
          // The queue is committed; a browser API failure cannot undo it.
          await this.resume(record);
        }
      }
      if (!item || (item.state === "interrupted" && !pausedDownload(item))) {
        await this.terminal(record, "captured");
        await this.note(true, __xdL.t("Download sent to Download Engine."));
        return;
      }
      await this.terminal(record, "browser");
      await this.note(false, __xdL.t("Download Engine has this file in its queue, but the browser copy could not be stopped. Check both download lists."));
    }
    async release(record, reason) {
      // Abort is atomic with commit. A confirmed abort prevents late requests
      // from queueing a file after we have resumed its browser copy.
      let reply;
      try {
        reply = await this.native({ action: "capture_abort", request_id: record.request_id });
      } catch {
        record.phase = "uncertain";
        await this.save(record).catch(error => console.warn(error));
        await this.resume(record);
        await this.save(record);
        await this.note(false, __xdL.t("The app did not confirm the handoff. The browser download was kept; check Download Engine too if a copy appears there."));
        return;
      }
      if (reply.capture_state === "committed" && record.phase !== "uncertain") {
        await this.committed(record);
        return;
      }
      await this.resume(record);
      await this.terminal(record, "browser");
      await this.note(reply.capture_state !== "committed",
        reply.capture_state === "committed"
          ? __xdL.t("The browser download was kept, and Download Engine also received it. Check both download lists.")
          : reason || __xdL.t("Download kept in the browser."));
    }
    async consider(id) {
      if (this.running.has(id) || this.running.size >= 3) return;
      this.running.add(id);
      let record;
      let journalSaved = false;
      try {
        if (await this.read(id) || !await this.enabled()) return;
        // Link recovery reuses this class without rules; automatic capture never may.
        if (typeof this.rules !== "function") throw new TypeError("Capture rules are required.");
        const ready = await this.file(id);
        if (!ready || !matchesRules(await this.rules(), ready.item)) return;
        const { item, file } = ready;
        record = { id, browserUrl: item.url, startTime: item.startTime,
          request_id: "capture:" + this.uuid(), phase: "pausing" };
        await this.save(record); // No pause unless recovery state is durable.
        journalSaved = true;
        // Re-read immediately before pausing; the download may have finished.
        if (!candidate(await this.item(record))) {
          await this.terminal(record, "browser");
          return;
        }
        await this.downloads.pause(id);
        record.phase = "preparing";
        await this.save(record);
        let current = await this.item(record);
        if (!pausedDownload(current) || !await this.enabled()) {
          await this.release(record);
          return;
        }
        const reply = await this.native({ action: "capture_prepare",
          request_id: record.request_id, ...file });
        if (reply.capture_state === "committed") {
          await this.committed(record);
          return;
        }
        current = await this.item(record);
        if (!pausedDownload(current) ||
            !safeDanger(current.danger) || !await this.enabled()) {
          await this.release(record);
          return;
        }
        record.phase = "committing";
        await this.save(record);
        await this.native({ action: "capture_commit", request_id: record.request_id });
        await this.committed(record);
      } catch (error) {
        if (journalSaved) {
          try { await this.release(record, __xdL.t("Download kept in the browser: ") + error.message); }
          catch (recoveryError) {
            await this.note(false, __xdL.t("Could not resume the browser download. Use Resume in the browser Downloads list. ") + recoveryError.message);
          }
        } else {
          await this.note(false, __xdL.t("Automatic capture could not start: ") + error.message);
        }
      } finally { this.running.delete(id); }
    }
    async recover(id) {
      if (this.running.has(id)) return;
      this.running.add(id);
      try {
        const record = await this.read(id);
        if (!record) return;
        if (["browser", "captured"].includes(record.phase)) {
          await this.terminal(record, record.phase);
        } else {
          await this.release(record, __xdL.t("Interrupted handoff recovered; download kept in the browser."));
        }
      } catch (error) {
        await this.note(false, __xdL.t("A handoff needs attention. Use Resume in the browser Downloads list if it is paused. ") + error.message);
      } finally { this.running.delete(id); }
    }
    async recoverAll() {
      const values = await this.storage.get(null);
      for (const key of Object.keys(values)) {
        if (key.startsWith(PREFIX)) await this.recover(values[key].id);
      }
    }
  }
  root.IDMCapture = { Manager, candidate, rules: Object.freeze({ KEY: RULES_KEY, DEFAULT_TYPES, DEFAULTS: DEFAULT_RULES,
    LEGACY: LEGACY_RULES, LIMITS, normalize: normalizeRules, parse: parseRules, matches: matchesRules }) };
})(globalThis);


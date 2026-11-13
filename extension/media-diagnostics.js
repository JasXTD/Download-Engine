/* Bounded session-only discovery trace. Never accepts URLs or free-form errors. */
(function (root) {
  "use strict";
  const KEY = "mediaDiagnostics", LIMIT = 120;
  const EVENTS = {
    relay_ready: "Browser video connection ready",
    relay_closed: "Browser video connection closed",
    relay_failed: "Browser video fetch failed",
    relay_unavailable: "Browser video connection not available",
    ready: "Media discovery ready (Dailymotion lookup v2, browser relay v1)",
    discovery_error: "Browser discovery could not update its list",
    network_saved: "Browser stream captured", network_ignored: "Browser candidate ignored",
    metadata_start: "Dailymotion metadata lookup started", metadata_result: "Dailymotion metadata resolved",
    playlist_start: "Checking Dailymotion playlist", playlist_result: "Dailymotion playlist is readable", playlist_failed: "Dailymotion playlist rejected",
    metadata_failed: "Dailymotion metadata unavailable", metadata_fallback: "Using browser detection fallback",
    desktop_alternatives: "Same-video alternatives retained for desktop preview",
    desktop_legacy: "Older desktop receives separate playlist choices; update the desktop for automatic fallback",
    scan_result: "Video scan completed", scan_cancelled: "Video scan cancelled after navigation",
    handoff: "Video review sent to desktop", acknowledged: "Desktop accepted video review", handoff_failed: "Desktop video review failed"
  };
  const REASONS = new Set(["success", "invalid_json", "unavailable", "wrong_video", "no_formats", "http", "playlist_http", "playlist_content", "content_type", "too_large", "cancelled", "timeout", "network", "disabled", "no_request", "stale_navigation", "unsupported", "init", "preview", "no_context", "legacy_desktop", "protocol"]);
  function record(event, fields = {}, time = Date.now()) {
    if (!Object.hasOwn(EVENTS, event)) return null;
    if (!fields || typeof fields !== "object") fields = {};
    const safe = {};
    for (const key of ["tab", "count", "dom", "observed", "status"]) if (Number.isInteger(fields[key]) && fields[key] >= 0 && fields[key] <= 1000000) safe[key] = fields[key];
    for (const key of ["enabled", "allowed", "cached", "metadata", "browser_agent", "browser_relay"]) if (typeof fields[key] === "boolean") safe[key] = fields[key];
    if (REASONS.has(fields.reason)) safe.reason = fields.reason;
    if (["hls", "dash", "video", "audio"].includes(fields.format)) safe.format = fields.format;
    return { time: Number.isFinite(time) && time >= 0 && time <= 8640000000000000 ? time : Date.now(), event, fields: safe };
  }
  class Log {
    constructor(storage) { this.storage = storage; this.tail = Promise.resolve(); this.failed = false; }
    add(event, fields) {
      const entry = record(event, fields);
      if (!entry) return Promise.resolve();
      this.tail = this.tail.then(async () => {
        const saved = (await this.storage.get(KEY))[KEY];
        const entries = Array.isArray(saved) ? saved.map(row => record(row?.event, row?.fields, row?.time)).filter(Boolean).slice(1 - LIMIT) : [];
        await this.storage.set({ [KEY]: [...entries, entry] });
        this.failed = false;
      }).catch(() => { this.failed = true; });
      return this.tail;
    }
    async read() {
      await this.tail;
      try {
        const saved = (await this.storage.get(KEY))[KEY];
        const entries = (Array.isArray(saved) ? saved : []).slice(-LIMIT).map(row => record(row?.event, row?.fields, row?.time)).filter(Boolean);
        return { trace: entries.map(row => ({ time: row.time, message: EVENTS[row.event] + "; " + Object.entries(row.fields).map(([key, value]) => key + "=" + value).join(", ") })), traceError: this.failed };
      } catch { return { trace: [], traceError: true }; }
    }
    clear() {
      this.tail = this.tail.then(() => this.storage.remove(KEY)).then(() => { this.failed = false; }).catch(() => { this.failed = true; });
      return this.tail;
    }
  }
  root.IDMMediaDiagnostics = { Log };
})(globalThis);

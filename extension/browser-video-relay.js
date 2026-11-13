/* Reviewed-tab transport only. No page messages, general proxy, or cookie API. */
(function (root) {
  "use strict";
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

  // Serialized by scripting.executeScript into the isolated top-frame world.
  // It must not close over extension/background state.
  function installPage(name, page, origins) {
    const api = typeof browser !== "undefined" ? browser : chrome;
    const normalize = value => { const url = new URL(value); url.hash = ""; return url.href; };
    if (window !== window.top || normalize(location.href) !== page) return false;
    const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
    const limit = 256 * 1024 * 1024, chunkSize = 48 * 1024, active = new Map();
    // Firefox MV2's extension fetch bypasses page CORS; content.fetch preserves
    // the page's ordinary network boundary. Chromium isolated fetch does too.
    const pageFetch = typeof content !== "undefined" && typeof content.fetch === "function"
      ? content.fetch.bind(content) : fetch.bind(globalThis);
    let port = null, closed = false;
    let lifetime = setTimeout(close, 15000);
    class RelayError extends Error {
      constructor(reason) { super(reason); this.reason = reason; }
    }
    const reason = error => error instanceof RelayError ? error.reason : "network";
    function cancel(id) {
      const state = active.get(id);
      if (state) { clearTimeout(state.timer); state.controller.abort(); void state.reader?.cancel().catch(() => {}); active.delete(id); }
    }
    function close() {
      if (closed) return;
      closed = true; clearTimeout(lifetime);
      api.runtime.onConnect.removeListener(connect);
      removeEventListener("pagehide", close);
      for (const id of [...active.keys()]) cancel(id);
      try { port?.disconnect(); } catch { /* Already disconnected. */ }
    }
    function send(message) { if (!closed) port.postMessage(message); }
    function deadline(state) {
      clearTimeout(state.timer);
      state.timer = setTimeout(() => { cancel(state.id); send({ kind: "failed", id: state.id, reason: "timeout" }); }, 30000);
    }
    async function fetchVideo(message) {
      if (active.size >= 8 || active.has(message.id) || normalize(location.href) !== page) throw new RelayError("unsupported");
      const url = new URL(message.url);
      if (message.url.length > 8192 || url.username || url.password || url.hash || !origins.includes(url.origin)) throw new RelayError("unsupported");
      const headers = {};
      if (message.range !== null) {
        const range = message.range;
        if (!range || !Number.isSafeInteger(range.start) || !Number.isSafeInteger(range.length) || range.start < 0 || range.length <= 0 || range.length > limit || !Number.isSafeInteger(range.start + range.length)) throw new RelayError("protocol");
        headers.Range = `bytes=${range.start}-${range.start + range.length - 1}`;
      }
      const state = { id: message.id, controller: new AbortController(), total: 0, maximum: message.range?.length || limit, reader: null, pending: null, busy: true, timer: null };
      active.set(message.id, state); deadline(state);
      try {
        const response = await pageFetch(url.href, { method: "GET", mode: "cors", credentials: "omit", redirect: "error", cache: "no-store", referrerPolicy: "strict-origin-when-cross-origin", headers, signal: state.controller.signal });
        if (active.get(message.id) !== state || normalize(location.href) !== page) throw new RelayError("cancelled");
        if (response.redirected || response.url !== url.href) throw new RelayError("unsupported");
        if (![200, 206].includes(response.status) || !response.body) throw new RelayError("http");
        const size = response.headers.get("content-length");
        if (size !== null && (!/^\d+$/.test(size) || Number(size) > state.maximum)) throw new RelayError("too_large");
        const safe = [];
        for (const key of ["content-type", "content-range", "etag", "last-modified"]) {
          const value = response.headers.get(key);
          if (value !== null) { if (value.length > 2048) throw new RelayError("protocol"); safe.push([key, value]); }
        }
        // fetch transparently decodes content encoding. Do not forward the
        // compressed Content-Length/Encoding as if they described these bytes.
        state.reader = response.body.getReader(); state.busy = false; clearTimeout(state.timer);
        send({ kind: "head", id: message.id, status: response.status, headers: safe });
      } catch (error) {
        if (active.get(message.id) === state) { cancel(message.id); send({ kind: "failed", id: message.id, reason: reason(error) }); }
      }
    }
    async function read(message) {
      const state = active.get(message.id);
      if (!state || state.busy || !state.reader) throw new RelayError("protocol");
      state.busy = true; deadline(state);
      try {
        while (!state.pending?.length) {
          const next = await state.reader.read();
          if (active.get(message.id) !== state) return;
          if (next.done) { cancel(message.id); send({ kind: "end", id: message.id }); return; }
          // Firefox content.fetch returns a view from the page realm. An
          // instanceof check against the extension realm rejects valid bytes.
          if (!ArrayBuffer.isView(next.value) || next.value.BYTES_PER_ELEMENT !== 1) throw new RelayError("protocol");
          state.total += next.value.byteLength;
          if (state.total > state.maximum) throw new RelayError("too_large");
          // A browser read has no fixed chunk size, particularly after
          // backpressure. Retain one bounded segment view and split it into
          // IPC frames; do not reject a valid read just because it exceeds 1 MB.
          state.pending = new Uint8Array(next.value.buffer, next.value.byteOffset, next.value.byteLength);
        }
        // Do not use subarray/slice here: Firefox's cross-context views deny
        // the constructor lookup performed by typed-array species creation.
        const length = Math.min(state.pending.byteLength, chunkSize);
        const part = new Uint8Array(state.pending.buffer, state.pending.byteOffset, length);
        state.pending = new Uint8Array(state.pending.buffer, state.pending.byteOffset + length, state.pending.byteLength - length);
        const data = btoa(String.fromCharCode(...part));
        state.busy = false; clearTimeout(state.timer);
        send({ kind: "chunk", id: message.id, data });
      } catch (error) {
        if (active.get(message.id) === state) { cancel(message.id); send({ kind: "failed", id: message.id, reason: reason(error) }); }
      }
    }
    function connect(candidate) {
      if (candidate.name !== name || candidate.sender?.id !== api.runtime.id || port || closed) return;
      port = candidate; clearTimeout(lifetime); lifetime = setTimeout(close, 6 * 60 * 60 * 1000);
      port.onDisconnect.addListener(close);
      port.onMessage.addListener(message => {
        if (!message || !uuid.test(message.id || "") || !["fetch", "read", "cancel"].includes(message.kind)) { close(); return; }
        if (normalize(location.href) !== page) { close(); return; }
        if (message.kind === "cancel") { cancel(message.id); return; }
        const task = message.kind === "fetch" ? fetchVideo(message) : read(message);
        void task.catch(error => { cancel(message.id); send({ kind: "failed", id: message.id, reason: reason(error) }); });
      });
      port.postMessage({ kind: "page_ready" });
    }
    api.runtime.onConnect.addListener(connect);
    addEventListener("pagehide", close, { once: true });
    return true;
  }

  class Manager {
    constructor({ api, trace = () => {} }) { this.api = api; this.trace = trace; this.sessions = new Map(); }
    closeTab(tabId) { for (const state of [...this.sessions.values()]) if (state.tabId === tabId) state.close(); }
    closeAll() { for (const state of [...this.sessions.values()]) state.close(); }
    async open(tabId, documentId, page, items) {
      if (!Number.isInteger(tabId) || !this.api.runtime.connectNative || !this.api.tabs.connect) { this.trace("relay_unavailable", { reason: "unavailable" }); return null; }
      const origins = [...new Set(items.filter(item => item.kind !== "file").flatMap(item => [item.url, ...(item.fallback_urls || [])]).map(value => new URL(value).origin))].sort();
      if (!origins.length || origins.length > 16) { this.trace("relay_unavailable", { reason: origins.length ? "too_large" : "no_formats" }); return null; }
      const pageUrl = new URL(page);
      const local = /^http:\/\/(127\.0\.0\.1|\[::1\])(?::\d+)?$/.test(pageUrl.origin) && origins.every(origin => origin === pageUrl.origin);
      if (!local && (pageUrl.protocol !== "https:" || origins.some(origin => new URL(origin).protocol !== "https:"))) { this.trace("relay_unavailable", { reason: "unsupported" }); return null; }
      const current = await this.api.tabs.get(tabId);
      if (current.incognito || IDMCore.normalizeUrl(current.url) !== page) throw Error("The video page changed. Find videos again.");
      for (const state of this.sessions.values()) if (state.tabId === tabId && state.page === page && state.documentId === documentId && JSON.stringify(state.origins) === JSON.stringify(origins) && state.session?.expires > Date.now() / 1000 + 60) return state.session;
      if (this.sessions.size >= 4) throw Error("Close another browser video tab before opening another browser-assisted video.");
      const name = "de-video-relay-" + crypto.randomUUID();
      const injected = await this.api.scripting.executeScript({ target: { tabId, frameIds: [0] }, func: installPage, args: [name, page, origins] });
      const result = injected.find(row => row.frameId === 0);
      if (!result?.result || result.error || (documentId && result.documentId !== documentId)) throw Error("The video page changed. Find videos again.");
      const pagePort = this.api.tabs.connect(tabId, documentId ? { documentId, name } : { frameId: 0, name });
      let nativePort;
      try { nativePort = this.api.runtime.connectNative(IDMCore.HOST); }
      catch (error) { pagePort.disconnect(); throw error; }
      let readyResolve, readyReject, finished = false, pageReady = false, session = null;
      const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
      const inflight = new Set();
      const retired = new Set();
      const retire = id => { inflight.delete(id); retired.add(id); if (retired.size > 64) retired.delete(retired.values().next().value); };
      const timer = setTimeout(() => state.close(), 15000);
      const state = { tabId, documentId, page, origins, session: null, close: () => {
        if (finished) return; finished = true; clearTimeout(timer); this.sessions.delete(name);
        try { pagePort.disconnect(); } catch { /* Closed. */ }
        try { nativePort.disconnect(); } catch { /* Closed. */ }
        this.trace("relay_closed", { count: inflight.size });
        readyReject(Error("Browser video connection closed. Keep the video tab open and find videos again."));
      } };
      this.sessions.set(name, state);
      const confirm = () => { if (pageReady && session && !finished) {clearTimeout(timer); state.session = session; readyResolve(session); this.trace("relay_ready", { count: origins.length });} };
      pagePort.onDisconnect.addListener(state.close); nativePort.onDisconnect.addListener(state.close);
      pagePort.onMessage.addListener(message => {
        if (message?.kind === "page_ready") {pageReady = true; confirm(); return;}
        if (message && retired.has(message.id)) return; // A reply already in transit when Pause cancelled it.
        if (!message || !UUID.test(message.id || "") || !inflight.has(message.id) || !["head", "chunk", "end", "failed"].includes(message.kind)) {state.close(); return;}
        if (message.kind === "end" || message.kind === "failed") retire(message.id);
        if (message.kind === "failed") {
          // Only fixed reason codes cross native messaging or enter logs.
          const failure = ["network", "timeout", "too_large", "unsupported", "cancelled", "protocol", "http"].includes(message.reason) ? message.reason : "network";
          this.trace("relay_failed", { reason: failure });
          nativePort.postMessage({ kind: "failed", id: message.id, reason: failure });
          return;
        }
        nativePort.postMessage(message);
      });
      nativePort.onMessage.addListener(message => {
        if (message?.browser_relay) {
          const value = message.browser_relay;
          if (session || message.ok !== true || !UUID.test(value.token || "") || !Number.isInteger(value.port) || value.port <= 0 || value.port > 65535 || value.page !== page || JSON.stringify(value.origins) !== JSON.stringify(origins) || !Number.isInteger(value.expires) || value.expires <= Date.now() / 1000 || value.expires > Date.now() / 1000 + 21660) {state.close(); return;}
          session = value; confirm(); return;
        }
        if (!session || !message || !UUID.test(message.id || "") || !["fetch", "read", "cancel"].includes(message.kind)) {state.close(); return;}
        if (message.kind === "fetch") {
          if (inflight.has(message.id) || inflight.size >= 8) {state.close(); return;}
          inflight.add(message.id);
        } else if (message.kind === "cancel") retire(message.id);
        else if (!inflight.has(message.id)) {state.close(); return;}
        pagePort.postMessage(message);
      });
      nativePort.postMessage({ action: "open_browser_video_relay", page, origins });
      return ready;
    }
  }
  root.IDMBrowserRelay = { Manager, installPage };
})(globalThis);

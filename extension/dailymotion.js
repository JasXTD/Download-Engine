/* Public, click-requested Dailymotion metadata. No cookies or page scripts. */
(function (root) {
  "use strict";
  const LIMIT = 1024 * 1024;
  function hlsDocument(text) {
    const lines = text.split(/\r?\n/);
    if (lines[0]?.trim() !== "#EXTM3U") return false;
    let needsUri = false;
    for (const value of lines.slice(1)) {
      const line = value.trim();
      if (/^#EXT(?:-X-STREAM-INF|INF):/.test(line)) needsUri = true;
      else if (needsUri && line && !line.startsWith("#")) return true;
    }
    return false;
  }
  async function readText(response, limit) {
    if (Number(response.headers.get("content-length")) > limit) throw { reason: "too_large" };
    if (!response.body?.getReader) throw { reason: "content_type" };
    const reader = response.body.getReader(), decoder = new TextDecoder("utf-8", { fatal: true });
    let bytes = 0, text = "";
    try {
      for (;;) {
        const chunk = await reader.read();
        if (chunk.done) return text + decoder.decode();
        bytes += chunk.value.byteLength;
        if (bytes > limit) throw { reason: "too_large" };
        text += decoder.decode(chunk.value, { stream: true });
      }
    } finally { await reader.cancel().catch(() => {}); }
  }
  function videoId(value) {
    try {
      const url = new URL(value);
      if (url.protocol !== "https:" || url.username || url.password || url.port ||
          !["www.dailymotion.com", "dailymotion.com"].includes(url.hostname)) return null;
      return /^\/(?:embed\/)?video\/(x[a-z0-9]{1,30})(?:_[^/]*)?\/?$/i.exec(url.pathname)?.[1] || null;
    } catch { return null; }
  }
  function stream(value, type) {
    if (typeof value !== "string" || value.length > 4096) return null;
    try {
      const url = new URL(value);
      // Metadata is untrusted. Only return HTTPS URLs on Dailymotion's own
      // domains; the desktop independently validates every redirect and fetch.
      if (url.protocol !== "https:" || url.username || url.password || url.port ||
          !/(^|\.)(dailymotion\.com|dmcdn\.net)$/.test(url.hostname)) return null;
      if (/\/init\.mp4$|\.(?:m4s|ts|pmp4)$/i.test(url.pathname) || /\/preview\//i.test(url.pathname)) return null;
      const mime = String(type || "").split(";")[0].trim().toLowerCase();
      const hls = /\.m3u8$/i.test(url.pathname) || /^(?:application|audio)\/(?:vnd\.apple\.mpegurl|x-mpegurl|mpegurl)$/.test(mime);
      const dash = /\.mpd$/i.test(url.pathname) || mime === "application/dash+xml";
      if (!hls && !dash && !/\.(?:mp4|webm)$/i.test(url.pathname)) return null;
      url.hash = "";
      return { url: url.href, hls, dash, mime };
    } catch { return null; }
  }
  function parse(value, id) {
    if (!value || typeof value !== "object" || Array.isArray(value)) return { links: [], reason: "invalid_json" };
    if (value.error) return { links: [], reason: "unavailable" };
    if (value.id != null && String(value.id) !== id) return { links: [], reason: "wrong_video" };
    const rows = [], seen = new Set();
    function add(raw, quality) {
      const item = stream(raw?.url || raw?.src || raw?.secure_url, raw?.type);
      if (!item || seen.has(item.url) || rows.length >= 40) return;
      seen.add(item.url);
      const label = /^\d{2,4}$/.test(quality) ? quality + "p" : "auto";
      rows.push({ ...item, filename: id + "-" + label + (item.hls ? ".m3u8" : item.dash ? ".mpd" : item.mime === "video/webm" || /\.webm$/i.test(new URL(item.url).pathname) ? ".webm" : ".mp4"),
        text: "Dailymotion · " + (label === "auto" ? "Video" : label), source: "network", quality });
    }
    if (value.qualities && typeof value.qualities === "object" && !Array.isArray(value.qualities)) {
      for (const [quality, items] of Object.entries(value.qualities).slice(0, 40)) {
        if (Array.isArray(items)) for (const item of items.slice(0, 40)) add(item, quality);
      }
    }
    if (!rows.length) add({ url: value.manifest_url || value.streaming_url }, "auto");
    // An advertised automatic HLS master is the quality chooser's entry point.
    // Do not show its individual renditions again as separate videos.
    const master = rows.find(row => row.quality === "auto" && row.hls);
    return { links: (master ? [master] : rows).map(({ quality, ...row }) => row), reason: rows.length ? "success" : "no_formats" };
  }
  function matchesVideo(page, value) {
    const id = videoId(page), item = stream(value, "");
    if (!id || !item?.hls || item.dash) return false;
    const url = new URL(item.url);
    if (["dmxleo.dailymotion.com", "cdndirector.dailymotion.com"].includes(url.hostname))
      return url.pathname === "/cdn/manifest/video/" + id + ".m3u8";
    // These CDN rendition paths identify the asset numerically; the public xID
    // is the same number in base 36. Unknown path layouts stay ungrouped.
    const asset = Number.parseInt(id.slice(1), 36);
    const path = /\/(?:video|cloud)\/fmp4\/(\d+)\/[a-zA-Z0-9_-]+\/\d+\/manifest\.m3u8$/.exec(url.pathname);
    return /(^|\.)dmcdn\.net$/.test(url.hostname) && Number.isSafeInteger(asset) && asset > 0 && path?.[1] === String(asset);
  }
  function groupStreams(page, resolved, observed) {
    const byUrl = new Map();
    for (const row of [...resolved, ...observed]) if (!byUrl.has(row.url)) byUrl.set(row.url, row);
    const links = [...byUrl.values()];
    if (resolved.length > 1) return links;
    const matching = observed.filter(row => row.hls && !row.dash && matchesVideo(page, row.url));
    if (!matching.length) return links;
    const primary = links.find(row => row.hls && !row.dash && matchesVideo(page, row.url));
    if (!primary) return links;
    // Prefer a captured CDN rendition over another player redirect if the
    // metadata entry fails in the desktop. Browser HTTP 200 alone cannot prove
    // the native client can use that entry. Never group ads or another video.
    matching.sort((a, b) => Number(new URL(a.url).hostname.endsWith("dailymotion.com")) - Number(new URL(b.url).hostname.endsWith("dailymotion.com")));
    const seen = new Set([primary.url]), alternatives = [];
    for (const row of matching) if (!seen.has(row.url)) {
      seen.add(row.url);
      if (alternatives.length < 4) alternatives.push(row.url);
    }
    const grouped = { ...primary, ...(alternatives.length ? { fallback_urls: alternatives } : {}) };
    const represented = new Set(alternatives);
    // A metadata success may occur during an ad. It does not prove another
    // captured playlist is irrelevant. Keep unknown layouts/other IDs and any
    // candidates beyond the fallback cap available as explicit review choices.
    return links.flatMap(row => row === primary ? [grouped] : represented.has(row.url) ? [] : [row]);
  }
  class Resolver {
    constructor({ fetch: request = (...args) => root.fetch(...args), timeout = 8000, trace = () => {} } = {}) {
      Object.assign(this, { request, timeout, trace });
    }
    async lookup(tab, signal) {
      const id = videoId(tab?.url);
      if (!id || tab.incognito) return null;
      const abort = new AbortController();
      let timedOut = false, status;
      const cancel = () => abort.abort();
      signal?.addEventListener("abort", cancel, { once: true });
      if (signal?.aborted) cancel();
      const timer = setTimeout(() => { timedOut = true; abort.abort(); }, this.timeout);
      this.trace("metadata_start", { tab: tab.id });
      try {
        const response = await this.request("https://www.dailymotion.com/player/metadata/video/" + id, {
          method: "GET", credentials: "omit", redirect: "error", cache: "no-store",
          referrerPolicy: "no-referrer", headers: { Accept: "application/json" }, signal: abort.signal
        });
        status = response.status;
        if (!response.ok || response.redirected) throw { reason: "http" };
        if (!/^application\/json(?:\s*;|$)/i.test(response.headers.get("content-type") || "")) throw { reason: "content_type" };
        const body = await readText(response, LIMIT);
        let metadata;
        try { metadata = JSON.parse(body); } catch { throw { reason: "invalid_json" }; }
        if (abort.signal.aborted) throw { reason: "cancelled" };
        const result = parse(metadata, id);
        if (!result.links.length) {
          this.trace("metadata_failed", { tab: tab.id, count: 0, reason: result.reason, status });
          return result;
        }
        const links = [];
        let lastFailure = { reason: "no_formats" };
        // Validate playlists, not media segments. The CSP also restricts every
        // redirect to Dailymotion/CDN HTTPS hosts. A successful metadata response
        // alone must not hide a working browser-captured stream.
        for (const item of result.links.slice(0, 4)) {
          try {
            this.trace("playlist_start", { tab: tab.id });
            const playlist = await this.request(item.url, { method: item.hls || item.dash ? "GET" : "HEAD",
              credentials: "omit", redirect: "follow", cache: "no-store", referrerPolicy: "no-referrer", signal: abort.signal });
            status = playlist.status;
            const final = stream(playlist.url || item.url, item.mime);
            if (!playlist.ok || !final) { await playlist.body?.cancel().catch(() => {}); throw { reason: "playlist_http" }; }
            if (item.hls || item.dash) {
              const document = (await readText(playlist, LIMIT * 2)).trimStart();
              // The document wins over a misleading MIME type or URL suffix.
              item.hls = hlsDocument(document);
              item.dash = !item.hls && /^(?:(?:<\?xml[^>]*\?>|<!--[\s\S]*?-->)\s*)*<(?:[\w.-]+:)?MPD(?:\s|>)/.test(document);
              if (!item.hls && !item.dash) throw { reason: "playlist_content" };
              item.filename = item.filename.replace(/\.(m3u8|mpd)$/, item.hls ? ".m3u8" : ".mpd");
            } else if (!/^video\//i.test(playlist.headers.get("content-type") || "")) throw { reason: "playlist_content" };
            links.push({ ...item, url: final.url });
            this.trace("playlist_result", { tab: tab.id, status });
          } catch (error) {
            if (abort.signal.aborted) throw error;
            lastFailure = error;
            this.trace("playlist_failed", { tab: tab.id, status,
              reason: ["playlist_http", "playlist_content", "too_large"].includes(error?.reason) ? error.reason : "network" });
          }
        }
        if (!links.length) throw lastFailure;
        this.trace("metadata_result", { tab: tab.id, count: links.length, reason: "success" });
        return { links, reason: "success" };
      } catch (error) {
        // Browser errors can contain signed URLs. Only allow fixed reason codes
        // into diagnostics; do not persist the exception or response body.
        const reason = signal?.aborted ? "cancelled" : timedOut ? "timeout" :
          ["http", "content_type", "too_large", "invalid_json", "cancelled", "playlist_http", "playlist_content"].includes(error?.reason) ? error.reason : "network";
        this.trace("metadata_failed", { tab: tab.id, reason, status });
        return { links: [], reason };
      } finally {
        clearTimeout(timer);
        signal?.removeEventListener("abort", cancel);
        abort.abort();
      }
    }
  }
  root.IDMDailymotion = { Resolver, videoId, parse, matchesVideo, groupStreams };
})(globalThis);

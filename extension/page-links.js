/* Shared page-link validation and session-owned batch handoffs. */
(function (root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const PREFIX = "page-links:";
  const VIDEO_PENDING = "pending-video-review";
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  const DESTINATIONS = ["automatic", "downloads", "documents", "programs", "compressed", "music", "video", "other"];
  function text(value, limit = 200) {
    return Array.from(String(value || "").toWellFormed().replace(/[\u0000-\u001f\u007f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/g, " ").replace(/\s+/g, " ").trim()).slice(0, limit).join("");
  }
  function filename(value) {
    let name = text(value, 160).replace(/[<>:"/\\|?*]/g, "_").replace(/[. ]+$/, "");
    if (!name) name = "download";
    if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(name)) name = "_" + name;
    return name;
  }
  // This function is copied into the isolated top frame. Keep every dependency
  // inside it: no page scripts, network requests, messages, cookies, or frames.
  function collect(mode = "links") {
    const links = [], seen = new Set();
    let bytes = 0, truncated = false, examined = 0;
    const videos = mode === "videos";
    const direct = /\.(mp4|webm|m4v|mov|ogv|avi|mkv|mpg|mpeg|3gp)$/i;
    const streaming = /\.(m3u8?|mpd|ism|ismc)(?:\/|$)/i;
    const mimeExtension = { "video/mp4": "mp4", "video/webm": "webm", "video/ogg": "ogv", "video/quicktime": "mov" };
    function add(raw, label, name, mime = "", media = false, source = "link", size = null) {
      if (++examined > 10000 || links.length >= 1000) { truncated = true; return false; }
      if (!raw || raw.trim().startsWith("#")) return true;
      let url;
      try { url = new URL(raw, document.baseURI); } catch { return true; }
      if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.href.length > 16384) return true;
      url.hash = "";
      const type = mime.split(";")[0].trim().toLowerCase();
      // Manifests only open the desktop preview; never queue them as files.
      const hls = /\.m3u8$/i.test(url.pathname) || (media && /^(application\/(vnd\.apple\.mpegurl|x-mpegurl|mpegurl)|audio\/(x-)?mpegurl)$/i.test(type));
      const dash = /\.mpd$/i.test(url.pathname) || (media && type === "application/dash+xml");
      if (videos && !hls && !dash && (streaming.test(url.pathname) || /mpegurl|dash\+xml/i.test(type) ||
          (!direct.test(url.pathname) && !(media && mimeExtension[type])))) return true;
      if (seen.has(url.href)) return true;
      seen.add(url.href);
      if (videos && !hls && !dash && !direct.test(url.pathname) && media && mimeExtension[type]) name = "video." + mimeExtension[type];
      const row = { url: url.href, text: (label || "").slice(0, 200), filename: (name || "").slice(0, 160), source };
      if (/^(video|audio)\//.test(type)) row.mime = type.slice(0, 100);
      if (Number.isSafeInteger(size) && size > 0 && !hls && !dash) row.size = size;
      if (hls) row.hls = true;
      if (dash) row.dash = true;
      const rowBytes = new TextEncoder().encode(JSON.stringify(row)).length;
      if (bytes + rowBytes > 700000) { truncated = true; return false; }
      bytes += rowBytes; links.push(row); return true;
    }
    // Resource timing exposes manifest URLs used by many blob/MSE players.
    // Read only this document's entries; never hook page code or request headers.
    if (videos && document.querySelectorAll("video").length && !Array.from(document.querySelectorAll("video")).every(v => v.mediaKeys)) {
      const resources = globalThis.performance?.getEntriesByType?.("resource") || [];
      for (const entry of resources.slice(-5000)) {
        let path; try { path = new URL(entry.name).pathname; } catch { continue; }
        if (/\.(m3u8|mpd)$/i.test(path) && !add(entry.name, "Video stream", "", "", true)) break;
      }
    }
    if (videos) {
      const elements = document.querySelectorAll("video");
      for (let i = 0; i < elements.length && !truncated; i++) {
        if (++examined > 10000) { truncated = true; break; }
        const video = elements[i];
        if (video.mediaKeys) continue;
        const label = video.getAttribute("title") || video.getAttribute("aria-label") || "Video file";
        if (!add(video.currentSrc, label, "", "", true) || !add(video.getAttribute("src"), label, "", video.getAttribute("type") || "", true)) break;
        const sources = video.querySelectorAll("source[src]");
        for (let n = 0; n < sources.length; n++) {
          const source = sources[n];
          if (!add(source.getAttribute("src"), label, "", source.getAttribute("type") || "", true)) break;
        }
      }
    }
    const nodes = document.links;
    for (let i = 0; i < nodes.length && !truncated; i++) {
      const node = nodes[i];
      if (!add(node.getAttribute("href"), node.textContent || node.getAttribute("alt") || "", node.getAttribute("download") || "")) break;
    }
    // Read existing page assets only. This never fetches images or follows frames.
    // Walking rather than querying every element keeps very large pages bounded.
    if (!videos && document.createTreeWalker && !truncated) {
      const roots = [document]; let visited = 0, styled = 0;
      const srcset = (value, label) => {
        if (!value || /data:/i.test(value)) return;
        for (const candidate of value.split(",")) {
          if (!add(candidate.trim().split(/\s+/)[0], label, "", "", false, "asset")) break;
        }
      };
      while (roots.length && !truncated) {
        const walker = document.createTreeWalker(roots.shift(), 1);
        let el;
        while ((el = walker.nextNode()) && !truncated) {
          if (++visited > 10000) { truncated = true; break; }
          if (el.shadowRoot && roots.length < 100) roots.push(el.shadowRoot);
          const tag = el.localName, attr = name => el.getAttribute(name) || "";
          const label = attr("alt") || attr("title") || attr("aria-label");
          if (tag === "a" || tag === "area") add(attr("href"), el.textContent, attr("download"));
          if (["img", "source", "audio", "video", "track"].includes(tag) && !el.mediaKeys) {
            if (el.currentSrc) add(el.currentSrc, label, "", attr("type"), ["audio", "video", "source"].includes(tag), "asset");
            add(attr("src"), label, "", attr("type"), ["audio", "video", "source"].includes(tag), "asset");
            srcset(attr("srcset"), label);
            if (tag === "video") add(attr("poster"), label, "", "", false, "asset");
          }
          if (["img", "source"].includes(tag)) {
            for (const name of ["data-src", "data-original", "data-lazy-src"]) add(attr(name), label, "", "", false, "asset");
            srcset(attr("data-srcset"), label);
          }
          if (typeof getComputedStyle === "function" && styled++ < 500) {
            const background = getComputedStyle(el).backgroundImage;
            for (const match of background.matchAll(/url\(\s*["']?([^"')]+)["']?\s*\)/g)) add(match[1], label, "", "", false, "asset");
          }
        }
      }
    }
    return { url: location.href, title: document.title.slice(0, 200), links, truncated, mode: videos ? "videos" : "links" };
  }
  function normalizeScan(value) {
    if (!value || !Array.isArray(value.links) || value.links.length > 1000) throw Error(__xdL.t("The page returned an invalid link list."));
    const seen = new Set(), links = [];
    for (const row of value.links) {
      const url = IDMCore.normalizeUrl(row.url);
      if (seen.has(url)) {
        const existing = links.find(link => link.url === url);
        if (Number.isSafeInteger(row.size) && row.size > 0) existing.size = row.size;
        if (row.hls === true) existing.hls = true;
        if (row.dash === true) existing.dash = true;
        if (/^(audio|video)\/[a-z0-9.+-]+$/.test(row.mime || "")) existing.mime = row.mime.slice(0, 100);
        continue;
      }
      seen.add(url);
      let inferred = new URL(url).pathname.split("/").pop() || "download";
      try { inferred = decodeURIComponent(inferred); } catch { /* Use the literal path if percent escapes are malformed. */ }
      const link = { url, filename: filename(row.filename || inferred), text: text(row.text) };
      link.source = ["asset", "network"].includes(row.source) ? row.source : "link";
      if (Number.isSafeInteger(row.size) && row.size > 0) link.size = row.size;
      if (/^(audio|video)\/[a-z0-9.+-]+$/.test(row.mime || "")) link.mime = row.mime.slice(0, 100);
      if (row.hls === true || /\.m3u8$/i.test(new URL(url).pathname)) link.hls = true;
      if (row.dash === true || /\.mpd$/i.test(new URL(url).pathname)) link.dash = true;
      links.push(link);
    }
    const scan = { url: IDMCore.normalizeUrl(value.url), title: text(value.title), links, truncated: value.truncated === true, mode: value.mode === "videos" ? "videos" : "links" };
    if (new TextEncoder().encode(JSON.stringify(scan)).length > 850000) throw Error(__xdL.t("This page contains too much link data. Try a smaller page."));
    return scan;
  }
  function category(name) {
    const dot = name.lastIndexOf(".");
    const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
    if (/^(pdf|docx?|odt|rtf|txt|md|csv|tsv|xlsx?|ods|pptx?|odp|epub|mobi)$/.test(ext)) return "documents";
    if (/^(exe|msi|msix|msixbundle|appx|appxbundle|msp|apk|dmg|pkg|deb|rpm|appimage|bat|cmd|ps1|com|scr)$/.test(ext)) return "programs";
    if (/^(zip|7z|rar|tar|gz|tgz|bz2|tbz2|xz|txz|zst|cab)$/.test(ext)) return "compressed";
    if (/^(mp3|flac|wav|m4a|aac|ogg|oga|opus|wma|aiff|aif|alac|mid|midi)$/.test(ext)) return "music";
    if (/^(mp4|m4v|mkv|webm|ogv|avi|mov|wmv|flv|mpg|mpeg|3gp|m2ts|vob)$/.test(ext)) return "video";
    if (/^(png|jpe?g|gif|webp|avif|bmp|svg|tiff?|ico|heic)$/.test(ext)) return "images";
    return "other";
  }
  function linkCategory(link) { return link.hls || link.dash || /^video\//.test(link.mime || "") ? "video" : /^audio\//.test(link.mime || "") ? "music" : category(link.filename); }
  function visibleLinks(links, search, type, source = "all", minSize = 0, sort = "page") {
    const needle = search.trim().toLowerCase();
    const result = links.map((link, index) => ({ ...link, index })).filter(link =>
      (type === "all" || linkCategory(link) === type) && (source === "all" || (link.source || "link") === source) &&
      (!minSize || link.size >= minSize) && [link.filename, link.text, link.url].some(value => value.toLowerCase().includes(needle)));
    if (sort === "name") result.sort((a, b) => a.filename.localeCompare(b.filename) || a.index - b.index);
    if (sort === "size") result.sort((a, b) => (b.size || 0) - (a.size || 0) || a.index - b.index);
    if (sort === "type") result.sort((a, b) => linkCategory(a).localeCompare(linkCategory(b)) || a.index - b.index);
    return result;
  }
  function selection(scan, indices, queue, destination) {
    if (!Array.isArray(indices) || !indices.length || indices.length > 1000 ||
        indices.some(index => !Number.isInteger(index) || index < 0 || index >= scan.links.length) || new Set(indices).size !== indices.length) throw Error(__xdL.t("Select between 1 and 1,000 listed links."));
    if (typeof queue !== "string" || !queue || queue.length > 64 || !DESTINATIONS.includes(destination)) throw Error(__xdL.t("Choose a saved queue and destination folder."));
    if (indices.some(index => scan.links[index].hls || scan.links[index].dash)) throw Error(__xdL.t("Use Preview HLS or Preview DASH to choose a quality before adding a stream."));
    return { indices: [...indices].sort((a, b) => a - b), queue_id: queue, destination };
  }
  class Manager {
    constructor({ api, native, uuid = () => crypto.randomUUID(), detected = async () => [] }) {
      this.api = api; this.native = native; this.uuid = uuid; this.detected = detected;
      this.inflight = new Set(); this.scanning = false;
    }
    key(id) {
      if (typeof id !== "string" || !UUID.test(id)) throw Error(__xdL.t("Invalid link picker identifier."));
      return PREFIX + id;
    }
    async read(id) {
      const key = this.key(id), record = (await this.api.storage.session.get(key))[key];
      if (!record) throw Error(__xdL.t("This link picker has expired. Collect the page links again from the extension."));
      return record;
    }
    async save(id, record) { await this.api.storage.session.set({ [this.key(id)]: record }); }
    async open(id, record) {
      // Bind the tab before its scripts can send anything back. Writing the old
      // record after loading links.html could overwrite an in-flight receipt.
      const tab = await this.api.tabs.create({ url: "about:blank", active: false });
      record.picker = tab.id;
      await this.save(id, record);
      await this.api.tabs.update(tab.id, { url: this.api.runtime.getURL("links.html") + "#" + id, active: true });
    }
    async scan(tab, mode = "links", expectedDocumentId = null) {
      if (!["links", "videos"].includes(mode)) throw Error(__xdL.t("Invalid page scan mode."));
      if (this.scanning) throw Error(__xdL.t("A page scan is already in progress."));
      this.scanning = true;
      try {
        tab ||= (await this.api.tabs.query({ active: true, currentWindow: true }))[0];
        if (!tab || !Number.isInteger(tab.id) || !/^https?:\/\//.test(tab.url || "")) throw Error(__xdL.t("Open an HTTP or HTTPS web page, then click the extension on that page."));
        if (tab.incognito) throw Error(__xdL.t("Page-link collection is not available in private windows."));
        const records = await this.api.storage.session.get(null);
        if (mode === "links" && Object.keys(records).filter(key => key.startsWith(PREFIX)).length >= 5) throw Error(__xdL.t("Close another link picker first. Use Recover handoffs for any pending batch."));
        const results = await this.api.scripting.executeScript({ target: { tabId: tab.id, frameIds: [0] }, func: collect, args: [mode] });
        const result = results.find(item => item.frameId === 0);
        // Scan the currently active top frame, then bind its result to the click.
        // A same-document URL change is fine; a replacement document is not.
        if (expectedDocumentId && result?.documentId !== expectedDocumentId) throw Error(__xdL.t("The page changed. Click its video button again."));
        if (!result?.result || result.error) throw Error(__xdL.t("The browser could not read this page. Try an ordinary web page."));
        const scan = normalizeScan(result.result);
        if (mode === "videos" && scan.url === IDMCore.normalizeUrl(tab.url)) {
          const observed = await this.detected(tab);
          scan.links = normalizeScan({ ...scan, links: [...scan.links, ...observed].slice(0, 1000) }).links;
        }
        // If a navigation raced the scan, show what was actually scanned rather
        // than silently attributing another document's links to the first page.
        return await this.openScan(scan);
      } finally { this.scanning = false; }
    }
    async openScan(value) {
      if (this.opening) throw Error(__xdL.t("A review is already opening."));
      this.opening = true;
      try {
        const scan = normalizeScan(value);
        if (scan.mode === "videos") return await this.openVideoReview(scan);
        const records = await this.api.storage.session.get(null);
        if (Object.keys(records).filter(key => key.startsWith(PREFIX)).length >= 5) throw Error(__xdL.t("Close another link picker first. Use Recover handoffs for any pending batch."));
        const id = this.uuid(), record = { scan, pending: null, receipt: null };
        await this.save(id, record);
        try { await this.open(id, record); }
        catch (error) { await this.api.storage.session.remove(this.key(id)); throw error; }
        return { ok: true, message: __xdL.f("Found {0} {1}. Review them in the new tab.", scan.links.length, scan.mode === "videos" ? __xdL.t("media items") : __xdL.t("files and links")) };
      } finally { this.opening = false; }
    }
    async openVideoReview(scan) {
      if ((await this.api.storage.session.get(VIDEO_PENDING))[VIDEO_PENDING]) throw Error(__xdL.t("A video review is awaiting confirmation. Use Browser options → Recover handoffs first."));
      const request = {
        action: "review_browser_video", request_id: "video:" + this.uuid(),
        review: { page_url: scan.url, page_title: scan.title, truncated: scan.truncated,
          items: scan.links.map(link => ({ url: link.url, filename: link.filename, label: link.text,
            kind: link.dash ? "dash" : link.hls ? "hls" : "file", size: link.size ?? null })) }
      };
      if (new TextEncoder().encode(JSON.stringify(request)).length > 900000) throw Error(__xdL.t("This page contains too much video data. Try a smaller page."));
      // Save before sending: a lost reply must reuse the same desktop receipt.
      await this.api.storage.session.set({[VIDEO_PENDING]:request});
      return this.sendVideoReview(request);
    }
    async sendVideoReview(request) {
      if (this.inflight.has(VIDEO_PENDING)) throw Error(__xdL.t("A video review is already opening in Download Engine."));
      this.inflight.add(VIDEO_PENDING);
      try {
        const reply = await this.native(request);
        if (reply.preview_id !== request.request_id) throw Error(__xdL.t("The desktop app did not confirm the video review. Update Download Engine, then use Recover handoffs."));
        await this.api.storage.session.remove(VIDEO_PENDING);
        return {ok:true,message:reply.message || __xdL.t("Review the video in Download Engine. Nothing has been downloaded.")};
      } catch (error) {
        if (error.definitive) await this.api.storage.session.remove(VIDEO_PENDING);
        throw error;
      } finally { this.inflight.delete(VIDEO_PENDING); }
    }
    async get(id) {
      const record = await this.read(id);
      let options = null, optionsError = null;
      if (!record.receipt && !record.pending) {
        try { options = (await this.native({ action: "batch_options" })).batch_options; }
        catch (error) { optionsError = error.message + " Open Browser integration in Download Engine or update the desktop app, then reload destinations."; }
      }
      return { ok: true, scan: record.scan, pending: record.pending?.selection || null, receipt: record.receipt, options, optionsError };
    }
    async previewHls(id, index) {
      if (this.inflight.has(id)) throw Error(__xdL.t("A request from this picker is already being sent."));
      this.inflight.add(id);
      try {
        const record = await this.read(id);
        if (record.pending) throw Error(__xdL.t("Resolve the pending batch before previewing a video."));
        if (!Number.isInteger(index) || index < 0 || !(record.scan.links[index]?.hls || record.scan.links[index]?.dash)) throw Error(__xdL.t("Choose a listed HLS or DASH video."));
        record.hlsRequests ||= {};
        record.hlsRequests[index] ||= { action: record.scan.links[index].dash ? "preview_video" : "preview_hls", request_id: "hls:" + this.uuid(), url: record.scan.links[index].url };
        await this.save(id,record);
        const reply = await this.native(record.hlsRequests[index]);
        delete record.hlsRequests[index];
        await this.save(id,record);
        return reply;
      } finally { this.inflight.delete(id); }
    }
    async submit(id, input) {
      if (this.inflight.has(id)) throw Error(__xdL.t("This batch is already being sent."));
      this.inflight.add(id);
      try {
        const record = await this.read(id);
        if (record.receipt) return { ok: true, ...record.receipt };
        if (record.pending) {
          if (input.retry !== true) throw Error(__xdL.t("Retry the pending batch before changing its selection."));
        } else {
          if (input.retry) throw Error(__xdL.t("There is no pending batch to retry."));
          const chosen = selection(record.scan, input.indices, input.queue_id, input.destination);
          const links = chosen.indices.map(index => ({ url: record.scan.links[index].url, filename: record.scan.links[index].filename }));
          const request = { action: "add_batch", request_id: "batch:" + this.uuid(), links, queue_id: chosen.queue_id, destination: chosen.destination };
          if (new TextEncoder().encode(JSON.stringify(request)).length > 900000) throw Error(__xdL.t("Select fewer links for this batch."));
          record.pending = { request, selection: chosen };
          // Persist before sending: a popup/tab close or worker restart must not
          // generate a new identifier for a request that might already commit.
          await this.save(id, record);
        }
        try {
          const reply = await this.native(record.pending.request);
          record.receipt = { count: reply.job_ids.length, message: reply.message };
          record.pending = null;
          await this.save(id, record);
          return { ok: true, ...record.receipt };
        } catch (error) {
          if (error.definitive) { record.pending = null; await this.save(id, record); }
          throw error;
        }
      } finally {
        this.inflight.delete(id);
        // A close event during a native request cannot erase its pending ID.
        // Release completed/unsent records once that request has settled.
        try {
          const record = await this.read(id);
          if (!record.pending && Number.isInteger(record.picker)) {
            let tab = null;
            try { tab = await this.api.tabs.get(record.picker); } catch { /* Closed. */ }
            if (!tab) await this.closed(record.picker);
          }
        } catch { /* Session storage may be unavailable; never hide the reply. */ }
      }
    }
    async closed(tabId) {
      const records = await this.api.storage.session.get(null);
      for (const [key, record] of Object.entries(records)) {
        if (key.startsWith(PREFIX) && record.picker === tabId && !record.pending && !this.inflight.has(key.slice(PREFIX.length))) await this.api.storage.session.remove(key);
      }
    }
    async recover() {
      const records = await this.api.storage.session.get(null);
      let count = 0;
      if (records[VIDEO_PENDING]) { await this.sendVideoReview(records[VIDEO_PENDING]); count++; }
      for (const [key, record] of Object.entries(records)) {
        if (!key.startsWith(PREFIX) || !record.pending) continue;
        let existing = null;
        if (Number.isInteger(record.picker)) {
          try { existing = await this.api.tabs.get(record.picker); } catch { /* Closed picker: open it again below. */ }
        }
        const url = this.api.runtime.getURL("links.html") + "#" + key.slice(PREFIX.length);
        if (existing?.url === url) await this.api.tabs.update(existing.id, { active: true });
        else {
          const id = key.slice(PREFIX.length);
          if (this.inflight.has(id)) continue;
          this.inflight.add(id);
          try { await this.open(id, await this.read(id)); }
          finally { this.inflight.delete(id); }
        }
        count++;
      }
      return count;
    }
  }
  root.IDMPageLinks = { collect, normalizeScan, category, linkCategory, visibleLinks, selection, Manager };
})(globalThis);

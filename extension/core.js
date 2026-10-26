/* Shared by the browser background and its contract tests. */
(function (root) {
  "use strict";
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
  const HOST = "com.xetro.downloadengine";
  function normalizeUrl(value) {
    if (typeof value !== "string" || value.length > 16384) throw new Error(__xdL.t("Enter a complete HTTP or HTTPS file URL."));
    let url;
    try { url = new URL(value.trim()); } catch { throw new Error(__xdL.t("Enter a complete HTTP or HTTPS file URL.")); }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) {
      throw new Error(__xdL.t("Only HTTP/HTTPS links without embedded login details are supported."));
    }
    url.hash = "";
    return url.href;
  }
  function makeRequest(url, requestId, sourcePage) {
    if (typeof requestId !== "string" || !requestId || requestId.length > 128) throw new Error(__xdL.t("Invalid request identifier."));
    const normalized = normalizeUrl(url);
    const parsed = new URL(normalized);
    if (["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be", "www.youtu.be", "youtube-nocookie.com", "www.youtube-nocookie.com"].includes(parsed.hostname)) {
      return { action: "preview_video", request_id: "hls:" + requestId, url: normalized };
    }
    // Bilibili video and Bangumi episode pages open the desktop's quality choice instead of queuing the page.
    if (["bilibili.com", "www.bilibili.com", "m.bilibili.com"].includes(parsed.hostname) && /^\/(video\/(BV[0-9A-Za-z]{10}|av\d+)|bangumi\/play\/(ep|ss)\d{1,12})\/?$/.test(parsed.pathname)) {
      return { action: "preview_video", request_id: "hls:" + requestId, url: normalized };
    }
    // So do Twitch videos (past broadcasts, highlights and uploads).
    if (["twitch.tv", "www.twitch.tv", "m.twitch.tv"].includes(parsed.hostname) && /^\/videos\/[1-9]\d{0,19}\/?$/.test(parsed.pathname)) {
      return { action: "preview_video", request_id: "hls:" + requestId, url: normalized };
    }
    // So do TikTok video pages; their files need the page's cookies, which the app fetches.
    if (["tiktok.com", "www.tiktok.com", "m.tiktok.com"].includes(parsed.hostname) && /^\/@[\w.]*\/video\/\d{1,25}\/?$/.test(parsed.pathname)) {
      return { action: "preview_video", request_id: "hls:" + requestId, url: normalized };
    }
    if (/\.mpd$/i.test(new URL(normalized).pathname)) return { action: "preview_video", request_id: "hls:" + requestId, url: normalized };
    if (/\.m3u8$/i.test(new URL(normalized).pathname)) return { action: "preview_hls", request_id: "hls:" + requestId, url: normalized };
    let source_page; try {if(sourcePage)source_page=normalizeUrl(sourcePage);}catch{}
    return { action: "add", request_id: requestId, url: normalized, ...(source_page?{source_page}:{}) };
  }
  async function handoff(sendNative, request) {
    const reply = await sendNative(HOST, request);
    if (!reply || reply.version !== 1 || typeof reply.ok !== "boolean") throw new Error(__xdL.t("The desktop app returned an invalid response."));
    if (!reply.ok) {
      const error = new Error(reply.message || __xdL.t("The desktop app could not add this download."));
      error.definitive = true;
      throw error;
    }
    if (request.action === "add_batch" && (!Array.isArray(reply.job_ids) || reply.job_ids.length !== request.links.length ||
        reply.job_ids.some(id => typeof id !== "string" || !id) || new Set(reply.job_ids).size !== reply.job_ids.length)) {
      throw new Error(__xdL.t("The desktop app did not confirm the complete saved batch. Retry this same batch."));
    }
    if (request.action === "batch_options" && (!Array.isArray(reply.batch_options?.queues) || !reply.batch_options.queues.length ||
        !Array.isArray(reply.batch_options?.destinations) || !reply.batch_options.destinations.length)) {
      throw new Error(__xdL.t("Update the desktop app to use page-link batches."));
    }
    if (["add", "add_browser_login"].includes(request.action) && (typeof reply.job_id !== "string" || !reply.job_id)) throw new Error(__xdL.t("The desktop app did not confirm a saved download."));
    if (["preview_hls", "preview_video"].includes(request.action) && reply.preview_id !== request.request_id) throw new Error(__xdL.t("The desktop app did not confirm the video preview. Update Download Engine or retry."));
    if (request.action === "review_browser_video" && reply.preview_id !== request.request_id) throw new Error(__xdL.t("The desktop app did not confirm the video review. Update Download Engine, then use Recover handoffs."));
    if (request.action.startsWith("capture_")) {
      const allowed = request.action === "capture_prepare" ? ["prepared", "committed"]
        : request.action === "capture_commit" ? ["committed"]
        : request.action === "capture_abort" ? ["aborted", "committed"]
        : ["missing", "prepared", "committed", "aborted"];
      if (!allowed.includes(reply.capture_state) ||
          (["prepared", "committed"].includes(reply.capture_state) && !reply.job_id)) {
        throw new Error(__xdL.t("The desktop app did not confirm the browser handoff."));
      }
    }
    if (request.action.startsWith("recovery_")) {
      const session=reply.recovery;
      if ((!session && request.action==="recovery_offer") || (session &&
          (typeof session.id!=="string" || !session.id || (request.id && request.id!==session.id) ||
           typeof session.name!=="string" || typeof session.page!=="string" ||
           !Number.isSafeInteger(session.expires) || !["waiting","offered","accepted","cancelled"].includes(session.state)))) {
        throw Error(__xdL.t("The desktop app did not confirm browser recovery. Update Download Engine and retry."));
      }
    }
    return reply;
  }
  root.IDMCore = { HOST, normalizeUrl, makeRequest, handoff };
})(globalThis);


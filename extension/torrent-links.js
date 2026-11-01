/* A real click offers a magnet to the desktop. Failure keeps the browser's
   normal protocol handling available; pages cannot trigger a background offer. */
(() => {
  "use strict";
  const api = typeof browser !== "undefined" ? browser : chrome;
  let enabled = false;
  let busy = false;
  api.storage.local.get("autoCapture").then(value => { enabled = value.autoCapture === true; }).catch(() => {});
  api.storage.onChanged.addListener((changes, area) => {
    if (area === "local" && changes.autoCapture) enabled = changes.autoCapture.newValue === true;
  });
  document.addEventListener("click", event => {
    if (!enabled || !event.isTrusted || event.defaultPrevented || event.button !== 0 ||
        event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    const anchor = event.composedPath().find(node => node instanceof HTMLAnchorElement);
    const url = anchor?.href;
    if (!url || !/^magnet:/i.test(url) || url.length > 16384) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (busy) return;
    busy = true;
    Promise.resolve().then(() => api.runtime.sendMessage({ action: "on_page_torrent", url }))
      .then(reply => { if (!reply?.ok) location.assign(url); }, () => location.assign(url))
      .finally(() => { busy = false; });
  }, true);
})();

(async () => {
"use strict";
await globalThis.IDMI18n?.ready;
globalThis.IDMI18n?.translateDocument();
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
const api = globalThis.browser || globalThis.chrome;
const ui = Object.fromEntries(["page-title", "page-url", "scan-note", "search", "type", "source", "sort", "min-size", "previous-page", "next-page", "page-range", "select-visible", "clear", "counts", "links", "empty", "queue", "destination", "reload", "folder-path", "status", "add"].map(id => [id, document.getElementById(id)]));
const id = location.hash.slice(1);
let state = null, selected = new Set(), busy = false;
const PAGE_SIZE = 100;
let pageIndex = 0, selectionAnchor = null;
const labels = { documents: __xdL.t("Document"), compressed: __xdL.t("Archive"), programs: __xdL.t("Program"), music: __xdL.t("Audio"), video: __xdL.t("Video"), images: __xdL.t("Image"), other: __xdL.t("Unknown") };
function status(message, kind = "") { ui.status.textContent = message; ui.status.className = kind; }
async function request(message) {
  const reply = await api.runtime.sendMessage({ ...message, id });
  if (!reply?.ok) throw Error(reply?.message || __xdL.t("The extension did not respond. Reload this tab to try again."));
  return reply;
}
function locked() { return busy || !state || !!state.pending || !!state.receipt; }
function visible() { return IDMPageLinks.visibleLinks(state?.scan.links || [], ui.search.value, ui.type.value, ui.source.value, Number(ui["min-size"].value), ui.sort.value); }
function sizeLabel(bytes) {
  if (!Number.isSafeInteger(bytes) || bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"]; let n = bytes, i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return (i ? n.toFixed(1) : n) + " " + units[i];
}
function controls() {
  const frozen = locked(), rows = visible(), hidden = [...selected].filter(index => !rows.some(row => row.index === index)).length;
  ui.counts.textContent = __xdL.f("{0} of {1} links · {2} selected{3}", rows.length, state?.scan.links.length || 0, selected.size, hidden ? " (" + __xdL.f("{0} hidden by filters", hidden) + ")" : "");
  ui["select-visible"].disabled = frozen || !rows.length;
  ui.clear.disabled = frozen || !selected.size;
  ui.queue.disabled = frozen || !state?.options;
  ui.destination.disabled = ui.queue.disabled;
  ui.reload.disabled = frozen;
  ui.add.disabled = busy || !state || !!state.receipt || (!state.pending && (!state.options || !selected.size));
  ui.add.textContent = state?.receipt ? __xdL.t("Added to queue") : busy ? __xdL.t("Sending…") : state?.pending ? __xdL.t("Retry same batch") : !selected.size ? __xdL.t("Add selected files") : __xdL.f("Add {0} file{1}", selected.size, (!__xdL.language || __xdL.language === "en") && selected.size !== 1 ? "s" : "");
  for (const check of ui.links.querySelectorAll("input")) check.disabled = frozen;
  for (const button of ui.links.querySelectorAll("button")) button.disabled = busy || !state || !!state.pending;
}
function renderRows() {
  const fragment = document.createDocumentFragment(), rows = visible();
  pageIndex = Math.max(0, Math.min(pageIndex, Math.ceil(rows.length / PAGE_SIZE) - 1));
  const first = pageIndex * PAGE_SIZE, last = Math.min(rows.length, first + PAGE_SIZE);
  ui["page-range"].textContent = rows.length ? __xdL.f("{0}–{1} of {2}", first + 1, last, rows.length) : __xdL.t("No matching files");
  ui["previous-page"].disabled = pageIndex === 0;
  ui["next-page"].disabled = last >= rows.length;
  for (const link of rows.slice(first, last)) {
    const row = document.createElement("tr"), checkCell = document.createElement("td"), fileCell = document.createElement("td"), sizeCell = document.createElement("td"), typeCell = document.createElement("td");
    checkCell.className = "check";
    const check = document.createElement("input");
    check.type = "checkbox"; check.checked = selected.has(link.index); check.disabled = locked(); check.dataset.index = link.index;
    check.setAttribute("aria-label", __xdL.t("Select ") + link.filename);
    row.classList.toggle("selected", check.checked);
    let range = false;
    check.addEventListener("click", event => { range = event.shiftKey; });
    check.addEventListener("change", () => {
      const checked = check.checked;
      const ordered = rows.filter(item => !item.hls && !item.dash), start = ordered.findIndex(item => item.index === selectionAnchor), end = ordered.findIndex(item => item.index === link.index);
      const affected = range && start >= 0 ? ordered.slice(Math.min(start, end), Math.max(start, end) + 1) : [link];
      for (const item of affected) checked ? selected.add(item.index) : selected.delete(item.index);
      selectionAnchor = link.index; range = false;
      for (const input of ui.links.querySelectorAll("input")) { input.checked = selected.has(Number(input.dataset.index)); input.closest("tr").classList.toggle("selected", input.checked); }
      controls();
    });
    if (link.hls || link.dash) {
      const preview = document.createElement("button");
      preview.type = "button"; preview.textContent = link.dash ? __xdL.t("Preview DASH") : __xdL.t("Preview HLS");
      preview.addEventListener("click", async () => {
        busy = true; controls();
        try { const reply = await request({ action: "page_links_hls", index: link.index }); status(reply.message,"success"); }
        catch (error) { status(error.message,"error"); }
        finally { busy = false; controls(); }
      });
      typeCell.append(preview);
    } else checkCell.append(check);
    const name = document.createElement("span"), url = document.createElement("span"), origin = document.createElement("span");
    // The name is its own isolated box: in Arabic its start stays visible and the source tag follows it.
    const filename = document.createElement("bdi"); filename.textContent = link.filename;
    name.className = "file-name"; name.append(filename); name.title = link.text;
    url.className = "link-url"; url.textContent = link.url; url.title = link.url;
    origin.className = "source-label"; origin.textContent = { link: __xdL.t("Page link"), asset: __xdL.t("Page asset"), network: __xdL.t("Network media") }[link.source || "link"];
    name.title = link.filename + (link.text ? " · " + link.text : ""); name.append(origin);
    fileCell.append(name, url); typeCell.className = "type-label";
    sizeCell.className = "size-label"; sizeCell.textContent = sizeLabel(link.size); sizeCell.title = link.size ? __xdL.t("Reported file size") : __xdL.t("Size not available from this page");
    if (!link.hls && !link.dash) typeCell.textContent = labels[IDMPageLinks.linkCategory(link)];
    row.append(checkCell, fileCell, sizeCell, typeCell); fragment.append(row);
  }
  ui.links.replaceChildren(fragment); ui.empty.hidden = rows.length > 0; controls();
}
function option(select, value, title) { const item = document.createElement("option"); item.value = value; item.textContent = title; select.append(item); }
function folderPath() {
  const folder = state?.options?.destinations.find(item => item.id === ui.destination.value);
  ui["folder-path"].textContent = folder?.path || (ui.destination.value === "automatic" ? __xdL.t("Uses the automatic folder rules in Download Engine Settings.") : __xdL.t("Folders are managed in Download Engine Settings."));
}
async function refresh() {
  busy = true; controls();
  try {
    const next = await request({ action: "page_links_get" });
    state = next;
    if (state.scan.mode === "videos") {
      document.title = __xdL.t("Video downloads — Download Engine");
      document.querySelector("h1").textContent = __xdL.t("Video downloads");
      document.querySelector(".tag").textContent = __xdL.t("VIDEO FILES");
      document.querySelector(".intro").textContent = __xdL.t("Select direct video files, or preview an HLS / DASH stream to choose quality and audio in Download Engine.");
    } else {
      document.title = __xdL.t("Page links · Download Engine");
      document.querySelector("h1").textContent = __xdL.t("Choose what to download");
      document.querySelector(".tag").textContent = __xdL.t("PAGE LINKS");
      document.querySelector(".intro").textContent = __xdL.t("Review the links and assets on this page, then add the files you want to your queue.");
    }
    ui["page-title"].textContent = state.scan.title || __xdL.t("Untitled page"); ui["page-url"].textContent = state.scan.url;
    ui["scan-note"].textContent = state.scan.truncated ? __xdL.t("This page is large. Showing a limited set of links; open a smaller page to collect more.") : __xdL.f("{0} unique HTTP/HTTPS links found. Nothing is selected automatically.", state.scan.links.length);
    if (state.scan.mode === "videos") ui["scan-note"].textContent += " " + __xdL.t("HLS / DASH opens a desktop preview; live/encrypted streams are unsupported. Enable media detection before playback to find more streams.");
    if (state.pending) selected = new Set(state.pending.indices);
    const queue = state.pending?.queue_id || ui.queue.value, destination = state.pending?.destination || ui.destination.value || "automatic";
    ui.queue.replaceChildren(); ui.destination.replaceChildren();
    if (state.options) {
      for (const entry of state.options.queues) option(ui.queue, entry.id, (entry.id === "main" && entry.name === "Main queue" ? __xdL.t("Main queue") : entry.name) + (entry.running ? " · " + __xdL.t("running") : " · " + __xdL.t("paused")));
      for (const entry of state.options.destinations) option(ui.destination, entry.id, __xdL.t(entry.name));
      if ([...ui.queue.options].some(item => item.value === queue)) ui.queue.value = queue;
      if ([...ui.destination.options].some(item => item.value === destination)) ui.destination.value = destination;
    } else if (state.pending) { option(ui.queue, queue, __xdL.t("Original queue")); option(ui.destination, destination, __xdL.t("Original destination")); }
    folderPath();
    if (state.receipt) status(state.receipt.message, "success");
    else if (state.pending) status(__xdL.t("The last batch has no confirmed reply. Retry sends the same batch and will not add it twice. Keep the browser open until resolved."), "error");
    else if (state.optionsError) status(state.optionsError, "error");
    else status(__xdL.t("Select the files you want to add."));
  } finally { busy = false; renderRows(); }
}
function filterChanged() { pageIndex = 0; selectionAnchor = null; renderRows(); }
ui.search.addEventListener("input", filterChanged);
for (const name of ["type", "source", "sort", "min-size"]) ui[name].addEventListener("change", filterChanged);
ui["previous-page"].addEventListener("click", () => { pageIndex--; renderRows(); document.querySelector(".table-wrap").scrollTop = 0; });
ui["next-page"].addEventListener("click", () => { pageIndex++; renderRows(); document.querySelector(".table-wrap").scrollTop = 0; });
ui["select-visible"].addEventListener("click", () => { if (!locked()) { for (const row of visible()) if (!row.hls && !row.dash) selected.add(row.index); renderRows(); } });
ui.clear.addEventListener("click", () => { if (!locked()) { selected.clear(); renderRows(); } });
ui.destination.addEventListener("change", folderPath);
ui.reload.addEventListener("click", () => { void refresh().catch(error => status(error.message, "error")); });
ui.add.addEventListener("click", async () => {
  if (ui.add.disabled) return;
  const pending = !!state.pending;
  busy = true; controls(); status(pending ? __xdL.t("Checking the original batch…") : __xdL.t("Saving the selected files…"));
  try {
    const reply = await request({ action: "page_links_submit", retry: pending, indices: [...selected], queue_id: ui.queue.value, destination: ui.destination.value });
    state.receipt = { count: reply.count, message: reply.message }; state.pending = null;
    status(reply.message, "success");
  } catch (error) {
    try { await refresh(); } catch { state = null; /* Reload this tab to recover after a background restart. */ }
    status(error.message + (state?.pending ? " " + __xdL.t("Retry the same batch to check its receipt.") : ""), "error");
    // If even the state check failed, do not permit a changed submission until a
    // fresh background state is available; reload will recover its pending ID.

  } finally { busy = false; controls(); }
});
void refresh().catch(error => status(error.message, "error"));

})().catch(error => { console.error(error); const output=document.getElementById("status"); if(output){output.textContent=String(error.message || error);output.dataset.error="true";} });

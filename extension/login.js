(async () => {
"use strict";
await globalThis.IDMI18n?.ready;
globalThis.IDMI18n?.translateDocument();
  const __xdL = globalThis.IDMI18n || {t: text => text, f: (text, ...values) => text.replace(/\{(\d+)\}/g, (_, n) => String(values[n]))};
const api = typeof browser !== "undefined" ? browser : chrome;
const id = new URL(location.href).searchParams.get("id");
const status = document.querySelector("#status");
const approve = document.querySelector("#approve");
const forget = document.querySelector("#forget");
let model;
function show(ok, message) { status.textContent = message; status.dataset.error = String(!ok); }
api.runtime.sendMessage({ action: "browser_login_get", id }).then(reply => {
  if (!reply?.ok) throw Error(reply?.message || __xdL.t("The extension did not respond."));
  model = reply;
  document.querySelector("#origin").textContent = reply.origin;
  document.querySelector("#file-url").textContent = reply.url;
  approve.disabled = forget.disabled = false;
}).catch(error => show(false, error.message));
approve.addEventListener("click", async () => {
  if (!model) return;
  approve.disabled = forget.disabled = true;
  try {
    // Must be the first asynchronous API call in this explicit user gesture.
    const granted = await api.permissions.request(model.grant);
    if (!granted) { show(false, __xdL.t("Site access was not granted. No login was sent.")); return; }
    const reply = await api.runtime.sendMessage({ action: "browser_login_submit", id });
    show(reply?.ok === true, reply?.message || __xdL.t("The extension did not respond. Retry this same request."));
    if (reply?.ok) { approve.textContent = __xdL.t("Added to Download Engine"); model.added = true; }
  } catch { show(false, __xdL.t("Browser login could not be sent. Check site access and the desktop connection.")); }
  finally { approve.disabled = model.added === true; forget.disabled = false; }
});
forget.addEventListener("click", async () => {
  if (!model) return;
  approve.disabled = forget.disabled = true;
  try {
    const removed = await api.permissions.remove(model.grant);
    if (!removed) throw Error(__xdL.t("Site permission could not be removed."));
    show(true, __xdL.t("Browser login access removed for all sites. Existing downloads keep their encrypted snapshot until you remove it in Download Engine."));
  } catch { show(false, __xdL.t("Could not remove site access. Use the browser's extension settings.")); }
  finally { approve.disabled = model.added === true; forget.disabled = false; }
});

})().catch(error => { console.error(error); const output=document.getElementById("status"); if(output){output.textContent=String(error.message || error);output.dataset.error="true";} });

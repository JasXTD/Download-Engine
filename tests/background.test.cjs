const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
require("../extension/core.js");
require("../extension/capture.js");
function event() {
  const listeners = [];
  return { addListener: fn => listeners.push(fn), emit: (...args) => listeners.map(fn => fn(...args)) };
}
async function flush() { for (let i = 0; i < 12; i++) await new Promise(setImmediate); }
for(const variant of ['chromium','firefox']) {
  test(variant+': content status honors default-on access and saved off without a native call',async()=>{
    const {state,api}=background(variant);await flush();
    const sender={id:api.runtime.id,url:'https://example.test/page',frameId:0,tab:{id:7}};
    const ask=overrides=>new Promise(resolve=>{
      const handled=api.runtime.onMessage.emit({action:'video_button_state'},{...sender,...overrides},resolve)[0];
      if(!handled)resolve(undefined);
    });
    assert.equal((await ask()).enabled,false);
    state.granted=true;assert.equal((await ask()).enabled,true);
    assert.equal((await ask({frameId:1})).enabled,false);
    assert.equal((await ask({tab:{id:7,incognito:true}})).enabled,false);
    assert.equal(await ask({id:'other-extension'}),undefined);
    state.values.videoButtonDisabledSites=['https://example.test/*'];
    state.values.mediaDetection=true;assert.equal((await ask()).enabled,false);
    assert.equal(state.native.length,0);
  });
  test(variant+': connection and on-page video results persist in the redacted diagnostic log',async()=>{
    const {state,api}=background(variant);await flush();
    assert.equal((await state.request({action:'ping'})).ok,true);
    let log=await state.request({action:'diagnostics_get'});
    assert.equal(log.entries.at(-1).setting,'connection');assert.equal(log.entries.at(-1).ok,true);
    assert.equal(log.entries.at(-1).enabled,undefined);
    api.runtime.sendNativeMessage=async()=>{throw Error('Failed at https://private.test/watch?token=secret');};
    assert.equal((await state.request({action:'ping'})).ok,false);
    log=await state.request({action:'diagnostics_get'});
    assert.equal(log.entries.at(-1).ok,false);assert(!JSON.stringify(log).includes('secret'));assert(log.entries.at(-1).message.includes('[address]'));
    const reply=await new Promise(resolve=>api.runtime.onMessage.emit({action:'on_page_videos'},
      {id:api.runtime.id,url:'https://example.test/page',frameId:0,tab:{id:7}},resolve));
    assert.equal(reply.ok,false);assert.match(reply.message,/disabled/);
    log=await state.request({action:'diagnostics_get'});
    assert.equal(log.entries.at(-1).setting,'review');assert.equal(log.entries.at(-1).ok,false);
    assert.equal(state.values.lastResult.message,reply.message);
    await state.request({action:'diagnostics_clear'});
    assert.equal((await state.request({action:'diagnostics_get'})).entries.length,0);
  });
}
function holdNativePing(api) {
  const native = api.runtime.sendNativeMessage;
  let release, reached;
  const pending = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { reached = resolve; });
  api.runtime.sendNativeMessage = async (host, request) => {
    if (request.action === "ping") { reached(); await pending; }
    return native(host, request);
  };
  return { started, release };
}
for (const variant of ["chromium", "firefox"]) {
  test(variant + ": fresh install captures clicked downloads and detects media without toggling", async () => {
    const { state, downloads, item } = background(variant, {}, true);
    await flush();
    assert.equal(state.native.length, 0, "Installation does not queue or fetch anything");
    assert.equal((await state.request({ action: "media_status" })).enabled, true);
    downloads.onCreated.emit(item); await flush();
    assert.equal(state.receipt, "committed"); assert.equal(state.cancel, 1);
    downloads.onCreated.emit(item); await flush(); assert.equal(state.cancel, 1);
  });
  test(variant + ": saved off choices survive startup and later permission grants", async () => {
    const { state, api, downloads, item } = background(variant, { autoCapture: false, mediaDetection: false }, true);
    await flush();
    api.permissions.onAdded.emit({ permissions: ["downloads", "webRequest"] }); await flush();
    downloads.onCreated.emit(item); await flush();
    assert(!state.native.some(request => request.action.startsWith("capture_") || request.action === "add"));
    assert.equal(state.cancel, 0); // A pending link-recovery lookup is independent of capture.
    assert.equal((await state.request({ action: "media_status" })).enabled, false);
  });
  test(variant + ": default capture leaves the browser download intact when desktop is unavailable", async () => {
    const { state, api, downloads, item } = background(variant, {}, true);
    await flush(); api.runtime.sendNativeMessage = async () => { throw Error("Host unavailable"); };
    downloads.onCreated.emit(item); await flush();
    assert.equal(state.cancel, 0); assert.equal(item.state, "in_progress"); assert.equal(item.paused, false);
  });
  test(variant + ": granting withheld website access starts default detection without resetting saved choices", async () => {
    const { state, api } = background(variant);
    await flush(); assert.equal((await state.request({ action: "media_status" })).enabled, false);
    state.granted = true; api.permissions.onAdded.emit({ permissions: ["webRequest"], origins: ["https://*/*", "http://*/*"] }); await flush();
    assert.equal((await state.request({ action: "media_status" })).enabled, true);
  });
}
test("chromium: fast downloads wait for the existing recovery-first capture decision", async () => {
  const {state,api,downloads,item}=background("chromium",{},true); await flush();
  const native=api.runtime.sendNativeMessage;
  let release, reached;
  const pending=new Promise(resolve=>{release=resolve;});
  const started=new Promise(resolve=>{reached=resolve;});
  api.runtime.sendNativeMessage=async(host,request)=>{
    if(request.action==="recovery_pending"){reached();await pending;}
    return native(host,request);
  };
  downloads.onCreated.emit(item); await started;
  let suggestions=0;
  const held=downloads.onDeterminingFilename.emit(item,(...args)=>{
    assert.equal(args.length,0,"Keep the browser's chosen filename"); suggestions++;
    if(item.state==="in_progress" && !item.paused)item.state="complete";
  });
  downloads.onChanged.emit({id:item.id,filename:{current:item.filename}});
  await flush();
  // Chrome completes a small transfer if no filename listener holds it open.
  if(held[0]!==true && suggestions===0)item.state="complete";
  assert.equal(suggestions,0,"Do not release while the earlier recovery lookup is pending");
  release(); await flush();
  assert.equal(state.receipt,"committed"); assert.equal(state.cancel,1);
  assert.equal(suggestions,1); assert.equal(item.state,"interrupted");
  assert.equal(state.native.filter(r=>r.action==="recovery_pending").length,1);
  assert.equal(state.native.filter(r=>r.action==="capture_commit").length,1);
});
for(const scenario of ["capture off","excluded type","host unavailable","storage failure"]) {
  test("chromium: filename decision releases the browser on "+scenario,async()=>{
    const {state,api,downloads,item}=background("chromium",scenario==="capture off"?{autoCapture:false}:{},true); await flush();
    if(scenario==="excluded type")item.filename="notes.docx";
    if(scenario==="host unavailable")api.runtime.sendNativeMessage=async()=>{throw Error("Host unavailable");};
    if(scenario==="storage failure")api.storage.local.get=async()=>{throw Error("Storage unavailable");};
    let suggestions=0;
    downloads.onDeterminingFilename.emit(item,(...args)=>{assert.equal(args.length,0);suggestions++;});
    await flush();
    assert.equal(suggestions,1);assert.equal(item.paused,false);assert.equal(state.cancel,0);
    assert(!state.native.some(r=>r.action==="capture_commit"));
  });
}

test("chromium: filename hold preserves explicit link recovery when capture is off",async()=>{
  const {state,api,downloads,item}=background("chromium",{autoCapture:false},true); await flush();
  item.startTime=new Date().toISOString();
  const session={id:"existing-recovery",state:"waiting",name:item.filename,size:item.totalBytes,
    page:"https://example.test/page",expires:Math.floor(Date.now()/1000)+1800};
  api.runtime.sendNativeMessage=async(host,request)=>{
    state.native.push(request);
    return {ok:true,version:1,recovery:{...session,state:request.action==="recovery_offer"?"accepted":"waiting"}};
  };
  let suggestions=0;
  downloads.onCreated.emit(item);
  assert.equal(downloads.onDeterminingFilename.emit(item,()=>{suggestions++;})[0],true);
  downloads.onChanged.emit({id:item.id,filename:{current:item.filename}});
  await flush();
  assert.deepEqual(state.native.map(r=>r.action),["recovery_pending","recovery_offer"]);
  assert.equal(suggestions,1);assert.equal(state.cancel,1);assert.equal(state.receipt,"missing");
});
test("chromium: ineligible files are released synchronously without native handoff",async()=>{
  const {state,downloads,item}=background("chromium",{},true);await flush();
  for(const change of [{incognito:true},{byExtensionId:"another-extension"},{danger:"dangerous"},
    {paused:true},{mime:"text/html"},{url:"blob:https://example.test/file"},{state:"complete"}]) {
    let suggestions=0;
    const returned=downloads.onDeterminingFilename.emit({...item,...change},(...args)=>{
      assert.equal(args.length,0);suggestions++;
    });
    assert.equal(returned[0],undefined);assert.equal(suggestions,1);
  }
  await flush();assert.equal(state.native.length,0);assert.equal(state.cancel,0);
});
test("chromium: setup failure releases the filename decision exactly once",async()=>{
  const {api,downloads,item}=background("chromium",{},true);await flush();
  api.alarms.get=async()=>{throw Error("Alarm unavailable");};
  let suggestions=0;
  downloads.onDeterminingFilename.emit(item,()=>{suggestions++;});
  await flush();assert.equal(suggestions,1);assert.equal(item.paused,false);
});

function background(variant, values = {}, granted = false) {
  const state = { granted, compatible: true, values: structuredClone(values), session: {}, native: [], receipt: "missing", cancel: 0, alarms: [] };
  const item = { id: 42, url: "https://example.test/file.zip", startTime: "today", filename: "file.zip",
    mime: "application/zip", totalBytes: 100000, state: "in_progress", paused: false, danger: "safe" };
  const downloads = {
    onCreated: event(), onChanged: event(),
    ...(variant === "chromium" ? { onDeterminingFilename: event() } : {}),
    search: async () => [structuredClone(item)],
    pause: async () => { item.paused = true; },
    resume: async () => { item.paused = false; },
    cancel: async () => { item.state = "interrupted"; item.paused = false; state.cancel++; }
  };
  const api = {
    runtime: {
      id: "extension-id", getURL: part => "extension://extension-id/" + part,
      onInstalled: event(), onStartup: event(), onMessage: event(),
      sendNativeMessage: async (host, req) => {
        assert.equal(host, "com.xetro.downloadengine"); state.native.push(req);
        if (req.action === "ping") return { ok: true, version: 1, message: "Connected",
          capabilities: state.compatible ? ["automatic_capture_v1"] : [] };
        if (req.action === "add") return { ok: true, version: 1, message: "Added", job_id: "manual" };
        if (req.action === "capture_prepare") state.receipt = "prepared";
        if (req.action === "capture_commit") state.receipt = "committed";
        if (req.action === "capture_abort" && state.receipt !== "committed") state.receipt = "aborted";
        return { ok: true, version: 1, capture_state: state.receipt, job_id: "captured" };
      }
    },
    storage: { local: {
      get: async key => structuredClone(key === null ? state.values : Array.isArray(key) ? Object.fromEntries(key.map(k => [k,state.values[k]])) : { [key]: state.values[key] }),
      set: async values => Object.assign(state.values, structuredClone(values)),
      remove: async key => { for (const name of Array.isArray(key) ? key : [key]) delete state.values[name]; }
    }, session: {
      get: async key => structuredClone(key === null ? state.session : { [key]: state.session[key] }),
      set: async values => Object.assign(state.session, structuredClone(values)),
      remove: async key => { delete state.session[key]; }
    } },
    tabs: { onRemoved: event(), onUpdated: event(), query: async () => [{id:7,url:'https://example.test/page'}], get: async id => ({id,url:'https://example.test/page'}) },
    webRequest: {onBeforeRequest:event(),onHeadersReceived:event(),onCompleted:event(),onErrorOccurred:event()},
    permissions: { contains: async () => state.granted, onAdded: event(), onRemoved: event() },
    alarms: {
      get: async () => state.alarms.length ? { name: "idm-ex-capture-recovery" } : undefined,
      create: async (...args) => { state.alarms.push(args); }, onAlarm: event()
    },
    contextMenus: { removeAll: async () => {}, create: () => {}, onClicked: event() },
    [variant === "firefox" ? "browserAction" : "action"]: {
      setBadgeText: async () => {}, setBadgeBackgroundColor: async () => {}
    }
  };
  Object.defineProperty(api, "downloads", { get: () => state.granted ? downloads : undefined });
  const context = vm.createContext({ [variant === "firefox" ? "browser" : "chrome"]: api,
    URL, performance, structuredClone, TextEncoder, crypto: { randomUUID: () => "random" }, setTimeout, clearTimeout, console });
  const load = name => vm.runInContext(fs.readFileSync(path.join(__dirname, "../extension", name), "utf8"), context, { filename: name });
  if (variant === "chromium") context.importScripts = (...files) => files.forEach(load);
  else { load("core.js"); load("capture.js"); load("link-recovery.js"); load("page-links.js"); load("browser-login.js"); load("video-button-manager.js"); load("github-download.js"); load("media.js"); load("permission-flow.js"); }
  load("background.js");
  state.request = message => new Promise(resolve => {
    const handled = api.runtime.onMessage.emit(message, {
      id: "extension-id", url: "extension://extension-id/popup.html"
    }, resolve)[0];
    if (!handled) resolve(undefined);
  });
  return { state, api, downloads, item };
}
for (const variant of ["chromium", "firefox"]) {
  test(variant+': browser recovery permission failures remain readable in diagnostics',async()=>{
    const {state}=background(variant);await flush();
    state.values.lastPopupError={time:123,setting:'recovery',ok:false,message:'Cannot reach https://site.test/file?secret=token'};
    const reply=await state.request({action:'diagnostics_get'});
    assert(reply.ok);assert.equal(reply.entries.at(-1).setting,'recovery');
    assert(!reply.entries.at(-1).message.includes('secret'));assert.match(reply.entries.at(-1).message,/\[address\]/);
  });
  test(variant + ": setting diagnostics retain concurrent results and popup errors across openings", async () => {
    const {state}=background(variant);await flush();
    const replies=await Promise.all([state.request({action:'configure_capture',enabled:false}),state.request({action:'configure_media',enabled:false})]);
    assert(replies.every(reply=>reply.ok));
    let log=await state.request({action:'diagnostics_get'});
    assert.equal(log.entries.length,2);
    assert.deepEqual(new Set(log.entries.map(entry=>entry.setting)),new Set(['capture','media']));
    assert(log.entries.every(entry=>entry.ok && entry.enabled===false));
    state.values.lastPopupError={time:123,setting:'capture',enabled:true,ok:false,message:'Permission bridge failed'};
    log=await state.request({action:'diagnostics_get'});
    assert.equal(log.entries.length,3);assert.equal(log.entries[0].time,123);
    await state.request({action:'permission_popup_opened',returnTo:{windowId:2,tabId:7}});
    assert.equal((await state.request({action:'diagnostics_get'})).entries.length,3);
    await state.request({action:'diagnostics_clear'});
    assert.equal((await state.request({action:'diagnostics_get'})).entries.length,0);
  });
  test(variant + ": setting diagnostics preserve failure reasons, redact addresses and paths, and stay bounded", async () => {
    const {state,api}=background(variant);await flush();state.granted=true;
    api.runtime.sendNativeMessage=async()=>{throw Error('Unavailable at https://fixture.test/private?token=secret or C:\\fixture\\secret.exe');};
    assert.equal((await state.request({action:'configure_capture',enabled:true})).ok,false);
    let log=await state.request({action:'diagnostics_get'});
    assert.equal(log.entries.length,1);assert.equal(log.entries[0].ok,false);
    assert.match(log.entries[0].message,/Unavailable/);
    assert(!JSON.stringify(log).includes('secret'));assert.match(log.entries[0].message,/\[address\]/);assert.match(log.entries[0].message,/\[path\]/);
    for(let i=0;i<35;i++)await state.request({action:'configure_media',enabled:false});
    log=await state.request({action:'diagnostics_get'});assert.equal(log.entries.length,30);
    assert(log.entries.every(entry=>entry.setting==='media' && entry.ok));
  });
  test(variant + ": diagnostic storage failure and malformed input cannot break subsequent toggles", async () => {
    const {state,api}=background(variant);await flush();state.granted=true;
    state.values.lastResult={ok:false,message:'Previous permission error'};
    const save=api.storage.local.set;
    api.storage.local.set=async values=>{if(Object.hasOwn(values,'settingDiagnostics'))throw Error('Fixture diagnostic storage failure');return save(values);};
    assert.equal((await state.request({action:'configure_media',enabled:true})).ok,true);
    assert.equal(state.values.mediaDetection,true);
    assert.equal(state.values.lastResult.ok,true,'Re-enabling clears a stale permission error when the popup is opened again');
    assert.match(state.values.lastResult.message,/Media detection is on/);
    api.storage.local.set=save;
    assert.equal((await state.request({action:'configure_media',enabled:'invalid'})).ok,false);
    assert.equal((await state.request({action:'configure_media',enabled:false})).ok,true);
    assert.equal((await state.request({action:'diagnostics_get'})).entries.length,1);
  });
  test(variant + ": missing desktop connector explains setup and allows retry", async () => {
    const { state, api } = background(variant);
    await flush();
    state.granted = true;
    const connected = api.runtime.sendNativeMessage;
    const unavailable = variant === "firefox"
      ? "No such native application com.xetro.downloadengine"
      : "Specified native messaging host not found.";
    api.runtime.sendNativeMessage = async () => { throw Error(unavailable); };
    const reply = await state.request({ action: "configure_capture", enabled: true });
    assert.equal(reply.ok, false);
    assert.notEqual(state.values.autoCapture, true);
    assert(reply.message.includes(unavailable), "Keep the browser's diagnostic");
    assert.match(reply.message, /Browser setup/);
    assert.match(reply.message, /Connect browser/);
    assert.equal(state.values.lastResult?.message, reply.message, "Explain the failure when the popup reopens");
    assert.equal(state.values.lastResult?.ok, false);
    assert.equal(state.cancel, 0);
    api.runtime.sendNativeMessage = connected;
    assert.equal((await state.request({ action: "configure_capture", enabled: true })).ok, true);
    assert.equal(state.values.autoCapture, true);
    assert.equal(state.values.lastResult.ok, true);
  });
  test(variant + ": a later capture setting wins while an earlier host check is pending", async t => {
    const { state, api } = background(variant);
    await flush();
    state.granted = true;
    const ping = holdNativePing(api);
    t.after(ping.release);
    const enabling = state.request({ action: "configure_capture", enabled: true });
    await ping.started;
    const disabling = state.request({ action: "configure_capture", enabled: false });
    ping.release();
    const replies = await Promise.all([enabling, disabling]);
    assert(replies.every(reply => reply.ok));
    assert.equal(state.values.autoCapture, false);
    assert.equal(state.values.lastResult.message, "Automatic capture is off.");
  });
  test(variant + ": revoking download access during a host check keeps capture off", async t => {
    for (const regrantBeforeReply of [false, true]) {
      const { state, api } = background(variant);
      await flush();
      state.granted = true;
      const ping = holdNativePing(api);
      t.after(ping.release);
      const enabling = state.request({ action: "configure_capture", enabled: true });
      await ping.started;
      state.granted = false;
      api.permissions.onRemoved.emit({ permissions: ["downloads"] });
      if (regrantBeforeReply) {
        state.granted = true;
        api.permissions.onAdded.emit({ permissions: ["downloads"] });
      }
      ping.release();
      assert.equal((await enabling).ok, false);
      await flush();
      assert.equal(state.values.autoCapture, false);
      state.granted = true;
      api.permissions.onAdded.emit({ permissions: ["downloads"] });
      await flush();
      assert.equal(state.values.autoCapture, false, "Regranting access cannot re-enable an old request");
      assert.equal((await state.request({ action: "configure_capture", enabled: true })).ok, true);
      assert.equal(state.values.autoCapture, true, "A failed update must not block later explicit changes");
    }
  });
  test(variant + ": background works without optional access; enable checks permission and host", async () => {
    const { state, api, downloads, item } = background(variant);
    await flush();
    assert.equal((await state.request({ action: "ping" })).ok, true);
    assert.equal((await state.request({ action: "configure_capture", enabled: true })).ok, false);
    assert.notEqual(state.values.autoCapture, true);
    state.granted = true; api.permissions.onAdded.emit({ permissions: ["downloads"] });
    state.compatible = false;
    assert.equal((await state.request({ action: "configure_capture", enabled: true })).ok, false);
    state.compatible = true;
    assert.equal((await state.request({ action: "configure_capture", enabled: true })).ok, true);
    downloads.onCreated.emit(item); await flush();
    assert.equal(state.cancel, 1);
    assert.equal(state.receipt, "committed");
    assert(state.alarms.some(([name, details]) => name === "idm-ex-capture-recovery" && details.periodInMinutes === 1));
    assert.equal((await state.request({ action: "configure_capture", enabled: false })).ok, true);
    assert.equal(state.values.autoCapture, false);
    assert.equal(state.alarms.length, 1, "Download events must not postpone the recovery alarm");
  });
  test(variant + ": rejects page messages and preserves manual link handoff", async () => {
    const { state, api } = background(variant);
    let replied = false;
    assert.equal(api.runtime.onMessage.emit({ action: "configure_capture", enabled: true },
      { id: "extension-id", url: "https://untrusted.test/" }, () => { replied = true; })[0], false);
    assert.equal(replied, false);
    const reply = await state.request({ action: "add", url: "https://example.test/file.zip", request_id: "manual-1" });
    assert.equal(reply.job_id, "manual");
    assert.equal(state.native.at(-1).request_id, "manual-1");
    assert.notEqual(state.values.autoCapture, true);
  });
  test(variant + ": spoofed or missing sender identity cannot reach native messaging", async () => {
    const { state, api } = background(variant);
    await flush();
    for (const sender of [{}, {id:"other-extension",url:"extension://extension-id/popup.html"},
      {id:"extension-id",url:"extension://extension-id.attacker/popup.html"},
      {id:"extension-id",url:"https://attacker.test/extension://extension-id/"}]) {
      let replied=false;
      const before=state.native.length;
      assert.equal(api.runtime.onMessage.emit({action:"add",url:"https://example.test/file",request_id:"spoof"},
        sender,()=>{replied=true;})[0],false);
      await flush();
      assert.equal(state.native.length,before);
      assert.equal(replied,false);
    }
  });

}


for (const variant of ["chromium", "firefox"]) {
  test(variant + ": background finishes approved settings and returns to the originating popup", async () => {
    const {state,api} = background(variant); await flush();
    const opened=[]; api.windows={get:async () => ({focused:true})}; api.extension={getViews:() => []};
    (api.action || api.browserAction).openPopup=async value => opened.push(value);
    for (const setting of ["capture", "media"]) {
      state.granted=false;
      let approve;
      api.permissions.request=() => new Promise(resolve => { approve=() => {state.granted=true;resolve(true);}; });
      const received=[];
      api.runtime.onMessage.emit({action:"request_setting_permission",setting,returnTo:{windowId:2,tabId:7}},
        {id:api.runtime.id,url:api.runtime.getURL("popup.html")}, value => received.push(value));
      assert.equal(typeof approve,"function");
      approve(); await flush();
      assert.equal(received.at(-1)?.ok,true);
      assert.equal(state.values[setting === "capture" ? "autoCapture" : "mediaDetection"],true);
      assert.equal(opened.at(-1).windowId,2);
    }
    assert.equal(opened.length,2);
  });
  test(variant + ": media commands require optional access and revocation clears discovery without native calls", async () => {
    const {state,api}=background(variant);await flush();
    assert.equal((await state.request({action:'media_status'})).enabled,false);
    assert.equal((await state.request({action:'configure_media',enabled:true})).ok,false);
    state.granted=true;api.permissions.onAdded.emit({permissions:['webRequest'],origins:['https://*/*','http://*/*']});await flush();
    assert.equal((await state.request({action:'configure_media',enabled:true})).ok,true);
    const details={requestId:'one',tabId:7,type:'xmlhttprequest',method:'GET',url:'https://example.test/clip.mp4',statusCode:200,responseHeaders:[{name:'Content-Type',value:'video/mp4'}]};
    api.webRequest.onBeforeRequest.emit(details);api.webRequest.onHeadersReceived.emit(details);await flush();
    assert.equal((await state.request({action:'media_status'})).items.length,1);
    state.granted=false;api.permissions.onRemoved.emit({permissions:['webRequest']});await flush();
    assert.equal((await state.request({action:'media_status'})).enabled,false);assert.deepEqual(state.session,{});assert.equal(state.native.length,0);
  });
  test(variant + ": page collection and batch commands require their exact extension page", async () => {
    const { state, api } = background(variant);
    await flush();
    for (const [action, page] of [["diagnostics_get", "links.html"], ["diagnostics_clear", "links.html"], ["request_setting_permission", "links.html"], ["scan_page", "links.html"], ["scan_videos", "links.html"], ["page_links_get", "popup.html"],
      ["page_links_submit", "popup.html"], ["page_links_hls", "popup.html"], ["page_links_hls", "links.html#wrong"], ["preview_hls", "popup.html"], ["review_browser_video", "popup.html"], ["review_browser_video", "links.html"], ["add_batch", "links.html"], ["batch_options", "popup.html"], ["browser_login_submit", "popup.html"], ["browser_login_get", "links.html"], ["add_browser_login", "login.html"], ["media_status", "links.html"], ["configure_media", "login.html"], ["review_media", "links.html"], ["clear_media", "links.html"]]) {
      const before = state.native.length;
      let replied = false;
      assert.equal(api.runtime.onMessage.emit({ action, id: "00000000-0000-4000-8000-000000000001" },
        { id: "extension-id", url: "extension://extension-id/" + page }, () => { replied = true; })[0], false);
      await flush();
      assert.equal(state.native.length, before);
      assert.equal(replied, false);
    }
    for (const action of ["diagnostics_get", "diagnostics_clear", "request_setting_permission", "scan_page", "scan_videos", "page_links_get", "page_links_submit", "media_status", "configure_media", "review_media", "clear_media"]) {
      assert.equal(api.runtime.onMessage.emit({ action },
        { id: "extension-id", url: "https://untrusted.test/links.html" }, () => {})[0], false);
    }
  });
  test(variant + ": new users get default capture rules; unmatched downloads and the context menu bypass capture", async () => {
    const { state, api, downloads, item } = background(variant);
    await flush();
    assert.equal(state.values.captureFilters, undefined, "No rules are written before capture is enabled");
    const current = await state.request({ action: "capture_rules_get" });
    // Replies come from the background's VM realm; compare their values.
    assert.deepEqual([...current.rules.types], [...IDMCapture.rules.DEFAULT_TYPES]);
    assert.deepEqual([...current.defaults], [...IDMCapture.rules.DEFAULT_TYPES]);
    state.granted = true; api.permissions.onAdded.emit({ permissions: ["downloads"] }); await flush();
    assert.equal((await state.request({ action: "configure_capture", enabled: true })).ok, true);
    assert.deepEqual(state.values.captureFilters.types, [...IDMCapture.rules.DEFAULT_TYPES]);
    Object.assign(item, { filename: "notes.docx", url: "https://example.test/notes.docx" });
    downloads.onCreated.emit(item); await flush();
    assert.equal(state.cancel, 0); assert(!state.native.some(r => r.action === "capture_prepare"));
    const saved = await state.request({ action: "capture_rules_save", rules: { types: "docx", minSize: "", unit: "MB", sites: "Example.TEST" } });
    assert.equal(saved.ok, true); assert.deepEqual(state.values.captureFilters.sites, ["example.test"]);
    const rejected = await state.request({ action: "capture_rules_save", rules: { types: "bad/x", minSize: "", unit: "MB", sites: "" } });
    assert.equal(rejected.ok, false); assert.deepEqual(state.values.captureFilters.types, ["docx"], "Invalid input never saves");
    downloads.onCreated.emit(item); await flush();
    assert.equal(state.cancel, 0, "Skipped site stays in the browser");
    api.contextMenus.onClicked.emit({ menuItemId: "idm-ex-download", linkUrl: "https://example.test/notes.docx", pageUrl: "https://example.test/" }, { url: "https://example.test/" });
    await flush();
    assert.equal(state.native.at(-1).action, "add", "The context menu ignores capture rules");
    assert.equal(api.runtime.onMessage.emit({ action: "capture_rules_save", rules: {} }, { id: "extension-id", url: "extension://extension-id/links.html" }, () => {})[0], false);
    assert.equal((await state.request({ action: "capture_rules_save", rules: { types: "docx", minSize: "", unit: "MB", sites: "" } })).ok, true);
    downloads.onCreated.emit(item); await flush();
    assert.equal(state.cancel, 1, "The same download is captured once its site is no longer skipped");
  });
  test(variant + ": users who enabled capture before rules existed keep capturing every type", async () => {
    const { state, api, downloads, item } = background(variant, { autoCapture: true });
    state.granted = true; api.permissions.onAdded.emit({ permissions: ["downloads"] }); await flush();
    assert.deepEqual(state.values.captureFilters, { version: 1, types: ["*"], minSizeKB: 0, sites: [] });
    Object.assign(item, { filename: "notes.docx", url: "https://example.test/notes.docx" });
    downloads.onCreated.emit(item); await flush();
    assert.equal(state.cancel, 1);
    const off = background(variant, { autoCapture: false }); await flush();
    assert.equal(off.state.values.captureFilters, undefined);
    const kept = background(variant, { autoCapture: true, captureFilters: { version: 1, types: ["zip"], minSizeKB: 0, sites: [] } }); await flush();
    assert.deepEqual(kept.state.values.captureFilters.types, ["zip"]);
  });
}

/* UI smoke only: the browser API is simulated; an installed Chromium browser renders the real popup. */
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright");
(async () => {
  const root = path.resolve(__dirname, "..");
  const packagedBrowser = process.argv.includes("--packaged-edge") ? "edge" : process.argv.includes("--packaged-firefox") ? "firefox" : null;
  const extensionRoot = path.join(root, packagedBrowser ? `dist/IDM-EX-UnsignedDev/extensions/${packagedBrowser}` : "extension");
  const output = path.join(root, "artifacts", packagedBrowser ? `extension-popup-${packagedBrowser}-dist` : "extension-popup-security");
  fs.mkdirSync(output, { recursive: true });
  const browser = await chromium.launch({ headless: true,
    executablePath: process.env.IDM_EX_TEST_CHROME || "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe" });
  const errors = [];
  try {
    const context = await browser.newContext({ viewport: { width: 370, height: 590 }, colorScheme: "light" });
    await context.addInitScript(() => {
      const state = window.testState = { autoCapture: false, grant: true, connected: true, permissions: 0, grantedPermissions:[], grantedOrigins:[], messages: [], mediaDetection: false, videoEnabled: true, items: [], permissionRequests: [], diagnostics:[] };
      window.chrome = {
        windows: { getCurrent: async () => ({ id: 2 }) },
        tabs: { query: async () => [{ id: 7, windowId: 2 }] },
        storage: { local: { get: async () => ({ autoCapture: state.autoCapture }), set: async value => { if(value.lastPopupError)state.lastPopupError=value.lastPopupError; } },
          session: { get: async key => ({[key]:state.returnMarker}), remove: async () => {state.returnMarker=null;} } },
        permissions: {
          contains: async grant => (grant.permissions||[]).every(p=>state.grantedPermissions.includes(p)) && (grant.origins||[]).every(p=>state.grantedOrigins.includes(p) || state.grantedOrigins.includes(new URL(p).protocol+'//*/*')),
          request: async grant => { state.permissionGesture=navigator.userActivation.isActive; state.permissionRequests.push(grant); if(state.requestError)throw Error(state.requestError); if (state.grant) { state.permissions++;state.grantedPermissions.push(...grant.permissions||[]);state.grantedOrigins.push(...grant.origins||[]); } return state.grant; }
        },
        runtime: { sendMessage: async request => {
          state.messages.push(request);
          if (request.action === "diagnostics_get") return {ok:true,entries:[...state.diagnostics,...state.lastPopupError?[state.lastPopupError]:[]]};
          if (request.action === "diagnostics_clear") {state.diagnostics=[];state.lastPopupError=null;return {ok:true,entries:[]};}
          if (request.action === "request_setting_permission") {
            const grants = {capture:{permissions:["downloads"]},media:{permissions:["webRequest"],origins:["https://*/*","http://*/*"]},video:{origins:[request.pattern]}};
            const denied = {capture:"Download access was not granted. Automatic capture stays off.",media:"Website access was not granted. Media detection stays off.",video:"Site access was not granted. Allow this site to show video buttons."};
            if (!await chrome.permissions.request(grants[request.setting])) return {ok:false,message:denied[request.setting]};
            return chrome.runtime.sendMessage({action:request.setting==='video'?'configure_video_button':"configure_"+request.setting,enabled:true,pattern:request.pattern});
          }
          if (request.action === "media_status") return {ok:true,tabId:7,enabled:state.mediaDetection,items:structuredClone(state.items)};
          if (request.action === "video_button_status") return state.videoPageUnavailable ? {ok:false,message:"Open a regular web page to configure its video button."} : {ok:true,pattern:"https://example.test/*",enabled:state.videoEnabled,permitted:await chrome.permissions.contains({origins:['https://example.test/*']})};
          if (request.action === "configure_video_button") {state.videoEnabled=request.enabled;return {ok:true,message:request.enabled?'Video buttons are on for this site.':'Video buttons are off for this site.'};}
          if (request.action === "ping") {
            if(state.holdPing) await new Promise(resolve=>state.finishPing=resolve);
            if(state.pingError)throw Error(state.pingError);
            return {ok:state.connected,message:state.connected?'Download Engine is connected':'The desktop app is not connected.'};
          }
          if (request.action === "configure_media") { state.mediaDetection=request.enabled; if(!request.enabled)state.items=[]; return {ok:true,message:request.enabled?"Media detection is on":"Media detection is off"}; }
          if (request.action === "clear_media") { state.items=[]; return {ok:true,message:"Media list cleared"}; }
          if (request.action === "review_media") return {ok:true,message:"Review opened. Nothing queued."};
          if (request.action === "configure_capture") {
            if (request.enabled && !state.connected) return { ok: false, message: "The desktop app is not connected." };
            state.autoCapture = request.enabled;
            return { ok: true, message: state.autoCapture ? "Automatic capture is on for direct file downloads." : "Automatic capture is off." };
          }
          return { ok: true, message: request.action === "add" ? "Added to the download queue" : "Download Engine is connected" };
        } }
      };
    });
    const manifest = JSON.parse(fs.readFileSync(path.join(extensionRoot,packagedBrowser ? "manifest.json" : "manifest.chromium.json"),"utf8"));
    const csp = typeof manifest.content_security_policy === "string" ? manifest.content_security_policy : manifest.content_security_policy.extension_pages;
    // Fulfilled locally: no request is sent to this reserved .test host.
    await context.route("**/*", async route => {
      const url = new URL(route.request().url());
      const name = url.pathname.slice(1);
      if (url.hostname !== "idm-ex.test" || !["popup.html","popup.css","popup.js","i18n.js","i18n-catalog.js"].includes(name)) {
        return route.abort("blockedbyclient");
      }
      const types = {"popup.html":"text/html","popup.css":"text/css","popup.js":"text/javascript"};
      await route.fulfill({status:200,contentType:types[name] || "text/javascript",
        headers:{"Content-Security-Policy":csp},body:name === "popup.js" ? fs.readFileSync(path.join(extensionRoot,name),"utf8").replace("})().catch(error =>", "Object.assign(window, {refreshVideoButton,refreshMedia,refreshToggle,initialize});\n})().catch(error =>") : fs.readFileSync(path.join(extensionRoot,name))});
    });
    const page = await context.newPage();
    page.on("pageerror", error => errors.push(error.message));
    await page.goto("https://idm-ex.test/popup.html");
    const diagnosticSummary = page.locator("#diagnostics > summary");
    assert(await diagnosticSummary.isVisible(), "Diagnostic log must be discoverable without opening Browser options");
    const diagnosticBounds = await diagnosticSummary.boundingBox();
    assert(diagnosticBounds.y + diagnosticBounds.height <= 590, "Diagnostic log must be visible without scrolling the initial popup");
    await page.screenshot({path:path.join(output,"diagnostics-available.png")});
    const toggle = page.locator("#auto-capture");
    assert.equal(await toggle.isChecked(), false);
    await toggle.click();
    await page.waitForFunction(() => document.querySelector("#auto-capture").checked && !document.querySelector("#auto-capture").disabled);
    assert.equal(await page.evaluate(() => testState.autoCapture), true);
    assert.equal(await page.evaluate(() => testState.permissions), 1);
    await page.screenshot({ path: path.join(output, "light.png") });
    await page.emulateMedia({ colorScheme: "dark" });
    await page.screenshot({ path: path.join(output, "dark.png") });
    await toggle.uncheck();
    await page.waitForFunction(() => !testState.autoCapture);
    await page.locator('#browser-options > summary').click();
    const videoToggle=page.locator('#video-button');
    await page.evaluate(async()=>{testState.videoPageUnavailable=true;await refreshVideoButton();});
    assert(await videoToggle.isChecked(),'An unavailable site must still show the default-on preference');
    assert(await videoToggle.isDisabled(),'A per-site preference cannot be edited without a site');
    assert.equal(await page.locator('#video-site-access').isVisible(),false);
    await page.screenshot({path:path.join(output,'video-default-unavailable-dark.png'),fullPage:true});
    await page.evaluate(async()=>{testState.videoPageUnavailable=false;await refreshVideoButton();});
    assert(await videoToggle.isChecked());assert(await page.locator('#video-site-access').isVisible());
    assert.equal(await page.evaluate(()=>testState.permissionRequests.length),1,'Opening options must not request website access');
    await page.screenshot({path:path.join(output,'video-access-dark.png'),fullPage:true});
    await page.evaluate(()=>{testState.grant=false;});await page.locator('#video-site-access').click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('Site access was not granted'));
    assert(await videoToggle.isChecked());assert(await page.locator('#video-site-access').isVisible());
    await page.evaluate(()=>{testState.grant=true;});await page.locator('#video-site-access').click();
    await page.waitForFunction(()=>document.querySelector('#video-site-access').hidden && !document.querySelector('#video-button').disabled);
    await videoToggle.uncheck();await page.waitForFunction(()=>!testState.videoEnabled && !document.querySelector('#video-button').disabled);
    await page.evaluate(()=>refreshVideoButton());assert.equal(await videoToggle.isChecked(),false);
    await page.evaluate(async()=>{testState.videoPageUnavailable=true;await refreshVideoButton();});
    assert(await videoToggle.isChecked(),'The default display must not inherit a previous site off choice');
    await page.evaluate(async()=>{testState.videoPageUnavailable=false;await refreshVideoButton();});
    assert.equal(await videoToggle.isChecked(),false,'Showing the default on an unavailable page must not overwrite a saved site choice');
    assert.match(await page.locator('#video-button-hint').textContent(),/choice is saved/);
    await page.screenshot({path:path.join(output,'video-off-dark.png'),fullPage:true});
    await page.evaluate(()=>{testState.grant=false;});
    await page.locator('#media-detection').click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('Media detection stays off'));
    assert.equal(await page.locator('#media-detection').isChecked(),false);
    await page.evaluate(()=>{testState.grant=true;});
    await page.locator('#media-detection').check();
    await page.waitForFunction(()=>testState.mediaDetection && !document.querySelector('#media-detection').disabled);
    assert.equal(await page.locator('#video-site-option').isVisible(),true);
    assert.equal(await videoToggle.isChecked(),false,'Media detection must not override a saved off choice');
    assert.match(await page.locator('#media-hint').textContent(),/Detection is on/);
    assert.deepEqual(await page.evaluate(()=>testState.permissionRequests.at(-1)),{permissions:['webRequest'],origins:['https://*/*','http://*/*']});
    const videoRequests=await page.evaluate(()=>testState.permissionRequests.length);
    await videoToggle.check();await page.waitForFunction(()=>testState.videoEnabled && !document.querySelector('#video-button').disabled);
    assert.equal(await page.evaluate(()=>testState.permissionRequests.length),videoRequests,'Video re-enable must reuse granted access');
    await page.screenshot({path:path.join(output,'video-default-dark.png'),fullPage:true});
    await page.emulateMedia({colorScheme:'light'});await page.screenshot({path:path.join(output,'video-default-light.png'),fullPage:true});
    await page.locator('#browser-options > summary').click();
    await page.evaluate(async()=>{testState.items=[{id:'stream',filename:'Nature documentary',url:'https://cdn.example.test/master.m3u8',hls:true,kind:'hls'},{id:'video',filename:'<img src=x onerror=alert(1)>.mp4',url:'https://cdn.example.test/clip.mp4',kind:'video',size:86402662},{id:'audio',filename:'Commentary.m4a',url:'https://cdn.example.test/audio.m4a',kind:'audio',size:13421772}];await refreshMedia();});
    assert.equal(await page.locator('.media-item').count(),3);assert.equal(await page.locator('img').count(),0);
    await page.evaluate(async()=>{testState.items[1].filename='Nature-documentary-4K.mp4';await refreshMedia();});
    await page.locator('.media-item').first().focus();
    assert(await page.evaluate(async()=>{const before=document.activeElement;testState.items[0].size=100;await refreshMedia();return document.activeElement===before && before.isConnected;}));
    await page.screenshot({path:path.join(output,'media-dark.png'),fullPage:true});
    await page.emulateMedia({colorScheme:'light'});await page.screenshot({path:path.join(output,'media-light.png'),fullPage:true});
    await page.locator('.media-item').first().click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('Nothing queued'));
    assert.deepEqual(await page.evaluate(()=>testState.messages.at(-1)),{action:'review_media',tabId:7,id:'stream'});
    await page.locator('#clear-media').click();await page.waitForFunction(()=>document.querySelectorAll('.media-item').length===0);
    await page.locator('#media-detection').uncheck();await page.waitForFunction(()=>!testState.mediaDetection);
    const grantsBeforeCycles=await page.evaluate(()=>testState.permissionRequests.length);
    await page.evaluate(()=>{testState.requestError='permissions.request may only be called from a user input handler';});
    for(let cycle=0;cycle<3;cycle++) {
      for(const [id,setting] of [['auto-capture','autoCapture'],['media-detection','mediaDetection']]) {
        await page.locator('#'+id).check();
        await page.waitForFunction(id=>!document.getElementById(id).disabled,id);
        assert.equal(await page.evaluate(setting=>testState[setting],setting),true,'Already granted access must survive an unavailable prompt API: '+id);
      }
      for(const id of ['auto-capture','media-detection'])await page.locator('#'+id).uncheck();
    }
    assert.equal(await page.evaluate(()=>testState.permissionRequests.length),grantsBeforeCycles,'Re-enabling reuses existing permissions');
    await page.reload();await toggle.waitFor({state:'visible'});
    // The permission store persists between popup openings in a real browser.
    await page.evaluate(async()=>{testState.grantedPermissions=['downloads','webRequest'];testState.grantedOrigins=['https://*/*','http://*/*'];testState.requestError='Unexpected permission prompt';await initialize();});
    await toggle.check();await page.waitForFunction(()=>testState.autoCapture && !document.querySelector('#auto-capture').disabled);
    await toggle.uncheck();
    await page.evaluate(async () => { testState.requestError=null;testState.grant = false;testState.grantedPermissions=[];testState.grantedOrigins=[];await refreshToggle();await refreshMedia(); });
    await toggle.click();
    await page.waitForFunction(() => document.querySelector("#status").textContent.includes("not granted"));
    assert.equal(await toggle.isChecked(), false);
    await page.evaluate(() => { testState.grant = true; testState.connected = false; });
    await toggle.click();
    await page.waitForFunction(() => document.querySelector("#status").textContent.includes("not connected"));
    assert.equal(await toggle.isChecked(), false);
    await page.locator("#manual-download summary").click();
    await page.locator("#url").fill("https://example.test/file.zip");
    await page.locator("#send").click();
    await page.waitForFunction(() => document.querySelector("#url").value === "");
    assert.equal(await page.evaluate(() => testState.messages.at(-1).action), "add");
    await page.locator("#url").fill("https://example.test/private.zip");
    assert.equal(await page.locator("#browser-login").isChecked(), false);
    await page.locator("#browser-login").check();
    await page.locator("#send").click();
    await page.waitForFunction(() => testState.messages.at(-1).action === "browser_login_open");
    assert.equal(await page.locator("#browser-login").isChecked(), false);
    await page.locator("#page-links").click();
    await page.waitForFunction(() => testState.messages.at(-1).action === "scan_page");
    await page.locator("#page-videos").click();
    await page.waitForFunction(() => testState.messages.at(-1).action === "scan_videos");
    await page.locator("#browser-options > summary").click();
    await page.evaluate(()=>{testState.connected=true;testState.holdPing=true;});
    await page.locator('#check').click();
    await page.waitForFunction(()=>typeof testState.finishPing==='function');
    assert.equal(await page.locator('#check').textContent(),'Checking…');assert(await page.locator('#check').isDisabled());
    assert.match(await page.locator('#connection-status').textContent(),/Connecting/);
    await page.evaluate(()=>testState.finishPing());
    await page.waitForFunction(()=>document.querySelector('#connection-status').textContent.includes('✓ Download Engine is connected'));
    assert(await page.locator('#connection-status').isVisible());
    await page.screenshot({path:path.join(output,'connection-success-light.png'),fullPage:true});
    await page.evaluate(()=>{testState.holdPing=false;testState.connected=false;});
    await page.locator('#check').click();
    await page.waitForFunction(()=>document.querySelector('#connection-status').dataset.error==='true');
    assert.match(await page.locator('#connection-status').textContent(),/not connected/);
    await page.emulateMedia({colorScheme:'dark'});
    await page.screenshot({path:path.join(output,'connection-error-dark.png'),fullPage:true});
    await page.evaluate(()=>{testState.pingError='Background connection interrupted';});
    await page.locator('#check').click();
    await page.waitForFunction(()=>document.querySelector('#connection-status').textContent.includes('Background connection interrupted'));
    assert.equal(await page.locator('#check').textContent(),'Check connection');assert.equal(await page.locator('#check').isDisabled(),false);
    await page.evaluate(()=>{testState.pingError=null;testState.connected=true;});
    await page.locator('#check').click();
    await page.waitForFunction(()=>document.querySelector('#connection-status').textContent.includes('✓ Download Engine is connected'));
    await page.locator("#recover").click();
    await page.waitForFunction(() => testState.messages.at(-1).action === "recover_capture");
    await page.locator("#manual-download summary").click();
    await page.locator("#browser-options > summary").click();
    const dimensions = await page.evaluate(() => ({
      width: document.documentElement.scrollWidth, viewport: innerWidth,
      bottom: document.querySelector("footer").getBoundingClientRect().bottom
    }));
    assert(dimensions.width <= dimensions.viewport);
    assert(dimensions.bottom < 590);
    assert.deepEqual(errors, []);
    await page.evaluate(async () => { testState.returnMarker={setting:"video",tabId:7};await initialize(); });
    assert.equal(await page.locator('#browser-options').getAttribute('open'), '');
    assert.equal(await page.evaluate(() => document.activeElement.id), 'video-button');
    assert.equal(await page.evaluate(() => testState.returnMarker), null);
    await page.evaluate(() => {
      testState.connected=true;testState.grantedPermissions=[];testState.grantedOrigins=[];
      chrome.extension={getBackgroundPage:() => ({requestPopupPermission:(message,requestInPopup) => {
        testState.directBackground={message,gesture:navigator.userActivation.isActive};
        return requestInPopup({permissions:["downloads"]}).then(granted => granted
          ? chrome.runtime.sendMessage({action:"configure_capture",enabled:true})
          : {ok:false,message:"Download access was not granted."});
      }})};
    });
    await page.evaluate(()=>refreshToggle());
    await toggle.check();await page.waitForFunction(() => testState.autoCapture);
    assert.equal(await page.evaluate(() => testState.directBackground.gesture),true);
    assert.equal(await page.evaluate(() => testState.permissionGesture),true);
    assert.deepEqual(await page.evaluate(() => testState.directBackground.message.returnTo),{windowId:2,tabId:7});
    await toggle.uncheck();
    await page.evaluate(async()=>{
      testState.grantedPermissions=[];
      chrome.extension.getBackgroundPage=()=>{throw Error('Permission bridge unavailable <img src=x onerror=alert(1)>');};
      await refreshToggle();
    });
    await toggle.click();await page.waitForFunction(()=>testState.lastPopupError && !document.querySelector('#auto-capture').disabled);
    assert.equal(await toggle.isChecked(),false);
    assert.equal(await page.evaluate(()=>testState.lastPopupError.setting),'capture');
    await page.locator('#diagnostics > summary').click();
    await page.waitForFunction(()=>document.getElementById('diagnostic-log').value.includes('Permission bridge unavailable'));
    assert.equal(await page.locator('#diagnostics img').count(),0);
    await page.locator('#select-log').click();
    assert(await page.evaluate(()=>{const field=document.getElementById('diagnostic-log');return document.activeElement===field && field.selectionStart===0 && field.selectionEnd===field.value.length;}));
    await page.screenshot({path:path.join(output,'diagnostic-log-light.png'),fullPage:true});
    await page.emulateMedia({colorScheme:'dark'});await page.screenshot({path:path.join(output,'diagnostic-log-dark.png'),fullPage:true});
    await page.locator('#clear-log').click();
    await page.waitForFunction(()=>document.getElementById('diagnostic-log').value==='No activity recorded yet.');
    assert.equal(await page.evaluate(()=>testState.lastPopupError),null);
    assert.deepEqual(errors,[]);
    await page.evaluate(async () => {
      window.policyViolations = [];
      addEventListener("securitypolicyviolation", e => policyViolations.push(e.effectiveDirective));
      const injected = document.createElement("script");
      injected.textContent = "window.inlineAttackExecuted = true";
      document.head.append(injected);
      try { await fetch("https://blocked.invalid/security-probe"); } catch {}
    });
    await page.waitForFunction(() => policyViolations.includes("script-src-elem") && policyViolations.includes("connect-src"));
    assert.equal(await page.evaluate(() => window.inlineAttackExecuted), undefined);
    const result = { ok: true, checks: ["capture and detection default off; video button defaults on", "enable and permission request", "disable", "connection progress, success, failure, exception and retry are visible beside the button", "video checkbox stays visible and independent of media detection", "default-on does not prompt without a user gesture", "site permission denial keeps preference on and shows Allow this site", "explicit off persists through refresh and media enable", "video re-enable reuses granted access",
      "denied permission", "disconnected desktop", "manual URL", "explicit browser login review", "page links button", "direct videos button", "recovery button", "light/dark layout", "diagnostic log is visible without expanding options or scrolling", "optional media grant and denial", "media names are text only", "media refresh preserves keyboard focus", "media review sends only tab and stored ID", "clear and disable detection", "three off/on cycles reuse granted access when the prompt API rejects", "granted access is restored on popup reopen", "return restores expanded options and checkbox focus", "direct background dispatch retains the click stack", "popup exceptions persist in the local diagnostic log", "diagnostic text is selectable, text-only and clearable", "no console exceptions", "inline script blocked by CSP", "web fetch blocked by CSP"],
      source: extensionRoot,
      unavailableSiteDefaultChecked: true,
      renderer: browser.version(),
      limitation: "An installed Chromium browser renders the real popup; extension APIs are simulated." };
    fs.writeFileSync(path.join(output, "result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify({ ...result, output }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });


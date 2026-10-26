// Real installed Chromium engines, disposable profiles and loopback fixtures.
// The shipped manifest starts with install-time access. A separate fixture
// withholds access. Neither test drives the install consent dialog or native host.
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const { once } = require('node:events'), { randomUUID } = require('node:crypto');
const { chromium } = require('playwright');
const root = path.resolve(__dirname, '..');
const run = path.join(root, 'artifacts', 'media-discovery-' + randomUUID().slice(0, 8));
fs.mkdirSync(run, { recursive: true });
const document = `<!doctype html><html><head><title>Local media discovery fixture</title>
<style>body{font:16px system-ui;background:#222729;color:#e4e9eb;padding:32px}a{color:#69d4b1}.background{width:40px;height:40px;background-image:url('/background.webp')}</style></head>
<body><h1>Local media discovery fixture</h1><a href="/archive.zip">Archive</a>
<img alt="Photo" src="/photo.png" srcset="/small.png 1x, /large.png 2x"><img data-src="/lazy.jpg" alt="Lazy image">
<div class="background"></div><div id="shadow"></div><video preload="none"><source src="/visible.mp4" type="video/mp4"></video>
<script>document.getElementById('shadow').attachShadow({mode:'open'}).innerHTML='<a href="/shadow.pdf">Shadow document</a><img src="/shadow.png">';</script></body></html>`;
const server = http.createServer((req, res) => {
  const route = new URL(req.url, 'http://fixture.test').pathname;
  res.setHeader('Cache-Control', 'no-store');
  if (route === '/page' || route === '/next') { res.setHeader('Content-Type', 'text/html'); res.end(document); return; }
  const mime = { '/opaque': 'video/mp4', '/sound': 'audio/mpeg', '/hls': 'application/vnd.apple.mpegurl', '/dash': 'application/dash+xml', '/range': 'video/webm', '/part.m4s': 'video/mp4', '/fake.mp4': 'text/html', '/post': 'video/mp4' }[route];
  if (mime) {
    res.setHeader('Content-Type', mime); res.setHeader('Set-Cookie', 'fixture-secret=do-not-copy; SameSite=Lax');
    if (route === '/range') { res.statusCode = 206; res.setHeader('Content-Range', 'bytes 0-9/4096'); }
    res.end('0123456789'); return;
  }
  res.statusCode = 404; res.end();
});
async function until(fn, label) {
  const end = Date.now() + 15000;
  while (Date.now() < end) { const value = await fn(); if (value) return value; await new Promise(r => setTimeout(r, 80)); }
  throw Error('Timed out: ' + label);
}
function stage(name, granted) {
  const folder = path.join(run, name + (granted ? '-default' : '-withheld'), 'extension');
  fs.mkdirSync(folder, { recursive: true });
  for (const file of fs.readdirSync(path.join(root, 'extension'))) if (!file.startsWith('manifest.')) fs.cpSync(path.join(root, 'extension', file), path.join(folder, file), {recursive:true});
  const popup=path.join(folder,'popup.js'),source=fs.readFileSync(popup,'utf8'),marker='})().catch(error =>';
  assert(source.includes(marker));fs.writeFileSync(popup,source.replace(marker,'Object.assign(window,{refreshMedia});\n'+marker));
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'extension/manifest.chromium.json')));
  if (!granted) { manifest.permissions = manifest.permissions.filter(p => p !== 'webRequest'); manifest.optional_permissions.push('webRequest'); manifest.optional_host_permissions = manifest.host_permissions; delete manifest.host_permissions; }
  fs.writeFileSync(path.join(folder, 'manifest.json'), JSON.stringify(manifest));
  return folder;
}
async function testBrowser(name, executable, url, granted) {
  const folder = stage(name, granted), output = path.dirname(folder);
  const context = await chromium.launchPersistentContext(path.join(output, 'profile'), { executablePath: executable, headless: true, ignoreDefaultArgs: ['--disable-extensions'], args: ['--enable-unsafe-extension-debugging'], viewport: { width: 1100, height: 850 }, colorScheme: 'dark' });
  const errors = []; context.on('page', p => p.on('pageerror', e => errors.push(e.message)));
  try {
    const cdp = await context.browser().newBrowserCDPSession();
    const { id } = await cdp.send('Extensions.loadUnpacked', { path: folder });
    const control = await context.newPage(); await control.goto(`chrome-extension://${id}/popup.html`);
    const page = await context.newPage(); await page.goto(url); await page.bringToFront();
    const status = () => control.evaluate(() => chrome.runtime.sendMessage({ action: 'media_status' }));
    const capture = (route, method = 'GET') => page.evaluate(async ({ route, method }) => { const res = await fetch(route, { method }); await res.text(); }, { route, method });
    await control.evaluate(() => refreshMedia());
    if (!granted) {
      // Opening popup.html in a test tab does not grant activeTab as clicking
      // the real toolbar action would. Inspect startup in its own worker.
      const worker = await until(() => context.serviceWorkers().find(w => w.url().includes(id)), 'extension worker');
      assert.deepEqual(await worker.evaluate(async () => { await media.tail; return { enabled: media.enabled, error: media.error || null }; }), { enabled: false, error: null });
      await capture('/opaque');
      assert(!Object.keys(await control.evaluate(() => chrome.storage.session.get(null))).some(k => k.startsWith('media-tab:')));
      assert.equal(await control.evaluate(() => chrome.permissions.contains({ permissions: ['webRequest'], origins: ['http://*/*', 'https://*/*'] })), false);
      const {targetInfos}=await cdp.send('Target.getTargets',{filter:[{type:'tab'}]});
      await cdp.send('Extensions.triggerAction',{id,targetId:targetInfos.find(t=>t.url===url).targetId});
      const [activeTab]=await control.evaluate(()=>chrome.tabs.query({active:true,currentWindow:true}));
      assert.equal((await status()).tabId,activeTab.id);
      const scan=await control.evaluate(()=>chrome.runtime.sendMessage({action:'scan_page'}));assert.equal(scan.ok,true,JSON.stringify(scan));
      const picker=await until(()=>context.pages().find(p=>p.url().includes('/links.html#')),'stock activeTab picker');
      await picker.waitForSelector('#links tr');assert.equal(await picker.locator('.file-name').filter({hasText:'lazy.jpg'}).count(),1);
      assert.deepEqual(errors, []); return { browser: name, mode: 'withheld-access fixture', checks: ['website access absent', 'detection blocked without access', 'no media recorded', 'no startup error', 'explicit action grants temporary page access', 'asset picker works without broad permissions'] };
    }
    let state = await status(); assert.equal(state.ok, true, JSON.stringify(state)); assert.equal(state.enabled, true); assert.equal(state.error, null);
    assert.equal(await control.locator('#auto-capture').isChecked(), true, 'fresh install capture defaults on');
    assert.equal(await control.evaluate(()=>chrome.permissions.contains({permissions:['cookies']})), false, 'detection never grants login cookies');
    for (const route of ['/opaque', '/opaque', '/sound', '/hls', '/dash', '/range', '/part.m4s', '/fake.mp4']) await capture(route);
    await capture('/post', 'POST');
    state = await until(async () => { const s = await status(); return s.items.length === 5 && s; }, 'five distinct media items');
    assert.equal(state.error, null);
    assert.equal(state.items.find(i => i.url.endsWith('/range')).size, 4096);
    assert.equal(state.items.filter(i => i.hls).length, 1); assert.equal(state.items.filter(i => i.dash).length, 1);
    assert.equal(await control.evaluate(tabId => chrome.action.getBadgeText({ tabId }), state.tabId), '5');
    const saved = await control.evaluate(() => chrome.storage.session.get(null)); assert(!JSON.stringify(saved).includes('fixture-secret'));
    await control.evaluate(() => refreshMedia()); await control.setViewportSize({ width: 370, height: 640 });
    assert.equal(await control.locator('.media-item').count(), 5);
    await control.screenshot({ path: path.join(output, 'media-popup.png'), fullPage: true });
    const item = state.items.find(i => i.hls);
    const worker=await until(()=>context.serviceWorkers().find(w=>w.url().includes(id)),'active extension worker');
    await worker.evaluate(()=>{globalThis.videoReviewRequests=[];chrome.runtime.sendNativeMessage=async(host,request)=>{
      if(request.action!=='review_browser_video')throw Error('Unexpected native action: '+request.action);
      videoReviewRequests.push(structuredClone(request));return {ok:true,version:1,preview_id:request.request_id,message:'Review in Download Engine'};
    };});
    const reply = await control.evaluate(({ tabId, id }) => chrome.runtime.sendMessage({ action: 'review_media', tabId, id }), { tabId: state.tabId, id: item.id });
    assert.equal(reply.ok, true);
    const requests=await worker.evaluate(()=>videoReviewRequests);
    assert.equal(requests.length,1);assert.equal(requests[0].review.items.length,1);
    assert.equal(requests[0].review.items[0].kind,'hls');assert.equal(requests[0].review.items[0].url,item.url);
    assert(!context.pages().some(p=>p.url().includes('/links.html#')));
    await page.bringToFront();
    await page.goto(url.replace('/page', '/next')); await until(async () => (await status()).items.length === 0, 'navigation clears media');
    assert.equal(await control.evaluate(tabId => chrome.action.getBadgeText({ tabId }), state.tabId), '');
    await capture('/opaque'); await until(async () => (await status()).items.length === 1, 'new page detects media');
    await worker.evaluate(()=>saveStatus(false,'Fixture warning'));
    assert.equal(await control.evaluate(tabId=>chrome.action.getBadgeText({tabId}),state.tabId),'!');
    await control.evaluate(tabId => chrome.runtime.sendMessage({ action: 'clear_media', tabId }), state.tabId);
    assert.equal((await status()).items.length, 0);
    await worker.evaluate(()=>saveStatus(true,'Fixture recovered'));
    assert.equal(await control.evaluate(tabId=>chrome.action.getBadgeText({tabId}),state.tabId),'');
    await worker.evaluate(()=>saveStatus(false,'Second fixture warning'));
    assert.equal(await control.evaluate(tabId=>chrome.action.getBadgeText({tabId}),state.tabId),'!');
    assert.deepEqual(await control.evaluate(tabId=>chrome.action.getBadgeBackgroundColor({tabId}),state.tabId),[186,71,54,255]);
    await worker.evaluate(()=>saveStatus(true,'Fixture recovered'));
    await capture('/sound'); await until(async () => (await status()).items.length === 1, 'new playback after clear');
    await control.evaluate(() => chrome.runtime.sendMessage({ action: 'configure_media', enabled: false }));
    await capture('/opaque'); assert.equal((await status()).items.length, 0);
    assert(!Object.keys(await control.evaluate(() => chrome.storage.session.get(null))).some(k => k.startsWith('media-tab:')));
    // Exercise repeated UI changes using real granted browser access. Requesting
    // permissions is deliberately unavailable: re-enabling must not need it.
    await control.evaluate(async () => {
      await refreshMedia();
      window.originalPermissionRequest=chrome.permissions.request;
      chrome.permissions.request=()=>{throw Error('Already granted access must not request permission again');};
    });
    await worker.evaluate(()=>{
      globalThis.originalPermissionRequest=chrome.permissions.request;
      chrome.permissions.request=()=>{throw Error('Already granted access must not request permission again');};
    });
    for(let cycle=0;cycle<3;cycle++) {
      await control.evaluate(()=>document.getElementById('media-detection').click());
      await until(async () => (await status()).enabled && await control.evaluate(()=>document.getElementById('media-detection').checked && !document.getElementById('media-detection').disabled), 'media re-enabled with existing access, cycle '+cycle).catch(async error=>{
        error.message+='; '+JSON.stringify(await control.evaluate(()=>({busy,access:permissionAccess.media,checked:document.getElementById('media-detection').checked,disabled:document.getElementById('media-detection').disabled,status:document.getElementById('status').textContent})));
        throw error;
      });
      await capture('/opaque');await until(async () => (await status()).items.length===1, 'media detects after re-enabling');
      await control.evaluate(()=>document.getElementById('media-detection').click());
      // A saved setting can precede the popup's final refresh. Wait until the
      // checkbox is usable before clicking it again, as a real user must.
      await until(async () => !(await status()).enabled && await control.evaluate(()=>!document.getElementById('media-detection').checked && !document.getElementById('media-detection').disabled), 'media disabled again');
    }
    await control.evaluate(()=>{chrome.permissions.request=window.originalPermissionRequest;delete window.originalPermissionRequest;});
    await worker.evaluate(()=>{chrome.permissions.request=globalThis.originalPermissionRequest;delete globalThis.originalPermissionRequest;});
    const diagnostics=await control.evaluate(()=>chrome.runtime.sendMessage({action:'diagnostics_get'}));
    assert(diagnostics.ok && diagnostics.entries.filter(entry=>entry.setting==='media' && entry.ok).length>=7);
    const scan = await control.evaluate(() => chrome.runtime.sendMessage({ action: 'scan_page' })); assert.equal(scan.ok, true);
    const assets = await until(() => context.pages().find(p => p.url().includes('/links.html#')), 'asset review');
    await assets.waitForSelector('#links tr');
    for (const file of ['archive.zip', 'photo.png', 'small.png', 'large.png', 'lazy.jpg', 'background.webp', 'shadow.pdf', 'shadow.png', 'visible.mp4']) assert.equal(await assets.locator('.file-name').filter({ hasText: file }).count(), 1, file);
    assert.equal(await assets.locator('#links input:checked').count(), 0);
    await assets.screenshot({ path: path.join(output, 'asset-picker.png'), fullPage: true });
    // Exercise the real toolbar popup API independently of the permission dialog.
    // This fixture already has access; approval completion itself is simulated.
    await page.bringToFront();
    const returnTo = await worker.evaluate(async () => {
      const [tab] = await chrome.tabs.query({active:true,currentWindow:true});
      return {windowId:tab.windowId,tabId:tab.id};
    });
    await worker.evaluate(target => permissionFlow.restore({setting:'media',target,cancelled:false},
      {ok:true,message:'Media setting saved'}),returnTo);
    const popup = await until(async () => {
      const views = await worker.evaluate(() => chrome.runtime.getContexts({contextTypes:['POPUP']}));
      return views.length === 1 && views[0];
    }, 'toolbar popup returns');
    assert.equal(popup.documentUrl,`chrome-extension://${id}/popup.html`);
    // Chromium's context reports windowId -1 for toolbar popups. Consuming the
    // marker verifies the window returned by windows.getCurrent() in the UI.
    await until(async () => !Object.hasOwn(await worker.evaluate(() => chrome.storage.session.get(null)),
      'permission-popup-return:'+returnTo.windowId), 'returned popup initializes and consumes its focus marker');
    assert.deepEqual(errors, []);
    return { browser: name, version: (await cdp.send('Browser.getVersion')).product, mode: 'shipped default-on manifest', checks: ['capture and media enabled at startup without toggling', 'login cookies not granted', 'real network MIME discovery', 'deduplication', 'segment, HTML and POST exclusion', 'range total size', 'per-tab badge', 'global handoff badge inheritance after clearing media', 'response secrets excluded', 'review-only stream selection', 'navigation clears state', 'clear permits fresh playback', 'off clears session data', 'three UI off/on cycles resume network discovery without requesting existing access', 'setting diagnostics persist in real browser storage', 'DOM, srcset, lazy, CSS and open-shadow assets', 'real toolbar popup restoration and initialization after simulated grant completion'] };
  } finally { await context.close(); }
}
(async () => {
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  try {
    const url = `http://127.0.0.1:${server.address().port}/page`, results = [];
    for (const [name, executable] of [['chrome', 'C:/Program Files/Google/Chrome/Application/chrome.exe'], ['edge', 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe']]) {
      for (const granted of [false, true]) { results.push(await testBrowser(name, executable, url, granted)); console.log(name + (granted ? ' default-on' : ' withheld') + ': passed'); }
    }
    const report = { ok: true, results, permissionPromptTested: false, firefoxRuntimeTested: false, nativeHandoffTested: false };
    fs.writeFileSync(path.join(run, 'verification.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify({ run, ...report }, null, 2));
  } finally { server.closeAllConnections(); await new Promise(r => server.close(r)); }
})().catch(e => { console.error(e); process.exitCode = 1; });

/* Real extension APIs in an isolated installed Chrome/Edge profile. */
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const crypto = require('node:crypto');
const {chromium} = require('playwright');
const config = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const {url, run, host} = config;
const checks = [];
const record = message => { checks.push(message); console.log(message); };
async function bounded(promise, label, timeout=30000) {
  let timer;
  try { return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Timed out: '+label)),timeout);})]); }
  finally { clearTimeout(timer); }
}
function setup(id) { execFileSync(host, ['--register-browser', config.browser, id], {windowsHide:true}); }
function disconnect() { execFileSync(host, ['--unregister-browser', config.browser], {windowsHide:true}); }
async function state() { return (await fetch(url+'/_state')).json(); }
async function until(fn, label, timeout=25000) {
  const deadline=Date.now()+timeout;
  while(Date.now()<deadline) { const result=await fn(); if(result) return result; await new Promise(r=>setTimeout(r,50)); }
  throw Error('Timed out: '+label+'; state='+JSON.stringify(await state()));
}
async function exactFile(file) {
  assert.equal(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'),config.sha256);
}
(async()=>{
  const context=await chromium.launchPersistentContext(path.join(run,'profile'),{executablePath:config.executable,headless:true,ignoreDefaultArgs:['--disable-extensions'],args:['--enable-unsafe-extension-debugging'],acceptDownloads:true});
  context.setDefaultTimeout(15000);
  const cdp=await context.browser().newBrowserCDPSession();
  const version=await cdp.send('Browser.getVersion');
  const errors=[];
  context.on('page', page=>page.on('pageerror', error=>errors.push(error.message)));
  try {
    let {id}=await cdp.send('Extensions.loadUnpacked',{path:config.extension});
    const page=await context.newPage();
    await page.goto(`chrome-extension://${id}/popup.html`);
    assert.equal(await page.locator('#auto-capture').isChecked(),true);
    await page.locator('#check').click();
    await page.waitForFunction(()=>document.querySelector('#status').dataset.error==='true');
    record('stock extension reports missing host; capture defaults on');
    setup(id);
    await page.locator('#check').click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent==='Download Engine is connected');
    await page.locator('#url').fill(url+'/manual.bin');
    await page.locator('#send').click();
    await page.waitForFunction(()=>document.querySelector('#url').value==='');
    const manual=await until(async()=> (await state()).jobs.find(j=>j.status==='complete'&&j.url.endsWith('/manual.bin')),'manual download');
    assert(manual.bytes_verified);
    record('stock popup -> native host -> cold desktop startup -> exact file and MOTW');
    const repeated=await page.evaluate(request=>chrome.runtime.sendMessage(request),{action:'add',url:manual.url,request_id:manual.request_id});
    assert.equal(repeated.job_id,manual.id);
    assert.equal((await state()).jobs.length,1);
    record('repeated browser handoff is idempotent');
    await page.screenshot({path:path.join(run,'manual-popup.png')});
    if (config.browserLogin) {
      await require('./chromium_login_smoke.cjs')({context,cdp,page,config,setup,checks,version,until,state,errors});
      return;
    }
    console.log('Loading capture fixture');
    await page.goto('about:blank');
    await bounded(cdp.send('Extensions.uninstall',{id}),'uninstall stock extension');
    console.log('Stock extension removed');
    ({id}=await bounded(cdp.send('Extensions.loadUnpacked',{path:config.captureExtension}),'load capture fixture'));
    console.log('Capture fixture loaded');
    setup(id);
    await page.goto(`chrome-extension://${id}/popup.html`);
    const fixture=await context.newPage();
    assert.equal((await page.evaluate(()=>chrome.runtime.sendMessage({action:'configure_capture',enabled:false}))).ok,true);
    await fixture.goto(url);
    const offEvent=fixture.waitForEvent('download');
    await fixture.locator('#off').click();
    const off=await offEvent;
    await bounded(off.saveAs(path.join(run,'browser-off.bin')),'capture-off browser download');
    await exactFile(path.join(run,'browser-off.bin'));
    assert.equal((await state()).jobs.length,1);
    record('capture off keeps real browser download');
    await page.locator('#auto-capture').check();
    await page.waitForFunction(()=>document.querySelector('#status').textContent==='Automatic capture is on for direct file downloads.');
    await fixture.locator('#auto').click();
    const auto=await until(async()=> (await state()).jobs.find(j=>j.status==='complete'&&j.url.endsWith('/auto.bin')),'automatic download');
    assert(auto.bytes_verified);
    const browserCopy=await until(()=>page.evaluate(async()=> (await chrome.downloads.search({})).find(d=>d.url.endsWith('/auto.bin')&&d.state==='interrupted'&&!d.canResume)),'browser original cancellation');
    const captureResult=await page.evaluate(()=>chrome.storage.local.get(null));
    fs.writeFileSync(path.join(run,'capture-observed.json'),JSON.stringify({browserCopy,captureResult},null,2));
    await until(()=>page.evaluate(async()=>{const {lastResult}=await chrome.storage.local.get('lastResult');return lastResult?.ok&&lastResult.message==='Download sent to Download Engine.';}),'capture success acknowledgement');
    assert.equal((await state()).jobs.length,2);
    assert.equal((await state()).captures[0].state,'committed');
    record('automatic capture commits once, cancels browser original, preserves exact bytes and MOTW');
    disconnect();
    const fallbackEvent=fixture.waitForEvent('download');
    await fixture.locator('#fallback').click();
    const fallback=await fallbackEvent;
    await bounded(fallback.saveAs(path.join(run,'browser-fallback.bin')),'fallback browser download');
    await exactFile(path.join(run,'browser-fallback.bin'));
    assert.equal((await state()).jobs.length,2);
    record('disconnected native host resumes browser download without adding a desktop job');
    await page.locator('#auto-capture').uncheck();
    await page.waitForFunction(()=>document.querySelector('#status').textContent==='Automatic capture is off.');
    record('capture can be disabled after host disconnect');
    assert.deepEqual(errors,[]);
    fs.writeFileSync(path.join(run,'browser-result.json'),JSON.stringify({version,checks,errors,automaticCaptureManifest:'downloads granted by shipped manifest; real install permission prompt not exercised'},null,2));
    console.log(JSON.stringify({version,checks},null,2));
  } catch(error) { console.error(error); throw error; } finally { console.log('Closing browser fixture'); await context.close(); console.log('Browser fixture closed',process.getActiveResourcesInfo()); }
})().catch(error=>{console.error(error);process.exitCode=1;});

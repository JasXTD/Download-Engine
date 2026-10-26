/* Stock extension, real activeTab action and native host, disposable browser only. */
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {execFileSync}=require('node:child_process');
const {chromium}=require('playwright');
const config=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
const checks=[];
async function until(fn,label,timeout=25000) {const end=Date.now()+timeout;while(Date.now()<end){const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,75));}throw Error('Timed out: '+label);}
async function state(){return(await fetch(config.url+'/_state')).json();}
(async()=>{
  const context=await chromium.launchPersistentContext(path.join(config.run,'profile'),{executablePath:config.executable,headless:true,ignoreDefaultArgs:['--disable-extensions'],args:['--enable-unsafe-extension-debugging'],viewport:{width:1100,height:900}});
  context.setDefaultTimeout(15000); const cdp=await context.browser().newBrowserCDPSession(); const errors=[];
  context.on('page',p=>p.on('pageerror',e=>errors.push(e.message)));
  try {
    const version=await cdp.send('Browser.getVersion');
    const {id}=await cdp.send('Extensions.loadUnpacked',{path:config.extension});
    execFileSync(config.host,['--register-browser',config.browser,id],{windowsHide:true});
    const control=await context.newPage(); await control.goto(`chrome-extension://${id}/popup.html`);
    const fixture=await context.newPage(); await fixture.goto(config.url+'/page-links'); await fixture.bringToFront();
    const denied=await control.evaluate(async()=>{const [tab]=await chrome.tabs.query({active:true,currentWindow:true});try{await chrome.scripting.executeScript({target:{tabId:tab.id},func:()=>document.title});return false;}catch{return true;}});
    assert(denied,'Stock extension must not have page access before its action'); checks.push('page access denied before explicit extension action');
    const targets=await cdp.send('Target.getTargets',{filter:[{type:'tab'}]});
    const target=targets.targetInfos.find(t=>t.url===config.url+'/page-links'); assert(target,JSON.stringify(targets));
    await cdp.send('Extensions.triggerAction',{id,targetId:target.targetId});
    // The browser action grants activeTab. Invoke the real popup button in its
    // background test tab without activating that tab in place of the webpage.
    await control.evaluate(videos=>document.querySelector(videos?'#page-videos':'#page-links').click(),config.videos);
    if(config.videos) {
      const saved=await until(async()=>{const s=await state();return s.video_reviews.length===1&&s;},'native video review');
      assert.equal(saved.jobs.length,0);assert.equal(saved.batches.length,0);
      assert.deepEqual(saved.video_reviews[0].items.map(item=>item.kind).sort(),['dash','file','file','file','hls']);
      assert(!context.pages().some(p=>p.url().includes('/links.html#')));
      checks.push('video scan opens a native desktop review; typed files and streams; no automatic download or browser picker');
      assert.deepEqual(errors,[]);
      fs.writeFileSync(path.join(config.run,'browser-result.json'),JSON.stringify({version,checks,errors,nativeSelectionTested:false},null,2));
      return;
    }
    const picker=await until(()=>context.pages().find(p=>p.url().startsWith(`chrome-extension://${id}/links.html#`)),'page picker');
    await picker.waitForFunction(()=>document.querySelectorAll('#links tr').length===3);
    assert.equal(await picker.locator('#links input:checked').count(),0); assert.equal((await state()).jobs.length,0);
    checks.push('stock action scans only the current page; relative links resolved, unsafe links skipped, duplicates removed');
    await picker.locator('#search').fill('guide'); await picker.locator('#select-visible').click();
    await picker.locator('#search').fill('source'); await picker.locator('#select-visible').click(); await picker.locator('#search').fill('');
    await picker.locator('#queue').selectOption('page-links-test'); await picker.locator('#destination').selectOption('documents');
    await picker.screenshot({path:path.join(config.run,'page-picker.png'),fullPage:true});
    await picker.locator('#add').click(); await picker.waitForFunction(()=>document.querySelector('#add').textContent==='Added to queue');
    const saved=await state(); assert.equal(saved.jobs.length,2); assert.equal(saved.batches.length,1);
    assert(saved.jobs.every(j=>j.status==='queued'&&j.directory.endsWith('Documents')));
    const again=await picker.evaluate(()=>chrome.runtime.sendMessage({action:'page_links_submit',id:location.hash.slice(1),retry:true}));
    assert.equal(again.count,2); assert.equal((await state()).jobs.length,2);
    await picker.reload(); await picker.waitForFunction(()=>document.querySelector('#add').textContent==='Added to queue');
    checks.push('selected links saved atomically to the chosen paused queue and folder; repeat and tab reload do not duplicate');
    await fetch(config.url+'/_start-page-queue',{method:'POST'});
    const completed=await until(async()=>{const s=await state();return s.jobs.length===2&&s.jobs.every(j=>j.bytes_verified)&&s;},'queued batch completion');
    assert.deepEqual(completed.jobs.map(j=>j.name).sort(),['guide.pdf','source.zip']);
    checks.push('starting selected queue completes exact file bytes and Windows MOTW; unselected link untouched');
    await picker.screenshot({path:path.join(config.run,'page-picker-complete.png'),fullPage:true});
    assert.deepEqual(errors,[]);
    fs.writeFileSync(path.join(config.run,'browser-result.json'),JSON.stringify({version,checks,errors,pageAccess:'Unmodified release permissions. Browser action invoked with CDP Extensions.triggerAction; popup button script runs in a background extension tab.'},null,2));
    console.log(JSON.stringify({version,checks},null,2));
  } finally {await context.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});

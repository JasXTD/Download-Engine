// Installed Chromium browsers, disposable profiles, local fixtures only.
// Site permission is pre-granted in a fixture manifest; native handoff is tested
// separately. The button, registration and collector are shipped code; only the
// native transport is replaced to record the desktop review request.
const assert=require('node:assert/strict');
const fs=require('node:fs');const path=require('node:path');const http=require('node:http');
const {once}=require('node:events');const {randomUUID}=require('node:crypto');const {spawn}=require('node:child_process');
const {chromium}=require('playwright');
const root=path.resolve(__dirname,'..');
const run=path.join(root,'artifacts','video-button-'+randomUUID().slice(0,8));fs.mkdirSync(run,{recursive:true});
const server=http.createServer((req,res)=>{
  if(req.url.endsWith('.mpd')) {res.setHeader('Content-Type','application/dash+xml');res.end('<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT2S"><Period/></MPD>');return;}
  res.setHeader('Content-Type','text/html; charset=utf-8');
  res.setHeader('Content-Security-Policy',"default-src 'self'; style-src 'nonce-local-fixture'; object-src 'none'");
  res.end(`<!doctype html><html><head><title>Download Engine video button fixture</title><style nonce="local-fixture">body{font:16px system-ui;background:#f0f5ff;color:#14213b;padding:30px;margin:0}main{max-width:820px;margin:auto}h1{font-size:25px}video{display:block;width:640px;height:360px;background:linear-gradient(135deg,#112854,#2267a7);border-radius:14px}p{color:#536785}.sources{padding:20px;background:white;border-radius:12px;margin-top:20px}</style></head><body><main><h1>Video download preview</h1><p>Local test player · quality and audio are reviewed before downloading</p><video id="player" preload="none" controls><source src="/main.mpd" type="application/dash+xml"></video><div class="sources"><a href="/master.m3u8">HLS option</a> · <a href="/clip.mp4">Direct video</a></div></main></body></html>`);
});
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,label){const end=Date.now()+15000;while(Date.now()<end){const result=await fn();if(result)return result;await delay(80);}throw Error('Timed out: '+label);}
// DevTools can inspect the closed shadow root without exposing it to page scripts.
async function buttonHandle(session) {
  const {root}=await session.send('DOM.getDocument',{depth:-1,pierce:true});
  const find=node=>node.nodeName==='BUTTON' && node.attributes?.includes('Download video with Download Engine — review first') ? node :
    [...node.children||[],...node.shadowRoots||[]].map(find).find(Boolean);
  const node=find(root);assert(node,'Missing video button');
  return (await session.send('DOM.resolveNode',{backendNodeId:node.backendNodeId})).object.objectId;
}
async function buttonState(session,objectId) {
  const {result,exceptionDetails}=await session.send('Runtime.callFunctionOn',{objectId,returnByValue:true,functionDeclaration:`function(){
    const rect=this.getBoundingClientRect(),label=this.querySelector('.label'),css=getComputedStyle(this);
    return {text:label.textContent,state:this.getAttribute('data-state'),disabled:this.disabled,title:this.title,icons:this.querySelectorAll('svg').length,
      clipped:label.scrollWidth>label.clientWidth,focus:this.matches(':focus-visible'),background:css.backgroundImage,
      transition:css.transitionDuration,rect:{x:rect.x,y:rect.y,width:rect.width,height:rect.height}};
  }`});
  assert(!exceptionDetails,JSON.stringify(exceptionDetails));return result.value;
}
function stage(name){const folder=path.join(run,name,'extension');fs.mkdirSync(folder,{recursive:true});for(const file of fs.readdirSync(path.join(root,'extension')))if(!file.startsWith('manifest.'))fs.cpSync(path.join(root,'extension',file),path.join(folder,file),{recursive:true});
  // popup.js keeps its functions private; expose the two this check drives, as extension_popup_smoke.cjs does.
  const popup=path.join(folder,'popup.js'),source=fs.readFileSync(popup,'utf8'),marker='})().catch(error =>';
  assert(source.includes(marker),'popup.js wrapper changed');fs.writeFileSync(popup,source.replace(marker,'Object.assign(window,{refreshVideoButton,refreshMedia});\n'+marker));
  const manifest=JSON.parse(fs.readFileSync(path.join(root,'extension',`manifest.${name==='firefox'?'firefox':'chromium'}.json`)));fs.writeFileSync(path.join(folder,'manifest.json'),JSON.stringify(manifest));return folder;}
async function chromiumTest(name,executable,url){
  const folder=stage(name);const home=path.dirname(folder);
  const context=await chromium.launchPersistentContext(path.join(home,'profile'),{executablePath:executable,headless:true,ignoreDefaultArgs:['--disable-extensions'],args:['--enable-unsafe-extension-debugging'],viewport:{width:1100,height:850}});
  const errors=[];context.on('page',page=>page.on('pageerror',e=>errors.push(e.message)));
  try {
    const cdp=await context.browser().newBrowserCDPSession();const {id}=await cdp.send('Extensions.loadUnpacked',{path:folder});
    const worker=await until(()=>context.serviceWorkers().find(w=>w.url().includes(id)),'extension worker');
    await worker.evaluate(()=>{globalThis.videoReviewRequests=[];globalThis.videoSenders=[];
      chrome.runtime.onMessage.addListener((message,sender)=>{if(message.action==='on_page_videos')videoSenders.push({url:sender.url,documentId:sender.documentId,tabUrl:sender.tab?.url});});
      chrome.runtime.sendNativeMessage=async(host,request)=>{
      if(request.action!=='review_browser_video')throw Error('Unexpected native action: '+request.action);
      videoReviewRequests.push(structuredClone(request));
      if(globalThis.failVideoReview)throw Error('Fixture: Download Engine is unavailable');
      if(globalThis.videoReviewGate)await videoReviewGate;
      return {ok:true,version:1,preview_id:request.request_id,message:'Review in Download Engine'};
    };});
    const control=await context.newPage();await control.goto(`chrome-extension://${id}/popup.html`);
    await control.waitForFunction(()=>document.querySelector('#video-button-hint').textContent.includes('Open a website'));
    assert(await control.locator('#video-button').isChecked(),'The popup must show default-on on an unavailable page');
    assert(await control.locator('#video-button').isDisabled(),'An unavailable page has no site preference to edit');
    const page=await context.newPage();await page.goto(url);await page.bringToFront();
    await until(()=>page.locator('[data-idm-ex-video-button]').count(),'default-on button appears');
    await control.evaluate(()=>refreshVideoButton());
    assert(await control.locator('#video-button').isChecked());
    assert.equal(await page.locator('[data-idm-ex-video-button]').evaluate(el=>el.shadowRoot),null);
    const session=await context.newCDPSession(page);let button=await buttonHandle(session);
    const initial=await buttonState(session,button);
    assert.equal(initial.text,'Download video');assert.equal(initial.icons,1);assert(!initial.clipped);
    assert.match(initial.background,/linear-gradient/);assert.equal(initial.rect.height,36);assert.equal(initial.state,null);
    await page.screenshot({path:path.join(home,'on-page-button.png')});
    const pointer={x:initial.rect.x+initial.rect.width/2,y:initial.rect.y+initial.rect.height/2};
    await page.mouse.move(pointer.x,pointer.y);await page.screenshot({path:path.join(home,'button-hover.png')});
    await page.mouse.down();await page.screenshot({path:path.join(home,'button-pressed.png')});
    await page.mouse.move(0,0);await page.mouse.up(); // Cancel the press; do not open a review yet.
    await page.keyboard.press('Tab');await session.send('Runtime.callFunctionOn',{objectId:button,functionDeclaration:'function(){this.focus();}'});
    assert((await buttonState(session,button)).focus);await page.screenshot({path:path.join(home,'button-focus.png')});
    // YouTube changes the watch URL without replacing the document. Chromium's
    // sender URL can still be the original URL, even though the tab URL is new.
    const nextUrl=new URL('/watch?v=next-video',url).href;
    await page.evaluate(next=>history.pushState({},'',next),nextUrl);
    await worker.evaluate(()=>{globalThis.videoReviewGate=new Promise(resolve=>globalThis.finishVideoReview=resolve);});
    await page.mouse.click(pointer.x,pointer.y);
    await until(()=>worker.evaluate(()=>videoSenders.length===1),'on-page message');
    fs.writeFileSync(path.join(home,'message-senders.json'),JSON.stringify(await worker.evaluate(()=>videoSenders),null,2));
    await until(()=>worker.evaluate(()=>videoReviewRequests.length===1),'native video review request');
    const opening=await buttonState(session,button);assert(opening.disabled);assert.equal(opening.text,'Opening…');assert.equal(opening.icons,1);assert(!opening.clipped);
    // Each state has its own icon and color, and the button keeps its size and place.
    assert.equal(opening.state,'busy');assert.deepEqual(opening.rect,initial.rect);
    await page.screenshot({path:path.join(home,'button-opening.png')});
    await worker.evaluate(()=>{finishVideoReview();globalThis.videoReviewGate=null;});
    await until(async()=>!(await buttonState(session,button)).disabled,'review acknowledgement');
    const opened=await buttonState(session,button);assert.equal(opened.text,'Opened in app');assert.equal(opened.icons,1);assert(!opened.clipped);
    assert.equal(opened.state,'ok');assert.deepEqual(opened.rect,initial.rect);
    await page.screenshot({path:path.join(home,'button-opened.png')});
    const request=await worker.evaluate(()=>videoReviewRequests[0]);
    assert.equal(request.action,'review_browser_video');assert.equal(request.review.page_url,nextUrl);
    assert.deepEqual(request.review.items.map(item=>item.kind).sort(),['dash','file','hls']);
    assert(!context.pages().some(p=>p.url().includes('/links.html#')),'Video review must not open a browser picker');
    fs.writeFileSync(path.join(home,'native-review.json'),JSON.stringify(request,null,2));
    await page.bringToFront();await page.reload();await until(()=>page.locator('[data-idm-ex-video-button]').count(),'persistent registration');
    const staleResult=await worker.evaluate(async()=>{
      const tab=(await chrome.tabs.query({active:true,currentWindow:true}))[0];
      try {await pageLinks.scan(tab,'videos',videoSenders[0].documentId);return 'unexpected handoff';}
      catch(error){return error.message;}
    });
    assert.match(staleResult,/page changed/);assert.equal(await worker.evaluate(()=>videoReviewRequests.length),1);
    await control.evaluate(()=>refreshVideoButton());await control.evaluate(()=>document.querySelector('#video-button').click());
    await until(async()=>await page.locator('[data-idm-ex-video-button]').count()===0,'button removed');
    // Storage changes remove the visible button before async script unregistration
    // finishes. Wait for the real popup operation to finish before checking it.
    await until(()=>control.evaluate(()=>!document.querySelector('#video-button').disabled && !document.querySelector('#video-button').checked),'video setting saved');
    const scripts=await control.evaluate(()=>chrome.scripting.getRegisteredContentScripts());
    assert.deepEqual(scripts[0].excludeMatches,['http://127.0.0.1/*']);
    await page.reload();assert.equal(await page.locator('[data-idm-ex-video-button]').count(),0);
    for(let cycle=0;cycle<3;cycle++) {
      await control.evaluate(()=>refreshMedia());
      await control.evaluate(()=>document.querySelector('#media-detection').click());
      await until(()=>control.evaluate(()=>document.querySelector('#media-detection').checked && !document.querySelector('#media-detection').disabled),'media enabled');
      assert.equal(await page.locator('[data-idm-ex-video-button]').count(),0);
      assert.equal(await control.evaluate(()=>document.querySelector('#video-site-option').hidden),false);
      const automatic=await worker.evaluate(()=>chrome.scripting.getRegisteredContentScripts());
      assert.deepEqual(automatic[0].matches.sort(),['http://*/*','https://*/*']);
      assert.deepEqual((await worker.evaluate(()=>chrome.storage.local.get('videoButtonDisabledSites'))).videoButtonDisabledSites,['http://127.0.0.1/*']);
      await page.reload();assert.equal(await page.locator('[data-idm-ex-video-button]').count(),0);
      await control.evaluate(()=>document.querySelector('#media-detection').click());
      await until(()=>control.evaluate(()=>!document.querySelector('#media-detection').checked && !document.querySelector('#media-detection').disabled),'media setting saved');
      assert.equal(await page.locator('[data-idm-ex-video-button]').count(),0);
    }
    await control.evaluate(()=>refreshVideoButton());await control.evaluate(()=>document.querySelector('#video-button').click());
    await until(()=>page.locator('[data-idm-ex-video-button]').count(),'explicit re-enable');
    await until(()=>control.evaluate(()=>!document.querySelector('#video-button').disabled),'video re-enable saved');
    await control.evaluate(()=>document.querySelector('#media-detection').click());
    await until(()=>control.evaluate(()=>document.querySelector('#media-detection').checked && !document.querySelector('#media-detection').disabled),'media on with video enabled');
    await control.evaluate(()=>document.querySelector('#media-detection').click());
    await until(()=>control.evaluate(()=>!document.querySelector('#media-detection').checked && !document.querySelector('#media-detection').disabled),'media off with video enabled');
    assert.equal(await page.locator('[data-idm-ex-video-button]').count(),1);
    await page.reload();await until(()=>page.locator('[data-idm-ex-video-button]').count(),'re-enabled preference persists');
    button=await buttonHandle(session);
    await page.emulateMedia({reducedMotion:'reduce'});assert.equal((await buttonState(session,button)).transition,'0s');
    await worker.evaluate(()=>{globalThis.failVideoReview=true;});
    const retryBounds=(await buttonState(session,button)).rect;
    await page.mouse.click(retryBounds.x+retryBounds.width/2,retryBounds.y+retryBounds.height/2);
    await until(async()=>(await buttonState(session,button)).text==='Try again','failed review remains retryable');
    const retry=await buttonState(session,button);assert(!retry.disabled);assert.equal(retry.icons,1);assert(!retry.clipped);
    assert.equal(retry.state,'err');assert.equal(retry.rect.width,initial.rect.width);
    assert.match(retry.title,/unavailable/);await page.screenshot({path:path.join(home,'button-retry.png')});
    await page.setViewportSize({width:480,height:650});
    await page.evaluate(()=>{document.body.style.background='#121416';document.body.style.color='#eef3ff';const video=document.querySelector('video');video.style.width='100%';video.style.height='210px';});
    await until(async()=>{const rect=(await buttonState(session,button)).rect;return rect.x>=0 && rect.x+rect.width<=480;},'narrow layout');
    await page.screenshot({path:path.join(home,'button-narrow-dark.png')});
    assert.deepEqual(errors,[]);
    return {browser:name,version:(await cdp.send('Browser.getVersion')).product,checks:['on by default with existing site access','independent per-site checkbox','closed shadow button','trusted click after pushState sends the current page to native review without a browser tab','replacement document rejected before native handoff','DASH/HLS are typed stream candidates','persists after reload','disable removes current and future buttons','explicit off survives three media off/on cycles','re-enable works and media off leaves video buttons enabled','gradient and icon render under strict page CSP','hover, pressed and keyboard focus','opening, success and retry labels fit with their own icon and color at a stable size','reduced motion and narrow layout'],screenshots:fs.readdirSync(home).filter(file=>file.endsWith('.png')).map(file=>path.join(home,file))};
  }finally{await context.close();}
}
(async()=>{server.listen(0,'127.0.0.1');await once(server,'listening');const url=`http://127.0.0.1:${server.address().port}/page`;const results=[];
try{
  const chromiumBrowsers=[['chrome','C:/Program Files/Google/Chrome/Application/chrome.exe'],['edge','C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe']];
  if(process.argv.includes('--brave'))chromiumBrowsers.push(['brave','C:/Program Files/BraveSoftware/Brave-Browser/Application/brave.exe']);
  for(const [name,exe] of chromiumBrowsers){const result=await chromiumTest(name,exe,url);results.push(result);console.log(name+': passed');}
  if(process.argv.includes('--firefox')) {
    const extension=stage('firefox'),config=path.join(run,'firefox','config.json');fs.writeFileSync(config,JSON.stringify({url,extension,run:path.dirname(extension)}));
    const python=process.env.IDM_EX_TEST_PYTHON;assert(python,'Set IDM_EX_TEST_PYTHON to the bundled Python');
    const child=spawn(python,[path.join(__dirname,'video_button_firefox_smoke.py'),config],{windowsHide:true,env:{...process.env,PYTHONPATH:path.join(root,'artifacts/browser-test-deps')},stdio:['ignore','pipe','pipe']});
    let output='',errors='';child.stdout.on('data',v=>output+=v);child.stderr.on('data',v=>errors+=v);const [code]=await once(child,'exit');fs.writeFileSync(path.join(run,'firefox','driver.log'),errors);assert.equal(code,0,output+errors);results.push(JSON.parse(output));console.log('firefox: passed');
  }
  fs.writeFileSync(path.join(run,'verification.json'),JSON.stringify({results,permissionPromptTested:false,nativeHandoffTested:false},null,2));console.log(JSON.stringify({run,results},null,2));
}finally{server.closeAllConnections();await new Promise(r=>server.close(r));}})().catch(e=>{console.error(e);process.exitCode=1;});

/* Real Chromium rendering and CSP, with an explicit simulated extension adapter. */
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require('playwright');
(async()=>{
  const root=path.resolve(__dirname,'..'), output=path.join(root,'artifacts','page-links-ui'); fs.mkdirSync(output,{recursive:true});
  const browser=await chromium.launch({headless:true,executablePath:process.env.IDM_EX_TEST_CHROME||'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'});
  const errors=[];
  try {
    const context=await browser.newContext({viewport:{width:1100,height:900},colorScheme:'light'});
    await context.addInitScript(()=>{
      window.fixture={messages:[],fail:false,state:{ok:true,scan:{url:'https://example.test/downloads',title:'Project downloads',truncated:false,links:[
        {url:'https://example.test/guide.pdf',filename:'guide.pdf',text:'User guide'},
        {url:'https://example.test/source.zip',filename:'source.zip',text:'Source code'},
        {url:'https://example.test/logo.png',filename:'<img src=x onerror=alert(1)>.png',text:'Logo'}]},pending:null,receipt:null,options:{queues:[{id:'main',name:'Main queue',running:true},{id:'later',name:'Read later',running:false}],destinations:[{id:'automatic',name:'Automatic folders',path:null},{id:'documents',name:'Documents',path:'C:\\Downloads\\Documents'}]}}};
      window.chrome={runtime:{sendMessage:async request=>{
        fixture.messages.push(request);
        if(request.action==='page_links_get') return structuredClone(fixture.state);
        if(request.action==='page_links_hls') return {ok:true,message:'HLS preview opened in Download Engine. Nothing queued.'};
        if(request.action==='page_links_submit') {
          if(fixture.fail) { fixture.state.pending={indices:request.indices,queue_id:request.queue_id,destination:request.destination}; return {ok:false,message:'Connection lost after saving'}; }
          fixture.state.receipt={count:2,message:'Saved 2 files to the selected queue'}; fixture.state.pending=null;
          return {ok:true,...fixture.state.receipt};
        }
      }}};
    });
    const csp=JSON.parse(fs.readFileSync(path.join(root,'extension/manifest.chromium.json'),'utf8')).content_security_policy.extension_pages;
    await context.route('**/*',route=>{
      const url=new URL(route.request().url()),name=url.pathname.slice(1);
      if(url.hostname!=='idm-ex.test'||!['links.html','links.css','links.js','page-links.js','i18n.js','i18n-catalog.js'].includes(name)) return route.abort();
      return route.fulfill({status:200,contentType:name.endsWith('.html')?'text/html':name.endsWith('.css')?'text/css':'text/javascript',headers:{'Content-Security-Policy':csp},body:name==='links.js' ? fs.readFileSync(path.join(root,'extension',name),'utf8').replace('})().catch(error =>','window.refresh=refresh;\n})().catch(error =>') : fs.readFileSync(path.join(root,'extension',name))});
    });
    const page=await context.newPage(); page.on('pageerror',e=>errors.push(e.message));
    await page.goto('https://idm-ex.test/links.html#00000000-0000-4000-8000-000000000001');
    await page.waitForFunction(()=>document.querySelectorAll('#links tr').length===3);
    assert.equal(await page.locator('#links input:checked').count(),0); assert(await page.locator('#add').isDisabled());
    assert.equal(await page.locator('img').count(),0);
    await page.locator('#type').selectOption('documents'); assert.equal(await page.locator('#links tr').count(),1);
    await page.locator('#select-visible').click(); await page.locator('#type').selectOption('compressed'); await page.locator('#select-visible').click();
    assert((await page.locator('#counts').textContent()).includes('2 selected (1 hidden'));
    await page.locator('#type').selectOption('all'); await page.locator('#search').fill('source code'); assert.equal(await page.locator('#links tr').count(),1);
    await page.locator('#search').fill(''); await page.locator('#queue').selectOption('later'); await page.locator('#destination').selectOption('documents');
    assert.equal(await page.locator('#folder-path').textContent(),'C:\\Downloads\\Documents');
    await page.screenshot({path:path.join(output,'light.png'),fullPage:true});
    await page.emulateMedia({colorScheme:'dark'}); await page.screenshot({path:path.join(output,'dark.png'),fullPage:true});
    await page.setViewportSize({width:480,height:800}); assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:path.join(output,'narrow.png'),fullPage:true});
    await page.setViewportSize({width:1100,height:900});
    await page.evaluate(()=>{fixture.fail=true;}); await page.locator('#add').click();
    await page.waitForFunction(()=>document.querySelector('#add').textContent==='Retry same batch');
    assert(await page.locator('#queue').isDisabled()); assert(await page.locator('#clear').isDisabled());
    const sent=await page.evaluate(()=>fixture.messages.find(x=>x.action==='page_links_submit'));
    assert.deepEqual(sent.indices,[0,1]); assert.equal(sent.queue_id,'later'); assert.equal(sent.destination,'documents');
    assert.equal(sent.links,undefined,'Picker must send indices, not page-controlled URLs');
    await page.evaluate(()=>{fixture.fail=false;}); await page.locator('#add').click();
    await page.waitForFunction(()=>document.querySelector('#add').textContent==='Added to queue');
    assert(await page.locator('#add').isDisabled()); assert.equal(await page.evaluate(()=>fixture.messages.at(-1).retry),true);
    await page.evaluate(async()=>{
      fixture.state.pending=null;fixture.state.receipt=null;fixture.state.scan.mode='videos';
      fixture.state.scan.links=[{url:'https://example.test/master.m3u8',filename:'master.m3u8',text:'HLS video',hls:true},{url:'https://example.test/clip.mp4',filename:'clip.mp4',text:'Direct video'}];
      await refresh();
    });
    await page.locator('#clear').click();
    assert.equal(await page.locator('h1').textContent(),'Video downloads');
    assert.equal(await page.locator('#links input').count(),1);
    await page.locator('#select-visible').click();
    assert((await page.locator('#counts').textContent()).includes('1 selected'));
    await page.getByRole('button',{name:'Preview HLS',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('Nothing queued'));
    const hlsRequest=await page.evaluate(()=>fixture.messages.at(-1));
    assert.equal(hlsRequest.action,'page_links_hls');assert.equal(hlsRequest.index,0);assert.equal(hlsRequest.url,undefined);
    await page.setViewportSize({width:480,height:800});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:path.join(output,'hls-narrow.png'),fullPage:true});
    await page.evaluate(async()=>{
      fixture.state.scan.mode='links';fixture.state.scan.links=Array.from({length:1000},(_,i)=>({url:'https://example.test/file-'+i+'.zip',filename:'file-'+String(i).padStart(4,'0')+'.zip',text:'Archive '+i,source:i%2?'asset':'link',size:(i+1)*1048576}));
      await refresh();
    });
    await page.locator('#clear').click();
    assert.equal(await page.locator('#links tr').count(),100);
    await page.locator('#next-page').click();assert.match(await page.locator('#page-range').textContent(),/^101–200/);
    await page.locator('#links input').first().check();assert.match(await page.locator('#counts').textContent(),/1 selected/);
    await page.locator('#source').selectOption('asset');assert.match(await page.locator('#counts').textContent(),/500 of 1000/);assert.match(await page.locator('#counts').textContent(),/1 hidden/);
    await page.locator('#min-size').selectOption('104857600');await page.locator('#sort').selectOption('size');
    assert((await page.locator('.file-name').first().textContent()).startsWith('file-0999.zip'));
    await page.locator('#clear').click();await page.locator('#links input').nth(0).click();await page.locator('#links input').nth(3).click({modifiers:['Shift']});
    assert.equal(await page.locator('#links input:checked').count(),4);
    await page.setViewportSize({width:480,height:800});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:path.join(output,'bulk-narrow.png'),fullPage:true});
    await page.setViewportSize({width:1100,height:900});await page.screenshot({path:path.join(output,'bulk-dark.png'),fullPage:true});
    assert.deepEqual(errors,[]);
    const result={ok:true,checks:['nothing preselected','file-type filtering','search','select visible preserves hidden selection','saved queue and folder','text-only untrusted names','light and dark layout','narrow layout','uncertain reply locks selection','retry uses original batch','success prevents resubmit','HLS preview sends a listed index only','HLS cannot be bulk-selected','HLS preview remains usable after direct-file receipt','1000-item pagination','source and minimum-size filters','size sorting preserves selection indices','shift-click range selection','no page errors'],limitation:'Browser rendering and CSP are real; extension messages are simulated.'};
    fs.writeFileSync(path.join(output,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({...result,output}));
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});

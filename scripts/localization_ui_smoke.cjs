// Real Chrome/Edge rendering with isolated, simulated extension APIs and no network.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {randomUUID}=require('node:crypto');
const root=path.resolve(__dirname,'..');
const output=path.join(root,'artifacts','localization-browser-'+randomUUID().slice(0,8));
fs.mkdirSync(output,{recursive:true});
const catalogs=JSON.parse(fs.readFileSync(path.join(root,'locales/extension.json'),'utf8'));
const languages=JSON.parse(fs.readFileSync(path.join(root,'locales/languages.json'),'utf8')).map(language=>language.code);
const additional=JSON.parse(fs.readFileSync(path.join(root,'locales/additional.json'),'utf8'));
const translated=(key,lang)=>{const index=languages.indexOf(lang);return index===0?key:index<4?catalogs[key][index-1]:additional[key][index-4];};
function fixture(language) {
  const listeners=[];
  const saved=JSON.parse(localStorage.getItem('fixtureStorage')||'null')||{displayLanguage:language};
  window.testState={requests:[],messages:[],grant:false,scan:{ok:true,scan:{url:'https://example.test/files',title:'Settings 用户网页',links:[
    {url:'https://example.test/Settings.zip',filename:'Settings.zip',text:'Settings'},
    {url:'https://example.test/report.pdf',filename:'<img src=x onerror=alert(1)>.pdf',text:'用户文件'},
  ]},options:{queues:[{id:'main',name:'Main queue',running:true}],destinations:[{id:'automatic',name:'Automatic folders',path:null}]}}};
  window.chrome={i18n:{getUILanguage:()=> 'en-US'},
    windows:{getCurrent:async()=>({id:1})},tabs:{query:async()=>[{id:1,windowId:1}]},
    storage:{onChanged:{addListener:fn=>listeners.push(fn)},local:{get:async()=>({...saved}),set:async update=>{Object.assign(saved,update);localStorage.setItem('fixtureStorage',JSON.stringify(saved));for(const fn of listeners)fn(Object.fromEntries(Object.entries(update).map(([key,newValue])=>[key,{newValue}])),'local');}},session:{get:async()=>({}),remove:async()=>{}}},
    permissions:{contains:async()=>false,request:async grant=>{testState.requests.push({...grant,userGesture:navigator.userActivation.isActive});return testState.grant;},remove:async()=>true},
    runtime:{sendMessage:async request=>{
      testState.messages.push(request);
      const t=text=>window.IDMI18n.t(text);
      switch(request.action) {
        case 'page_links_get':return structuredClone(testState.scan);
        case 'page_links_submit': return {ok:true,count:request.indices.length,message:t('Added to queue')};
        case 'browser_login_get':return {ok:true,url:'https://example.test/private/archive.zip',origin:'https://example.test',permissionOrigin:'https://example.test/*',grant:{permissions:['cookies']}};
        case 'browser_login_submit':return {ok:true,message:t('Added to Download Engine')};
        case 'media_list':return {ok:true,enabled:false,items:[],tabId:1};
        case 'media_status':return {ok:true,enabled:true,tabId:1,items:[
          {id:'hls',url:'https://cdn.example.test/master.m3u8',filename:'Conference keynote 2026 — full session recording.mp4',hls:true},
          {id:'video',url:'https://cdn.example.test/clip.mp4',filename:'clip.mp4',kind:'video',size:48234496},
          {id:'audio',url:'https://cdn.example.test/episode.m4a',filename:'Episode 42.m4a',kind:'audio',size:31457280}]};
        case 'video_button_status':return {ok:true,available:true,enabled:true,allowed:false,pattern:'https://example.test/*',host:'example.test'};
        case 'link_recovery_status':return {ok:true,recovery:null};
        case 'diagnostics_get':return {ok:true,entries:[]};
        case 'capture_rules_get':return {ok:true,rules:{version:1,types:['3gp','7z','aac','apk','avi','exe','iso','mkv','mp3','mp4','msi','pdf','r0*','rar','zip'],minSizeKB:2048,sites:['drive.example.test','*.cdn.example.org']},defaults:['zip']};
        default:return {ok:true,message:t('Added to queue')};
      }
    }}
  };
}
(async()=>{
  const result=[];
  for(const [name,exe] of [['chrome','C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'],['edge','C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe']]) {
    const browser=await chromium.launch({executablePath:exe,headless:true});
    try {
      for(const language of languages) {
        const context=await browser.newContext({viewport:{width:370,height:650},locale:'en-US'});
        const errors=[];
        await context.addInitScript(fixture,language);
        const csp=JSON.parse(fs.readFileSync(path.join(root,'extension/manifest.chromium.json'),'utf8')).content_security_policy.extension_pages;
        await context.route('**/*',route=>{
          const url=new URL(route.request().url()),file=url.pathname.slice(1);
          if(url.hostname!=='localization.test'|| !/^(popup|links|login)\.(html|css|js)$|^(page-links|i18n|i18n-catalog)\.js$/.test(file))return route.abort();
          return route.fulfill({status:200,contentType:file.endsWith('.html')?'text/html':file.endsWith('.css')?'text/css':'text/javascript',headers:{'Content-Security-Policy':csp},body:fs.readFileSync(path.join(root,'extension',file))});
        });
        const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));
        async function capture(view) {
          assert.equal(await page.locator('html').getAttribute('lang'),language);
          assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`${name} ${language} ${view}: horizontal overflow`);
          await page.screenshot({path:path.join(output,`${name}-${language}-${view}.png`),fullPage:true});
        }
        await page.goto('https://localization.test/popup.html');
        await page.waitForFunction(()=>window.IDMI18n && document.querySelector('#display-language').value!=='system');
        assert.equal(await page.locator('#check').textContent(),translated('Check connection',language));
        await page.locator('#browser-options').evaluate(node=>node.open=true);
        await page.locator('#capture-rules').evaluate(node=>node.open=true);
        await page.waitForFunction(()=>document.querySelector('#capture-sites').value.includes('drive.example.test'));
        assert.equal(await page.locator('#capture-rules summary').textContent(),translated('Capture rules',language));
        assert.equal(await page.locator('#capture-rules-save').textContent(),translated('Save rules',language));
        assert.equal(await page.locator('#capture-min-size').inputValue(),'2');
        await page.waitForFunction(()=>document.querySelectorAll('.media-item').length===3);
        const popup=await page.evaluate(()=>{
          const rtl=document.documentElement.dir==='rtl',edge=rect=>rtl?rect.right:rect.left,style=id=>getComputedStyle(document.getElementById(id));
          const names=[...document.querySelectorAll('.media-name')],first=document.createRange();
          first.setStart(names[0].firstChild,0);first.setEnd(names[0].firstChild,1);
          const box=names[0].getBoundingClientRect(),char=first.getBoundingClientRect();
          const look=element=>[element.backgroundColor,element.borderTopColor,element.fontSize,element.fontFamily].join('|');
          return {names:names.map(name=>edge(name.getBoundingClientRect())),
            firstVisible:char.left>=box.left-0.5&&char.right<=box.right+0.5,
            // The site follows the size: after it on the same line, or on the next line when long text wraps.
            metaOrder:[...document.querySelectorAll('.media-meta')].every(meta=>{const [size,host]=[...meta.querySelectorAll('bdi')].map(part=>part.getBoundingClientRect());return host.top>=size.bottom-1||(rtl?size.left>=host.right:size.right<=host.left);}),
            select:look(style('display-language')),input:look(style('url')),
            buttons:['page-videos','page-links'].map(id=>style(id).justifyContent+'|'+!!document.querySelector(`#${id} [aria-hidden]`))};
        });
        assert(Math.max(...popup.names)-Math.min(...popup.names)<=0.5,`${name} ${language}: media names start at ${popup.names}`);
        assert(popup.firstVisible,`${name} ${language}: the start of a long media name is clipped`);
        assert(popup.metaOrder,`${name} ${language}: media size and site are out of reading order`);
        assert.equal(popup.select,popup.input,`${name} ${language}: language picker does not match the other fields`);
        assert.equal(popup.buttons[0],popup.buttons[1],`${name} ${language}: page buttons differ`);
        await capture('popup');
        // Verify selection persists through the popup's own reload.
        const next=language==='ms'?'en':'ms';
        await page.locator('#display-language').selectOption(next);
        await page.waitForFunction(lang=>document.documentElement.lang===lang && document.querySelector('#display-language').value===lang,next);
        await page.locator('#browser-options').evaluate(node=>node.open=true);
        await page.locator('#display-language').selectOption(language);
        await page.waitForFunction(lang=>document.documentElement.lang===lang && document.querySelector('#display-language').value===lang,language);
        await page.setViewportSize({width:480,height:800});
        await page.goto('https://localization.test/links.html#00000000-0000-4000-8000-000000000001');
        await page.waitForSelector('#links tr');
        assert.equal(await page.locator('#page-title').textContent(),'Settings 用户网页');
        assert.equal(await page.locator('img').count(),0);
        await page.locator('#select-visible').click();
        assert.equal(await page.locator('#add').textContent(),translated('Add {0} file{1}',language).replace('{0}','2').replace('{1}',language==='en'?'s':''));
        const links=await page.evaluate(()=>{
          const rtl=document.documentElement.dir==='rtl';
          // The visible start of the text: a name cut off with … is clipped to its own box.
          const text=element=>{const range=document.createRange();range.selectNodeContents(element);const shown=range.getBoundingClientRect(),own=element.getBoundingClientRect();return rtl?Math.min(shown.right,own.right):Math.max(shown.left,own.left);};
          return {pairs:[[text(document.querySelector('#page-title')),text(document.querySelector('#page-url'))],
            ...[...document.querySelectorAll('#links tr')].map(row=>[text(row.querySelector('.file-name bdi')),text(row.querySelector('.link-url'))])],
            filters:['#search','#type'].map(selector=>document.querySelector(selector).getBoundingClientRect().width)};
        });
        for(const [title,address] of links.pairs) assert(Math.abs(title-address)<=1,`${name} ${language}: title and address start at ${title} and ${address}`);
        assert(Math.abs(links.filters[0]-links.filters[1])<=1,`${name} ${language}: narrow filters ${links.filters}`);
        await capture('links');
        await page.locator('#add').click();
        await page.waitForFunction(()=>document.querySelector('#add').disabled && testState.messages.some(m=>m.action==='page_links_submit'));
        const sent=await page.evaluate(()=>testState.messages.find(m=>m.action==='page_links_submit'));
        assert.deepEqual(sent.indices,[0,1]);assert.equal(sent.queue_id,'main');assert.equal(sent.destination,'automatic');assert.equal(sent.links,undefined);
        await page.goto('https://localization.test/login.html?id=00000000-0000-4000-8000-000000000001');
        await page.waitForFunction(()=>!document.querySelector('#approve').disabled);
        const login=await page.evaluate(()=>{
          const rtl=document.documentElement.dir==='rtl';
          const text=selector=>{const range=document.createRange();range.selectNodeContents(document.querySelector(selector));const rect=range.getBoundingClientRect();return rtl?rect.right:rect.left;};
          return {edges:[text('#origin'),text('#file-url')],widths:['#approve','#forget'].map(selector=>document.querySelector(selector).getBoundingClientRect().width)};
        });
        assert(Math.abs(login.edges[0]-login.edges[1])<=1,`${name} ${language}: login site and file address start at ${login.edges}`);
        assert(Math.abs(login.widths[0]-login.widths[1])<=0.5,`${name} ${language}: login buttons ${login.widths}`);
        await capture('login');
        await page.locator('#approve').click();
        await page.waitForFunction(()=>document.querySelector('#status').dataset.error==='true');
        assert.equal(await page.locator('#status').textContent(),translated('Site access was not granted. No login was sent.',language));
        assert.equal(await page.evaluate(()=>testState.messages.filter(m=>m.action==='browser_login_submit').length),0);
        assert.equal(await page.evaluate(()=>testState.requests[0].userGesture),true);
        await page.evaluate(()=>testState.grant=true);await page.locator('#approve').click();
        await page.waitForFunction(()=>testState.messages.some(m=>m.action==='browser_login_submit'));
        assert.deepEqual(errors,[]);
        result.push({browser:name,language,pages:3,checks:['translations','narrow layouts','persisted switch','user text and identifiers unchanged','selection submission','permission denial and user gesture','no script errors']});
        await context.close();
      }
    }finally{await browser.close();}
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({result,limitation:'Real browser rendering and CSP; simulated extension and native APIs, no personal browser profiles.'},null,2));
  console.log(JSON.stringify({passed:result.length,output}));
})().catch(error=>{console.error(error);process.exitCode=1;});

const {test}=require('node:test');
const assert=require('node:assert/strict');
require('../extension/core.js');require('../extension/video-button-manager.js');require('../extension/page-links.js');require('../extension/capture.js');
function fixture(){
  const state={values:{},granted:false,mediaGranted:false,registered:[],injected:[],reviews:[],tab:{id:5,url:'https://example.test/video'}};
  const api={storage:{local:{get:async key=>({[key]:structuredClone(state.values[key])}),set:async value=>Object.assign(state.values,structuredClone(value))}},
    permissions:{getAll:async()=>({origins:state.mediaGranted?['https://*/*','http://*/*']:state.granted?['https://example.test/*']:[]}),contains:async request=>(request.permissions||[]).every(()=>state.mediaGranted) && (request.origins||[]).every(origin=>state.mediaGranted || state.granted && origin==='https://example.test/*')},
    tabs:{query:async()=>[state.tab],get:async()=>state.tab},
    scripting:{getRegisteredContentScripts:async()=>state.registered,unregisterContentScripts:async()=>{state.registered=[];},registerContentScripts:async values=>{state.registered=values;},executeScript:async spec=>{state.injected.push(spec);}}};
  const manager=new IDMVideoButtons.Manager({api,pageLinks:{scan:async(tab,mode,documentId)=>{state.reviews.push({tab,mode,documentId});return{ok:true};}}});
  return {state,manager,api};
}

test('video preference defaults on without requesting access and explicit off survives reloads',async()=>{
  const {state,manager,api}=fixture();
  assert.equal((await manager.status()).enabled,true);
  assert.equal((await manager.status()).permitted,false);
  await manager.sync(true);assert.deepEqual(state.injected,[]);
  state.mediaGranted=true;await manager.sync(true);
  assert.equal((await manager.status()).enabled,true);
  assert.equal(state.injected.length,1);
  await manager.configure(false,'https://example.test/*');
  state.values.mediaDetection=true;await manager.sync(true);
  assert.equal((await manager.status()).enabled,false);
  assert.deepEqual(state.registered[0].excludeMatches,['https://example.test/*']);
  const restarted=new IDMVideoButtons.Manager({api,pageLinks:{}});
  assert.equal((await restarted.status()).enabled,false);
  state.mediaGranted=false;await restarted.sync();
  state.mediaGranted=true;await restarted.sync();
  assert.equal((await restarted.status()).enabled,false);
  await restarted.configure(true,'https://example.test/*');
  assert.equal((await restarted.status()).enabled,true);
});
test('video button access is optional, scoped to one site and persisted without cookies',async()=>{
  const {state,manager}=fixture();const pattern='https://example.test/*';
  await manager.sync();assert.deepEqual(state.registered,[]);
  assert.deepEqual(await manager.status(),{ok:true,pattern,enabled:true,permitted:false});
  await assert.rejects(manager.configure(true,pattern),/Allow this site/);assert.deepEqual(state.injected,[]);
  state.granted=true;await manager.configure(true,pattern);
  assert.deepEqual(state.registered[0].matches,[pattern]);assert.equal(state.registered[0].allFrames,false);
  assert.deepEqual(state.injected[0],{target:{tabId:5,frameIds:[0]},files:['i18n-catalog.js','i18n.js','video-button.js']});
  await manager.configure(false,pattern);assert.deepEqual(state.registered,[]);assert.deepEqual(state.values.videoButtonDisabledSites,[pattern]);
});
test('on-page requests open only a review for the current approved top frame',async()=>{
  const {state,manager}=fixture();const sender={frameId:0,url:state.tab.url,tab:state.tab};
  await assert.rejects(manager.open(sender),/disabled/);state.granted=true;await manager.configure(true,'https://example.test/*');
  for(const invalid of [{...sender,frameId:1},{...sender,tab:{...sender.tab,incognito:true}},{...sender,url:'https://example.test/old'}])await assert.rejects(manager.open(invalid));
  assert.equal(state.reviews.length,0);await manager.open(sender);assert.equal(state.reviews.length,1);assert.equal(state.reviews[0].mode,'videos');
  await assert.rejects(manager.open(sender),/already opening/);
});
test('permission revocation removes registration and serial setting changes preserve the last choice',async()=>{
  const {state,manager}=fixture();state.granted=true;
  await Promise.all([manager.configure(true,'https://example.test/*'),manager.configure(false,'https://example.test/*')]);
  assert.deepEqual(state.values.videoButtonDisabledSites,['https://example.test/*']);assert.deepEqual(state.registered,[]);
  await manager.configure(true,'https://example.test/*');state.granted=false;await manager.sync();
  assert.deepEqual(state.values.videoButtonDisabledSites,[]);assert.deepEqual(state.registered,[]);
  assert.equal((await manager.status()).enabled,true);assert.equal((await manager.status()).permitted,false);
  state.granted=true;await manager.sync();assert.equal(state.registered.length,1);
});

test('same-document navigation uses browser document identity instead of the stale sender URL',async()=>{
  const {state,manager}=fixture();state.granted=true;await manager.configure(true,'https://example.test/*');
  const sender={frameId:0,url:state.tab.url,tab:{...state.tab},documentId:'document-one',documentLifecycle:'active'};
  state.tab.url='https://example.test/watch?v=next';
  await assert.rejects(manager.open({...sender,url:'https://other.test/video'}),/disabled/);
  for(const invalid of [{...sender,documentLifecycle:'cached'},{...sender,documentId:undefined}])await assert.rejects(manager.open(invalid),/page changed/);
  await manager.open(sender);
  assert.equal(state.reviews[0].tab.url,state.tab.url);assert.equal(state.reviews[0].documentId,'document-one');
});

test('default-on buttons register and inject using existing access, independently of media detection',async()=>{
  const {state,manager,api}=fixture();state.values.mediaDetection=true;
  await manager.sync(true);assert.deepEqual(state.registered,[]);assert.deepEqual(state.injected,[]);
  state.mediaGranted=true;
  api.tabs.query=async query=>query.active?[state.tab]:[state.tab,{id:6,url:state.tab.url,incognito:true},{id:7,url:'chrome://settings'},{id:8,url:'https://other.test/player'},{id:9,url:'https://user:password@example.test/video'},{id:10,url:'https://['}];
  await manager.sync(true);
  assert.deepEqual(state.registered[0].matches,['http://*/*','https://*/*']);
  assert.deepEqual(state.injected.map(spec=>spec.target.tabId),[5,8]);
  assert.equal((await manager.status()).permitted,true);assert.equal((await manager.status()).enabled,true);
  assert.equal(state.values.videoButtonDisabledSites,undefined);
  await assert.rejects(manager.open({frameId:0,url:'https://other.test/video',tab:state.tab,documentId:'one'}),/page changed/);
  await manager.open({frameId:0,url:state.tab.url,tab:state.tab,documentId:'one'});
  assert.equal(state.reviews.length,1);
  await manager.configure(false,'https://example.test/*');assert.equal((await manager.status()).enabled,false);
  state.values.mediaDetection=false;await manager.sync();assert.equal(state.registered.length,1);
  assert.deepEqual(state.registered[0].excludeMatches,['https://example.test/*']);
  state.values.mediaDetection=true;state.mediaGranted=false;await manager.sync();assert.deepEqual(state.registered,[]);
});

test('per-site opt-outs survive media changes and permission changes without affecting other sites',async()=>{
  const {state,manager}=fixture();state.granted=true;await manager.configure(true,'https://example.test/*');
  await manager.configure(false,'https://example.test/*');
  state.values.mediaDetection=true;state.mediaGranted=true;await manager.sync();
  assert.equal(await manager.active(state.tab.url),false);assert.equal(await manager.active('https://other.test/player'),true);
  state.values.mediaDetection=false;await manager.sync();assert.equal(await manager.active(state.tab.url),false);
  state.values.mediaDetection=true;state.mediaGranted=false;await manager.sync();assert.deepEqual(state.registered,[]);
  state.mediaGranted=true;await manager.sync();assert.equal(await manager.active(state.tab.url),false);
  assert.equal(await manager.active('https://other.test/player'),true);
});

test('content status requires a regular top frame, site access and an enabled preference',async()=>{
  const {state,manager}=fixture(),sender={frameId:0,url:state.tab.url,tab:state.tab};
  assert.equal((await manager.pageStatus(sender)).enabled,false);
  state.granted=true;assert.equal((await manager.pageStatus(sender)).enabled,true);
  for(const invalid of [{...sender,frameId:1},{...sender,tab:{...state.tab,incognito:true}},{...sender,tab:null}])assert.equal((await manager.pageStatus(invalid)).enabled,false);
  await manager.configure(false,'https://example.test/*');assert.equal((await manager.pageStatus(sender)).enabled,false);
  state.granted=false;assert.equal((await manager.pageStatus(sender)).enabled,false);
});

test('registration accepts only web origins and sanitizes stored opt-outs',async()=>{
  const {state,manager,api}=fixture();
  state.values.videoButtonDisabledSites=[null,42,'file:///*','https://example.test/page','https://example.test/*','https://example.test/*'];
  api.permissions.getAll=async()=>({origins:['<all_urls>','file:///*','moz-extension://*/*','https://*/*']});
  await manager.sync();
  assert.deepEqual(state.registered[0].matches,['http://*/*','https://*/*']);
  assert.deepEqual(state.registered[0].excludeMatches,['https://example.test/*']);
});
test('DASH handoff requires an exact preview receipt and never an ordinary download acknowledgement',async()=>{
  const request=IDMCore.makeRequest('https://example.test/main.MPD?signature=abc','00000000-0000-4000-8000-000000000001');
  assert.equal(request.action,'preview_video');
  await assert.rejects(IDMCore.handoff(async()=>({ok:true,version:1,job_id:'unexpected'}),request),/preview/);
  assert.equal((await IDMCore.handoff(async()=>({ok:true,version:1,preview_id:request.request_id}),request)).preview_id,request.request_id);
});
test('automatic capture leaves manifests in the browser for explicit video review',()=>{
  const base={state:'in_progress',totalBytes:100,mime:'application/octet-stream',url:'https://example.test/file'};
  for(const value of [{url:'https://example.test/main.mpd'},{url:'https://example.test/main.m3u8'},{mime:'application/dash+xml'},{mime:'application/vnd.apple.mpegurl'}])assert.equal(IDMCapture.candidate({...base,...value}),null);
});
test('YouTube URL entry opens a native preview and does not queue the watch page as a file',()=>{
  for (const url of ['https://www.youtube.com/watch?v=YE7VzlLtp-4','https://youtu.be/YE7VzlLtp-4','https://m.youtube.com/shorts/YE7VzlLtp-4']) {
    const request=IDMCore.makeRequest(url,'00000000-0000-4000-8000-000000000001');
    assert.equal(request.action,'preview_video'); assert(request.request_id.startsWith('hls:'));
  }
  assert.equal(IDMCore.makeRequest('https://youtube.com.example.test/file.zip','id').action,'add');
});
test('content UI exits before observing a private document',()=>{
  const vm=require('node:vm'),fs=require('node:fs');
  const script=fs.readFileSync(require('node:path').join(__dirname,'../extension/video-button.js'),'utf8');
  vm.runInNewContext(script,{browser:{extension:{inIncognitoContext:true}}});
});

// Fake layout: a button is 40 px plus 7 px per character of its label.
const fakeWidth=text=>40+7*text.length;
async function contentFixture(rectangles,site={hostname:'example.test',pathname:'/video'},{translate}={}) {
  const vm=require('node:vm'),fs=require('node:fs');
  const script=fs.readFileSync(require('node:path').join(__dirname,'../extension/video-button.js'),'utf8');
  const events={},timers=new Map(),buttons=[],hosts=[],replies=[];let sequence=0,enabled=true,permitted=true,changed;
  const element=tag=>({tagName:tag,style:{},isConnected:true,children:[],setAttribute(){},addEventListener(){},remove(){this.removed=true;},append(...nodes){this.children.push(...nodes);},
    getBoundingClientRect(){return {width:fakeWidth(this.children.map(node=>node.textContent||'').join('')),height:36};},
    attachShadow(){return{append:node=>{if(node.tagName==='button')buttons.push(node);}};}});
  const videos=rectangles.map(box=>({getBoundingClientRect:()=>({...box,width:box.right-box.left,height:box.bottom-box.top}),
    closest:selector=>box.main && selector.split(', ').includes(box.main)?{}:null}));
  const location={protocol:'https:',...site};
  const document={documentElement:element(),createElement:tag=>{const node=element(tag);if(tag==='div')hosts.push(node);return node;},createElementNS:(_,tag)=>element(tag),querySelectorAll:()=>videos,addEventListener:(name,fn)=>events[name]=fn,removeEventListener:name=>delete events[name]};
  const browser={runtime:{sendMessage:async()=>replies.length?replies.shift():{ok:true,enabled:enabled&&permitted}},storage:{onChanged:{addListener:fn=>changed=fn}}};
  const context={browser,document,location,innerHeight:850,innerWidth:1100,
    window:{addEventListener(){},removeEventListener(){}},getComputedStyle:()=>({}),MutationObserver:class{observe(){}disconnect(){}},
    setTimeout:fn=>{timers.set(++sequence,fn);return sequence;},clearTimeout:id=>timers.delete(id)};
  if(translate)context.IDMI18n={ready:Promise.resolve(),language:'de',t:translate};
  const settle=()=>new Promise(setImmediate);
  vm.runInNewContext(script,context);await settle();
  return {buttons,hosts,events,timers,replies,settle,location,refresh:()=>context.__idmVideoButton.refresh(),
    async enable(value){enabled=value;changed({videoButtonDisabledSites:{}},'local');await settle();},
    async permission(value){permitted=value;changed({videoButtonPermissions:{}},'local');await settle();},
    layout(){events.scroll();for(const [id,callback] of [...timers]){timers.delete(id);callback();}}};
}

test('revocation removes an existing overlay and an older refresh cannot resurrect a disabled overlay',async()=>{
  const f=await contentFixture([{top:100,bottom:460,left:0,right:640}]);
  assert.equal(f.hosts.filter(h=>!h.removed).length,1);
  await f.permission(false);assert.equal(f.hosts.filter(h=>!h.removed).length,0);
  await f.permission(true);assert.equal(f.hosts.filter(h=>!h.removed).length,1);
  let resolve;f.replies.push(new Promise(r=>resolve=r));const old=f.refresh();
  await f.enable(false);assert.equal(f.hosts.filter(h=>!h.removed).length,0);
  resolve({ok:true,enabled:true});await old;
  assert.equal(f.hosts.filter(h=>!h.removed).length,0);
});
test('re-enabling a button after pending layout work still follows the video',async()=>{
  const box={top:100,bottom:460,left:0,right:640},f=await contentFixture([box]);
  assert.equal(f.buttons.at(-1).style.top,'58px');f.events.scroll();assert.equal(f.timers.size,1);
  await f.enable(false);assert.equal(f.timers.size,0);await f.enable(true);
  box.top=200;box.bottom=560;f.layout();
  assert.equal(f.buttons.at(-1).style.top,'158px');
});
test('buttons stay outside players with below/side fallbacks and hide when no space exists',async()=>{
  const box={top:100,bottom:460,left:200,right:840},f=await contentFixture([box]);
  assert.equal(f.buttons.at(-1).style.top,'58px');
  box.top=0;box.bottom=360;f.layout();assert.equal(f.buttons.at(-1).style.top,'366px');
  box.bottom=850;f.layout();assert.equal(f.buttons.at(-1).style.left,'846px');
  box.left=0;box.right=1100;f.layout();assert(f.buttons.every(button=>button.removed));
  box.top=100;box.bottom=460;box.left=200;box.right=840;f.layout();assert.equal(f.buttons.at(-1).style.top,'58px');
});
test('buttons share the width of their widest state label, capped at 240 px',async()=>{
  const box={top:100,bottom:460,left:200,right:840},f=await contentFixture([box]);
  const widest=Math.max(...['Download video','Opening…','Opened in app','Try again'].map(fakeWidth));
  assert.equal(f.buttons.at(-1).style.width,`${widest}px`);assert.equal(f.buttons.at(-1).style.left,`${840-widest}px`);
  const long=await contentFixture([box],undefined,{translate:text=>`${text} in a very long translated wording`});
  assert.equal(long.buttons.at(-1).style.width,'240px');assert.equal(long.buttons.at(-1).style.left,'600px');
});
test('a button cannot cover a neighboring video or another button',async()=>{
  const players=[{left:200,right:840,top:100,bottom:460},{left:200,right:840,top:0,bottom:100}];
  const f=await contentFixture(players),placed=[];
  for(const button of f.buttons.filter(b=>!b.removed)) {
    const p={left:parseFloat(button.style.left),top:parseFloat(button.style.top)};p.right=p.left+parseFloat(button.style.width);p.bottom=p.top+36;
    for(const other of [...players,...placed])assert(p.right<=other.left || p.left>=other.right || p.bottom<=other.top || p.top>=other.bottom);
    placed.push(p);
  }
  assert.equal(placed.length,2);
});
test('YouTube offers the button only beside a video page main player, never banners or previews',async()=>{
  // The Music page carousel banner plays a muted preview; the watch player sits below it.
  const banner={top:100,bottom:560,left:40,right:858},player={top:600,bottom:800,left:40,right:400,main:'#movie_player'};
  const f=await contentFixture([banner,player],{hostname:'www.youtube.com',pathname:'/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ'});
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
  f.location.pathname='/watch';f.layout();
  const shown=f.buttons.filter(b=>!b.removed);
  // One button, below the player and clear of the banner it still avoids.
  assert.equal(shown.length,1);assert.equal(shown[0].style.left,'262px');assert.equal(shown[0].style.top,'806px');
  assert(parseFloat(shown[0].style.top)>=banner.bottom);
  player.main='#shorts-player';f.location.pathname='/shorts/1_CFSqGgvaI';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
  f.location.pathname='/results';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
});
test('other sites keep a button for every visible player',async()=>{
  const f=await contentFixture([{top:100,bottom:460,left:200,right:840}],{hostname:'youtube.com.example.test',pathname:'/channel/x'});
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
});
test('Bilibili offers the button only beside a video page main player, never card previews',async()=>{
  // Home page cards play muted hover previews; a video page has one main player.
  const card={top:100,bottom:300,left:40,right:400},player={top:400,bottom:800,left:40,right:700,main:'.bpx-player-container'};
  const f=await contentFixture([card,player],{hostname:'www.bilibili.com',pathname:'/'});
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
  f.location.pathname='/video/BV1hCnw65EaE/';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
  f.location.pathname='/video/av170001';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
  // Bangumi episode and season pages use the same player.
  f.location.pathname='/bangumi/play/ep733316';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
  f.location.pathname='/bangumi/play/ss28747/';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
  f.location.pathname='/bangumi/media/md28747';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
  f.location.pathname='/v/popular/all';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
});
test('Bilibili video links open a native preview and other Bilibili pages are not previews',()=>{
  for (const url of ['https://www.bilibili.com/video/BV1hCnw65EaE/?p=2','https://bilibili.com/video/BV1hCnw65EaE','https://m.bilibili.com/video/av170001','https://www.bilibili.com/bangumi/play/ep733316?from_spmid=666.25','https://m.bilibili.com/bangumi/play/ss28747/']) {
    const request=IDMCore.makeRequest(url,'00000000-0000-4000-8000-000000000001');
    assert.equal(request.action,'preview_video',url); assert(request.request_id.startsWith('hls:'));
  }
  for (const url of ['https://www.bilibili.com/','https://space.bilibili.com/1/video','https://bilibili.com.example.test/video/BV1hCnw65EaE','https://www.bilibili.com/video/BV1hCnw65EaE/extra','https://www.bilibili.com/bangumi/media/md28747','https://www.bilibili.com/bangumi/play/ep12x']) {
    assert.equal(IDMCore.makeRequest(url,'id').action,'add',url);
  }
});
test('TikTok offers the button only beside the page video, never the preloaded next one',async()=>{
  const id='7696495296011930887';
  const current={top:100,bottom:719,left:40,right:388,main:`[id^="xgwrapper-"][id$="-${id}"]`},next={top:100,bottom:719,left:500,right:848};
  const f=await contentFixture([current,next],{hostname:'www.tiktok.com',pathname:'/foryou'});
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
  f.location.pathname=`/@jannanickofficial/video/${id}`;f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
  f.location.pathname='/@jannanickofficial/video/1234';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
});
test('Twitch offers the button only beside a video page player, never live previews',async()=>{
  const preview={top:100,bottom:300,left:40,right:400},player={top:400,bottom:800,left:40,right:700,main:'[data-a-target="video-player"]'};
  const f=await contentFixture([preview,player],{hostname:'www.twitch.tv',pathname:'/directory'});
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
  f.location.pathname='/videos/2902385151';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,1);
  // Live channels are not finished videos.
  f.location.pathname='/twitch';f.layout();
  assert.equal(f.buttons.filter(b=>!b.removed).length,0);
});
test('Twitch video links open a native preview and other Twitch pages are not previews',()=>{
  for (const url of ['https://www.twitch.tv/videos/2902385151?t=1h2m3s','https://m.twitch.tv/videos/155322654/','https://twitch.tv/videos/1']) {
    const request=IDMCore.makeRequest(url,'00000000-0000-4000-8000-000000000001');
    assert.equal(request.action,'preview_video',url); assert(request.request_id.startsWith('hls:'));
  }
  for (const url of ['https://www.twitch.tv/twitch','https://www.twitch.tv/videos/0123','https://clips.twitch.tv/SomeClip','https://twitch.tv.example.test/videos/1','https://www.twitch.tv/videos/1/extra']) {
    assert.equal(IDMCore.makeRequest(url,'id').action,'add',url);
  }
});
test('TikTok video links open a native preview and other TikTok pages are not previews',()=>{
  for (const url of ['https://www.tiktok.com/@jannanickofficial/video/7696495296011930887?is_from_webapp=1','https://m.tiktok.com/@user.name_1/video/7696495296011930887/']) {
    const request=IDMCore.makeRequest(url,'00000000-0000-4000-8000-000000000001');
    assert.equal(request.action,'preview_video',url); assert(request.request_id.startsWith('hls:'));
  }
  for (const url of ['https://www.tiktok.com/foryou','https://www.tiktok.com/@jannanickofficial','https://www.tiktok.com/@user/photo/7696495296011930887','https://tiktok.com.example.test/@user/video/1']) {
    assert.equal(IDMCore.makeRequest(url,'id').action,'add',url);
  }
});

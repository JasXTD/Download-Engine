const {test}=require('node:test');
const assert=require('node:assert/strict');
require('../extension/core.js');require('../extension/media.js');
const {Manager,describe,TTL,ITEM_LIMIT,TAB_LIMIT}=IDMMedia;
function event(){const listeners=[];return {listeners,addListener:fn=>listeners.push(fn),emit:d=>listeners.forEach(fn=>fn(d))};}
function fixture(){
  const f={local:{},session:{},granted:true,time:1000000,seq:0,active:7,badges:new Map(),tabs:new Map([[7,{id:7,url:'https://page.test/watch',title:'Watch'}]])};
  const storage=values=>({get:async key=>structuredClone(key===null?values:typeof key==='string'?{[key]:values[key]}:Object.fromEntries(key.map(k=>[k,values[k]]))),set:async data=>{if(f.failStorage)throw Error('Storage unavailable');Object.assign(values,structuredClone(data));},remove:async key=>{delete values[key];}});
  f.badge=id=>f.badges.get(id)??f.defaultBadge??'';
  f.api={storage:{local:storage(f.local),session:storage(f.session)},permissions:{contains:async()=>f.granted},action:{setBadgeText:async({tabId,text})=>{if(tabId===undefined)f.defaultBadge=text;else if(text===null)f.badges.delete(tabId);else f.badges.set(tabId,text);},setBadgeBackgroundColor:async()=>{}},tabs:{query:async()=>[f.tabs.get(f.active)],get:async id=>{if(!f.tabs.has(id))throw Error('Closed');return structuredClone(f.tabs.get(id));}},webRequest:{onBeforeRequest:event(),onHeadersReceived:event(),onCompleted:event(),onErrorOccurred:event()}};
  f.manager=new Manager({api:f.api,now:()=>f.time,uuid:()=>String(++f.seq)});
  f.begin=async()=>{await f.manager.start();};
  f.enable=async()=>{await f.manager.configure(true);};
  f.before=async d=>{f.api.webRequest.onBeforeRequest.emit(d);await f.manager.tail;};
  f.headers=async d=>{f.api.webRequest.onHeadersReceived.emit(d);await f.manager.tail;};
  f.request=(url='https://cdn.test/clip.mp4',extra={})=>({requestId:'req-'+(++f.seq),tabId:7,type:'xmlhttprequest',method:'GET',url,statusCode:200,responseHeaders:[{name:'Content-Type',value:'video/mp4'},{name:'Content-Length',value:'2048'}],...extra});
  f.capture=async d=>{await f.before(d);await f.headers(d);};
  return f;
}
test('discovery traces distinguish lost request ownership, stale navigation, initialization fragments and cached success',async()=>{
  const f=fixture(),events=[];f.manager.trace=(...event)=>events.push(event);await f.begin();
  const stream=f.request('https://vod.dmcdn.net/token/cloud/fmp4/123/h264_aac_hd/1/manifest.m3u8',{fromCache:true});
  await f.headers(stream);assert.equal(events.at(-1)[1].reason,'no_request');
  await f.before(stream);f.manager.navigated(7,{url:'https://page.test/new'});await f.manager.tail;
  await f.headers(stream);assert.equal(events.at(-1)[1].reason,'stale_navigation');
  await f.capture(f.request('https://vod.dmcdn.net/token/cloud/fmp4/123/h264_aac_hd/1/init.mp4'));
  assert.equal(events.at(-1)[1].reason,'init');
  await f.capture(stream);assert.equal(events.at(-1)[0],'network_saved');assert.equal(events.at(-1)[1].cached,true);
  assert(!JSON.stringify(events).includes('https:'));
});

test('media discovery defaults on with access, but saved off and denied access stay off',async()=>{
  const f=fixture();await f.begin();await f.capture(f.request());assert.equal((await f.manager.status()).items.length,1);
  for (const saved of [false, undefined]) {
    const off=fixture();off.local.mediaDetection=saved;off.granted=saved===false;
    await off.begin();await off.capture(off.request());assert.equal((await off.manager.status()).enabled,false);assert.deepEqual(off.session,{});
  }
  f.granted=false;await assert.rejects(f.enable(),/Allow media/);
});
test('response classification uses MIME for opaque URLs and rejects unsafe, failed and segment requests',()=>{
  const f=fixture();
  for(const [mime,kind] of [['application/vnd.apple.mpegurl','hls'],['application/dash+xml','dash'],['audio/mpeg','audio'],['video/webm','video']]){const x=describe(f.request('https://cdn.test/opaque?token=x',{responseHeaders:[{name:'Content-Type',value:mime}]}));assert.equal(x.kind,kind);}
  for(const extra of [{method:'POST'},{statusCode:403},{url:'https://u:secret@cdn.test/x.mp4'},{url:'blob:https://cdn.test/id'},{url:'https://cdn.test/part.m4s'},{responseHeaders:[{name:'Content-Type',value:'text/html'}]},{url:'https://cdn.test/x',responseHeaders:[{name:'Content-Type',value:'application/json'}]}])assert.equal(describe(f.request(undefined,extra)),null);
  assert.equal(describe(f.request('https://cdn.test/master.m3u8',{responseHeaders:[{name:'Content-Type',value:'text/plain'}]})).hls,true);
});
test('range responses report full file size, never segment size or manifest size',()=>{
  const f=fixture(),headers=[{name:'Content-Type',value:'video/mp4'},{name:'Content-Length',value:'10'},{name:'Content-Range',value:'bytes 0-9/1000'}];
  assert.equal(describe(f.request(undefined,{statusCode:206,responseHeaders:headers})).size,1000);
  assert.equal(describe(f.request(undefined,{statusCode:206})).size,null);
  assert.equal(describe(f.request('https://cdn.test/master.m3u8')).size,null);
});

test('Dailymotion initialization fragments are excluded without hiding ordinary MP4 files',()=>{
  const f=fixture();
  const path='/signed-token/video/fmp4/717461554/h264_aac_hd/1/init.mp4';
  assert.equal(describe(f.request('https://vod3.cf.dmcdn.net'+path)),null);
  assert.equal(describe(f.request('https://vod2.cf.dmcdn.net'+path.replace('/video/','/cloud/'))),null);
  assert.equal(describe(f.request('https://vod2.cf.dmcdn.net/token/preview/fmp4/717479018/h264_aac_hq/1.pmp4')),null);
  for(const url of ['https://example.test/init.mp4','https://vod3.cf.dmcdn.net/ordinary/init.mp4',
    'https://vod3.cf.dmcdn.net.attacker.test'+path,'https://vod3.cf.dmcdn.net'+path.replace('init.mp4','movie.mp4')]) {
    assert.equal(describe(f.request(url)).kind,'video',url);
  }
});

test('Dailymotion manifest refreshes replace one stream while different assets and qualities stay separate',async()=>{
  const f=fixture();await f.begin();
  const base='/video/fmp4/717461554/h264_aac_hd/1/manifest.m3u8';
  const first='https://vod3.cf.dmcdn.net/old-token'+base;
  const fresh='https://vod4.cf.dmcdn.net/new-token'+base;
  const details={responseHeaders:[{name:'Content-Type',value:'application/vnd.apple.mpegurl'}]};
  await f.capture(f.request(first,details));const id=(await f.manager.status()).items[0].id;
  await f.capture(f.request(fresh,details));
  const items=(await f.manager.status()).items;assert.equal(items.length,1);assert.equal(items[0].id,id);assert.equal(items[0].url,fresh);
  for(const url of [fresh.replace('h264_aac_hd','h264_aac_fhd'),fresh.replace('717461554','717461555'),
    fresh+'?variant=other',fresh.replace('/video/','/cloud/'),'https://example.test/one'+base,'https://example.test/two'+base]) {
    await f.capture(f.request(url,details));
  }
  assert.equal((await f.manager.status()).items.length,7);
  await f.manager.review(7,id,{openScan:async scan=>assert.equal(scan.links[0].url,fresh)});
});

test('Dailymotion alternative playlists remain available when a rendition is observed',async()=>{
  const f=fixture();await f.begin();
  const entry='https://dmxleo.dailymotion.com/cdn/manifest/video/xbv62cq.m3u8?fixture=one';
  const director='https://cdndirector.dailymotion.com/cdn/manifest/video/xbv62cq.m3u8?sec=fixture';
  const stream='https://vod2.cf.dmcdn.net/token/cloud/fmp4/717479018/h264_aac_hd/1/manifest.m3u8';
  const details={responseHeaders:[{name:'Content-Type',value:'application/vnd.apple.mpegurl'}]};
  await f.capture(f.request(entry,details));await f.capture(f.request(director,details));
  assert.equal((await f.manager.status()).items.length,2);
  await f.capture(f.request(stream,details));
  let items=(await f.manager.status()).items;assert.deepEqual(items.map(i=>i.url),[entry,director,stream]);
  assert.equal(items[0].text,'HLS · player playlist');assert.equal(items[1].text,'HLS · alternate playlist');assert.equal(items[2].text,'HLS · HD');
  await f.capture(f.request(entry,details));assert.equal((await f.manager.status()).items.length,3);assert.equal(f.badge(7),'3');
  const another=entry.replace('xbv62cq','xatdymm');await f.capture(f.request(another,details));
  assert.equal((await f.manager.status()).items.length,4,'Different videos must not be merged');
  const anotherStream=stream.replace('/cloud/','/video/').replace('717479018','654022030').replace('h264_aac_hd','h264_aac_hq');
  await f.capture(f.request(anotherStream,details));
  items=(await f.manager.status()).items;assert.deepEqual(items.map(i=>i.url),[entry,director,stream,another,anotherStream]);assert.equal(items[4].text,'HLS · HQ');
  await f.manager.review(7,items[0].id,{openScan:async scan=>assert.equal(scan.links[0].url,entry)});
});

test('restarting discovery cleans old Dailymotion fragments and repeated manifests from session storage',async()=>{
  const f=fixture(),stream='https://vod2.cf.dmcdn.net/token/cloud/fmp4/717479018/h264_aac_hd/1/manifest.m3u8';
  const urls=['https://cdndirector.dailymotion.com/cdn/manifest/video/xbv62cq.m3u8',stream,
    stream.replace('manifest.m3u8','init.mp4'),stream.replace('/token/','/refreshed/')];
  f.session['media-tab:7']={url:f.tabs.get(7).url,title:'Watch',updated:f.time,
    items:urls.map((url,i)=>({url,id:String(i),seen:f.time-10+i}))};
  await f.begin();const items=(await f.manager.status()).items;
  assert.equal(items.length,2);assert.equal(items[0].url,urls[0]);assert.equal(items[1].id,'1');assert.equal(items[1].url,urls[3]);assert.equal(f.badge(7),'2');
});

test('media labels neutralize directional controls without splitting Unicode characters',()=>{
  const f=fixture(),item=describe(f.request('https://cdn.test/%E2%80%AEclip.mp4'));
  assert(!item.filename.includes('\u202e'));assert(item.filename.isWellFormed());
});

test('global media settings remain accessible from a browser settings tab',async()=>{
  const f=fixture();await f.begin();await f.enable();f.tabs.get(7).url='chrome://settings/';
  const state=await f.manager.status();assert.equal(state.enabled,true);assert.equal(state.tabId,null);assert.deepEqual(state.items,[]);
  await f.manager.configure(false);assert.equal((await f.manager.status()).enabled,false);
  await assert.rejects(f.manager.review(7,'anything',{}),/regular web page/);
});
test('deduplicated tab list and badge use session storage without copying response secrets',async()=>{
  const f=fixture();await f.begin();await f.enable();const d=f.request();d.responseHeaders.push({name:'Set-Cookie',value:'PRIVATE-COOKIE'},{name:'Authorization',value:'PRIVATE-AUTH'});
  await f.capture(d);const first=(await f.manager.status()).items[0];await f.capture({...d,requestId:'second'});
  assert.equal((await f.manager.status()).items.length,1);assert.equal((await f.manager.status()).items[0].id,first.id);assert.equal(f.badges.get(7),'1');
  assert(!JSON.stringify(f.session).includes('PRIVATE'));assert.deepEqual(Object.keys(f.local),['mediaDetection']);
});
test('private, untabbed and unmatched responses never enter the list',async()=>{
  const f=fixture();await f.begin();await f.enable();await f.capture(f.request(undefined,{incognito:true}));await f.capture(f.request(undefined,{tabId:-1}));await f.headers(f.request());
  f.tabs.get(7).incognito=true;await f.capture(f.request());assert.deepEqual(f.session,{});
});

test('Firefox embedded-player HLS metadata reaches desktop review from an empty top page',async()=>{
  require('../extension/page-links.js');require('../extension/video-button-manager.js');
  const f=fixture(),requests=[],page='https://www.dailymotion.com/video/xbv5ovm';
  f.tabs.get(7).url=page;
  // Firefox MV2 exposes browserAction rather than action.
  f.api.browserAction=f.api.action;delete f.api.action;
  f.manager=new IDMMedia.Manager({api:f.api,now:()=>f.time,uuid:()=>String(++f.seq)});
  f.api.scripting={executeScript:async spec=>{
    assert.deepEqual(spec.target,{tabId:7,frameIds:[0]});
    return [{frameId:0,documentId:'top-document',result:{url:page,title:'Embedded player',mode:spec.args[0],links:[]}}];
  }};
  const links=new IDMPageLinks.Manager({api:f.api,detected:tab=>f.manager.list(tab),
    uuid:()=> '00000000-0000-4000-8000-000000000001',native:async request=>{
      requests.push(request);return {ok:true,version:1,preview_id:request.request_id};
    }});
  const buttons=new IDMVideoButtons.Manager({api:f.api,pageLinks:links});
  await f.begin();
  const stream='https://cdn.test/stream/master.m3u8?fixture=public';
  await f.capture(f.request(stream,{frameId:2,parentFrameId:0,originUrl:'https://geo.dailymotion.com/player/xtv3w.html',
    responseHeaders:[{name:'Content-Type',value:'application/vnd.apple.mpegurl'}]}));
  await buttons.open({frameId:0,url:page,tab:f.tabs.get(7),documentId:'top-document'});
  assert.equal(requests.length,1);assert.equal(requests[0].action,'review_browser_video');
  assert.equal(requests[0].review.page_url,page);
  assert.deepEqual(requests[0].review.items.map(item=>({url:item.url,kind:item.kind})),[{url:stream,kind:'hls'}]);
  assert.equal(f.session['pending-video-review'],undefined);
  // No GET permission or stale responses from a previous top document may be reused.
  const stale=f.request('https://cdn.test/stale.m3u8',{frameId:2});await f.before(stale);
  f.tabs.get(7).url='https://www.dailymotion.com/video/next';
  f.manager.navigated(7,{url:f.tabs.get(7).url});await f.manager.tail;await f.headers(stale);
  assert.deepEqual(await f.manager.list(f.tabs.get(7)),[]);
});
test('navigation invalidates in-flight media even for a POST navigation',async()=>{
  const f=fixture();await f.begin();await f.enable();const old=f.request();await f.before(old);
  await f.before(f.request('https://page.test/next',{type:'main_frame',method:'POST'}));await f.headers(old);assert.equal((await f.manager.status()).items.length,0);
  await f.capture(f.request());assert.equal((await f.manager.status()).items.length,1);
  f.tabs.get(7).url='https://page.test/elsewhere';assert.equal((await f.manager.status()).items.length,0);
});
test('redirects retain request ownership until the final media response',async()=>{
  const f=fixture();await f.begin();await f.enable();const d=f.request();await f.before(d);await f.headers({...d,statusCode:302});await f.headers({...d,url:'https://cdn.test/final.mp4'});assert.equal((await f.manager.status()).items[0].url,'https://cdn.test/final.mp4');
});

test('history navigation rejects delayed media even when there was no prior cached item',async()=>{
  const f=fixture();await f.begin();await f.enable();const old=f.request();await f.before(old);
  f.tabs.get(7).url='https://page.test/next';f.manager.navigated(7,{url:f.tabs.get(7).url});await f.manager.tail;
  await f.headers(old);assert.equal((await f.manager.status()).items.length,0);
  await f.capture(f.request());assert.equal((await f.manager.status()).items.length,1);
});
test('worker restart restores media only while enabled and still permitted',async()=>{
  const f=fixture();await f.begin();await f.enable();await f.capture(f.request());const saved=(await f.manager.status()).items;
  const restarted=new Manager({api:f.api,now:()=>f.time});await restarted.start();assert.deepEqual((await restarted.status()).items,saved);
  f.granted=false;const revoked=new Manager({api:f.api,now:()=>f.time});await revoked.start();assert.deepEqual(f.session,{});assert.equal((await revoked.status()).items.length,0);
});
test('clear, disable and permission revocation clear session state and badge',async()=>{
  const f=fixture();await f.begin();await f.enable();await f.capture(f.request());const old=f.request();await f.before(old);await f.manager.clear(7);await f.headers(old);assert.equal((await f.manager.status()).items.length,0);
  await f.capture(f.request());await f.manager.configure(false);assert.deepEqual(f.session,{});assert.equal(f.badge(7),'');
  await f.enable();await f.capture(f.request());f.manager.revoked();await f.manager.tail;assert.equal(f.local.mediaDetection,false);assert.deepEqual(f.session,{});
});

test('cleared media inherits later global errors and successful handoffs clear old tab errors',async()=>{
  const f=fixture();await f.begin();await f.enable();await f.capture(f.request());
  await f.api.action.setBadgeText({text:'!'});await f.manager.setAttention(true);assert.equal(f.badge(7),'!');
  await f.manager.clear(7);
  await f.api.action.setBadgeText({text:''});await f.manager.setAttention(false);assert.equal(f.badge(7),'');
  await f.api.action.setBadgeText({text:'!'});await f.manager.setAttention(true);assert.equal(f.badge(7),'!');
});
test('a revoked permission cannot be resurrected by an older enable request',async()=>{
  const f=fixture();await f.begin();let release;f.api.permissions.contains=()=>new Promise(r=>{release=r;});const task=f.enable();await new Promise(setImmediate);f.manager.revoked();release(true);await assert.rejects(task,/changed/);await f.manager.tail;assert.equal(f.local.mediaDetection,false);
});
test('media review resolves a stored ID for the active tab and opens only a review',async()=>{
  const f=fixture();await f.begin();await f.enable();await f.capture(f.request('https://cdn.test/master.m3u8'));const item=(await f.manager.status()).items[0],reviews=[];const picker={openScan:async s=>reviews.push(s)};
  await f.manager.review(7,item.id,picker);assert.equal(reviews[0].links[0].hls,true);assert.equal(reviews[0].mode,'videos');
  await assert.rejects(f.manager.review(8,item.id,picker),/page changed/);await assert.rejects(f.manager.review(7,'arbitrary-url',picker),/expired/);assert.equal(reviews.length,1);
});
test('lists are bounded by entries, tabs and expiry',async()=>{
  const f=fixture();await f.begin();await f.enable();for(let i=0;i<ITEM_LIMIT+5;i++)await f.capture(f.request('https://cdn.test/'+i+'.mp4'));assert.equal((await f.manager.status()).items.length,ITEM_LIMIT);
  for(let i=10;i<10+TAB_LIMIT+3;i++){f.tabs.set(i,{id:i,url:'https://page.test/'+i});await f.capture(f.request(undefined,{tabId:i}));}assert.equal(Object.keys(f.session).length,TAB_LIMIT);
  f.time+=TTL+1;assert.equal((await f.manager.status()).items.length,0);assert.deepEqual(f.session,{});
});
test('closed tabs release their data and request ownership',async()=>{
  const f=fixture();await f.begin();await f.enable();await f.capture(f.request());f.manager.closed(7);await f.manager.tail;assert.deepEqual(f.session,{});assert.equal(f.manager.requests.size,0);
});
test('storage failures surface in discovery status without acknowledging a saved list',async()=>{
  const f=fixture();await f.begin();await f.enable();f.failStorage=true;await f.capture(f.request());const status=await f.manager.status();assert.equal(status.items.length,0);assert.match(status.error,/could not update/);
});

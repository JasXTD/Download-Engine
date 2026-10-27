const {test} = require('node:test');
const assert = require('node:assert/strict');
require('../extension/core.js');
require('../extension/page-links.js');
const {collect, normalizeScan, category, visibleLinks, selection, Manager} = globalThis.IDMPageLinks;
const options = {queues:[{id:'main',name:'Main queue',running:true}],destinations:[{id:'automatic',name:'Automatic folders',path:null}]};
const scan = () => ({url:'https://example.test/list',title:'Files',links:[{url:'https://example.test/a.pdf#x',filename:'a.pdf',text:'Report'},{url:'https://example.test/b.zip',filename:'b.zip',text:'Archive'}]});
function fixture() {
  const f={values:{},tabs:new Map([[0,{id:0,url:'https://example.test/list'}]]),requests:[],scans:0,sequence:0};
  f.uuid=()=>`00000000-0000-4000-8000-${String(++f.sequence).padStart(12,'0')}`;
  f.api={runtime:{getURL:file=>'extension://id/'+file},storage:{session:{
    get:async key=>structuredClone(key===null?f.values:{[key]:f.values[key]}),
    set:async value=>{ if(f.failSave?.(value)) throw Error('Storage unavailable'); Object.assign(f.values,structuredClone(value)); },
    remove:async key=>{delete f.values[key];}
  }},tabs:{
    query:async()=>[f.tabs.get(0)],
    create:async args=>{const tab={id:f.tabs.size+1,...args}; f.tabs.set(tab.id,tab); return structuredClone(tab);},
    get:async id=>{if(!f.tabs.has(id)) throw Error('Missing tab'); return structuredClone(f.tabs.get(id));},
    update:async(id,args)=>{const tab=f.tabs.get(id); if(!tab) throw Error('Missing tab'); Object.assign(tab,args); await f.onNavigate?.(tab); return structuredClone(tab);}
  },scripting:{executeScript:async spec=>{ f.scans++; assert.deepEqual(spec.target,{tabId:0,frameIds:[0]}); assert.equal(spec.func,collect); return [{frameId:0,result:{...scan(),mode:spec.args[0]}}]; }}};
  f.native=async req=>{f.requests.push(structuredClone(req)); if(req.action==='batch_options') return {batch_options:options}; if(f.send) return f.send(req); if(req.action==='review_browser_video')return {ok:true,version:1,preview_id:req.request_id,message:'Review in Download Engine'}; return {job_ids:req.links.map((_,i)=>'job-'+i),message:'Saved '+req.links.length+' files'};};
  f.manager=new Manager({api:f.api,native:f.native,uuid:f.uuid});
  f.start=async()=>{await f.manager.scan(); return Object.keys(f.values)[0].slice('page-links:'.length);};
  f.input={indices:[0,1],queue_id:'main',destination:'automatic'};
  return f;
}

test('collector resolves base URLs, skips unsafe links, deduplicates and never traverses frames',()=>{
  const node=(href,filename='',text='')=>({getAttribute:name=>({href,download:filename}[name]||''),textContent:text});
  globalThis.document={baseURI:'https://example.test/sub/',title:'Files',links:[node('../a.pdf#one','report.pdf','Report'),node('../a.pdf#two'),node('#section'),node('mailto:a@b'),node('data:text/plain,x'),node('blob:https://example.test/x'),node('https://user:secret@example.test/a'),node('/b.zip'),node('/b.zip')]};
  globalThis.location={href:'https://example.test/sub/list'};
  try {
    const result=collect(); assert.equal(result.links.length,2); assert.equal(result.links[0].url,'https://example.test/a.pdf'); assert.equal(result.links[0].filename,'report.pdf'); assert.equal(result.truncated,false);
    document.links=Array.from({length:1001},(_,i)=>node('/'+i)); assert.equal(collect().links.length,1000); assert.equal(collect().truncated,true);
    document.links=Array.from({length:100},(_,i)=>node('/'+i+'x'.repeat(16000))); const bounded=collect(); assert(bounded.links.length<100); assert.equal(bounded.truncated,true); assert(JSON.stringify(bounded).length<701000);
  } finally {delete globalThis.document; delete globalThis.location;}
});
test('normalized names are safe text; search, final-suffix categories and indices agree',()=>{
  const input=scan(); input.links.push({...input.links[0],url:'https://example.test/a.pdf#duplicate'});
  input.links[1].filename='../CON.\u202eexe';
  const normalized=normalizeScan(input); assert.equal(normalized.links.length,2); assert(!normalized.links[1].filename.includes('/')&&!normalized.links[1].filename.includes('\u202e'));
  assert.equal(category('report.pdf.exe'),'programs'); assert.equal(category('pdf'),'other'); assert.equal(category('.pdf'),'other'); assert.equal(category('a.TAR.GZ'),'compressed');
  assert.deepEqual(visibleLinks(normalized.links,'report','documents').map(x=>x.index),[0]);
  assert.deepEqual(selection(normalized,[1,0],'main','downloads').indices,[0,1]);
  for(const indices of [[],[0,0],[-1],[2],['0'],[0.5]]) assert.throws(()=>selection(normalized,indices,'main','automatic'));
  assert.throws(()=>selection(normalized,[0],'main','C:\\Windows'));
  assert.throws(()=>normalizeScan({...scan(),links:[{url:'javascript:alert(1)'}]}));
  const unicode=normalizeScan({...scan(),links:[{url:'https://example.test/a',filename:'x'.repeat(159)+'\ud83d'}]});
  assert(unicode.links[0].filename.isWellFormed());
});
test('page collection and batch selection preserve MEGA public file keys', () => {
  const url = 'https://mega.nz/file/abcdefgh#AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8';
  globalThis.document = {baseURI:'https://example.test/', title:'Files', links:[{getAttribute: name => name === 'href' ? url : '', textContent:'Shared file'}]};
  globalThis.location = {href:'https://example.test/'};
  try {
    const collected = collect();
    assert.equal(collected.links[0].url, url);
    assert.equal(normalizeScan(collected).links[0].url, url);
  } finally { delete globalThis.document; delete globalThis.location; }
});
test('scan is explicit, main-frame only and binds a session before loading its picker',async()=>{
  const f=fixture(); assert.equal(f.scans,0); assert.equal(f.requests.length,0);
  f.onNavigate=async tab=>{if(tab.url.includes('links.html')) {const id=tab.url.split('#')[1]; assert.equal((await f.manager.read(id)).picker,tab.id);}};
  const id=await f.start(); assert.equal(f.scans,1); assert.equal(f.requests.length,0);
  const state=await f.manager.get(id); assert.deepEqual(state.options,options); assert.equal(state.pending,null);
  assert.equal(state.scan.links.length,2);
  await assert.rejects(f.manager.scan({id:0,url:'chrome://settings'}),/HTTP/);
  await assert.rejects(f.manager.scan({id:0,url:'https://example.test/',incognito:true}),/private/);
});

test('asset metadata filters and sorting preserve original submission indices',()=>{
  const normalized=normalizeScan({...scan(),links:[
    {url:'https://example.test/z',filename:'z',source:'network',mime:'video/mp4',size:300},
    {url:'https://example.test/a.jpg',source:'asset',size:100},
    {url:'https://example.test/b.jpg',source:'asset'},
    {url:'https://example.test/c.mp3',source:'network',mime:'audio/mpeg',size:200}
  ]});
  assert.deepEqual(visibleLinks(normalized.links,'','video').map(x=>x.index),[0]);
  assert.deepEqual(visibleLinks(normalized.links,'','music','network').map(x=>x.index),[3]);
  assert.deepEqual(visibleLinks(normalized.links,'','all','asset',1).map(x=>x.index),[1]);
  assert.deepEqual(visibleLinks(normalized.links,'','all','all',0,'size').map(x=>x.index),[0,3,1,2]);
  assert.deepEqual(visibleLinks(normalized.links,'','all','all',0,'name').map(x=>x.index),[1,2,3,0]);
  assert.deepEqual(selection(normalized,[3,1],'main','downloads').indices,[1,3]);
});

test('network discoveries enrich native video reviews with kind and size only',async()=>{
  const f=fixture();
  f.manager.detected=async tab=>{assert.equal(tab.id,0);return [{url:'https://example.test/a.pdf',hls:true,source:'network'}, {url:'https://example.test/opaque',filename:'clip.mp4',mime:'video/mp4',source:'network',size:300}];};
  await f.manager.scan(null,'videos');const review=f.requests[0].review;
  assert.equal(review.items.length,3);assert.equal(review.items[0].kind,'hls');assert.equal(review.items[2].size,300);
  assert.deepEqual(review.items[2],{url:'https://example.test/opaque',filename:'clip.mp4',label:'',kind:'file',size:300});
  assert.equal(f.tabs.size,1);
});

test('page scans do not merge detections attributed to a different navigated document',async()=>{
  const f=fixture();f.api.scripting.executeScript=async()=>[{frameId:0,result:{...scan(),url:'https://example.test/new',mode:'videos'}}];
  f.manager.detected=async()=>{throw Error('Must not read a different page');};
  await f.manager.scan(null,'videos');const review=f.requests[0].review;assert.equal(review.page_url,'https://example.test/new');assert.equal(review.items.length,2);
});

test('on-page video scans accept the clicked document after pushState and reject replacement documents before handoff',async()=>{
  const f=fixture();let documentId='new-document';
  f.api.scripting.executeScript=async spec=>{
    assert.deepEqual(spec.target,{tabId:0,frameIds:[0]});
    return [{frameId:0,documentId,result:{...scan(),url:'https://example.test/watch?v=next',mode:'videos'}}];
  };
  for(const id of ['new-document',undefined]) {
    documentId=id;
    await assert.rejects(f.manager.scan(f.tabs.get(0),'videos','clicked-document'),/page changed/);
    assert.equal(f.requests.length,0);assert.deepEqual(f.values,{});
  }
  documentId='clicked-document';
  await f.manager.scan(f.tabs.get(0),'videos','clicked-document');
  assert.equal(f.requests.length,1);assert.equal(f.requests[0].review.page_url,'https://example.test/watch?v=next');
});
test('invalid selections never send native data; successful replay sends no second batch',async()=>{
  const f=fixture(),id=await f.start();
  await assert.rejects(f.manager.submit(id,{...f.input,indices:[100]}),/Select/); assert.equal(f.requests.length,0);
  const reply=await f.manager.submit(id,f.input); assert.equal(reply.count,2);
  assert.equal(f.requests.length,1); assert.deepEqual(f.requests[0].links.map(x=>x.url),['https://example.test/a.pdf','https://example.test/b.zip']);
  assert.equal((await f.manager.submit(id,f.input)).count,2); assert.equal(f.requests.length,1);
});
test('lost acknowledgements survive worker restart and closed-tab recovery with the same request',async()=>{
  const f=fixture(),id=await f.start(); f.send=async()=>{throw Error('Connection lost after commit');};
  await assert.rejects(f.manager.submit(id,f.input),/Connection lost/);
  const original=f.requests[0]; const tab=(await f.manager.read(id)).picker;
  f.tabs.delete(tab); await f.manager.closed(tab); assert((await f.manager.get(id)).pending);
  f.manager=new Manager({api:f.api,native:f.native,uuid:f.uuid});
  assert.equal(await f.manager.recover(),1);
  await assert.rejects(f.manager.submit(id,{...f.input,indices:[0]}),/Retry/);
  f.send=async()=>({job_ids:['one','two'],message:'Saved'});
  await f.manager.submit(id,{retry:true}); assert.deepEqual(f.requests[1],original);
  const picker=(await f.manager.read(id)).picker; f.tabs.delete(picker); await f.manager.closed(picker);
  await assert.rejects(f.manager.get(id),/expired/);
});
test('definitive native rejection unlocks corrections, uncertain receipt keeps the batch locked',async()=>{
  const f=fixture(),id=await f.start(); f.send=async()=>{const error=Error('Queue removed');error.definitive=true;throw error;};
  await assert.rejects(f.manager.submit(id,f.input),/Queue/); assert.equal((await f.manager.get(id)).pending,null);
  f.send=async()=>({job_ids:['a'],message:'Saved'});
  await f.manager.submit(id,{...f.input,indices:[0]});
  const batchRequests=f.requests.filter(x=>x.action==='add_batch'); assert.notEqual(batchRequests[0].request_id,batchRequests[1].request_id);
});
test('session storage failure before send leaves no desktop batch',async()=>{
  const f=fixture(),id=await f.start(); f.failSave=value=>Object.values(value).some(x=>x.pending);
  await assert.rejects(f.manager.submit(id,f.input),/Storage/); assert.equal(f.requests.length,0);
});
test('receipt-save failure retries original native ID instead of a new submission',async()=>{
  const f=fixture(),id=await f.start(); f.failSave=value=>Object.values(value).some(x=>x.receipt);
  await assert.rejects(f.manager.submit(id,f.input),/Storage/); assert((await f.manager.read(id)).pending);
  f.failSave=null; await f.manager.submit(id,{retry:true}); assert.deepEqual(f.requests[0],f.requests[1]);
});
test('double submission is blocked and closing a sending picker cleans up after success',async()=>{
  const f=fixture(),id=await f.start(); let release,entered; const sending=new Promise(resolve=>{entered=resolve;});
  f.send=()=>new Promise(resolve=>{release=resolve;entered();});
  const first=f.manager.submit(id,f.input); await sending;
  await assert.rejects(f.manager.submit(id,f.input),/already/);
  const picker=(await f.manager.read(id)).picker; f.tabs.delete(picker); await f.manager.closed(picker);
  assert((await f.manager.read(id)).pending); release({job_ids:['a','b'],message:'Saved'}); await first;
  await assert.rejects(f.manager.get(id),/expired/);
});
test('picker navigation cannot overwrite a request that starts as soon as the page loads',async()=>{
  const f=fixture(); f.onNavigate=async tab=>{await f.manager.submit(tab.url.split('#')[1],f.input);};
  const id=await f.start(); assert((await f.manager.read(id)).receipt); assert.equal(f.requests.length,1);
});
test('at most five outstanding page scans are held in memory',async()=>{
  const f=fixture(); for(let n=0;n<5;n++) await f.manager.scan();
  await assert.rejects(f.manager.scan(),/Close another/); assert.equal(f.scans,5);
});
test('native batch acknowledgement must cover every selected link with distinct IDs',async()=>{
  const request={action:'add_batch',links:[{},{}]};
  for(const job_ids of [undefined,[],['a'],['a','a'],['a',null]]) {
    await assert.rejects(IDMCore.handoff(async()=>({version:1,ok:true,job_ids}),request),/complete saved batch/);
  }
  const reply=await IDMCore.handoff(async()=>({version:1,ok:true,job_ids:['a','b']}),request); assert.equal(reply.job_ids.length,2);
  await assert.rejects(IDMCore.handoff(async()=>({version:1,ok:false,message:'Rejected'}),request),error=>error.definitive===true);
});

test('video collection separates HLS and DASH previews and excludes blobs, DRM and non-video links',()=>{
  const node=(attrs, sources=[], currentSrc='')=>({getAttribute:n=>attrs[n]||'',textContent:attrs.text||'',currentSrc,querySelectorAll:()=>sources,mediaKeys:attrs.drm?{}:null});
  const videos=[node({src:'/clip.mp4'},[node({src:'/clip.mp4#duplicate'}),node({src:'/second.webm',type:'video/webm'}),node({src:'/opaque?id=3',type:'video/mp4'}),node({src:'/live.m3u8',type:'video/mp4'}),node({src:'/dash',type:'application/dash+xml'})], 'https://example.test/clip.mp4'),node({src:'blob:https://example.test/abc'}),node({src:'/private.mp4',drm:true}),node({src:'/unknown'})];
  globalThis.document={baseURI:'https://example.test/page',title:'Videos',querySelectorAll:()=>videos,links:[node({href:'/linked.MP4',download:'movie.mp4'}),node({href:'/document.pdf'}),node({href:'https://u:p@example.test/leak.mp4'}),node({href:'/stream.mpd'}),node({href:'javascript:alert(1)'})]};
  globalThis.location={href:'https://example.test/page'};
  try {
    const value=collect('videos'); assert.equal(value.mode,'videos');
    assert.deepEqual(value.links.map(x=>x.url),['https://example.test/clip.mp4','https://example.test/second.webm','https://example.test/opaque?id=3','https://example.test/live.m3u8','https://example.test/dash','https://example.test/linked.MP4','https://example.test/stream.mpd']);
    assert.equal(value.links[3].hls,true);
    assert.equal(value.links[4].dash,true); assert.equal(value.links[6].dash,true);
    assert.throws(()=>selection(normalizeScan(value),[4],'main','video'),/Preview DASH/);
    assert.throws(()=>selection(normalizeScan(value),[3],'main','video'),/Preview HLS/);
    assert.equal(value.links[2].filename,'video.mp4'); assert.equal(normalizeScan(value).mode,'videos');
    document.querySelectorAll=()=>Array.from({length:1001},(_,i)=>node({src:`/${i}.mp4`}));
    const bounded=collect('videos'); assert.equal(bounded.links.length,1000); assert(bounded.truncated);
  } finally {delete globalThis.document;delete globalThis.location;}
});
test('video scans open a native review without a browser tab or a download request',async()=>{
  const f=fixture(); f.api.scripting.executeScript=async spec=>{assert.deepEqual(spec.args,['videos']); assert.deepEqual(spec.target.frameIds,[0]); return [{frameId:0,result:{...scan(),mode:'videos',links:[{url:'https://example.test/clip.mp4'}]}}];};
  await assert.rejects(f.manager.scan(undefined,'streams'),/Invalid/);
  await f.manager.scan(undefined,'videos'); assert.equal(f.requests.length,1);
  assert.equal(f.tabs.size,1); assert.deepEqual(f.values,{});
  assert.equal(f.requests[0].action,'review_browser_video');
  assert(f.requests[0].request_id.startsWith('video:'));
  assert.deepEqual(f.requests[0].review.items,[{url:'https://example.test/clip.mp4',filename:'clip.mp4',label:'',kind:'file',size:null}]);
});

test('empty video scans and HLS/DASH candidates reach the desktop without queueing files',async()=>{
  const f=fixture();
  await f.manager.openScan({...scan(),mode:'videos',links:[]});
  assert.deepEqual(f.requests[0].review.items,[]);
  await f.manager.openScan({...scan(),mode:'videos',links:[{url:'https://example.test/master.m3u8',cookies:['not forwarded']},{url:'https://example.test/video.mpd'},{url:'https://example.test/opaque',mime:'audio/mp4',size:4200}]});
  assert.deepEqual(f.requests[1].review.items.map(item=>item.kind),['hls','dash','file']);
  assert.equal(f.requests[1].review.items[2].size,4200);
  assert(!JSON.stringify(f.requests).includes('not forwarded'));
  assert.equal(f.tabs.size,1); assert.deepEqual(f.values,{});
});

test('lost native video replies survive worker restart and recover with the same receipt',async()=>{
  const f=fixture();f.send=async()=>{throw Error('Lost reply')};
  await assert.rejects(f.manager.openScan({...scan(),mode:'videos'}),/Lost reply/);
  const first=structuredClone(f.requests[0]);
  await assert.rejects(f.manager.openScan({...scan(),mode:'videos'}),/awaiting confirmation/);
  assert.equal(f.requests.length,1);
  const restarted=new Manager({api:f.api,native:f.native,uuid:f.uuid});
  f.send=async req=>({ok:true,version:1,preview_id:req.request_id});
  assert.equal(await restarted.recover(),1);assert.deepEqual(f.requests[1],first);
  assert.deepEqual(f.values,{});assert.equal(f.tabs.size,1);
});

test('native video review rejects a wrong acknowledgement and releases definitive failures for retry',async()=>{
  const f=fixture();f.send=async()=>({ok:true,version:1,job_id:'not-a-review'});
  await assert.rejects(f.manager.openScan({...scan(),mode:'videos'}),/did not confirm/);
  assert(f.values['pending-video-review']);
  f.send=async()=>{throw Object.assign(Error('Update Download Engine'),{definitive:true});};
  await assert.rejects(f.manager.recover(),/Update/);assert.deepEqual(f.values,{});
  const request=f.requests[0];
  await assert.rejects(IDMCore.handoff(async()=>({ok:true,version:1,preview_id:'wrong'}),request),/did not confirm/);
});

test('HLS preview survives a lost reply with its original ID and cannot enter a direct-file batch',async()=>{
  const f=fixture(),id=await f.start(),record=await f.manager.read(id);
  record.scan=normalizeScan({...scan(),mode:'videos',links:[{url:'https://example.test/master.m3u8'},{url:'https://example.test/direct.mp4'}]});
  await f.manager.save(id,record);
  await assert.rejects(f.manager.submit(id,{indices:[0,1],queue_id:'main',destination:'video'}),/Preview HLS/);
  assert.equal(f.requests.length,0);
  f.send=async()=>{throw Error('Lost reply')};
  await assert.rejects(f.manager.previewHls(id,0),/Lost reply/);
  const first=f.requests[0]; assert.equal(first.action,'preview_hls');assert.equal(first.url,'https://example.test/master.m3u8');
  const restarted=new Manager({api:f.api,native:f.native,uuid:f.uuid});
  f.send=async req=>({ok:true,version:1,preview_id:req.request_id,message:'Review in Download Engine'});
  await restarted.previewHls(id,0);
  assert.deepEqual(f.requests[1],first);
  await assert.rejects(restarted.previewHls(id,1),/listed HLS/);
  await assert.rejects(restarted.previewHls(id,-1),/listed HLS/);
  assert.equal(f.requests.length,2);
  await restarted.previewHls(id,0);assert.notEqual(f.requests[2].request_id,first.request_id);
});

test('HLS manual handoff validates a preview receipt rather than a download receipt',async()=>{
  const request=IDMCore.makeRequest('https://example.test/master.M3U8?token=x','00000000-0000-4000-8000-000000000001');
  assert.equal(request.action,'preview_hls');assert(request.request_id.startsWith('hls:'));
  await assert.rejects(IDMCore.handoff(async()=>({ok:true,version:1,job_id:'fake'}),request),/video preview/);
  const reply=await IDMCore.handoff(async()=>({ok:true,version:1,preview_id:request.request_id,message:'Preview ready'}),request);
  assert.equal(reply.preview_id,request.request_id);assert.equal(reply.job_id,undefined);
});

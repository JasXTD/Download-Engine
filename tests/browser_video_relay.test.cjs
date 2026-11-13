'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const http=require('node:http'),{once}=require('node:events'),{randomUUID,createHash}=require('node:crypto');
require('../extension/core.js');
require('../extension/browser-video-relay.js');
function event(){const listeners=new Set();return{addListener:f=>listeners.add(f),removeListener:f=>listeners.delete(f),emit:(...args)=>{for(const f of [...listeners])f(...args)}}}
function ports(name){
  const a={name,sender:{id:'fixture'},onMessage:event(),onDisconnect:event()},b={...a,onMessage:event(),onDisconnect:event()};
  let closed=false;
  a.postMessage=m=>{if(!closed)queueMicrotask(()=>b.onMessage.emit(structuredClone(m)))};
  b.postMessage=m=>{if(!closed)queueMicrotask(()=>a.onMessage.emit(structuredClone(m)))};
  a.disconnect=b.disconnect=()=>{if(!closed){closed=true;a.onDisconnect.emit();b.onDisconnect.emit()}};
  return[a,b];
}
async function server(t){
  const requests=[],handlers=new Map();
  const s=http.createServer((req,res)=>{requests.push({url:req.url,headers:req.headers});const h=handlers.get(req.url);if(h)h(req,res);else{res.writeHead(404);res.end()}});
  s.listen(0,'127.0.0.1');await once(s,'listening');
  t.after(()=>{s.closeAllConnections();s.close()});
  return{url:`http://127.0.0.1:${s.address().port}`,requests,handlers};
}
function page(t,origin,options={}){
  const api={runtime:{id:'fixture',onConnect:event()}},window={};window.top=window;
  const location={href:origin+'/page'},events=new Map(),calls=[];
  const fetcher=async(...args)=>{calls.push(args);return (options.fetch || fetch)(...args)};
  const context=vm.createContext({chrome:api,window,location,URL,AbortController,setTimeout,clearTimeout,btoa,fetch:options.firefox?()=>{throw Error('Privileged extension fetch must not be used')}:fetcher,
    ...(options.firefox?{content:{fetch:fetcher}}:{}),addEventListener:(n,f)=>events.set(n,f),removeEventListener:n=>events.delete(n)});
  vm.runInContext(fs.readFileSync(require.resolve('../extension/browser-video-relay.js'),'utf8'),context);
  assert.notEqual(vm.runInContext('Uint8Array',context),Uint8Array,'Fetch bytes must cross a realm, as Firefox content.fetch does');
  if(options.firefox){const View=vm.runInContext('Uint8Array',context);context.Uint8Array=new Proxy(View,{construct(target,args){const view=Reflect.construct(target,args);Object.defineProperty(view,'constructor',{get(){throw Error('Forbidden cross-context constructor')}});return view}})}
  const name='de-video-relay-'+randomUUID();
  assert.equal(context.IDMBrowserRelay.installPage(name,location.href,[origin]),true);
  const [background,content]=ports(name),messages=[],waiters=[];
  background.onMessage.addListener(m=>{if(waiters.length)waiters.shift()(m);else messages.push(m)});
  api.runtime.onConnect.emit(content);
  t.after(()=>background.disconnect());
  async function next(){if(messages.length)return messages.shift();return new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('No relay reply')),2500);waiters.push(m=>{clearTimeout(timer);resolve(m)})})}
  return{background,location,calls,messages,next,events};
}
async function readAll(p,id){const chunks=[];for(let i=0;i<100;i++){p.background.postMessage({kind:'read',id});const m=await p.next();assert.equal(m.id,id);if(m.kind==='end')return Buffer.concat(chunks);assert.equal(m.kind,'chunk');const bytes=Buffer.from(m.data,'base64');assert(bytes.length<=48*1024);chunks.push(bytes)}throw Error('Unbounded stream')}

test('large browser stream reads are split into bounded IPC chunks without truncation',async t=>{
  const data=Buffer.alloc(2*1024*1024+7);for(let i=0;i<data.length;i++)data[i]=i%251;
  const origin='https://fixture.example';
  const p=page(t,origin,{fetch:async url=>({url,status:200,redirected:false,headers:new Headers(),body:new ReadableStream({start(controller){controller.enqueue(data);controller.close()}})})});
  await p.next();const id=randomUUID();
  p.background.postMessage({kind:'fetch',id,url:origin+'/video',range:null});
  assert.equal((await p.next()).kind,'head');
  assert.deepEqual(await readAll(p,id),data);
});

test('streamed range overflow and body failure retain safe error reasons',async t=>{
  const origin='https://fixture.example';
  for(const fail of [false,true]){
    const p=page(t,origin,{fetch:async url=>({url,status:206,redirected:false,headers:new Headers(),body:new ReadableStream({pull(controller){if(fail)controller.error(Error('secret site token'));else controller.enqueue(new Uint8Array(5))}})})});
    await p.next();const id=randomUUID();
    p.background.postMessage({kind:'fetch',id,url:origin+'/video',range:{start:0,length:4}});
    assert.equal((await p.next()).kind,'head');
    p.background.postMessage({kind:'read',id});
    assert.deepEqual(await p.next(),{kind:'failed',id,reason:fail?'network':'too_large'});
  }
});

for(const firefox of [false,true])test(`${firefox?'Firefox page fetch':'Chromium page fetch'}: real HTTP bytes, pull backpressure, no credentials and bounded frames`,async t=>{
  const s=await server(t),data=Buffer.alloc(48*1024*3+29);for(let i=0;i<data.length;i++)data[i]=i%251;
  s.handlers.set('/video',(_req,res)=>{res.setHeader('Set-Cookie','private=DO_NOT_FORWARD');res.end(data)});
  const p=page(t,s.url,{firefox});assert.equal((await p.next()).kind,'page_ready');const id=randomUUID();
  p.background.postMessage({kind:'fetch',id,url:s.url+'/video',range:null});const head=await p.next();assert.equal(head.kind,'head');assert(!head.headers.some(([k])=>/cookie|length|encoding/.test(k)));
  await new Promise(resolve=>setImmediate(resolve));assert.equal(p.messages.length,0,'No body without a read command');
  const bytes=await readAll(p,id);assert.equal(createHash('sha256').update(bytes).digest('hex'),createHash('sha256').update(data).digest('hex'));
  assert.equal(s.requests.length,1);assert.equal(s.requests[0].headers.cookie,undefined);assert.equal(s.requests[0].headers.authorization,undefined);
  assert.equal(p.calls[0][1].credentials,'omit');assert.equal(p.calls[0][1].mode,'cors');assert.equal(p.calls[0][1].redirect,'error');
});

test('redirects, unreviewed origins and oversized bodies are rejected without following a target',async t=>{
  const s=await server(t),other=await server(t);
  s.handlers.set('/redirect',(_req,res)=>{res.writeHead(302,{Location:other.url+'/private'});res.end()});
  s.handlers.set('/large',(_req,res)=>{res.writeHead(200,{'Content-Length':256*1024*1024+1});res.end()});
  const p=page(t,s.url);await p.next();
  for(const [url,reason] of [[s.url+'/redirect','network'],[other.url+'/private','unsupported'],[s.url+'/large','too_large']]){
    const id=randomUUID();p.background.postMessage({kind:'fetch',id,url,range:null});assert.deepEqual(await p.next(),{kind:'failed',id,reason});
  }
  assert.equal(other.requests.length,0);
});

test('byte ranges retain Content-Range; invalid ranges never reach the server',async t=>{
  const s=await server(t);s.handlers.set('/range',(req,res)=>{assert.equal(req.headers.range,'bytes=4-7');res.writeHead(206,{'Content-Range':'bytes 4-7/12','Content-Type':'video/mp4'});res.end('4567')});
  const p=page(t,s.url);await p.next();const id=randomUUID();
  p.background.postMessage({kind:'fetch',id,url:s.url+'/range',range:{start:4,length:4}});const head=await p.next();assert.equal(head.status,206);assert(head.headers.some(([k,v])=>k==='content-range'&&v==='bytes 4-7/12'));
  assert.equal((await readAll(p,id)).toString(),'4567');
  for(const range of [{start:-1,length:4},{start:0,length:0},{start:Number.MAX_SAFE_INTEGER,length:2}]){
    const bad=randomUUID();p.background.postMessage({kind:'fetch',id:bad,url:s.url+'/range',range});assert.equal((await p.next()).kind,'failed');
  }
  assert.equal(s.requests.length,1);
});

test('cancel interrupts a pending body read and the same page connection can resume with a fresh request',async t=>{
  const s=await server(t);let closedResolve;const closed=new Promise(r=>closedResolve=r);
  s.handlers.set('/slow',(_req,res)=>{res.writeHead(200);res.flushHeaders();res.once('close',closedResolve)});
  s.handlers.set('/done',(_req,res)=>res.end('resumed'));
  const p=page(t,s.url);await p.next();const id=randomUUID();
  p.background.postMessage({kind:'fetch',id,url:s.url+'/slow',range:null});assert.equal((await p.next()).kind,'head');
  p.background.postMessage({kind:'read',id});p.background.postMessage({kind:'cancel',id});
  await Promise.race([closed,new Promise((_,reject)=>{const timer=setTimeout(()=>reject(Error('Server not cancelled')),2000);timer.unref()})]);
  const resumed=randomUUID();p.background.postMessage({kind:'fetch',id:resumed,url:s.url+'/done',range:null});const head=await p.next();assert.equal(head.id,resumed);assert.equal(head.kind,'head');assert.equal((await readAll(p,resumed)).toString(),'resumed');
});

test('navigation closes the page connection before a further fetch',async t=>{
  const s=await server(t),p=page(t,s.url);await p.next();let disconnected=false;p.background.onDisconnect.addListener(()=>disconnected=true);
  p.location.href=s.url+'/another';p.background.postMessage({kind:'fetch',id:randomUUID(),url:s.url+'/video',range:null});
  await new Promise(resolve=>setImmediate(resolve));assert.equal(disconnected,true);assert.equal(s.requests.length,0);
});

test('manager refuses private tabs, non-HTTPS remote pages and unbounded reviewed origins',async()=>{
  const manager=new IDMBrowserRelay.Manager({api:{runtime:{connectNative:()=>{throw Error('Not expected')}},tabs:{connect(){},get:async()=>({incognito:true,url:'https://example.test/'})}}});
  await assert.rejects(manager.open(1,'document','https://example.test/',[{kind:'hls',url:'https://cdn.test/video.m3u8'}]),/changed/);
  assert.equal(await manager.open(1,null,'http://example.test/',[{kind:'hls',url:'http://cdn.test/video.m3u8'}]),null);
  assert.equal(await manager.open(1,null,'https://example.test/',Array.from({length:17},(_,i)=>({kind:'hls',url:`https://cdn${i}.test/video.m3u8`}))),null);
});

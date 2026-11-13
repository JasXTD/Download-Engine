/* Explicit, isolated Firefox/Edge test; not part of the automatic Node suite.
 * Usage: node tests/browser_video_relay_live.cjs BROWSER DE_HOST RUST_TEST_BINARY
 * Uses only already-installed binaries, a fresh profile and harmless loopback TS.
 * Native-message framing is relayed to the real de-host stdin/stdout by the
 * fixture, so no user's native-host registration or browser profile is changed.
 */
'use strict';
const fs=require('node:fs/promises'),path=require('node:path'),http=require('node:http'),net=require('node:net');
const {spawn}=require('node:child_process'),{once}=require('node:events'),{randomUUID}=require('node:crypto');
const assert=require('node:assert/strict');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function bounded(promise,ms,label){let timer;try{return await Promise.race([promise,new Promise((_,reject)=>timer=setTimeout(()=>reject(Error(label)),ms))])}finally{clearTimeout(timer)}}
async function freePort(){const s=net.createServer();s.listen(0,'127.0.0.1');await once(s,'listening');const port=s.address().port;await new Promise(r=>s.close(r));return port}
async function marionette(port){
  let socket;const end=Date.now()+30000;
  while(Date.now()<end){try{socket=net.connect(port,'127.0.0.1');await once(socket,'connect');break}catch{socket?.destroy();await delay(100)}}
  if(!socket||!socket.readyState.includes('open'))throw Error('Firefox automation did not start');
  let bytes=Buffer.alloc(0),waiting=null;const pending=[];
  socket.on('data',data=>{bytes=Buffer.concat([bytes,data]);for(;;){const colon=bytes.indexOf(58);if(colon<0)break;const size=Number(bytes.subarray(0,colon));if(!Number.isSafeInteger(size)||size>4e6)throw Error('Invalid automation frame');if(bytes.length<colon+1+size)break;const message=JSON.parse(bytes.subarray(colon+1,colon+1+size));bytes=bytes.subarray(colon+1+size);if(waiting){const ready=waiting;waiting=null;ready(message)}else pending.push(message)}});
  const next=()=>bounded(pending.length?Promise.resolve(pending.shift()):new Promise(r=>waiting=r),30000,'Firefox automation timed out');
  await next();let id=0;
  return{socket,async call(command,params={}){const current=++id,payload=Buffer.from(JSON.stringify([0,current,command,params]));socket.write(Buffer.concat([Buffer.from(payload.length+':'),payload]));const response=await next();assert.equal(response[1],current);if(response[2])throw Error(command+': '+response[2].message);return response[3]}};
}
function extensionMain(base,token){
  const event=()=>{const listeners=new Set();return{addListener:f=>listeners.add(f),emit:value=>{for(const f of listeners)f(value)}}};
  const headers={'X-Fixture-Token':token,'Content-Type':'application/json'};
  const send=(endpoint,value)=>fetch(base+endpoint,{method:'POST',headers,body:JSON.stringify(value),credentials:'omit'}).then(r=>{if(!r.ok)throw Error('Fixture bridge rejected request');return r.text()});
  const connectNative=()=>{
    const port={onMessage:event(),onDisconnect:event()};let closed=false,tail=Promise.resolve();
    port.postMessage=message=>{tail=tail.then(()=>send('/native',message)).catch(()=>port.disconnect())};
    port.disconnect=()=>{if(!closed){closed=true;port.onDisconnect.emit();void send('/disconnect',{})}};
    void(async()=>{while(!closed){try{const reply=await fetch(base+'/next',{headers,credentials:'omit'});if(reply.status===204)continue;if(!reply.ok)throw Error('Host closed');port.onMessage.emit(await reply.json())}catch{port.disconnect()}}})();
    return port;
  };
  const api=typeof browser!=='undefined'?browser:chrome;
  const manager=new IDMBrowserRelay.Manager({api:{runtime:{connectNative},tabs:api.tabs,scripting:api.scripting},trace:(event,fields)=>{void send('/trace',{event,fields})}});
  void(async()=>{try{
    await send('/trace',{event:'fixture_started'});
    const tab=await api.tabs.create({url:base+'/watch'});
    await new Promise(resolve=>{const loaded=async id=>{if(id!==tab.id)return;const current=await api.tabs.get(id);if(current.status==='complete'&&current.url===base+'/watch'){api.tabs.onUpdated.removeListener(loaded);resolve()}};api.tabs.onUpdated.addListener(loaded);void loaded(tab.id)});
    await send('/trace',{event:'fixture_page_loaded'});
    let review;
    const picker=new IDMPageLinks.Manager({api,relay:manager,trace:(event,fields)=>{void send('/trace',{event,fields})},native:async request=>{
      if(request.action==='ping')return {capabilities:['browser_video_relay_v1']};
      if(request.action!=='review_browser_video')throw Error('Unexpected review command');
      review=request.review;return {ok:true,version:1,preview_id:request.request_id};
    }});
    await picker.scan(await api.tabs.get(tab.id),'videos');
    if(!review?.browser_relay||review.items.length!==1)throw Error('Full page scan did not attach the relay');
    await send('/ready',review.browser_relay);
  }catch(error){await send('/error',{message:String(error)})}})();
}
async function main(){
  const [browserExe,hostExe,testExe]=process.argv.slice(2).map(p=>path.resolve(p));
  for(const file of [browserExe,hostExe,testExe])await fs.access(file);
  const isFirefox=/firefox(?:\.exe)?$/i.test(browserExe),browserName=isFirefox?'Firefox':'Edge';
  const parent=path.resolve('artifacts/browser-relay-20261113');await fs.mkdir(parent,{recursive:true});
  const root=await fs.mkdtemp(path.join(parent,'live-')),profile=path.join(root,'browser'),extension=path.join(root,'extension');
  await fs.mkdir(profile);await fs.mkdir(extension);
  const token=randomUUID(),commands=[],traces=[],counts={},hostBuffer={bytes:Buffer.alloc(0)};let waiter=null,readyResolve,readyReject,driver,ff,mar,debugPort;
  const ready=new Promise((resolve,reject)=>{readyResolve=resolve;readyReject=reject});ready.catch(()=>{});
  const host=spawn(hostExe,[],{windowsHide:true,env:{...process.env,DOWNLOAD_ENGINE_HOME:path.join(root,'host')},stdio:['pipe','pipe','pipe']});
  let hostErrors='';host.stderr.on('data',b=>hostErrors+=b);host.on('error',readyReject);host.stdin.on('error',readyReject);
  host.stdout.on('data',data=>{hostBuffer.bytes=Buffer.concat([hostBuffer.bytes,data]);while(hostBuffer.bytes.length>=4){const n=hostBuffer.bytes.readUInt32LE();if(n>1024*1024)throw Error('Host frame too large');if(hostBuffer.bytes.length<n+4)break;const message=JSON.parse(hostBuffer.bytes.subarray(4,n+4));hostBuffer.bytes=hostBuffer.bytes.subarray(n+4);if(waiter){const res=waiter;waiter=null;res.end(JSON.stringify(message))}else commands.push(message)}});
  const s=http.createServer(async(req,res)=>{try{
    const url=req.url;
    if(['/native','/next','/ready','/error','/trace','/disconnect'].includes(url)){
      if(req.headers['x-fixture-token']!==token){res.writeHead(403);res.end();return}
      if(url==='/next'){res.setHeader('Content-Type','application/json');if(commands.length)res.end(JSON.stringify(commands.shift()));else{assert(!waiter);waiter=res;const timeout=setTimeout(()=>{if(waiter===res){waiter=null;res.writeHead(204);res.end()}},5000);res.on('close',()=>{clearTimeout(timeout);if(waiter===res)waiter=null})}return}
      const chunks=[];let size=0;for await(const chunk of req){size+=chunk.length;if(size>1024*1024)throw Error('Fixture frame too large');chunks.push(chunk)}
      const message=JSON.parse(Buffer.concat(chunks));
      if(url==='/native'){const body=Buffer.from(JSON.stringify(message)),head=Buffer.alloc(4);head.writeUInt32LE(body.length);host.stdin.write(Buffer.concat([head,body]))}
      if(url==='/ready'){await fs.writeFile(path.join(root,'session.json'),JSON.stringify(message));readyResolve()}
      if(url==='/error')readyReject(Error(message.message));
      if(url==='/trace')traces.push(message);
      if(url==='/disconnect')host.stdin.end();
      res.end('ok');return;
    }
    if(url==='/watch'){res.setHeader('Content-Type','text/html');res.setHeader('Set-Cookie','fixture_private=DO_NOT_FORWARD; SameSite=Strict');res.end('<!doctype html><title>Harmless local video fixture</title><p>Isolated browser transfer test</p><a href="/index.m3u8">Playlist</a><script>window.fetch = () => { throw new Error("Page fetch override must not be used") };</script>');return}
    const bulk=/^\/bulk-(\d+)\.ts$/.exec(url);
    if(!['/index.m3u8','/media.m3u8','/a.ts','/b.ts','/bulk.m3u8','/bulk-media.m3u8'].includes(url)&&!(bulk&&Number(bulk[1])<34)){res.writeHead(404);res.end();return}
    const browser=/Firefox\/|Chrome\//.test(req.headers['user-agent']||'');
    counts[(browser?'browser:':'desktop:')+url]=(counts[(browser?'browser:':'desktop:')+url]||0)+1;
    if(!browser){res.writeHead(403);res.end('Desktop transport rejected by local fixture');return}
    assert(!req.headers.cookie&&!req.headers.authorization,'Browser credentials were sent');
    if(url==='/index.m3u8'){res.setHeader('Content-Type','application/vnd.apple.mpegurl');res.end('#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nmedia.m3u8\n');return}
    if(url==='/media.m3u8'){res.setHeader('Content-Type','application/vnd.apple.mpegurl');res.end('#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\na.ts\n#EXTINF:6,\nb.ts\n#EXT-X-ENDLIST\n');return}
    if(url==='/bulk.m3u8'){res.setHeader('Content-Type','application/vnd.apple.mpegurl');res.end('#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000\nbulk-media.m3u8\n');return}
    if(url==='/bulk-media.m3u8'){res.setHeader('Content-Type','application/vnd.apple.mpegurl');res.end('#EXTM3U\n#EXT-X-TARGETDURATION:6\n'+Array.from({length:34},(_,i)=>`#EXTINF:6,\nbulk-${i}.ts\n`).join('')+'#EXT-X-ENDLIST\n');return}
    if(bulk){const data=Buffer.alloc(188*22310,Number(bulk[1])+8);for(let i=0;i<data.length;i+=188)data[i]=0x47;res.setHeader('Content-Type','video/mp2t');res.end(data);return}
    if(url==='/a.ts'||url==='/b.ts'){
      if(url==='/b.ts'){while(!res.destroyed){try{await fs.access(path.join(root,'resume.ready'));break}catch{}await delay(25)}if(res.destroyed)return;}
      const data=Buffer.alloc(188*500,url==='/a.ts'?3:4);for(let i=0;i<data.length;i+=188)data[i]=0x47;
      res.setHeader('Content-Type','video/mp2t');res.end(data);return;
    }
    res.writeHead(404);res.end();
  }catch(error){readyReject(error);res.destroy();console.error(error.message)}});
  try{
    s.listen(0,'127.0.0.1');await once(s,'listening');const base='http://127.0.0.1:'+s.address().port,port=await freePort();
    await fs.writeFile(path.join(profile,'user.js'),Object.entries({'marionette.port':port,'browser.shell.checkDefaultBrowser':false,'browser.startup.homepage':'about:blank','browser.startup.page':0,'datareporting.policy.dataSubmissionEnabled':false,'toolkit.telemetry.enabled':false}).map(([key,value])=>`user_pref(${JSON.stringify(key)},${JSON.stringify(value)});`).join('\n'));
    for(const name of ['core.js','page-links.js','browser-video-relay.js'])await fs.copyFile(path.resolve('extension',name),path.join(extension,name));
    await fs.writeFile(path.join(extension,'fixture.js'),'('+extensionMain.toString()+')('+JSON.stringify(base)+','+JSON.stringify(token)+');');
    if(isFirefox){
      await fs.writeFile(path.join(extension,'manifest.json'),JSON.stringify({manifest_version:2,name:'Download Engine isolated browser test',version:'1.0',browser_specific_settings:{gecko:{id:'de-browser-fixture@local'}},permissions:['tabs','storage','scripting','http://127.0.0.1/*'],background:{scripts:['core.js','page-links.js','browser-video-relay.js','fixture.js']}}));
      ff=spawn(browserExe,['--headless','--no-remote','--profile',profile,'--marionette','about:blank'],{windowsHide:true,stdio:'ignore'});
      mar=await marionette(port);const started=await mar.call('WebDriver:NewSession',{capabilities:{alwaysMatch:{browserName:'firefox'}}});
      console.log('Isolated Firefox',started.capabilities?.browserVersion);
      await mar.call('WebDriver:Navigate',{url:base+'/watch'});
      console.log('Installed fixture:',await mar.call('Addon:Install',{path:extension,temporary:true}));
    }else{
      await fs.writeFile(path.join(extension,'background.js'),'importScripts("core.js","page-links.js","browser-video-relay.js","fixture.js");');
      await fs.writeFile(path.join(extension,'manifest.json'),JSON.stringify({manifest_version:3,name:'Download Engine isolated browser test',version:'1.0',permissions:['tabs','storage','scripting'],host_permissions:['http://127.0.0.1/*'],background:{service_worker:'background.js'}}));
      debugPort=port;
      ff=spawn(browserExe,['--headless=new','--no-first-run','--no-default-browser-check','--disable-background-networking','--user-data-dir='+profile,'--load-extension='+extension,'--disable-extensions-except='+extension,'--remote-debugging-port='+port,'--remote-debugging-address=127.0.0.1','about:blank'],{windowsHide:true,stdio:'ignore'});
    }
    await bounded(ready,20000,'Extension relay did not become ready');
    console.log(browserName+' relay ready; running real native-host and HLS recovery test.');
    driver=spawn(testExe,['--ignored','--exact','real_browser_download_pause_resume','--nocapture'],{windowsHide:true,env:{...process.env,DE_BROWSER_RELAY_FIXTURE:root},stdio:['ignore','pipe','pipe']});
    let log='';driver.stdout.on('data',b=>{log+=b;process.stdout.write(b)});driver.stderr.on('data',b=>{log+=b;process.stderr.write(b)});
    const [code]=await bounded(once(driver,'exit'),240000,'Real browser test timed out');
    assert.equal(code,0,'Rust recovery test failed');
    assert.equal(counts['browser:/a.ts'],1,'Resume downloaded the saved segment again');
    assert(counts['browser:/b.ts']>=1);assert(counts['desktop:/index.m3u8']>=1);
    for(let i=0;i<34;i++)assert.equal(counts[`browser:/bulk-${i}.ts`],1,'Parallel transfer fetched a segment twice');
    const report={browser:browserName,status:'passed',counts,traces,log};await fs.writeFile(path.join(parent,browserName.toLowerCase()+'-live-report.json'),JSON.stringify(report,null,2));
    console.log('PASS: real '+browserName+' fetch, exact bytes, pause/resume, database reload, no credentials, no duplicate job or saved-segment request.');
  }catch(error){
    console.error('Safe trace:',JSON.stringify(traces));if(hostErrors)console.error('Native host:',hostErrors);
    throw error;
  }
  finally{
    if(driver&&driver.exitCode===null)driver.kill();
    if(mar){try{await bounded(mar.call('Marionette:Quit',{flags:['eForceQuit']}),5000,'Firefox quit timeout')}catch{}mar.socket.destroy()}
    if(debugPort){try{const info=await fetch('http://127.0.0.1:'+debugPort+'/json/version',{redirect:'error',signal:AbortSignal.timeout(3000)}).then(r=>r.json());const socket=new WebSocket(info.webSocketDebuggerUrl);await bounded(once(socket,'open'),3000,'Browser close connection');socket.send(JSON.stringify({id:1,method:'Browser.close'}));await bounded(once(socket,'close'),3000,'Browser close');}catch{}}
    if(ff&&ff.exitCode===null)ff.kill();host.stdin.end();if(host.exitCode===null)host.kill();
    waiter?.destroy();s.closeAllConnections();s.close();
    // Delete only the exact mkdtemp tree after its own processes have exited.
    await delay(500);const realParent=await fs.realpath(parent),realRoot=await fs.realpath(root);assert(path.dirname(realRoot)===realParent&&path.basename(realRoot).startsWith('live-'));
    await fs.rm(realRoot,{recursive:true,force:true,maxRetries:20,retryDelay:100});
  }
}
main().catch(error=>{console.error(error);process.exitCode=1});

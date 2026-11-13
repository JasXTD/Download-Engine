const {test} = require('node:test');
const assert = require('node:assert/strict');
require('../extension/dailymotion.js');
require('../extension/media-diagnostics.js');
const {Resolver,videoId,parse} = globalThis.IDMDailymotion;
const {matchesVideo,groupStreams} = globalThis.IDMDailymotion;
const tab={id:7,url:'https://www.dailymotion.com/video/xbv5ovm'};
const url='https://cdndirector.dailymotion.com/cdn/manifest/video/xbv5ovm.m3u8?token=fixture-secret';
const metadata=()=>({id:'xbv5ovm',qualities:{auto:[{type:'application/x-mpegURL',url}]}});
const json=value=>new Response(JSON.stringify(value),{headers:{'content-type':'application/json'}});
const hls=()=>new Response('#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,RESOLUTION=1280x720\n720.m3u8\n',{headers:{'content-type':'video/mp4'}});
const rendition='https://vod.dmcdn.net/sec-token/cloud/fmp4/717461554/h264_aac_hd/1/manifest.m3u8';

test('same-video alternatives match the exact Dailymotion asset, never ads or init files',()=>{
  assert.equal(Number.parseInt('bv5ovm',36),717461554);
  for(const value of [url,rendition,rendition.replace('/cloud/','/video/')]) assert(matchesVideo(tab.url,value),value);
  for(const value of [rendition.replace('717461554','717479018'),rendition.replace('manifest.m3u8','init.mp4'),rendition.replace('dmcdn.net','dmcdn.net.evil.test'),rendition.replace('https:','http:'),rendition.replace('vod.dmcdn.net','u:p@vod.dmcdn.net'),rendition.replace('vod.dmcdn.net','vod.dmcdn.net:444'),rendition.replace('/cloud/','/preview/'),'https://vod.dmcdn.net/unknown/manifest.m3u8']) assert(!matchesVideo(tab.url,value),value);
  assert(!matchesVideo('https://evil.test/video/xbv5ovm',rendition));
});

test('browser-readable metadata retains native alternatives in one row, bounded and deduplicated',()=>{
  const primary={url,hls:true,text:'Video'},player={url:url.replace('cdndirector','dmxleo'),hls:true};
  const captured=[player,{url:rendition,hls:true},{url:rendition,hls:true},{url:rendition.replace('717461554','717479018'),hls:true}];
  const grouped=groupStreams(tab.url,[primary],captured);
  assert.equal(grouped.length,2);assert.equal(grouped[0].url,url);
  assert.equal(grouped[1].url,captured[3].url,'other assets remain an explicit choice, never an automatic substitution');
  assert.deepEqual(grouped[0].fallback_urls,[rendition,player.url]);
  assert(!Object.hasOwn(primary,'fallback_urls'));
  const bounded=groupStreams(tab.url,[primary],Array.from({length:8},(_,i)=>({url:rendition+'?fixture_case='+i,hls:true})));
  assert.equal(bounded[0].fallback_urls.length,4);
  assert.equal(bounded.length,5,'candidates beyond the automatic fallback cap remain selectable');
  const fromBrowser=groupStreams(tab.url,[],[player,{url:rendition,hls:true}]);
  assert.equal(fromBrowser.length,1);assert.deepEqual(fromBrowser[0].fallback_urls,[rendition]);
  assert.equal(groupStreams(tab.url,[primary,{url:rendition,hls:true}],captured).length,4,'explicit qualities and other captures must remain available');
});

test('metadata success during an ad cannot hide an unmatched real-video playlist',()=>{
  const primary={url,hls:true,text:'Metadata'},unknown={url:'https://vod.dmcdn.net/another-layout/main.m3u8',hls:true};
  assert.deepEqual(groupStreams(tab.url,[primary],[unknown,primary]),[primary,unknown]);
});
function fixture(overrides={}) {
  const calls=[],events=[];
  const resolver=new Resolver({trace:(...event)=>events.push(event),fetch:async(...args)=>{calls.push(args);return calls.length===1?json(metadata()):hls();},...overrides});
  return {resolver,calls,events};
}
test('Dailymotion lookup recognizes only exact public HTTPS page hosts and IDs',async()=>{
  assert.equal(videoId(tab.url),'xbv5ovm');
  assert.equal(videoId('https://dailymotion.com/embed/video/xbv5ovm_title?x=1'),'xbv5ovm');
  for(const input of ['http://www.dailymotion.com/video/xbv5ovm','https://www.dailymotion.com.evil.test/video/xbv5ovm','https://www.dailymotion.com@evil.test/video/xbv5ovm','https://u:p@www.dailymotion.com/video/xbv5ovm','https://www.dailymotion.com:444/video/xbv5ovm','https://www.dailymotion.com/video/%2e%2e/metadata','https://www.dailymotion.com/playlist/x123']) assert.equal(videoId(input),null,input);
  const f=fixture();
  assert.equal(await f.resolver.lookup({...tab,incognito:true}),null);
  assert.equal(await f.resolver.lookup({...tab,url:'https://example.test'}),null);
  assert.equal(f.calls.length,0);
});
test('fresh metadata is validated as an HLS document without fetching any media or using cookies',async()=>{
  const f=fixture(),result=await f.resolver.lookup(tab);
  assert.equal(result.reason,'success');assert.equal(result.links.length,1);assert(result.links[0].hls);
  assert.equal(f.calls.length,2);
  assert.equal(f.calls[0][0],'https://www.dailymotion.com/player/metadata/video/xbv5ovm');
  assert.equal(f.calls[1][0],url);
  for(const [,options] of f.calls){assert.equal(options.credentials,'omit');assert.equal(options.cache,'no-store');assert.equal(options.referrerPolicy,'no-referrer');}
  assert.equal(f.calls[0][1].redirect,'error');
  assert.deepEqual(f.events.map(event=>event[0]),['metadata_start','playlist_start','playlist_result','metadata_result']);
  assert(!JSON.stringify(f.events).includes('fixture-secret'));
});
test('automatic master replaces duplicate renditions but numbered quality-only streams remain available',()=>{
  const value=metadata();value.qualities.auto.push({...value.qualities.auto[0]});
  value.qualities['720']=[{type:'video/mp4',url:'https://vod.dmcdn.net/video/file.mp4'}];
  assert.equal(parse(value,'xbv5ovm').links.length,1);
  delete value.qualities.auto;
  const parsed=parse(value,'xbv5ovm');assert.equal(parsed.links.length,1);assert.match(parsed.links[0].text,/720p/);
});
test('untrusted metadata cannot inject external hosts, fragments, user info, ports or initialization files',()=>{
  const invalid=['https://localhost/a.m3u8','https://127.0.0.1/a.m3u8','https://dmcdn.net.evil.test/a.m3u8','http://vod.dmcdn.net/a.m3u8','https://a:b@vod.dmcdn.net/a.m3u8','https://vod.dmcdn.net:444/a.m3u8','file:///a.m3u8','javascript:alert(1)','https://vod.dmcdn.net/init.mp4','https://vod.dmcdn.net/preview/x.mp4'];
  for(const target of invalid){assert.equal(parse({qualities:{auto:[{url:target}]}},'xbv5ovm').links.length,0,target);}
  assert.equal(parse({id:'another',qualities:metadata().qualities},'xbv5ovm').reason,'wrong_video');
  assert.equal(parse({error:{message:'secret'}},'xbv5ovm').reason,'unavailable');
});
for(const [name,response,reason] of [
  ['HTTP failure',()=>new Response('',{status:403}),'http'],
  ['HTML instead of JSON',()=>new Response('<html>error</html>',{headers:{'content-type':'text/html'}}),'content_type'],
  ['malformed JSON',()=>new Response('{',{headers:{'content-type':'application/json'}}),'invalid_json'],
  ['oversized length',()=>new Response('{}',{headers:{'content-type':'application/json','content-length':String(1024*1024+1)}}),'too_large'],
  ['oversized streamed body',()=>new Response(' '.repeat(1024*1024+1),{headers:{'content-type':'application/json'}}),'too_large'],
  ['missing formats',()=>json({qualities:{}}),'no_formats']
]) test(name+' permits browser fallback with a safe reason',async()=>{
  const f=fixture({fetch:async()=>response()}),result=await f.resolver.lookup(tab);
  assert.equal(result.reason,reason);assert.deepEqual(result.links,[]);
});
for(const [name,response,reason] of [
  ['403 playlist',()=>new Response('',{status:403}),'playlist_http'],
  ['HTML playlist with video MIME',()=>new Response('<html>Unavailable</html>',{headers:{'content-type':'video/mp4'}}),'playlist_content'],
  ['empty HLS playlist',()=>new Response('#EXTM3U\n#EXT-X-ENDLIST'),'playlist_content'],
  ['foreign redirected URL',()=>{const r=hls();Object.defineProperty(r,'url',{value:'https://evil.test/a.m3u8'});return r;},'playlist_http']
]) test(name+' cannot replace a browser-captured playable stream',async()=>{
  let count=0;const f=fixture({fetch:async()=>++count===1?json(metadata()):response()});
  const result=await f.resolver.lookup(tab);assert.equal(result.reason,reason);assert.equal(result.links.length,0);
});
test('timeout, caller cancellation and network exceptions are bounded and do not expose errors',async()=>{
  const wait=(_url,{signal})=>new Promise((resolve,reject)=>{if(signal.aborted)reject(Error('aborted'));else signal.addEventListener('abort',()=>reject(Error('aborted')),{once:true});});
  assert.equal((await fixture({fetch:wait,timeout:15}).resolver.lookup(tab)).reason,'timeout');
  const controller=new AbortController(),f=fixture({fetch:wait});const pending=f.resolver.lookup(tab,controller.signal);controller.abort();
  assert.equal((await pending).reason,'cancelled');
  const bad=fixture({fetch:async()=>{throw Error('https://secret.test/token=SECRET');}});
  assert.equal((await bad.resolver.lookup(tab)).reason,'network');assert(!JSON.stringify(bad.events).includes('SECRET'));
});
test('direct media uses HEAD only and a readable video content type',async()=>{
  let call=0;
  const f=fixture({fetch:async(_url,options)=>{if(++call===1)return json({qualities:{720:[{url:'https://vod.dmcdn.net/a.mp4',type:'video/mp4'}]}});assert.equal(options.method,'HEAD');return new Response(null,{headers:{'content-type':'video/mp4'}});}});
  assert.equal((await f.resolver.lookup(tab)).links.length,1);
  assert.match(parse({qualities:{720:[{url:'https://vod.dmcdn.net/720.webm'}]}},'xbv5ovm').links[0].filename,/\.webm$/);
});
test('playlist bytes decide HLS versus DASH despite conflicting metadata',async()=>{
  let count=0;
  const f=fixture({fetch:async()=>++count===1?json({qualities:{auto:[{url:'https://vod.dmcdn.net/master.m3u8',type:'application/dash+xml'}]}}):hls()});
  const result=await f.resolver.lookup(tab);
  assert.equal(result.links.length,1);assert.equal(result.links[0].hls,true);assert.equal(result.links[0].dash,false);
});
test('diagnostics persist across worker restart, stay bounded and never store unrecognized fields',async()=>{
  const values={},storage={get:async key=>structuredClone({[key]:values[key]}),set:async data=>Object.assign(values,structuredClone(data)),remove:async key=>{delete values[key];}};
  const log=new IDMMediaDiagnostics.Log(storage);
  await Promise.all(Array.from({length:150},(_,i)=>log.add('network_saved',{count:i,url:'https://private/SECRET',cookie:'SECRET',reason:'SECRET',format:'SECRET'})));
  const restored=new IDMMediaDiagnostics.Log(storage),result=await restored.read();
  assert.equal(result.trace.length,120);assert.equal(result.traceError,false);assert.match(result.trace[0].message,/count=30/);
  assert(!JSON.stringify(values).includes('SECRET'));
  await log.add('handoff',{count:1,browser_agent:true,browser_relay:true,token:'SECRET relay token',user_agent:'SECRET browser value',headers:{Cookie:'SECRET'}});
  assert.match((await restored.read()).trace.at(-1).message,/browser_agent=true/);
  assert.match((await restored.read()).trace.at(-1).message,/browser_relay=true/);
  await log.add('relay_unavailable',{reason:'legacy_desktop',url:'https://private/SECRET'});
  assert.match((await restored.read()).trace.at(-1).message,/reason=legacy_desktop/);
  assert(!JSON.stringify(values).includes('SECRET'));
  values.mediaDiagnostics.push({event:'metadata_failed',fields:null},{event:'SECRET',fields:{}});
  assert(!(await restored.read()).traceError);
  await restored.clear();assert.equal((await restored.read()).trace.length,0);
  storage.set=async()=>{throw Error('Secret storage error');};await restored.add('ready');
  assert.equal((await restored.read()).traceError,true);
});

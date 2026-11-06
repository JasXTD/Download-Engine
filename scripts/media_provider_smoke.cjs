// Real Lua workers, transfer core and bundled FFmpeg. Generated media and
// loopback HTTP only; never reads or changes an installed user profile.
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const {randomUUID, createHash, createCipheriv} = require('node:crypto');
const {spawn, spawnSync} = require('node:child_process');
const {once} = require('node:events');
const {setTimeout: delay} = require('node:timers/promises');
const openDatabase = require('./profile_database.cjs');
const root = path.resolve(__dirname, '..');
const bin = path.resolve(process.argv[2] || path.join(root, 'target/debug'));
const exe = name => path.join(bin, name + (process.platform === 'win32' ? '.exe' : ''));
const ffmpeg = path.resolve(process.env.MEDIA_TEST_FFMPEG || path.join(root, 'tools/ffmpeg/ffmpeg.exe'));
const ffprobe = path.resolve(process.env.MEDIA_TEST_FFPROBE || path.join(root, 'artifacts/ffmpeg-tools/ffprobe.exe'));
const evidence = path.resolve(process.argv[3] || path.join(root, 'artifacts/media-provider-3.11.0'));
fs.mkdirSync(evidence, {recursive:true});
const home = fs.mkdtempSync(path.join(root, 'artifacts/media-provider-smoke-'));
const env = {...process.env, DOWNLOAD_ENGINE_HOME:home, IDM_EX_HOME:home};
for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const key = '603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4'; // public fixture
const checks = [], requests = [], routes = new Map();
const unknownPaths = new Set();
let app, db, success = false, slow = false, redirectedRequests = 0, forbiddenUrl, serverFailure;
const redirectServer = http.createServer((_req,res) => {
  redirectedRequests++; res.end('This origin must never be contacted');
});
function execute(program,args) {
  const result = spawnSync(program,args,{cwd:home,windowsHide:true,encoding:'utf8',timeout:30000,maxBuffer:8*1024*1024});
  assert.ifError(result.error); assert.equal(result.status,0,result.stderr); return result.stdout;
}
const probe = (file, args=[]) => JSON.parse(execute(ffprobe,['-v','error',...args,'-of','json',file]));
const server = http.createServer((req,res) => {
  try {
  const url = new URL(req.url,'http://127.0.0.1');
  requests.push({url:req.url,range:req.headers.range,credentials:!!(req.headers.cookie||req.headers.authorization)});
  if(url.pathname==='/redirect') {res.writeHead(302,{Location:forbiddenUrl});res.end();return;}
  if(url.pathname.startsWith('/authenticated/')) {
    assert.equal(req.headers.authorization,'Bearer offline-session');
    assert.equal(req.headers['user-agent'],'OfflineFixture/1');
    assert.equal(req.headers.cookie,url.pathname.endsWith('/video')?'fixture=seed':'fixture=next');
    res.setHeader('Set-Cookie','fixture=next; Path=/');
  }
  if(url.pathname==='/oversized-unknown') {res.writeHead(200,{'Content-Length':268435457});res.end();return;}
  if(url.pathname==='/truncated-unknown') {res.writeHead(200,{'Content-Length':10,'Connection':'close'});res.end('short');return;}
  if(url.pathname==='/authenticated-redirect') {
    assert.equal(req.headers.authorization,'Bearer offline-session');
    res.writeHead(302,{Location:forbiddenUrl});res.end();return;
  }
  const bytes = routes.get(url.pathname);
  if(!bytes) {res.writeHead(404);res.end();return;}
  let start=0,end=bytes.length-1;
  if(req.headers.range){const m=/^bytes=(\d+)-(\d+)$/.exec(req.headers.range);assert(m);start=Number(m[1]);end=Number(m[2]);}
  const data=bytes.subarray(start,end+1);
  res.writeHead(req.headers.range?206:200,{...(!unknownPaths.has(url.pathname)?{'Content-Length':data.length}:{'Transfer-Encoding':'chunked'}),...(req.headers.range?{'Content-Range':`bytes ${start}-${end}/${bytes.length}`}:{})});
  if(!slow){res.end(data);return;}
  let offset=0,timer;
  function send(){if(res.destroyed)return;const n=Math.min(offset+1024,data.length);res.write(data.subarray(offset,n));offset=n;if(offset===data.length)res.end();else timer=setTimeout(send,25);}
  res.on('close',()=>clearTimeout(timer));send();
  } catch(error) { serverFailure=error;res.destroy(); }
});
async function until(check,label,ms=60000){const end=Date.now()+ms;while(Date.now()<end){if(serverFailure)throw serverFailure;const value=await check();if(value)return value;await delay(40);}throw Error(`Timed out: ${label}`);}
async function cli(args,input){
  const child=spawn(exe('de-cli'),[...args,'--no-start'],{env,windowsHide:true,stdio:[input===undefined?'ignore':'pipe','pipe','pipe']});
  if(input!==undefined)child.stdin.end(input);
  let out='',err='';child.stdout.on('data',b=>out+=b);child.stderr.on('data',b=>err+=b);
  const timer=setTimeout(()=>child.kill(),30000);
  try{const [code]=await once(child,'exit');assert.equal(code,0,`${args}\n${out}\n${err}`);const result=JSON.parse(out);return result.receipt?result.receipt.result:result.result;}finally{clearTimeout(timer);}
}
const job = id => JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data);
async function finished(id,status='complete') { return until(()=>{const j=job(id);if(j.status==='failed'&&status!=='failed')throw Error(j.error);return j.status===status&&j;},status); }
async function launch(){
  const log=fs.openSync(path.join(home,'desktop.log'),'a');
  try{app=spawn(exe('de'),[],{env,windowsHide:true,stdio:['ignore',log,log]});}finally{fs.closeSync(log);}
  const pid=app.pid;
  await until(()=>{assert(app.exitCode===null&&app.signalCode===null,'Desktop exited');const file=path.join(home,'data/logs/desktop.log');return fs.existsSync(file)&&fs.readFileSync(file,'utf8').split(/\r?\n/).filter(Boolean).some(line=>{try{const x=JSON.parse(line);return x.pid===pid&&x.event==='ready';}catch{return false;}});},'desktop ready');
}
async function stop(){if(app&&app.exitCode===null&&app.signalCode===null){const ended=once(app,'exit');app.kill();await ended;}app=null;}
async function save(body){const script={id:randomUUID(),revision:0,name:'Offline media provider',language:'lua',trigger:{kind:'provider'},body,enabled:false,change_files:false,timeout_seconds:60};const file=path.join(home,script.id+'.json');fs.writeFileSync(file,JSON.stringify(script));return cli(['scripts','save',file]);}
const mark = name => {checks.push(name);console.log(name);};
function receipt(id){try{return JSON.parse(fs.readFileSync(path.join(home,'data/parts',id,'provider-media.json')));}catch(e){if(['ENOENT','EBUSY','EACCES'].includes(e.code))return null;throw e;}}
async function main(){
  redirectServer.listen(0,'127.0.0.1');await once(redirectServer,'listening');
  forbiddenUrl=`http://127.0.0.1:${redirectServer.address().port}/private`;
  server.listen(0,'127.0.0.1');await once(server,'listening');
  const base=`http://127.0.0.1:${server.address().port}`;
  const names=['video','english','japanese','captions'];
  for(const [index,name] of names.entries()){
    const file=path.join(home,name+(index===3?'.srt':'.mp4'));
    if(index===3){fs.writeFileSync(file,'1\n00:00:00,000 --> 00:00:02,000\nOwned offline fixture\n\n');continue;}
    execute(ffmpeg,['-hide_banner','-nostdin','-loglevel','error','-f','lavfi','-i',index===0?'testsrc2=size=160x90:rate=25:duration=3':`sine=frequency=${index===1?440:880}:sample_rate=48000:duration=3`,
      ...(index===0?['-c:v','libx264','-threads','1','-g','25']:['-c:a','aac','-b:a','96k']),'-movflags','frag_keyframe+empty_moov+default_base_moof',file]);
  }
  const counters={}, tracks=names.map((name,index)=>{
    const bytes=fs.readFileSync(path.join(home,name+(index===3?'.srt':'.mp4')));
    const counter=Buffer.alloc(16);counter.writeUInt32BE(index+1,0);counters[name]=counter.toString('hex');
    const cipher=createCipheriv('aes-256-ctr',Buffer.from(key,'hex'),counter);
    const encrypted=index===3?bytes:Buffer.concat([cipher.update(bytes),cipher.final()]);
    // Use ranged requests for one track and separate resources for the others.
    routes.set('/'+name,encrypted);
    const segments=[];
    for(let offset=0;offset<bytes.length;offset+=8192){const n=Math.min(8192,bytes.length-offset),id='part'+segments.length;
      const encryptedPart=encrypted.subarray(offset,offset+n);routes.set('/'+name+'/'+id,encryptedPart);
      segments.push({id,url:index===2?'/'+name:'/'+name+'/'+id,size:n,...(index===2?{range_start:offset}:{}),processing:index===3?'copy':'lua',input_sha256:digest(encryptedPart),output_sha256:digest(bytes.subarray(offset,offset+n))});}
    return {id:name,kind:index===0?'video':index===3?'subtitle':'audio',format:index===3?'srt':'mp4',language:index===2?'jpn':'eng',title:name,default:index!==2,segments};
  });
  const plan={content_id:'offline-media-v1',container:'mkv',tracks};
  const source=`assert(xd.media_api == 1)\nlocal counters=xd.json([==[${JSON.stringify(counters)}]==])\nlocal function resolve(request)\n local plan=xd.json([==[${JSON.stringify(plan)}]==])\n for _,track in ipairs(plan.tracks) do for _,segment in ipairs(track.segments) do segment.url=xd.url_join(request.url,segment.url)..'?renewed=1' end end\n return xd.media_plan(plan)\nend\nreturn {resolve=resolve,refresh=function(request,previous) return resolve(request) end,transform=function(context,bytes,offset)\n assert(xd.fetch==nil and context.url==nil and context.path==nil)\n local key='${key}'\n return xd.aes_ctr(key,counters[context.track_id],bytes,context.track_offset+offset)\nend,finish=function(plan,result) assert(result.stage=='before_mux' and #plan.tracks==4); return true end}`;
  await cli(['queues']);db=openDatabase(path.join(home,'data/queue.sqlite3'));
  db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0');
  db.exec("UPDATE interface_preferences SET data=json_set(data,'$.check_updates_on_launch',json('false')) WHERE id=1");
  db.prepare('UPDATE proxy_settings SET data=? WHERE id=1').run(JSON.stringify({mode:'direct'}));
  await launch();
  const script=await save(source), request=randomUUID();
  const added=await cli(['scripts','download',script.id,base,'--name','multitrack','--request-id',request]);
  const complete=await finished(added.id);
  assert(complete.saved_path.endsWith('.mkv'));assert(path.resolve(complete.saved_path).startsWith(path.join(home,'downloads')+path.sep));
  const streams=probe(complete.saved_path,['-show_streams']).streams;
  assert.deepEqual(streams.map(s=>s.codec_type),['video','audio','audio','subtitle']);
  assert.deepEqual(streams.map(s=>s.tags.language),['eng','eng','jpn','eng']);
  assert.equal(streams[1].disposition.default,1);assert.equal(streams[2].disposition.default,0);
  for(let i=0;i<3;i++){
    const hashes=(file,select)=>probe(file,['-select_streams',select,'-show_packets','-show_data_hash','sha256','-show_entries','packet=data_hash']).packets.map(p=>p.data_hash);
    assert.deepEqual(hashes(complete.saved_path,String(i)),hashes(path.join(home,names[i]+'.mp4'),'0'),'Mux must preserve encoded media packets');
  }
  const again=await cli(['scripts','download',script.id,base,'--name','multitrack','--request-id',request]);assert.equal(again.id,added.id);
  const stored=db.prepare('SELECT data FROM jobs WHERE id=?').get(added.id).data;
  assert(!stored.includes('?renewed=1')&&!stored.includes(key));assert(requests.some(r=>r.range));assert(!requests.some(r=>r.credentials));
  mark('Encrypted segments and ranges produce MKV with video, two exact audio tracks, subtitles, language/default metadata and one idempotent download');

  const mp4=await save(source.replace('"container":"mkv"','"container":"mp4"'));
  const mp4job=await cli(['scripts','download',mp4.id,base,'--name','multitrack-mp4']);
  const mp4done=await finished(mp4job.id);assert(mp4done.saved_path.endsWith('.mp4'));
  assert.deepEqual(probe(mp4done.saved_path,['-show_streams']).streams.map(s=>s.codec_type),['video','audio','audio','subtitle']);
  mark('MP4 output maps all four tracks and converts subtitles using the fixed mov_text option');

  const extensionSource=source.replace(`local key='${key}'`,"local key=xd.key_setting('content_key')");
  const extension={format:'download-engine-extension',format_version:1,id:'offline.media',name:'Offline media',version:'1.0.0',publisher:'Local fixture',description:'Generated samples',api_version:1,provider_api:1,permissions:{network:['example.invalid']},settings:[{id:'content_key',type:'aes_key',label:'Fixture key'}],jobs:[{id:'media',name:'Media',entry:'provider.lua',timeout_seconds:60,trigger:{kind:'provider'}}],files:{'provider.lua':extensionSource}};
  const extFile=path.join(home,'offline.de-extension.json');fs.writeFileSync(extFile,JSON.stringify(extension));
  await cli(['extensions','install',extFile]);await cli(['extensions','enable','offline.media']);await cli(['extensions','set-secret','offline.media','content_key'],key+'\n');
  const loaded=await cli(['extensions','show','offline.media']);
  const extensionJob=await cli(['scripts','download',loaded.jobs[0].script_id,base,'--name','extension']);await finished(extensionJob.id);
  mark('Installed Lua extension decrypts and muxes through an opaque saved AES key');

  const sessionSource=source
    .replace('local function resolve(request)', `local function resolve(request)\n for _,name in ipairs({'video','english','japanese'}) do xd.handoff_key(name,string.char(${[...Buffer.from(key,'hex')].join(',')})) end`)
    .replace(`local key='${key}'`, 'local key=xd.session_key(context.track_id)');
  const sessionExtension={...extension,id:'offline.session',settings:[],files:{'provider.lua':sessionSource}};
  const sessionFile=path.join(home,'session.de-extension.json');fs.writeFileSync(sessionFile,JSON.stringify(sessionExtension));
  await cli(['extensions','install',sessionFile]);await cli(['extensions','enable','offline.session']);
  const sessionLoaded=await cli(['extensions','show','offline.session']);
  const extensionSession=await cli(['scripts','download',sessionLoaded.jobs[0].script_id,base,'--name','extension-session']);
  const sessionComplete=await finished(extensionSession.id);
  assert.deepEqual(probe(sessionComplete.saved_path,['-show_streams']).streams.map(s=>s.codec_type),['video','audio','audio','subtitle']);
  const extensionStored=db.prepare('SELECT data FROM jobs WHERE id=?').get(extensionSession.id).data;
  assert(!extensionStored.includes('provider-key-v1:')&&!extensionStored.includes('handoff'));
  mark('An installed extension hands off three session keys without saved key settings and muxes all four tracks');
  const sessionScript=await save(sessionSource);
  slow=true;const paused=await cli(['scripts','download',sessionScript.id,base,'--name','resume']);
  await until(()=>receipt(paused.id)?.inputs.filter(Boolean).length>=2,'two durable segments');
  await cli(['pause',paused.id]);await finished(paused.id,'paused');
  const count=receipt(paused.id).inputs.filter(Boolean).length;
  const savedSession=db.prepare('SELECT data FROM jobs WHERE id=?').get(paused.id).data;
  assert(!savedSession.includes(key)&&!savedSession.includes('provider-key-v1:')&&!savedSession.includes('handoff'));
  assert(!JSON.stringify(receipt(paused.id)).includes('provider-key-v1:'));
  const before=requests.length;await stop();await launch();slow=false;
  await cli(['resume',paused.id]);await finished(paused.id);
  const first=tracks[0].segments[0].url;assert(!requests.slice(before).some(r=>r.url.startsWith(first+'?')));
  assert.deepEqual(probe(job(paused.id).saved_path,['-show_streams']).streams.map(s=>s.codec_type),['video','audio','audio','subtitle']);
  mark(`Pause and process restart reacquire three session keys and reuse ${count} verified segments without duplicating the job; all four tracks mux correctly`);

  slow=true;const crashed=await cli(['scripts','download',sessionScript.id,base,'--name','crash']);
  await until(()=>receipt(crashed.id)?.inputs.filter(Boolean).length>=2,'crash checkpoint');await stop();const durable=receipt(crashed.id).inputs.filter(Boolean).length;
  slow=false;await launch();if(job(crashed.id).status!=='complete')await cli(['resume',crashed.id]);await finished(crashed.id);
  mark(`Forced desktop termination reacquires session keys, recovers ${durable} durable segments and completes the existing download`);

  slow=true;const corrupt=await cli(['scripts','download',script.id,base,'--name','corrupt']);
  await until(()=>receipt(corrupt.id)?.inputs[0],'corruption fixture checkpoint');await cli(['pause',corrupt.id]);await finished(corrupt.id,'paused');
  const cache=path.join(home,'data/parts',corrupt.id,'provider-media-0.in');const bytes=fs.readFileSync(cache);bytes[0]^=1;fs.writeFileSync(cache,bytes);
  slow=false;await cli(['resume',corrupt.id]);assert.match((await finished(corrupt.id,'failed')).error,/Saved media segment changed/);assert(!job(corrupt.id).saved_path);
  await cli(['restart',corrupt.id]);await finished(corrupt.id);
  mark('Corrupt saved ciphertext fails closed; explicit Restart clears its plan and recovers');

  const changed=await save(source.replace('refresh=function(request,previous) return resolve(request) end',
    "refresh=function(request,previous) local plan=resolve(request);plan.content_id='different-revision';return plan end"));
  slow=true;const stale=await cli(['scripts','download',changed.id,base,'--name','changed-content']);
  await until(()=>receipt(stale.id)?.inputs[0],'changed-content checkpoint');await cli(['pause',stale.id]);await finished(stale.id,'paused');
  slow=false;const requestsBeforeRefresh=requests.length;await cli(['resume',stale.id]);
  assert.match((await finished(stale.id,'failed')).error,/media content changed/);assert(!job(stale.id).saved_path);
  assert.equal(requests.length,requestsBeforeRefresh,'Changed content must fail before fetching another segment');
  mark('Refresh rejects changed content before downloading or combining it with saved segments');

  const wrong=await save(source.replace(`local key='${key}'`,"local key='"+'00'.repeat(32)+"'"));
  const bad=await cli(['scripts','download',wrong.id,base,'--name','wrong-key']);assert.match((await finished(bad.id,'failed')).error,/plaintext.*checksum/);assert(!job(bad.id).saved_path);
  mark('Wrong-key plaintext never reaches muxing or publication');

  const damaged=await save(source.replace('return xd.media_plan(plan)',
    'for _,track in ipairs(plan.tracks) do for _,segment in ipairs(track.segments) do segment.output_sha256=nil end end;return xd.media_plan(plan)')
    .replace('return xd.aes_ctr(key,counters[context.track_id],bytes,context.track_offset+offset)',"return string.rep('Z',#bytes)"));
  const invalid=await cli(['scripts','download',damaged.id,base,'--name','invalid-media']);
  assert.equal((await finished(invalid.id,'failed')).error,'Could not mux the media tracks. They may be damaged or incompatible with the selected container.');
  assert(!job(invalid.id).saved_path);
  mark('Invalid media fails muxing without publishing a file or exposing decrypted bytes in the job error');

  const hanging=await save(sessionSource.replace('return xd.aes_ctr(key,counters[context.track_id],bytes,context.track_offset+offset)','while true do end'));
  const busy=await cli(['scripts','download',hanging.id,base,'--name','stop']);
  await until(()=>receipt(busy.id)?.inputs.every(Boolean),'all encrypted segments');await delay(300);
  // Desktop Stop sends Command::Pause; CLI pause reaches the identical scheduler
  // cancellation path. Window-closing behavior is outside this headless test.
  const started=Date.now();await cli(['pause',busy.id]);await finished(busy.id,'paused');assert(Date.now()-started<10000);assert(!job(busy.id).saved_path);
  mark('The Stop/Pause scheduler path kills a busy Lua processor promptly and keeps segment receipts');
  const redirect=await save(source.replace("segment.url=xd.url_join(request.url,segment.url)..'?renewed=1'","segment.url=xd.url_join(request.url,'/redirect')"));
  const refused=await cli(['scripts','download',redirect.id,base,'--name','redirect']);await finished(refused.id,'failed');assert(!job(refused.id).saved_path);
  assert.equal(redirectedRequests,0,'The redirect target must never receive a request');
  mark('Cross-origin segment redirects are refused before contacting the new origin');
  const stopping=[];
  for(let i=0;i<2;i++)stopping.push(await cli(['scripts','download',hanging.id,base,'--name',`stop-all-${i}`]));
  await until(()=>stopping.every(j=>receipt(j.id)?.inputs.every(Boolean)),'two active processors');
  const stopAll=Date.now();await cli(['pause-all']);for(const j of stopping)await finished(j.id,'paused');
  assert(Date.now()-stopAll<10000);
  const stable=stopping.map(j=>JSON.stringify(receipt(j.id)));await delay(300);
  assert.deepEqual(stopping.map(j=>JSON.stringify(receipt(j.id))),stable);assert(stopping.every(j=>!job(j.id).saved_path));
  mark('Stop all scheduler path joins both busy processors; no receipt writes continue after pause acknowledgment');

  // A deliberately simple owned envelope exercises variable-length MP4 processing.
  // DEMP + public CTR nonce + original byte offset precedes each encrypted piece.
  // This is a teaching fixture, not a CENC or Widevine implementation.
  const wholeTracks=structuredClone(tracks);
  for(const track of wholeTracks){
    if(track.kind==='subtitle')continue;
    track.init_segment=track.segments[0].id;
    let originalOffset=0;
    for(const segment of track.segments){
      const resource=routes.get(segment.url);
      const cipher=segment.range_start===undefined?resource:resource.subarray(segment.range_start,segment.range_start+segment.size);
      const header=Buffer.alloc(28);header.write('DEMP');Buffer.from(counters[track.id],'hex').copy(header,4);header.writeBigUInt64BE(BigInt(originalOffset),20);
      originalOffset+=segment.size;
      const bytes=Buffer.concat([header,cipher]);
      segment.url='/whole/'+track.id+'/'+segment.id;delete segment.range_start;
      routes.set(segment.url,bytes);segment.size=bytes.length;segment.processing='lua_segment';segment.input_sha256=digest(bytes);
    }
  }
  const wholePlan={...plan,content_id:'offline-whole-segments-v1',tracks:wholeTracks};
  const wholeSource=`assert(xd.segment_api==1 and xd.supports('media.segment_processing'))
local prepared={}
local function nonce(bytes) assert(bytes:sub(1,4)=='DEMP');return (bytes:sub(5,20):gsub('.',function(c)return string.format('%02x',string.byte(c))end)) end
local function resolve(request)
 for _,id in ipairs({'video','english','japanese'}) do xd.handoff_key(id,string.char(${[...Buffer.from(key,'hex')].join(',')})) end
 local plan=xd.json([==[${JSON.stringify(wholePlan)}]==])
 for _,track in ipairs(plan.tracks) do for _,segment in ipairs(track.segments) do segment.url=xd.url_join(request.url,segment.url) end end
 return xd.media_plan(plan)
end
return {resolve=resolve,refresh=function(request) return resolve(request) end,
 transform=function(_,bytes)return bytes end,
 prepare_track=function(context,bytes) prepared[context.track_id]=nonce(bytes);return true end,
 transform_segment=function(context,bytes)
  assert(xd.fetch==nil and xd.http_session==nil and context.url==nil)
  assert(prepared[context.track_id]==nonce(bytes),'Initialization must replay after resume')
  assert(context.output_offset==string.unpack('>I8',bytes,21),'Output offsets must include resumed processed segments')
  if context.track_id=='video' and context.segment_id=='part1' and not xd.setting('continue_processing') then while true do end end
  return xd.aes_ctr(xd.session_key(context.track_id),prepared[context.track_id],bytes:sub(29),(string.unpack('>I8',bytes,21)))
 end,
 finish=function(plan,result) assert(result.size<${wholePlan.tracks.flatMap(t=>t.segments).reduce((n,s)=>n+s.size,0)});return true end}`;
  const wholeExtension={...extension,id:'offline.whole',settings:[{id:'continue_processing',type:'boolean',label:'Continue offline processing',default:false}],files:{'provider.lua':wholeSource}};
  const wholeFile=path.join(home,'whole.de-extension.json');fs.writeFileSync(wholeFile,JSON.stringify(wholeExtension));
  await cli(['extensions','install',wholeFile]);await cli(['extensions','enable','offline.whole']);
  const wholeLoaded=await cli(['extensions','show','offline.whole']);
  const wholeScript=wholeLoaded.jobs[0].script_id;
  const whole=await cli(['scripts','download',wholeScript,base,'--name','whole-resume']);
  await until(()=>receipt(whole.id)?.outputs[0],'first variable-length output checkpoint');
  await cli(['pause',whole.id]);await finished(whole.id,'paused');
  const firstReceipt=receipt(whole.id);
  assert.equal(firstReceipt.output_sizes[0],wholeTracks[0].segments[0].size-28);
  assert.equal(firstReceipt.outputs[1],null);
  const wholeRequestCount=requests.length;
  await stop();await launch();await cli(['extensions','set','offline.whole','continue_processing','true']);
  await cli(['resume',whole.id]);const wholeDone=await finished(whole.id);
  assert.equal(requests.length,wholeRequestCount,'Resume must reuse every verified input');
  assert.deepEqual(probe(wholeDone.saved_path,['-show_streams']).streams.map(s=>s.codec_type),['video','audio','audio','subtitle']);
  for(let i=0;i<3;i++){
    const hashes=(file,select)=>probe(file,['-select_streams',select,'-show_packets','-show_data_hash','sha256','-show_entries','packet=data_hash']).packets.map(p=>p.data_hash);
    assert.deepEqual(hashes(wholeDone.saved_path,String(i)),hashes(path.join(home,names[i]+'.mp4'),'0'));
  }
  mark('Whole-segment callbacks remove envelope metadata, replay initialization after Pause/restart, reuse all inputs and preserve every MP4 media packet');

  await cli(['extensions','set','offline.whole','continue_processing','false']);
  const wholeCrash=await cli(['scripts','download',wholeScript,base,'--name','whole-crash']);
  await until(()=>receipt(wholeCrash.id)?.outputs[0],'whole-segment crash checkpoint');await stop();
  await launch();await cli(['pause',wholeCrash.id]);await finished(wholeCrash.id,'paused');
  await cli(['extensions','set','offline.whole','continue_processing','true']);
  await cli(['resume',wholeCrash.id]);await finished(wholeCrash.id);
  mark('Crash recovery verifies variable-length outputs and restores initialization for a fresh processor');

  await cli(['extensions','set','offline.whole','continue_processing','false']);
  const wholeCorrupt=await cli(['scripts','download',wholeScript,base,'--name','whole-corrupt']);
  await until(()=>receipt(wholeCorrupt.id)?.outputs[0],'whole corruption checkpoint');await cli(['pause',wholeCorrupt.id]);await finished(wholeCorrupt.id,'paused');
  const wholeOutput=path.join(home,'data/parts',wholeCorrupt.id,'provider-media-0.out');fs.appendFileSync(wholeOutput,Buffer.from([1]));
  await cli(['resume',wholeCorrupt.id]);assert.match((await finished(wholeCorrupt.id,'failed')).error,/Saved media segment size changed/);
  assert(!job(wholeCorrupt.id).saved_path);
  mark('A changed variable-length output is rejected before initialization or muxing');

  const wholeStopping=[];
  for(let i=0;i<2;i++)wholeStopping.push(await cli(['scripts','download',wholeScript,base,'--name',`whole-stop-${i}`]));
  await until(()=>wholeStopping.every(j=>receipt(j.id)?.outputs[0]),'two whole-segment processors');
  const stopWhole=Date.now();await cli(['pause-all']);for(const j of wholeStopping)await finished(j.id,'paused');
  assert(Date.now()-stopWhole<10000);
  const wholeStable=wholeStopping.map(j=>JSON.stringify(receipt(j.id)));await delay(300);
  assert.deepEqual(wholeStopping.map(j=>JSON.stringify(receipt(j.id))),wholeStable);
  mark('Stop all cancels both whole-segment Lua callbacks without late checkpoint writes');
  const ass=Buffer.from(`[Script Info]\nScriptType: v4.00+\nPlayResX: 160\nPlayResY: 90\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: FixtureStyle,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,5,5,5,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:02.00,FixtureStyle,,0,0,0,,{\\b1\\c&H00FF00&}Owned styled subtitle\n`);
  const originalVideo=fs.readFileSync(path.join(home,'video.mp4'));
  routes.set('/authenticated/video',originalVideo);routes.set('/authenticated/ass',ass);
  unknownPaths.add('/authenticated/video');unknownPaths.add('/authenticated/ass');
  const styledPlan={content_id:'styled-offline',container:'mkv',tracks:[
    {id:'video',kind:'video',format:'mp4',segments:[{id:'one',url:'/authenticated/video',session:'media',input_sha256:digest(originalVideo)}]},
    {id:'ass',kind:'subtitle',format:'ass',language:'eng',segments:[{id:'one',url:'/authenticated/ass',session:'media',input_sha256:digest(ass)}]}
  ]};
  const styledSource=`local function resolve(request)
    local session=xd.http_session(request.url,{cookies={fixture='seed'},headers={Authorization='Bearer offline-session',['User-Agent']='OfflineFixture/1'}})
    xd.handoff_session('media',session)
    local p=xd.json([==[${JSON.stringify(styledPlan)}]==])
    for _,t in ipairs(p.tracks) do for _,s in ipairs(t.segments) do s.url=xd.url_join(request.url,s.url) end end
    return xd.media_plan(p)
  end
  return {resolve=resolve,refresh=resolve,transform=function(_,b)return b end,finish=function(_,r)return r.size>0 end}`;
  const styledScript=await save(styledSource);
  const styled=await cli(['scripts','download',styledScript.id,base,'--name','styled-authenticated']);
  const styledDone=await finished(styled.id);
  const assText=file=>execute(ffmpeg,['-hide_banner','-nostdin','-loglevel','error','-i',file,'-map','0:s:0','-c','copy','-f','ass','pipe:1']);
  const renderedAss=assText(styledDone.saved_path);
  assert(renderedAss.includes('Style: FixtureStyle,Arial,20') && renderedAss.includes('{\\b1\\c&H00FF00&}Owned styled subtitle'));
  assert.equal(probe(styledDone.saved_path,['-show_streams']).streams[1].codec_name,'ass');
  assert(!JSON.stringify(job(styled.id)).includes('Bearer offline-session'));
  assert(!JSON.stringify(job(styled.id)).includes('fixture=next'));
  assert.equal(styledDone.provider.media.tracks[0].segments[0].size,0);
  mark('Unknown-size authenticated segments use supplied headers, learn cookies between requests, preserve ASS styles in MKV and keep credentials out of saved jobs');

  const styledMp4=await save(styledSource.replace('"container":"mkv"','"container":"mp4"'));
  const styledMp4Job=await cli(['scripts','download',styledMp4.id,base,'--name','styled-mp4']);
  assert.equal(probe((await finished(styledMp4Job.id)).saved_path,['-show_streams']).streams[1].codec_name,'mov_text');
  mark('ASS subtitles convert to mov_text for MP4 without changing the selected container');

  const sessionRedirect=await save(styledSource.replace('/authenticated/video','/authenticated-redirect'));
  const beforeRedirect=redirectedRequests;
  const redirectJob=await cli(['scripts','download',sessionRedirect.id,base,'--name','session-redirect']);
  assert.match((await finished(redirectJob.id,'failed')).error,/origin|redirect/i);
  assert.equal(redirectedRequests,beforeRedirect);
  assert(!job(redirectJob.id).saved_path);
  mark('Authenticated segment redirects fail before another origin receives a request or credentials');

  const free=Buffer.alloc(9*1024*1024);free.writeUInt32BE(free.length,0);free.write('free',4);
  const largeVideo=Buffer.concat([originalVideo,free]);
  const streamCipher=createCipheriv('aes-256-ctr',Buffer.from(key,'hex'),Buffer.alloc(16));
  const largeEncrypted=Buffer.concat([Buffer.from('DEMP'),streamCipher.update(largeVideo),streamCipher.final()]);
  routes.set('/stream/video',largeEncrypted);routes.set('/stream/ass',ass);
  unknownPaths.add('/stream/video');unknownPaths.add('/stream/ass');
  const streamPlan={...styledPlan,content_id:'streamed-offline',tracks:structuredClone(styledPlan.tracks)};
  for(const track of streamPlan.tracks){const s=track.segments[0];s.url='/stream/'+track.id;delete s.session;s.processing='lua_stream';s.input_sha256=digest(track.id==='video'?largeEncrypted:ass);s.output_sha256=digest(track.id==='video'?largeVideo:ass);}
  const streamSource=`assert(xd.supports('media.stream_processing'))
  local function resolve(request)
    local p=xd.json([==[${JSON.stringify(streamPlan)}]==])
    for _,t in ipairs(p.tracks) do for _,s in ipairs(t.segments) do s.url=xd.url_join(request.url,s.url) end end
    return xd.media_plan(p)
  end
  return {resolve=resolve,refresh=resolve,transform=function(_,b)return b end,finish=function(_,r)return r.size>0 end,
    transform_stream=function(context,input,output)
      if context.track_id=='ass' and not xd.setting('continue_processing') then while true do end end
      local offset=context.track_id=='video' and 4 or 0
      while offset<input:size() do
        local bytes=input:read(offset,262144)
        if context.track_id=='video' then bytes=xd.aes_ctr('${key}','00000000000000000000000000000000',bytes,offset-4) end
        output:write(bytes);offset=offset+#bytes
      end
      return true
    end}`;
  const streamExtension={...wholeExtension,id:'offline.streaming',files:{'provider.lua':streamSource}};
  const streamFile=path.join(home,'stream.de-extension.json');fs.writeFileSync(streamFile,JSON.stringify(streamExtension));
  await cli(['extensions','install',streamFile]);await cli(['extensions','enable','offline.streaming']);
  const streamScript=(await cli(['extensions','show','offline.streaming'])).jobs[0].script_id;
  const streaming=await cli(['scripts','download',streamScript,base,'--name','stream-resume']);
  await until(()=>receipt(streaming.id)?.outputs[0],'large streamed output checkpoint');
  await cli(['pause',streaming.id]);await finished(streaming.id,'paused');
  const checkpoint=receipt(streaming.id);
  assert.deepEqual(checkpoint.input_sizes,[largeEncrypted.length,ass.length]);
  assert.equal(checkpoint.output_sizes[0],largeVideo.length);
  const savedCount=requests.length;
  await stop();await launch();await cli(['extensions','set','offline.streaming','continue_processing','true']);
  await cli(['resume',streaming.id]);const streamDone=await finished(streaming.id);
  assert.equal(requests.length,savedCount);
  const packets=file=>probe(file,['-select_streams','v:0','-show_packets','-show_data_hash','sha256','-show_entries','packet=data_hash']).packets.map(p=>p.data_hash);
  assert.deepEqual(packets(streamDone.saved_path),packets(path.join(home,'video.mp4')));
  assert(assText(streamDone.saved_path).includes('Owned styled subtitle'));
  mark('A >9 MiB encrypted stream is rewritten in bounded chunks; Pause/restart/resume reuses measured inputs and verified outputs without new downloads');

  await cli(['extensions','set','offline.streaming','continue_processing','false']);
  const crashStream=await cli(['scripts','download',streamScript,base,'--name','stream-crash']);
  await until(()=>receipt(crashStream.id)?.outputs[0],'stream crash checkpoint');await stop();
  await launch();await cli(['pause',crashStream.id]);await finished(crashStream.id,'paused');
  await cli(['extensions','set','offline.streaming','continue_processing','true']);await cli(['resume',crashStream.id]);
  await finished(crashStream.id);
  mark('Crash recovery restores streamed output receipts and finishes the same download');

  await cli(['extensions','set','offline.streaming','continue_processing','false']);
  const corruptStream=await cli(['scripts','download',streamScript,base,'--name','stream-corrupt']);
  await until(()=>receipt(corruptStream.id)?.outputs[0],'stream corruption checkpoint');
  await cli(['pause',corruptStream.id]);await finished(corruptStream.id,'paused');
  const savedInput=path.join(home,'data/parts',corruptStream.id,'provider-media-0.in');fs.appendFileSync(savedInput,Buffer.from([1]));
  await cli(['resume',corruptStream.id]);assert.match((await finished(corruptStream.id,'failed')).error,/Saved media segment size changed/);
  assert(!job(corruptStream.id).saved_path);
  mark('Unknown-size checkpoint corruption is rejected before processing or muxing');

  const stoppedStream=await cli(['scripts','download',streamScript,base,'--name','stream-stop']);
  await until(()=>receipt(stoppedStream.id)?.outputs[0],'stream stop checkpoint');
  await cli(['pause-all']);await finished(stoppedStream.id,'paused');
  const stableStream=JSON.stringify(receipt(stoppedStream.id));await delay(300);assert.equal(JSON.stringify(receipt(stoppedStream.id)),stableStream);
  mark('Stop all terminates a streaming callback with no late writes');

  const badPlan={...styledPlan,content_id:'oversized',tracks:[{id:'video',kind:'video',format:'mp4',segments:[{id:'one',url:base+'/oversized-unknown'}]}]};
  const badScript=await save(`local function resolve()return xd.media_plan(xd.json([==[${JSON.stringify(badPlan)}]==]))end return {resolve=resolve,refresh=resolve,transform=function(_,b)return b end,finish=function()return true end}`);
  const oversizedJob=await cli(['scripts','download',badScript.id,base,'--name','oversized']);
  assert.match((await finished(oversizedJob.id,'failed')).error,/input limit/);
  assert(!job(oversizedJob.id).saved_path);
  mark('An oversized unknown-length segment is rejected from its headers before any file is published');
  const truncatedScript=await save(badScript.body.replace('/oversized-unknown','/truncated-unknown'));
  const truncatedJob=await cli(['scripts','download',truncatedScript.id,base,'--name','truncated']);
  await finished(truncatedJob.id,'failed');
  assert(!job(truncatedJob.id).saved_path);
  mark('A truncated response with an advertised size cannot publish an unknown-size segment');
  success=true;
}
main().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  await stop();db?.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
  redirectServer.closeAllConnections();await new Promise(resolve=>redirectServer.close(resolve));
  assert(path.resolve(home).startsWith(path.join(root,'artifacts')+path.sep)&&path.basename(home).startsWith('media-provider-smoke-'));
  fs.rmSync(home,{recursive:true,force:true,maxRetries:10,retryDelay:150});
  fs.writeFileSync(path.join(evidence,'media-smoke.json'),JSON.stringify({success,checks,network:'loopback only; generated encrypted samples',binary:bin,cleaned:!fs.existsSync(home)},null,2)+'\n');
});

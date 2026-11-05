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
let app, db, success = false, slow = false, redirectedRequests = 0, forbiddenUrl;
const redirectServer = http.createServer((_req,res) => {
  redirectedRequests++; res.end('This origin must never be contacted');
});
function execute(program,args) {
  const result = spawnSync(program,args,{cwd:home,windowsHide:true,encoding:'utf8',timeout:30000,maxBuffer:8*1024*1024});
  assert.ifError(result.error); assert.equal(result.status,0,result.stderr); return result.stdout;
}
const probe = (file, args=[]) => JSON.parse(execute(ffprobe,['-v','error',...args,'-of','json',file]));
const server = http.createServer((req,res) => {
  const url = new URL(req.url,'http://127.0.0.1');
  requests.push({url:req.url,range:req.headers.range,credentials:!!(req.headers.cookie||req.headers.authorization)});
  if(url.pathname==='/redirect') {res.writeHead(302,{Location:forbiddenUrl});res.end();return;}
  const bytes = routes.get(url.pathname);
  if(!bytes) {res.writeHead(404);res.end();return;}
  let start=0,end=bytes.length-1;
  if(req.headers.range){const m=/^bytes=(\d+)-(\d+)$/.exec(req.headers.range);assert(m);start=Number(m[1]);end=Number(m[2]);}
  const data=bytes.subarray(start,end+1);
  res.writeHead(req.headers.range?206:200,{'Content-Length':data.length,...(req.headers.range?{'Content-Range':`bytes ${start}-${end}/${bytes.length}`}:{})});
  if(!slow){res.end(data);return;}
  let offset=0,timer;
  function send(){if(res.destroyed)return;const n=Math.min(offset+1024,data.length);res.write(data.subarray(offset,n));offset=n;if(offset===data.length)res.end();else timer=setTimeout(send,25);}
  res.on('close',()=>clearTimeout(timer));send();
});
async function until(check,label,ms=60000){const end=Date.now()+ms;while(Date.now()<end){const value=await check();if(value)return value;await delay(40);}throw Error(`Timed out: ${label}`);}
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

  slow=true;const paused=await cli(['scripts','download',script.id,base,'--name','resume']);
  await until(()=>receipt(paused.id)?.inputs.filter(Boolean).length>=2,'two durable segments');
  await cli(['pause',paused.id]);await finished(paused.id,'paused');
  const count=receipt(paused.id).inputs.filter(Boolean).length;
  const before=requests.length;await stop();await launch();slow=false;
  await cli(['resume',paused.id]);await finished(paused.id);
  const first=tracks[0].segments[0].url;assert(!requests.slice(before).some(r=>r.url.startsWith(first+'?')));
  mark(`Pause and process restart reuse ${count} verified segments without redownloading the first segment or creating another job`);

  slow=true;const crashed=await cli(['scripts','download',script.id,base,'--name','crash']);
  await until(()=>receipt(crashed.id)?.inputs.filter(Boolean).length>=2,'crash checkpoint');await stop();const durable=receipt(crashed.id).inputs.filter(Boolean).length;
  slow=false;await launch();if(job(crashed.id).status!=='complete')await cli(['resume',crashed.id]);await finished(crashed.id);
  mark(`Forced desktop termination recovers ${durable} durable segments and completes the existing download`);

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

  const hanging=await save(source.replace('return xd.aes_ctr(key,counters[context.track_id],bytes,context.track_offset+offset)','while true do end'));
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
  success=true;
}
main().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  await stop();db?.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
  redirectServer.closeAllConnections();await new Promise(resolve=>redirectServer.close(resolve));
  assert(path.resolve(home).startsWith(path.join(root,'artifacts')+path.sep)&&path.basename(home).startsWith('media-provider-smoke-'));
  fs.rmSync(home,{recursive:true,force:true,maxRetries:10,retryDelay:150});
  fs.writeFileSync(path.join(evidence,'media-smoke.json'),JSON.stringify({success,checks,network:'loopback only; generated encrypted samples',binary:bin,cleaned:!fs.existsSync(home)},null,2)+'\n');
});

// Real packaged desktop/CLI/Lua workers, own profile, generated local media only.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const {spawn,spawnSync} = require('node:child_process');
const {once} = require('node:events');
const {setTimeout:delay} = require('node:timers/promises');
const {createDecipheriv} = require('node:crypto');
const openDatabase = require('./profile_database.cjs');
const {PUBLIC_KEY,sha256} = require('./build_offline_episode_demo.cjs');
const root = path.resolve(__dirname,'..');
const bundle = path.resolve(process.argv[2] || path.join(root,'dist/examples/Offline-Episode-1.0.0'));
const bin = path.resolve(process.env.MEDIA_DEMO_BIN || path.join(root,'dist/IDM-EX-UnsignedDev'));
const ffprobe = path.resolve(process.env.MEDIA_TEST_FFPROBE || path.join(root,'artifacts/ffmpeg-tools/ffprobe.exe'));
const home = fs.mkdtempSync(path.join(root,'artifacts/episode-smoke-'));
const env = {...process.env, DOWNLOAD_ENGINE_HOME:home, IDM_EX_HOME:home};
for(const key of Object.keys(env)) if(key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];
const checks = [], id = 'download-engine.offline-episode';
let app,server,db,base,serverOutput='',success=false;
const exe = name => path.join(bin,name+(process.platform==='win32'?'.exe':''));
const mark = text => {checks.push(text);console.log(text);};
async function until(check,label,ms=60000){const end=Date.now()+ms;while(Date.now()<end){const value=await check();if(value)return value;await delay(50);}throw Error('Timed out: '+label);}
async function cli(args,input){
  const child=spawn(exe('de-cli'),[...args,'--no-start'],{env,windowsHide:true,stdio:[input===undefined?'ignore':'pipe','pipe','pipe']});
  let out='',err='';child.stdout.on('data',b=>out+=b);child.stderr.on('data',b=>err+=b);
  if(input!==undefined)child.stdin.end(input);
  const timer=setTimeout(()=>child.kill(),30000);
  try{const [code]=await once(child,'exit');assert.equal(code,0,`${args}\n${out}\n${err}`);const reply=JSON.parse(out);return reply.receipt?reply.receipt.result:reply.result;}
  finally{clearTimeout(timer);}
}
async function terminate(child){if(child&&child.exitCode===null&&child.signalCode===null){const done=once(child,'exit');child.kill();await done;}}
async function startServer(port=0,slow=0){
  serverOutput='';
  server=spawn('pwsh',['-NoProfile','-File',path.join(bundle,'Start-Server.ps1'),'-Port',String(port),'-ChunkDelayMs',String(slow)],{windowsHide:true,stdio:['ignore','pipe','pipe']});
  server.stdout.on('data',b=>serverOutput+=b);server.stderr.on('data',b=>serverOutput+=b);
  base=await until(()=>{assert(server.exitCode===null&&server.signalCode===null,serverOutput);return /READY (http:\/\/127\.0\.0\.1:\d+)\/sample-episode/.exec(serverOutput)?.[1];},'sample server');
}
function request(target,options={}){return new Promise((resolve,reject)=>{
  const child=http.request(base,{path:target,method:options.method||'GET',headers:options.headers||{},agent:false},res=>{
    const chunks=[];res.on('data',b=>chunks.push(b));res.on('end',()=>resolve({status:res.statusCode,headers:res.headers,body:Buffer.concat(chunks)}));
  });child.on('error',reject);child.setTimeout(6000,()=>child.destroy(Error('fixture HTTP timeout')));child.end();
});}
function probe(file,args=[]){const result=spawnSync(ffprobe,['-v','error',...args,'-of','json',file],{windowsHide:true,encoding:'utf8',timeout:30000,maxBuffer:8*1024*1024});assert.ifError(result.error);assert.equal(result.status,0,result.stderr);return JSON.parse(result.stdout);}
const job = key => JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(key).data);
async function finish(key,status='complete'){return until(()=>{const row=job(key);if(row.status==='failed'&&status!=='failed')throw Error(row.error);return row.status===status&&row;},status);}
const set = (key,value) => cli(['extensions','set',id,key,value]);
function receipt(key){try{return JSON.parse(fs.readFileSync(path.join(home,'data/parts',key,'provider-media.json')));}catch(error){if(['ENOENT','EACCES','EBUSY'].includes(error.code))return null;throw error;}}
async function main(){
  const pack=JSON.parse(fs.readFileSync(path.join(bundle,'offline-episode.de-extension.json')));
  assert.equal(pack.id,id);assert.match(pack.files['lib/upstream.lua'],/Copyright 2025-2026 CuteTenshii/);
  for(const line of fs.readFileSync(path.join(bundle,'SHA256SUMS'),'utf8').trim().split('\n')){
    const [hash,relative]=line.split('  ');assert(relative&&!relative.includes('..')&&!path.isAbsolute(relative));assert.equal(sha256(fs.readFileSync(path.join(bundle,relative))),hash);
  }
  const catalog=JSON.parse(fs.readFileSync(path.join(bundle,'catalog.json')));
  const routes=JSON.parse(fs.readFileSync(path.join(bundle,'fixtures/routes.json')));
  const references={};
  for(const track of Object.values(catalog.by_id).filter(t=>t.kind!=='subtitle')){
    const init=fs.readFileSync(path.join(bundle,'fixtures/assets',track.id,'init.mp4'));
    const parts=track.segments.map((_,index)=>fs.readFileSync(path.join(bundle,'fixtures/assets',track.id,String(index+1).padStart(5,'0')+'.enc')));
    const decoder=createDecipheriv('aes-256-ctr',Buffer.from(PUBLIC_KEY,'hex'),Buffer.from(track.counter,'hex'));
    decoder.update(Buffer.alloc(init.length));
    const clear=Buffer.concat([init,...parts.map(p=>decoder.update(p)),decoder.final()]);
    assert.equal(sha256(clear),track.original_sha256);
    references[track.id]=path.join(home,track.id+'.mp4');fs.writeFileSync(references[track.id],clear);
  }
  mark('Bundle hashes, embedded upstream MIT notice and generated media hashes match');
  await startServer();
  const resource=routes.find(r=>r.url.endsWith('.enc'));
  assert.equal((await request('/../catalog.json')).status,404);
  assert.equal((await request('/%2e%2e/catalog.json')).status,404);
  assert.equal((await request('/catalog.json')).status,404);
  assert.equal((await request(resource.url,{method:'POST'})).status,400);
  assert.equal((await request(resource.url,{headers:{Host:'untrusted.example'}})).status,400);
  const head=await request(resource.url,{method:'HEAD'});assert.equal(head.status,200);assert.equal(head.body.length,0);assert.equal(Number(head.headers['content-length']),resource.size);
  const ranged=await request(resource.url,{headers:{Range:'bytes=0-15'}});assert.equal(ranged.status,206);
  assert.deepEqual(ranged.body,fs.readFileSync(path.join(bundle,'fixtures',resource.file)).subarray(0,16));
  assert.equal((await request(resource.url,{headers:{Range:`bytes=0-${resource.size}`}})).status,416);
  mark('Shipped loopback server handles HEAD/ranges and rejects traversal, uploads and foreign Host headers');
  await cli(['queues']);db=openDatabase(path.join(home,'data/queue.sqlite3'));
  db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0');
  db.exec("UPDATE interface_preferences SET data=json_set(data,'$.check_updates_on_launch',json('false')) WHERE id=1");
  db.prepare('UPDATE proxy_settings SET data=? WHERE id=1').run(JSON.stringify({mode:'direct'}));
  const log=fs.openSync(path.join(home,'app.log'),'a');
  try{app=spawn(exe('de'),[],{env,windowsHide:true,stdio:['ignore',log,log]});}finally{fs.closeSync(log);}
  await until(()=>{assert(app.exitCode===null&&app.signalCode===null);const file=path.join(home,'data/logs/desktop.log');return fs.existsSync(file)&&fs.readFileSync(file,'utf8').split('\n').some(line=>{try{const row=JSON.parse(line);return row.pid===app.pid&&row.event==='ready';}catch{return false;}});},'desktop ready');
  await cli(['extensions','review',path.join(bundle,'offline-episode.de-extension.json')]);
  await cli(['extensions','install',path.join(bundle,'offline-episode.de-extension.json')]);
  let installed=await cli(['extensions','show',id]);assert.equal(installed.extension.enabled,false);
  await cli(['extensions','enable',id]);installed=await cli(['extensions','show',id]);
  const script=installed.jobs[0].script_id;
  const add=(name,source=base+'/sample-episode')=>cli(['scripts','download',script,source,'--name',name]);
  const first=await finish((await add('Default sample')).id);
  const streams=probe(first.saved_path,['-show_streams']).streams;
  assert(first.saved_path.endsWith('.mkv'));assert.equal(streams[0].height,144);
  assert.deepEqual(streams.map(s=>s.codec_type),['video','audio','audio','subtitle']);
  assert.deepEqual(streams.slice(1).map(s=>s.tags.language),['jpn','eng','eng']);
  assert.equal(streams[1].disposition.default,1);assert.equal(streams[2].disposition.default,0);
  for(const [index,track] of ['video-144','audio-ja','audio-en'].entries()){
    const hashes=(file,stream)=>probe(file,['-select_streams',stream,'-show_packets','-show_data_hash','sha256','-show_entries','packet=data_hash']).packets.map(p=>p.data_hash);
    assert.deepEqual(hashes(first.saved_path,String(index)),hashes(references[track],'0'));
  }
  mark('Unmodified installable extension produces default MKV with correct tracks, language defaults and exact encoded media packets');
  await set('audio_languages','en-US,ALL');await set('subtitle_languages','ALL');await set('quality','90p');await set('container','mp4');
  const alternate=await finish((await add('English first')).id);
  const multi=probe(alternate.saved_path,['-show_streams']).streams;
  assert(alternate.saved_path.endsWith('.mp4'));assert.equal(multi[0].height,90);
  assert.deepEqual(multi.slice(1).map(s=>s.tags.language),['eng','jpn','eng','jpn']);
  assert(multi.slice(3).every(s=>s.codec_name==='mov_text'));assert.equal(multi[1].disposition.default,1);assert.equal(multi[2].disposition.default,0);
  mark('English-first ALL ordering, alternate quality, all subtitles and MP4 output work');
  await set('audio_languages','ja-JP,ja-JP');await set('subtitle_languages','NONE');
  const minimal=await finish((await add('One audio')).id);
  assert.deepEqual(probe(minimal.saved_path,['-show_streams']).streams.map(s=>s.codec_type),['video','audio']);
  await set('audio_languages','fr-FR');
  const before=serverOutput;const unavailable=await finish((await add('Unavailable locale')).id,'failed');
  assert.match(unavailable.error,/locale unavailable/);assert(!unavailable.saved_path);assert.equal(serverOutput,before);
  await set('audio_languages','ALL');
  const invalidSource=await finish((await add('Wrong source',base+'/not-the-sample')).id,'failed');
  assert.match(invalidSource.error,/offline example/);assert.equal(serverOutput,before);
  mark('Duplicate locales are deduplicated; NONE omits subtitles; unavailable locale and wrong source fail before network requests');
  await set('use_saved_key','true');
  await cli(['extensions','set-secret',id,'sample_key'],'00'.repeat(32)+'\n');
  const wrong=await finish((await add('Wrong test key')).id,'failed');assert.match(wrong.error,/plaintext.*checksum/);assert(!wrong.saved_path);
  await cli(['extensions','set-secret',id,'sample_key'],PUBLIC_KEY+'\n');
  const protectedJob=await finish((await add('Saved sample key')).id);assert(protectedJob.saved_path);
  mark('Optional opaque saved key succeeds; wrong key fails verification without publishing a movie');
  const port=Number(new URL(base).port);await terminate(server);await startServer(port,45);
  const paused=await add('Pause and resume');
  await until(()=>receipt(paused.id)?.inputs.filter(Boolean).length>=2,'two saved segments');
  await cli(['pause',paused.id]);await finish(paused.id,'paused');
  const count=receipt(paused.id).inputs.filter(Boolean).length;
  const boundary=serverOutput.length;
  await set('quality','144p');await set('container','mkv');await set('audio_languages','en-US');await set('subtitle_languages','ALL');
  await cli(['resume',paused.id]);const resumed=await finish(paused.id);
  assert(resumed.saved_path.endsWith('.mp4'));
  const resumedStreams=probe(resumed.saved_path,['-show_streams']).streams;
  assert.equal(resumedStreams[0].height,90);assert.deepEqual(resumedStreams.slice(1).map(s=>s.tags.language),['jpn','eng']);
  assert(!serverOutput.slice(boundary).includes('/assets/video-90/init.mp4'));
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs WHERE id=?').get(paused.id).n,1);
  mark(`Pause/resume reuses ${count} completed segments and preserves the saved track/container choices despite changed preferences`);
  success=true;
}
main().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  await terminate(app);await terminate(server);db?.close();
  assert(path.dirname(home)===path.join(root,'artifacts')&&path.basename(home).startsWith('episode-smoke-'));
  fs.rmSync(home,{recursive:true,force:true,maxRetries:10,retryDelay:200});
  const evidence=path.join(root,'artifacts/offline-episode-demo');fs.mkdirSync(evidence,{recursive:true});
  fs.writeFileSync(path.join(evidence,'validation-summary.json'),JSON.stringify({success,checks,bundle,binary:bin,network:'generated fixtures on loopback only',cleaned:!fs.existsSync(home)},null,2)+'\n');
});

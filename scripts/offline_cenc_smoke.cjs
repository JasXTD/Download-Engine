// Owned CENC media, an explicitly synthetic license exchange, real Lua workers,
// installed extension and app lifecycle. Only loopback segment requests occur.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const crypto = require('node:crypto');
const {spawn} = require('node:child_process');
const {once} = require('node:events');
const {setTimeout:delay} = require('node:timers/promises');
const fixture = require('./lib/cenc_fixture.cjs');
const {withWorker,luaBytes} = require('./lib/provider_worker.cjs');
const openDatabase = require('./profile_database.cjs');
const root = path.resolve(__dirname,'..');
const bin = path.resolve(process.argv[2] || path.join(root,'dist/IDM-EX-UnsignedDev'));
const evidence = path.resolve(process.argv[3] || path.join(root,'artifacts/offline-cenc'));
const exe = name => path.join(bin,name+(process.platform==='win32'?'.exe':''));
const ffmpeg = path.resolve(process.env.MEDIA_TEST_FFMPEG || path.join(root,'tools/ffmpeg/ffmpeg.exe'));
const ffprobe = path.resolve(process.env.MEDIA_TEST_FFPROBE || path.join(root,'artifacts/ffmpeg-tools/ffprobe.exe'));
fs.mkdirSync(evidence,{recursive:true});
const home = fs.mkdtempSync(path.join(root,'artifacts/offline-cenc-run-'));
const env = {...process.env,DOWNLOAD_ENGINE_HOME:home,IDM_EX_HOME:home};
for(const key of Object.keys(env)) if(key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];
const original = path.join(root,'examples/offline-cenc');
const files = Object.fromEntries(['provider.lua','lib/cenc.lua','lib/license.lua'].map(name=>[name,fs.readFileSync(path.join(original,name),'utf8')]));
const checks=[], requests=[];
let db, app, server, data, slow=false, serverFailure, success=false;
const mark = message => { checks.push(message); console.log(message); };
const execute = (program,args) => fixture.execute(program,args,home);
const probe = (file,args=[]) => JSON.parse(execute(ffprobe,['-v','error',...args,'-of','json',file]));
const packets = (file,select='0') => probe(file,[
  '-select_streams',select,'-show_packets','-show_data_hash','sha256','-show_entries','packet=data_hash']).packets.map(p=>p.data_hash);
async function until(check,label,timeout=60000) {
  const end=Date.now()+timeout;
  while(Date.now()<end) {if(serverFailure)throw serverFailure;const result=await check();if(result)return result;await delay(40);}
  throw Error('Timed out: '+label);
}
async function cli(args,input) {
  const child=spawn(exe('de-cli'),[...args,'--no-start'],{env,windowsHide:true,stdio:[input===undefined?'ignore':'pipe','pipe','pipe']});
  if(input!==undefined)child.stdin.end(input);
  let out='',err=''; child.stdout.on('data',b=>out+=b);child.stderr.on('data',b=>err+=b);
  const timer=setTimeout(()=>child.kill(),30000);
  try {const [code]=await once(child,'exit');assert.equal(code,0,`${args[0]} ${args[1]}: ${out}\n${err}`);
    const result=JSON.parse(out);return result.receipt?result.receipt.result:result.result;
  } finally {clearTimeout(timer);}
}
const job = id => JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data);
async function finished(id,status='complete') {
  return until(()=>{const value=job(id);if(value.status==='failed'&&status!=='failed')throw Error(value.error);return value.status===status&&value;},status);
}
function receipt(id) {
  try{return JSON.parse(fs.readFileSync(path.join(home,'data/parts',id,'provider-media.json')));}
  catch(error){if(['ENOENT','EBUSY','EACCES'].includes(error.code))return null;throw error;}
}
async function launch() {
  const log=fs.openSync(path.join(home,'desktop.log'),'a');
  try{app=spawn(exe('de'),[],{env,windowsHide:true,stdio:['ignore',log,log]});}finally{fs.closeSync(log);}
  const pid=app.pid;
  await until(()=>{assert(app.exitCode===null&&app.signalCode===null,'Desktop exited');
    const file=path.join(home,'data/logs/desktop.log');return fs.existsSync(file)&&fs.readFileSync(file,'utf8').split(/\r?\n/).some(line=>{
      try{const record=JSON.parse(line);return record.pid===pid&&record.event==='ready';}catch{return false;}
    });},'desktop ready');
}
async function stop() {
  if(app&&app.exitCode===null&&app.signalCode===null){const ended=once(app,'exit');app.kill();await ended;}
  app=null;
}
const wrapper = body => `return {resolve=function()end,refresh=function()end,transform=function()end,finish=function()return true end,transform_segment=function(_,bytes) ${body} end}`;
async function licenseResult(privateKey,reply,nonce,content,kids) {
  const body=`local m=require('lib.license');local key=xd.rsa_private_key(${luaBytes(privateKey)});local keys=m.open(key,bytes,${JSON.stringify(content)},${luaBytes(nonce)},{${kids.map(luaBytes).join(',')}});return xd.to_json(keys)`;
  // Return only a fixed marker: binary content keys must not appear in reports.
  return withWorker(exe('de-cli'),wrapper(body.replace('return xd.to_json(keys)',"local n=0;for _ in pairs(keys) do n=n+1 end;return tostring(n)")),files,async w=>(await w.segment(reply)).toString(),env);
}
async function parserResult(track,bytes,init=track.parts[0].encrypted,initializeOnly=false) {
  const source=`local c=require('lib.cenc');local state;return {resolve=function()end,refresh=function()end,transform=function()end,finish=function()return true end,
    prepare_track=function(_,b)state=c.init(c.string_input(b));return true end,
    transform_stream=function(ctx,input,output)return c.transform(input,output,state,'${track.key.toString('hex')}',ctx.segment_index==0)end}`;
  return withWorker(exe('de-cli'),source,files,async w=>{
    await w.segment(init,{prepare:true,index:0});
    if(initializeOnly)return;
    return w.segment(bytes,{stream:true,index:1});
  },env);
}
async function main() {
  data=fixture.generate(home,ffmpeg);
  for(const track of data.tracks) {
    const expectedPath=path.join(home,track.id+'-expected.mp4');
    fs.writeFileSync(expectedPath,Buffer.concat(track.parts.map(p=>p.expected)));
    assert.deepEqual(packets(expectedPath),packets(track.clearPath),'Clear fixture packets');
    const decoded=execute(ffmpeg,['-v','error','-i',expectedPath,'-f','framemd5','-']);
    const clear=execute(ffmpeg,['-v','error','-i',track.clearPath,'-f','framemd5','-']);
    assert.equal(decoded,clear,'Decrypted frames/audio must match generated originals');
  }
  mark('FFmpeg verifies expected clear AVC/AAC packets and decoded frame hashes; encrypted-input oracle unavailable because the bundled demuxer rejects PSSH');
  for(const track of data.tracks) for(const part of track.parts.slice(1)) {
    assert.deepEqual(await parserResult(track,part.encrypted),part.expected,'Lua sample decryption and metadata rewrite');
  }
  mark('Lua streaming CENC processing matches every expected clear fragment byte, including CTR continuation across clear/encrypted subsamples');

  const pair=crypto.generateKeyPairSync('rsa',{modulusLength:2048});
  const privateKey=Buffer.from(pair.privateKey.export({format:'pem',type:'pkcs8'}));
  const nonce=crypto.randomBytes(32), kids=data.tracks.map(t=>t.kid), content=data.plan.content_id;
  const body=`return require('lib.license').request(xd.rsa_private_key(${luaBytes(privateKey)}),${JSON.stringify(content)},{${kids.map(luaBytes).join(',')}},${luaBytes(nonce)})`;
  const request=await withWorker(exe('de-cli'),wrapper(body),files,w=>w.segment(Buffer.from('request')),env);
  const reply=fixture.issue(request,pair.publicKey,content,data.tracks);
  assert.equal(await licenseResult(privateKey,reply,nonce,content,kids),String(kids.length));
  mark('Lua signs a binary fixture request; independent Node issuer verifies it; Lua opens RSA-wrapped authenticated content keys');
  const tamperedRequest=Buffer.from(request);tamperedRequest[tamperedRequest.length-1]^=1;
  assert.throws(()=>fixture.issue(tamperedRequest,pair.publicKey,content,data.tracks));
  for(const [name,change,expected] of [
    ['modified MAC',b=>{b[b.length-1]^=1;return b;},/authentication failed/],
    ['truncated reply',b=>b.subarray(0,b.length-1),/Truncated/],
    ['trailing reply',b=>Buffer.concat([b,Buffer.from([0])]),/Trailing/],
  ]) await assert.rejects(()=>licenseResult(privateKey,change(Buffer.from(reply)),nonce,content,kids),expected,name);
  await assert.rejects(()=>licenseResult(privateKey,reply,crypto.randomBytes(32),content,kids),/nonce mismatch/);
  await assert.rejects(()=>licenseResult(privateKey,reply,nonce,content+'-other',kids),/content mismatch/);
  await assert.rejects(()=>licenseResult(privateKey,reply,nonce,content,[crypto.randomBytes(16),kids[1]]),/Unexpected/);
  mark('Synthetic license rejects changed signatures/MACs, truncation, trailing bytes, wrong nonce/content and wrong key IDs');

  const video=data.tracks[0], clearFragment=video.parts[1].encrypted;
  const locate=(b,name)=>fixture.child(b,fixture.child(b,fixture.one(b,'moof'),'traf'),name);
  const mutations=[
    ['sample count',b=>b.writeUInt32BE(999,locate(b,'senc').start+12),/sample count mismatch/],
    ['data offset',b=>b.writeInt32BE(0,locate(b,'trun').start+16),/outside mdat/],
    ['auxiliary offset',b=>b.writeUInt32BE(0,locate(b,'saio').start+16),/auxiliary offset/],
    ['auxiliary size',b=>b[locate(b,'saiz').start+17]^=1,/auxiliary size/],
    ['subsample range',b=>b.writeUInt32BE(0xffffffff,locate(b,'senc').start+16+16+2+2),/outside sample/],
    ['unsupported senc flags',b=>b.writeUInt32BE(1,locate(b,'senc').start+8),/senc flags/],
    ['oversized box',b=>b.writeUInt32BE(0xffffffff,0),/box size/],
  ];
  for(const [name,mutate,error] of mutations) {const bytes=Buffer.from(clearFragment);mutate(bytes);await assert.rejects(()=>parserResult(video,bytes),error,name);}
  const wrongScheme=Buffer.from(video.parts[0].encrypted), schemeAt=wrongScheme.indexOf(Buffer.from('schm'));
  assert(schemeAt>0);wrongScheme.write('cbcs',schemeAt+8,'ascii');
  await assert.rejects(()=>parserResult(video,null,wrongScheme,true),/Only cenc/);
  const badIv=Buffer.from(video.parts[0].encrypted), tencAt=badIv.indexOf(Buffer.from('tenc'));
  assert(tencAt>0);badIv[tencAt+11]=7;
  await assert.rejects(()=>parserResult(video,null,badIv,true),/IV size/);
  mark('Lua parser rejects malformed counts, offsets, ranges, auxiliary metadata, boxes and unsupported scheme/IV/flags');

  files['catalog.lua']='return '+`{plan=[==[${JSON.stringify(data.plan)}]==],nonce='${nonce.toString('hex')}',kids={${kids.map(b=>"'"+b.toString('hex')+"'").join(',')}}}`;
  files['fixture_credentials.lua']=`-- Generated fixture material; never use real credentials here.\nreturn {identity=${luaBytes(privateKey)},license=${luaBytes(reply)}}`;
  const fixtureSource=files['provider.lua'];
  const extension={...JSON.parse(fs.readFileSync(path.join(original,'manifest.json'),'utf8')),
    settings:[{id:'continue_processing',type:'boolean',label:'Test-only processing gate',default:true}],
    files:{...files,'provider.lua':fixtureSource.replace('local state = assert(states[context.track_id]',
      "if context.track_id=='video' and context.segment_index==2 and not xd.setting('continue_processing') then while true do end end\n        local state = assert(states[context.track_id]")}};
  const extensionPath=path.join(home,'offline-cenc.de-extension.json');fs.writeFileSync(extensionPath,JSON.stringify(extension));
  const protectedExtension={...extension,id:'offline.cenc-protected',settings:[
    {id:'identity',type:'rsa_private_key',label:'Generated test identity'},
    {id:'license',type:'binary_secret',label:'Generated test reply'}],files:{...files,
      'provider.lua':files['provider.lua'].replace("local credentials = require('fixture_credentials')",'')
        .replace('xd.rsa_private_key(credentials.identity), credentials.license',"xd.rsa_key_setting('identity'), xd.binary_setting('license')")}};
  delete protectedExtension.files['fixture_credentials.lua'];
  const protectedPath=path.join(home,'protected-cenc.de-extension.json');fs.writeFileSync(protectedPath,JSON.stringify(protectedExtension));
  const identityPath=path.join(home,'generated-identity.pem'), licensePath=path.join(home,'generated-license.bin');
  fs.writeFileSync(identityPath,privateKey);fs.writeFileSync(licensePath,reply);
  server=http.createServer((req,res)=>{
    try {
      requests.push(req.url);
      assert(!req.headers.authorization&&!req.headers.cookie,'No credentials belong on fixture media requests');
      const bytes=data.routes.get(req.url);if(!bytes){res.writeHead(404);res.end();return;}
      res.writeHead(200,{'Content-Length':bytes.length});
      if(!slow){res.end(bytes);return;}
      let offset=0,timer;
      const send=()=>{if(res.destroyed)return;const end=Math.min(offset+512,bytes.length);res.write(bytes.subarray(offset,end));offset=end;if(offset===bytes.length)res.end();else timer=setTimeout(send,60);};
      res.on('close',()=>clearTimeout(timer));send();
    } catch(error){serverFailure=error;res.destroy();}
  });
  server.listen(0,'127.0.0.1');await once(server,'listening');
  const url=`http://127.0.0.1:${server.address().port}/sample-cenc`;
  await cli(['queues']);db=openDatabase(path.join(home,'data/queue.sqlite3'));
  db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0');
  db.exec("UPDATE interface_preferences SET data=json_set(data,'$.check_updates_on_launch',json('false')) WHERE id=1");
  db.prepare('UPDATE proxy_settings SET data=? WHERE id=1').run(JSON.stringify({mode:'direct'}));
  await launch();
  await cli(['extensions','install',protectedPath]);await cli(['extensions','enable',protectedExtension.id]);
  await cli(['extensions','import-secret',protectedExtension.id,'identity',identityPath]);
  await cli(['extensions','import-secret',protectedExtension.id,'license',licensePath]);
  const protectedScript=(await cli(['extensions','show',protectedExtension.id])).jobs[0].script_id;
  const protectedJob=await cli(['scripts','download',protectedScript,url,'--name','protected-host-check']);
  assert.match((await finished(protectedJob.id,'failed')).error,/saved secret.*declared hosts/);
  assert.equal(requests.length,0);
  mark('Protected imports and Lua license decoding reach plan validation; the saved-secret host policy correctly refuses loopback before any transfer');
  await cli(['extensions','install',extensionPath]);await cli(['extensions','enable',extension.id]);
  const script=(await cli(['extensions','show',extension.id])).jobs[0].script_id;
  async function add(name) {return cli(['scripts','download',script,url,'--name',name]);}
  function verifyOutput(file) {
    const streams=probe(file,['-show_streams']).streams;
    assert.deepEqual(streams.map(s=>s.codec_type),['video','audio','subtitle']);assert.equal(streams[2].codec_name,'ass');
    for(let i=0;i<2;i++)assert.deepEqual(packets(file,String(i)),packets(data.tracks[i].clearPath));
    const subtitles=execute(ffmpeg,['-v','error','-i',file,'-map','0:s:0','-c:s','copy','-f','ass','-']);
    assert(subtitles.includes('Offline CENC sample')&&subtitles.includes('{\\i1}'));
  }
  const first=await add('cenc-multitrack');verifyOutput((await finished(first.id)).saved_path);
  mark('Installed fixture-only extension decodes generated licenses, hands keys to the offline worker, downloads CENC and muxes exact AVC/AAC packets plus styled ASS');
  for(const mode of ['pause','crash','stop-all']) {
    await cli(['extensions','set',extension.id,'continue_processing','false']);
    const added=await add('cenc-'+mode);
    const companions=mode==='stop-all'?[await add('cenc-stop-all-second')]:[];
    for(const candidate of [added,...companions]) {
      await until(()=>receipt(candidate.id)?.outputs.filter(Boolean).length===2,'two CENC output checkpoints before the processing gate');
    }
    if(mode==='pause') {await cli(['pause',added.id]);await finished(added.id,'paused');await stop();}
    else if(mode==='crash')await stop();
    else {
      const start=Date.now();await cli(['pause-all']);
      for(const candidate of [added,...companions])await finished(candidate.id,'paused');
      assert(Date.now()-start<10000,'Stop all did not cancel promptly');
      const saved=[added,...companions].map(j=>JSON.stringify(receipt(j.id)));
      await delay(300);
      assert.deepEqual([added,...companions].map(j=>JSON.stringify(receipt(j.id))),saved,'Late writes after Stop all');
      await stop();
    }
    const checkpoint=receipt(added.id), reused=checkpoint.inputs.map((value,i)=>value?i:-1).filter(i=>i>=0);
    assert(reused.length>=2);
    assert.equal(checkpoint.outputs.filter(Boolean).length,2,'The test must interrupt unfinished processing');
    const persisted=db.prepare('SELECT data FROM jobs WHERE id=?').get(added.id).data+JSON.stringify(checkpoint);
    for(const t of data.tracks)assert(!persisted.includes(t.key.toString('hex')));
    assert(!persisted.includes('BEGIN PRIVATE KEY')&&!persisted.includes('provider-key-v1:'));
    const before=requests.length;slow=false;
    await cli(['extensions','set',extension.id,'continue_processing','true']);
    await launch();await cli(['resume',added.id]);
    verifyOutput((await finished(added.id)).saved_path);
    const all=data.plan.tracks.flatMap(t=>t.segments);
    for(const index of reused)assert(!requests.slice(before).includes(all[index].url),'A durable CENC input was fetched again');
    assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs WHERE id=?').get(added.id).n,1);
    for(const companion of companions) {await cli(['resume',companion.id]);verifyOutput((await finished(companion.id)).saved_path);}
    mark(`${mode}: reacquires fixture keys, replays initialization, reuses ${reused.length} checked segments and completes the same job`);
  }
  const tampered=Buffer.from(reply);tampered[tampered.length-1]^=1;fs.writeFileSync(licensePath,tampered);
  const invalidExtension={...extension,id:'offline.cenc-invalid',files:{...files,
    'fixture_credentials.lua':`return {identity=${luaBytes(privateKey)},license=${luaBytes(tampered)}}`}};
  const invalidPath=path.join(home,'invalid-license.de-extension.json');fs.writeFileSync(invalidPath,JSON.stringify(invalidExtension));
  await cli(['extensions','install',invalidPath]);await cli(['extensions','enable',invalidExtension.id]);
  const invalidScript=(await cli(['extensions','show',invalidExtension.id])).jobs[0].script_id;
  const before=requests.length,bad=await cli(['scripts','download',invalidScript,url,'--name','bad-license']);await finished(bad.id,'failed');
  assert(!job(bad.id).saved_path);assert.equal(requests.length,before,'Invalid license must fail before media transfer');
  mark('Tampered fixture license fails in the installed extension before any media request or output publication');
  success=true;
}
main().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  await stop();db?.close();
  if(server){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
  assert(path.dirname(home)===path.join(root,'artifacts')&&path.basename(home).startsWith('offline-cenc-run-'));
  fs.rmSync(home,{recursive:true,force:true,maxRetries:10,retryDelay:150});
  fs.writeFileSync(path.join(evidence,'verification.json'),JSON.stringify({success,checks,binary:bin,
    network:'loopback media only; synthetic license exchange over local worker IPC, no live service or HTTP license endpoint',
    cleaned:!fs.existsSync(home),limits:'AVC/AAC cenc CTR, one track/traf/trun, 8/16-byte IVs; no cbcs, key rotation or real license protocol'},null,2)+'\n');
});

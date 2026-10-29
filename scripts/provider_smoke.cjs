// Real app/CLI/provider workers. Generated AES data, loopback server, private profile only.
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const {randomUUID, createHash, createCipheriv} = require('node:crypto');
const {spawn} = require('node:child_process'), {once} = require('node:events');
const {setTimeout: delay} = require('node:timers/promises');
const openDatabase = require('./profile_database.cjs');
const root = path.resolve(__dirname, '..');
const binary = path.resolve(process.argv[2] || path.join(root, 'target/debug'));
const home = fs.mkdtempSync(path.join(root, 'artifacts/provider-smoke-'));
const evidence = path.resolve(process.argv[3] || path.join(root, 'artifacts/lua-provider-integration-20261029'));
fs.mkdirSync(evidence, {recursive:true});
const env = {...process.env, IDM_EX_HOME:home, DOWNLOAD_ENGINE_HOME:home};
for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];
const plain = Buffer.alloc(2*1024*1024+37);
for (let i=0;i<plain.length;i++) plain[i]=(i*17+9)%251;
const key='603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4', counter='000102030405060708090a0b0c0d0e0f';
const cipher = createCipheriv('aes-256-ctr',Buffer.from(key,'hex'),Buffer.from(counter,'hex'));
const encrypted = Buffer.concat([cipher.update(plain),cipher.final()]);
const hash = b=>createHash('sha256').update(b).digest('hex');
const template = fs.readFileSync(path.join(root,'examples/download-provider.lua'),'utf8')
  .replace('REPLACE_WITH_CIPHERTEXT_SHA256', hash(encrypted)).replace('REPLACE_WITH_PLAINTEXT_SHA256',hash(plain));
let app, db, requests=0, success=false; const checks=[], ranges=[];
const server=http.createServer((req,res)=>{
  requests++;
  const chunked=req.url.includes('chunked');
  const m=chunked?null:/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||'');
  const start=m?+m[1]:0,end=m&&m[2]?Math.min(+m[2],encrypted.length-1):encrypted.length-1;
  ranges.push({url:req.url,start,end});
  if(start>end){res.writeHead(416);res.end();return;}
  const headers={'Content-Type':'application/octet-stream','Content-Length':end-start+1,ETag:'"provider-fixture-v1"','Accept-Ranges':'bytes'};
  if(chunked){delete headers['Content-Length'];headers['Transfer-Encoding']='chunked';}
  if(m)headers['Content-Range']=`bytes ${start}-${end}/${encrypted.length}`;
  res.writeHead(m?206:200,headers);
  if(req.url.includes('network-pause') && end-start>1){
    let at=start,timer;
    const send=()=>{if(res.destroyed)return;const next=Math.min(at+16384,end+1);res.write(encrypted.subarray(at,next));at=next;if(at>end)res.end();else timer=setTimeout(send,30);};
    res.on('close',()=>clearTimeout(timer));send();
  }else res.end(encrypted.subarray(start,end+1));
});
async function until(fn,label,ms=45000){const end=Date.now()+ms;while(Date.now()<end){const value=await fn();if(value)return value;await delay(40);}throw Error(`Timed out: ${label}`);}
async function cli(args,expected=0,input){
  const child=spawn(path.join(binary,'de-cli.exe'),[...args,'--no-start'],{env,windowsHide:true,stdio:[input===undefined?'ignore':'pipe','pipe','pipe']});
  if(input!==undefined)child.stdin.end(input);
  let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
  const timer=setTimeout(()=>child.kill(),30000);
  try {const [code,signal]=await once(child,'exit');assert.equal(signal,null);assert.equal(code,expected,`${args}\n${stdout}\n${stderr}`);const value=JSON.parse(stdout||stderr);return value.receipt?value.receipt.result:value.result;}finally{clearTimeout(timer);}
}
const job=id=>JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data);
async function finished(id,status='complete'){return until(()=>{const item=job(id);if(item.status==='failed'&&status!=='failed')throw Error(item.error);if(item.saved_path)assert(path.resolve(item.saved_path).startsWith(path.join(home,'downloads')+path.sep),'Output must stay in the isolated profile');return item.status===status&&item;},`${id} ${status}`);}
async function save(body=template,timeout=10,old){
  const script=old||{id:randomUUID(),revision:0,name:'Offline provider',language:'lua',trigger:{kind:'provider'},enabled:false,change_files:false};
  script.body=body;script.timeout_seconds=timeout;
  const file=path.join(home,`${script.id}.json`);fs.writeFileSync(file,JSON.stringify(script));
  return cli(['scripts','save',file]);
}
async function launch(){
  const log=fs.openSync(path.join(home,'desktop.log'),'a');
  try{app=spawn(path.join(binary,'de.exe'),[],{env,windowsHide:true,stdio:['ignore',log,log]});}finally{fs.closeSync(log);}
  const pid=app.pid;
  await until(()=>{assert(app.exitCode===null&&app.signalCode===null,'Desktop exited');const file=path.join(home,'data/logs/desktop.log');return fs.existsSync(file)&&fs.readFileSync(file,'utf8').split(/\r?\n/).filter(Boolean).some(v=>{try{const e=JSON.parse(v);return e.pid===pid&&e.event==='ready';}catch{return false;}});},'desktop ready');
}
async function stop(){if(app&&app.exitCode===null&&app.signalCode===null){const exited=once(app,'exit');app.kill();await exited;}app=null;}
function mark(name){checks.push(name);console.log(name);}
async function main(){
  server.listen(0,'127.0.0.1');await once(server,'listening');
  const url=`http://127.0.0.1:${server.address().port}/sample.bin`;
  await cli(['queues']);
  db=openDatabase(path.join(home,'data/queue.sqlite3'));
  db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0');
  db.exec("UPDATE interface_preferences SET data=json_set(data,'$.check_updates_on_launch',json('false')) WHERE id=1");
  db.prepare('UPDATE proxy_settings SET data=? WHERE id=1').run(JSON.stringify({mode:'direct'}));
  await launch();
  const script=await save();const request=randomUUID();
  const added=await cli(['scripts','download',script.id,url,'--name','native-plain.bin','--request-id',request]);
  const complete=await finished(added.id);
  assert.equal(hash(fs.readFileSync(complete.saved_path)),hash(plain));
  const again=await cli(['scripts','download',script.id,url,'--name','native-plain.bin','--request-id',request]);
  assert.equal(again.id,added.id);assert.equal(db.prepare('SELECT count(*) n FROM jobs').get().n,1);
  assert.equal(hash(encrypted),complete.provider.plan.input_sha256);
  mark('Native AES provider downloads, verifies and publishes one exact plaintext file; request retries do not duplicate it');

  const custom=fs.readFileSync(path.join(root,'examples/download-provider-custom-ctr.lua'),'utf8')
    .replace('REPLACE_WITH_CIPHERTEXT_SHA256',hash(encrypted)).replace('REPLACE_WITH_PLAINTEXT_SHA256',hash(plain));
  assert(!custom.includes('xd.aes_ctr('));
  const customScript=await save(custom);
  const customJob=await cli(['scripts','download',customScript.id,url,'--name','lua-plain.bin']);
  assert.equal(hash(fs.readFileSync((await finished(customJob.id)).saved_path)),hash(plain));
  mark('A user-written Lua CTR construction using only the AES block primitive produces identical plaintext');

  let protectedScript=await save();
  const {token}=await cli(['scripts','protect-key',protectedScript.id],0,key+'\n');
  assert(!token.includes(key));
  const protectedSource=template.replace(`local key = "${key}"`,`local key = xd.key(${JSON.stringify(token)})`);
  protectedScript=await save(protectedSource,10,protectedScript);
  const protectedJob=await cli(['scripts','download',protectedScript.id,url,'--name','protected-key.bin']);
  const protectedComplete=await finished(protectedJob.id);
  assert.equal(hash(fs.readFileSync(protectedComplete.saved_path)),hash(plain));
  assert.equal(protectedComplete.provider.script.format,'provider-script-v1');
  assert.equal(protectedComplete.provider.script.body,undefined);
  assert.equal(protectedComplete.provider.plan.url,undefined);
  const storedProtected=db.prepare('SELECT data FROM jobs WHERE id=?').get(protectedJob.id).data;
  assert(!storedProtected.includes(key)&&!storedProtected.includes(token));
  const foreign=await save(protectedSource);
  const foreignJob=await cli(['scripts','download',foreign.id,url,'--name','foreign-key.bin']);
  assert.match((await finished(foreignJob.id,'failed')).error,/different provider|unlock/i);
  assert(!job(foreignJob.id).saved_path);
  mark('DPAPI-backed opaque keys decrypt in the owning provider only; frozen jobs persist neither source/key tokens nor resolved URLs');

  const capabilitySource=`local cached=xd.fetch\nlocal processor=xd.phase=='processor'\nif processor then assert(cached==nil and not xd.supports('xd.fetch')) end\n`+template
    .replace('return xd.aes_ctr(key, counter, bytes, offset)',"assert(processor and cached==nil and plan.url==nil)\n        assert(not pcall(function() cached('https://example.invalid/',{method='POST',body=bytes}) end))\n        return xd.aes_ctr(key, counter, bytes, offset)")
    .replace('assert(result.sha256 == plan.output_sha256)','assert(processor and cached==nil and xd.fetch==nil and plan.url==nil)\n        assert(result.sha256 == plan.output_sha256)');
  const isolatedScript=await save(capabilitySource);
  const isolatedJob=await cli(['scripts','download',isolatedScript.id,url,'--name','no-fetch.bin']);
  assert.equal(hash(fs.readFileSync((await finished(isolatedJob.id)).saved_path)),hash(plain));
  mark('Real processor workers cannot cache fetch during initialization or access it during transform/finish; verified output still publishes');

  const refreshing=template.replace('return describe(request) end','local p=describe(request); p.url=request.url.."?renewed=1"; return p end');
  const netScript=await save(refreshing);
  const netJob=await cli(['scripts','download',netScript.id,url.replace('sample.bin','network-pause.bin'),'--name','network-resume.bin']);
  await until(()=>{const j=job(netJob.id);return j.downloaded>0&&j.downloaded<plain.length;},'network progress');
  await cli(['pause',netJob.id]);const pausedNet=await finished(netJob.id,'paused');
  assert(pausedNet.downloaded<plain.length);
  await cli(['resume',netJob.id]);
  assert.equal(hash(fs.readFileSync((await finished(netJob.id)).saved_path)),hash(plain));
  assert(ranges.some(r=>r.url.includes('network-pause.bin?renewed=1')&&r.start>0));
  assert.equal(job(netJob.id).provider.plan.url,undefined);
  assert(!db.prepare('SELECT data FROM jobs WHERE id=?').get(netJob.id).data.includes('?renewed=1'));
  mark('Network pause/resume refreshes the source URL and requests the remaining byte range');

  const singleJob=await cli(['scripts','download',script.id,url.replace('sample.bin','chunked.bin'),'--name','chunked-plain.bin']);
  assert.equal(hash(fs.readFileSync((await finished(singleJob.id)).saved_path)),hash(plain));
  const tooShort=await save(template.replace('size = 2097189','size = 1024'));
  const boundedJob=await cli(['scripts','download',tooShort.id,url.replace('sample.bin','chunked.bin'),'--name','overrun.bin']);
  assert.match((await finished(boundedJob.id,'failed')).error,/more bytes than declared/);
  assert(job(boundedJob.id).downloaded<=1024&&!job(boundedJob.id).saved_path);
  mark('Chunked non-range transfers work, while data exceeding the provider plan is rejected before writing beyond its size');

  const slow=template.replace('return xd.aes_ctr(key, counter, bytes, offset)', 'for i=1,60000000 do local x=i*7 end\n        return xd.aes_ctr(key, counter, bytes, offset)');
  const pausedScript=await save(slow,20);
  const pausedJob=await cli(['scripts','download',pausedScript.id,url,'--name','resumed-plain.bin']);
  const progress=path.join(home,'data/parts',pausedJob.id,'provider-progress.json');
  const readProgress=()=>{try{return JSON.parse(fs.readFileSync(progress));}catch(e){if(['ENOENT','EBUSY','EACCES'].includes(e.code))return null;throw e;}};
  await until(()=>{const p=readProgress();return p&&p.offset>0&&p.offset<plain.length;},'first durable transformed chunk');
  await cli(['pause',pausedJob.id]);await finished(pausedJob.id,'paused');
  const checkpoint=readProgress();assert(checkpoint.offset>0&&checkpoint.offset<plain.length);
  const partial=fs.readFileSync(path.join(home,'data/parts',pausedJob.id,'provider-output.part'));
  assert.equal(hash(partial.subarray(0,checkpoint.offset)),checkpoint.prefix_sha256);
  await save('error("Edited library code must not replace a frozen download")',20,pausedScript);
  await stop();await launch();
  assert.equal(job(pausedJob.id).status,'paused');
  await cli(['resume',pausedJob.id]);
  const resumed=await finished(pausedJob.id);
  assert.equal(hash(fs.readFileSync(resumed.saved_path)),hash(plain));
  assert.equal(resumed.provider.script.format,'provider-script-v1');
  assert.equal(resumed.provider.script.body,undefined);
  mark(`Pause, desktop restart and resume preserve a ${checkpoint.offset}-byte plaintext checkpoint and the frozen script revision`);

  const corruptScript=await save(slow,20);
  const corruptJob=await cli(['scripts','download',corruptScript.id,url,'--name','corrupt-recovery.bin']);
  const corruptRoot=path.join(home,'data/parts',corruptJob.id);
  await until(()=>{try{return JSON.parse(fs.readFileSync(path.join(corruptRoot,'provider-progress.json'))).offset>0;}catch(e){if(['ENOENT','EBUSY','EACCES'].includes(e.code))return false;throw e;}},'checkpoint to corrupt');
  await cli(['pause',corruptJob.id]);await finished(corruptJob.id,'paused');
  const fd=fs.openSync(path.join(corruptRoot,'provider-output.part'),'r+');try{fs.writeSync(fd,Buffer.from([plain[0]^1]),0,1,0);}finally{fs.closeSync(fd);}
  await cli(['resume',corruptJob.id]);
  assert.match((await finished(corruptJob.id,'failed')).error,/Saved provider output changed/);
  assert(!job(corruptJob.id).saved_path);
  await cli(['restart',corruptJob.id]);
  assert.equal(hash(fs.readFileSync((await finished(corruptJob.id)).saved_path)),hash(plain));
  mark('Corrupted saved plaintext is refused; explicit Restart clears checkpoints and safely completes');

  const invalid=await save(template.replace(key,'703deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4'));
  const bad=await cli(['scripts','download',invalid.id,url,'--name','wrong-key.bin']);
  const failed=await finished(bad.id,'failed');assert.match(failed.error,/plaintext checksum/);
  assert(!failed.saved_path);assert(!fs.existsSync(path.join(failed.directory,failed.name)));
  mark('Wrong-key output fails independent plaintext verification and is never published');

  const corruptSource=await save(template.replace(hash(encrypted),'0'.repeat(64)));
  const inputJob=await cli(['scripts','download',corruptSource.id,url,'--name','bad-source.bin']);
  assert.match((await finished(inputJob.id,'failed')).error,/encrypted source checksum/);
  assert(!fs.existsSync(path.join(home,'data/parts',inputJob.id,'provider-output.part')));
  mark('An unexpected encrypted source is rejected before Lua processes any bytes');

  const memory=await save(template.replace('return xd.aes_ctr(key, counter, bytes, offset)',"return string.rep('x',512*1024*1024)"));
  const memoryJob=await cli(['scripts','download',memory.id,url,'--name','memory.bin']);
  const memoryFailure=await finished(memoryJob.id,'failed');assert.match(memoryFailure.error,/memory|allocat|worker stopped/i);assert(!memoryFailure.saved_path);
  mark('Excessive Lua allocation fails inside the bounded worker without publishing output');

  const hanging=await save(template.replace('return xd.aes_ctr(key, counter, bytes, offset)','while true do end'),1);
  const timed=await cli(['scripts','download',hanging.id,url,'--name','timeout.bin']);
  assert.match((await finished(timed.id,'failed')).error,/timed out/);
  mark('An infinite transform is terminated by the worker deadline without freezing the desktop');

  const stopScript=await save(template.replace('return xd.aes_ctr(key, counter, bytes, offset)','while true do end'),60);
  const stopping=await cli(['scripts','download',stopScript.id,url,'--name','cancelled.bin']);
  const zeroProgress=path.join(home,'data/parts',stopping.id,'provider-progress.json');
  await until(()=>fs.existsSync(zeroProgress),'transform worker active');await delay(400);
  const start=Date.now();await cli(['pause',stopping.id]);await finished(stopping.id,'paused');
  assert(Date.now()-start<10000,'Cancellation must not wait for the 60 second deadline');
  assert(!job(stopping.id).saved_path);
  mark('Pause cancels a busy transform promptly while preserving its checkpoint');

  const noNetwork=await save(template.replace('url = request.url,','url = xd.fetch(request.url).url,'));
  const before=requests;
  const refused=await cli(['scripts','download',noNetwork.id,url,'--name','private-fetch.bin']);
  await finished(refused.id,'failed');assert.equal(requests,before);
  mark('Provider metadata fetch cannot access loopback; only the explicitly submitted file origin is granted to the transfer core');
  success=true;
}
main().catch(e=>{console.error(e);process.exitCode=1;}).finally(async()=>{
  await stop();db?.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
  const absolute=path.resolve(home),allowed=path.join(root,'artifacts')+path.sep;
  assert(absolute.startsWith(allowed)&&path.basename(absolute).startsWith('provider-smoke-'));
  fs.rmSync(absolute,{recursive:true,force:true,maxRetries:10,retryDelay:150});
  fs.writeFileSync(path.join(evidence,'app-smoke.json'),JSON.stringify({success,binary,checks,network:'loopback only',bytes:plain.length,plaintextSha256:hash(plain),cleaned:!fs.existsSync(home)},null,2)+'\n');
});

const openProfileDatabase = require('./profile_database.cjs');
// Real packaged downloads against a gated loopback server, with an isolated profile.
// No browser registration, proxy, antivirus setting or shutdown action is changed.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const {spawn, spawnSync} = require('node:child_process');
const {randomUUID, createHash} = require('node:crypto');
const root = path.resolve(__dirname, '..');
const binaries = path.resolve(process.argv[2] || path.join(root, 'dist', 'IDM-EX-UnsignedDev'));
const home = path.join(root, 'artifacts', 'queue-concurrency-' + randomUUID().slice(0, 8));
fs.mkdirSync(path.join(home, 'data'), {recursive:true});
const db = openProfileDatabase(path.join(home, 'data', 'queue.sqlite3'));
db.exec(`CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY,close_to_tray INTEGER NOT NULL,notify_complete INTEGER NOT NULL,notify_failed INTEGER NOT NULL);
INSERT INTO desktop_preferences VALUES(1,0,0,0);
CREATE TABLE interface_preferences(id INTEGER PRIMARY KEY,data TEXT NOT NULL);
INSERT INTO interface_preferences VALUES(1,'{"auto_download_status":false}');
CREATE TABLE download_queues(id TEXT PRIMARY KEY,name TEXT NOT NULL UNIQUE COLLATE NOCASE,max_active INTEGER NOT NULL CHECK(max_active BETWEEN 1 AND 8),running INTEGER NOT NULL,scheduled_at INTEGER);
INSERT INTO download_queues VALUES('main','Main queue',3,0,NULL);`);
const env = {...process.env, IDM_EX_HOME:home};
for (const key of Object.keys(env)) if(key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
const payload=Buffer.alloc(128*1024,71), held=new Set(), samples=[];
let released=false, app, sampler;
const jobs=()=>db.prepare('SELECT data FROM jobs').all().map(row=>JSON.parse(row.data));
const server=http.createServer((req,res)=>{
  const range=/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||'');
  const start=range?+range[1]:0, end=range&&range[2]?+range[2]:payload.length-1;
  res.writeHead(range?206:200, {'Content-Type':'application/octet-stream','Content-Length':end-start+1,
    'Content-Range':`bytes ${start}-${end}/${payload.length}`,ETag:'"concurrency-fixture"'});
  if(req.method==='HEAD'){res.end();return;}
  if(end-start<=1 || released){res.end(payload.subarray(start,end+1));return;}
  const split=Math.min(start+32768,end+1);
  res.write(payload.subarray(start,split));
  const pending={res,remaining:payload.subarray(split,end+1)};
  held.add(pending);res.on('close',()=>held.delete(pending));
});
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function until(check,label,timeout=30000){const deadline=Date.now()+timeout;for(;;){if(check())return;assert(Date.now()<deadline,label);await sleep(100);}}
function cli(args, succeeds=true){return new Promise((resolve,reject)=>{
  const child=spawn(path.join(binaries,'de-cli.exe'),args,{env,windowsHide:true});
  let out='',err='';const timeout=setTimeout(()=>{child.kill();reject(new Error('CLI timeout'));},20000);
  child.stdout.on('data',bytes=>out+=bytes);child.stderr.on('data',bytes=>err+=bytes);
  child.on('error',reject);child.on('exit',code=>{clearTimeout(timeout);try{
    assert.equal(code===0,succeeds,`${args.join(' ')}: ${out} ${err}`);resolve(out.trim());
  }catch(error){reject(error);}});
});}
function closeApp(){const result=spawnSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',path.join(__dirname,'status_windows.ps1'),'-WindowProcess',String(app.pid),'-Action','CloseApp'],{encoding:'utf8',windowsHide:true,timeout:10000});assert.equal(result.status,0,result.stderr);}
(async()=>{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  app=spawn(path.join(binaries,'de.exe'),[],{env,windowsHide:true,stdio:'ignore'});
  await until(()=>db.prepare("SELECT 1 FROM sqlite_schema WHERE name='control_requests'").get(),'App did not initialize');
  await cli(['queue','limit','main','20']);
  const url=i=>`http://127.0.0.1:${server.address().port}/file-${i}.bin`;
  await cli(['add',...Array.from({length:25},(_,i)=>url(i))]);
  sampler=setInterval(()=>samples.push({at:Date.now(),active:jobs().filter(job=>job.status==='downloading').length}),100);
  await until(()=>jobs().filter(job=>job.status==='downloading').length===20,'20 downloads did not start');
  await sleep(500);
  assert.equal(jobs().filter(job=>job.status==='downloading').length,20);
  await cli(['queue','limit','main','30']);
  await until(()=>jobs().filter(job=>job.status==='downloading').length===25,'Raising the limit to 30 did not start the waiting files');
  await cli(['queue','create','Second']);
  await cli(['queue','limit','Second','30']);
  await cli(['add',...Array.from({length:20},(_,i)=>url(i+25)),'--queue','Second']);
  await cli(['queue','start','Second']);
  await until(()=>jobs().filter(job=>job.status==='downloading').length===32,'Global cap did not reach 32');
  for(const value of ['0','33','999']) await cli(['queue','limit','main',value],false);
  assert.equal(db.prepare("SELECT max_active FROM download_queues WHERE id='main'").get().max_active,30);
  await sleep(500);
  assert.equal(jobs().filter(job=>job.status==='downloading').length,32);
  released=true;for(const pending of held)pending.res.end(pending.remaining);
  await until(()=>jobs().length===45&&jobs().every(job=>job.status==='complete'),'Downloads did not complete',60000);
  clearInterval(sampler);sampler=null;
  assert(Math.max(...samples.map(sample=>sample.active))<=32,'Global cap exceeded');
  const hash=createHash('sha256').update(payload).digest('hex');
  const saved=jobs();
  for(const job of saved)assert.equal(createHash('sha256').update(fs.readFileSync(job.saved_path)).digest('hex'),hash);
  closeApp();await until(()=>app.exitCode!==null,'App did not close');
  for(const job of saved){const file=fs.realpathSync(job.saved_path);assert(file.startsWith(fs.realpathSync(home)+path.sep));fs.unlinkSync(file);}
  const result={success:true,version:await cli(['--version']),legacyDatabaseMigrated:true,typedLimitsVerified:[20,30],rejectedLimits:[0,33,999],globalPeak:Math.max(...samples.map(sample=>sample.active)),hashVerifiedDownloads:saved.length,removedPayloadBytes:saved.length*payload.length,samples,scope:'Packaged GUI/CLI and real loopback HTTP. Isolated profile; no browser extension, installed antivirus or actual shutdown test.'};
  fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(result,null,2));
  console.log(JSON.stringify({success:true,report:path.join(home,'verification.json'),globalPeak:result.globalPeak,hashVerifiedDownloads:saved.length}));
})().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  if(sampler)clearInterval(sampler);
  if(app&&app.exitCode===null){try{closeApp();await until(()=>app.exitCode!==null,'cleanup',5000);}catch{app.kill();}}
  server.closeAllConnections();await new Promise(resolve=>server.close(resolve));db.close();
});

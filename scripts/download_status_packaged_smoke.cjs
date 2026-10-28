const openProfileDatabase = require('./profile_database.cjs');
// A real local HTTP download, native-host handoff and owned native windows.
// No browser profile, personal queue, registration or computer shutdown is touched.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const {spawn, spawnSync} = require('node:child_process');
const {randomUUID, createHash} = require('node:crypto');
const root = path.resolve(__dirname,'..');
const binaries = path.resolve(process.argv[2] || path.join(root,'dist','IDM-EX-UnsignedDev'));
const home = path.join(root,'artifacts','download-status-20261017','packaged-'+randomUUID().slice(0,8));
fs.mkdirSync(path.join(home,'data'),{recursive:true});
const db = openProfileDatabase(path.join(home,'data','queue.sqlite3'));
db.exec(`CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY,close_to_tray INTEGER NOT NULL,notify_complete INTEGER NOT NULL,notify_failed INTEGER NOT NULL);
INSERT INTO desktop_preferences VALUES(1,0,0,0);`);
const body = Buffer.alloc(16*1024*1024,42), requests=[];
let transferReleased = false;
const server = http.createServer((req,res)=>{
  const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range||'');
  const start = range ? +range[1] : 0, end=range && range[2] ? +range[2] : body.length-1;
  requests.push({start,end});
  res.writeHead(range?206:200,{'Content-Type':'application/octet-stream','Content-Length':end-start+1,'Content-Range':`bytes ${start}-${end}/${body.length}`,ETag:'"status-runtime-fixture"'});
  let offset=start;
  const tick=setInterval(()=>{
    if(offset>end){clearInterval(tick);res.end();return;}
    // Hold only the end of real transfers, not the one-byte capability probe.
    // This makes the close-while-downloading check independent of machine speed.
    if(!transferReleased && end-start>65536 && offset+32768>end)return;
    const next=Math.min(offset+32768,end+1);res.write(body.subarray(offset,next));offset=next;
  },40);
  res.on('close',()=>clearInterval(tick));
});
const delay = ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function until(fn,label,timeout=25000){const deadline=Date.now()+timeout;for(;;){const result=await fn();if(result)return result;assert(Date.now()<deadline,label);await delay(100);}}
function windows(pid,action='List') {
  const result=spawnSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',path.join(__dirname,'status_windows.ps1'),'-WindowProcess',String(pid),'-Action',action],{windowsHide:true,encoding:'utf8',timeout:10000});
  assert.equal(result.status,0,result.stderr);return JSON.parse(result.stdout);
}
// The helper returns only this test process's main and download-status windows.
// Status titles now include progress/filename or "Download complete".
const isStatusWindow = window => window.Title !== 'Download Engine';
function native(request,env) {
  return new Promise((resolve,reject)=>{
    const child=spawn(path.join(binaries,'de-host.exe'),[],{env,windowsHide:true,stdio:['pipe','pipe','pipe']});
    let out=[],err=[]; const timer=setTimeout(()=>{child.kill();reject(new Error('Native host timed out'));},15000);
    child.stdout.on('data',data=>out.push(data));child.stderr.on('data',data=>err.push(data));
    child.on('error',reject);child.on('exit',code=>{
      clearTimeout(timer);try { assert.equal(code,0,Buffer.concat(err).toString());const bytes=Buffer.concat(out);assert.equal(bytes.readUInt32LE(),bytes.length-4);const reply=JSON.parse(bytes.subarray(4));assert(reply.ok,reply.message);resolve(reply); } catch(error){reject(error);}
    });
    const data=Buffer.from(JSON.stringify(request)),length=Buffer.alloc(4);length.writeUInt32LE(data.length);child.stdin.end(Buffer.concat([length,data]));
  });
}
let app;
(async()=>{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const env={...process.env,IDM_EX_HOME:home};
  for(const key of Object.keys(env))if(key.startsWith('IDM_EX_TEST_')||key.includes('SCREENSHOT_TO'))delete env[key];
  app=spawn(path.join(binaries,'de.exe'),[],{env,windowsHide:true,stdio:'ignore'});
  app.on('error',error=>{throw error;});
  await until(()=>{try{return db.prepare("SELECT name FROM sqlite_master WHERE name='download_status_requests'").get();}catch{return false;}},'Desktop did not initialize');
  const request={action:'add',request_id:randomUUID(),url:`http://127.0.0.1:${server.address().port}/status.bin`,filename:'status.bin'};
  const reply=await native(request,env);
  const job=()=>JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(reply.job_id).data);
  await until(()=>job().downloaded>0,'Download did not start transferring');
  let visible=await until(()=>{const all=windows(app.pid);return all.some(isStatusWindow)?all:null;},'Native status window did not open');
  assert.equal(visible.filter(isStatusWindow).length,1);
  assert(visible.some(w=>w.Title==='Download Engine'));
  const repeat=await native(request,env);assert.equal(repeat.job_id,reply.job_id);
  assert.equal(windows(app.pid).filter(isStatusWindow).length,1);
  const before=job().downloaded;assert.notEqual(job().status,'complete','Fixture completed before native close check');
  windows(app.pid,'CloseStatus');
  await until(()=>!windows(app.pid).some(isStatusWindow),'Native status did not close');
  assert(windows(app.pid).some(w=>w.Title==='Download Engine'),'Closing status closed the main application');
  await native(request,env);
  transferReleased = true;
  await until(()=>job().downloaded>before,'Download did not continue after closing status');
  assert.equal(windows(app.pid).filter(isStatusWindow).length,0,'Duplicate handoff reopened a hidden window');
  await until(()=>job().status==='complete','Download did not finish');
  const saved=job(), bytes=fs.readFileSync(saved.saved_path);
  assert.equal(createHash('sha256').update(bytes).digest('hex'),createHash('sha256').update(body).digest('hex'));
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n,1);
  windows(app.pid,'CloseApp');await until(()=>app.exitCode!==null,'Test app did not close');
  const payload = fs.realpathSync(saved.saved_path);
  assert(payload.startsWith(fs.realpathSync(home) + path.sep), 'Payload must belong to this isolated profile');
  fs.unlinkSync(payload);
  const result={version:spawnSync(path.join(binaries,'de-cli.exe'),['--version'],{encoding:'utf8',windowsHide:true}).stdout.trim(),nativeWindow:true,closeKeepsMainAndTransfer:true,idempotentHandoff:true,jobCount:1,bytes:bytes.length,generatedPayloadRemoved:true,sha256:createHash('sha256').update(bytes).digest('hex'),requests,scope:'Isolated local HTTP and native host; no live browser extension or shutdown test.'};
  fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({ok:true,report:path.join(home,'verification.json'),...result},null,2));
})().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  if(app && app.exitCode===null){try{windows(app.pid,'CloseApp');await until(()=>app.exitCode!==null,'cleanup',5000);}catch{app.kill();}}
  server.closeAllConnections();await new Promise(resolve=>server.close(resolve));db.close();
});

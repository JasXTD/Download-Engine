const openProfileDatabase = require('./profile_database.cjs');
// Real CLI/desktop, isolated SQLite, loopback bytes. Only owned fixture processes are stopped.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const http=require('node:http'),{spawn,spawnSync}=require('node:child_process');
const {once}=require('node:events'),{randomUUID,createHash}=require('node:crypto');
const {setTimeout:delay}=require('node:timers/promises');
const root=path.resolve(__dirname,'..'),binary=path.resolve(process.argv[2]||path.join(root,'dist/IDM-EX-UnsignedDev'));
const home=path.join(root,'artifacts',`cli-automation-smoke-${randomUUID().slice(0,8)}`);
fs.mkdirSync(home,{recursive:true});
const env={...process.env,IDM_EX_HOME:home};
for(const key of Object.keys(env)) if(key.startsWith('IDM_EX_TEST_')||key.includes('SCREENSHOT_TO'))delete env[key];
const checks=[],requests=[];let app,autoPid,db;
const bytes=Buffer.alloc(24*1024*1024);for(let i=0;i<bytes.length;i++)bytes[i]=(i*31+7)%251;
const sha=b=>createHash('sha256').update(b).digest('hex');
const server=http.createServer((req,res)=>{
  requests.push({method:req.method,url:req.url,range:req.headers.range});
  const data=req.url.includes('slow')?bytes:bytes.subarray(0,256*1024);
  const match=/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||'');
  const start=match?+match[1]:0,end=match&&match[2]?Math.min(+match[2],data.length-1):data.length-1;
  if(start>=data.length){res.writeHead(416,{'Content-Range':`bytes */${data.length}`});res.end();return;}
  const headers={'Content-Length':end-start+1,'Accept-Ranges':'bytes','ETag':'"cli-fixture-v1"','Content-Type':'application/octet-stream'};
  if(match)headers['Content-Range']=`bytes ${start}-${end}/${data.length}`;
  res.writeHead(match?206:200,headers);
  if(req.method==='HEAD'){res.end();return;}
  if(!req.url.includes('slow')){res.end(data.subarray(start,end+1));return;}
  let at=start;const timer=setInterval(()=>{const next=Math.min(at+16384,end+1);res.write(data.subarray(at,next));at=next;if(at>end){clearInterval(timer);res.end();}},15);
  res.once('close',()=>clearInterval(timer));
});
async function until(fn,label,ms=30000){const end=Date.now()+ms;while(Date.now()<end){const v=await fn();if(v)return v;await delay(50);}throw Error(`Timed out: ${label}`);}
async function cli(args,expected=0){
  const child=spawn(path.join(binary,'de-cli.exe'),args,{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
  let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
  const timeout=setTimeout(()=>child.kill(),20000);
  try {const [code,signal]=await once(child,'exit');assert.equal(signal,null);assert.equal(code,expected,`${args.join(' ')}\n${stdout}\n${stderr}`);return {value:JSON.parse(stdout||stderr),pid:child.pid};}finally{clearTimeout(timeout);}
}
async function command(args){const {value}=await cli(args);assert(value.ok);return value.receipt.result;}
function row(id){const r=db.prepare('SELECT data FROM jobs WHERE id=?').get(id);return r&&JSON.parse(r.data);}
async function stop(){if(app&&app.exitCode===null&&app.signalCode===null){const ended=once(app,'exit');app.kill();await ended;}app=null;
  if(autoPid){process.kill(autoPid);autoPid=null;await delay(200);}}
function launch(){app=spawn(path.join(binary,'de.exe'),[],{env,windowsHide:true,stdio:'ignore'});}
function mark(label){checks.push(label);console.log(label);}
(async()=>{
  server.listen(0,'127.0.0.1');await once(server,'listening');const url=`http://127.0.0.1:${server.address().port}`;
  try {
    const initial=await cli(['list']);assert.deepEqual(initial.value.result,[]);
    db=openProfileDatabase(path.join(home,'data/queue.sqlite3'));db.exec('PRAGMA busy_timeout=5000;UPDATE download_queues SET running=0;');
    // A stopped app must not execute timed-out requests on a later startup.
    const timeout=await cli(['queue','create','Must not exist','--no-start','--wait','1'],1);assert.equal(timeout.value.receipt.state,'cancelled');
    // This command cold-launches the real desktop. Find only its direct child for cleanup.
    const cold=await cli(['queue','create','Work']);
    const query=`Get-CimInstance Win32_Process -Filter "Name='de.exe'" | Where-Object { $_.ParentProcessId -eq ${cold.pid} } | Select-Object -ExpandProperty ProcessId`;
    const found=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(query,'utf16le').toString('base64')],{encoding:'utf8',windowsHide:true,timeout:15000});
    assert.ifError(found.error);assert.equal(found.status,0,found.stderr);autoPid=Number(found.stdout.trim());assert(Number.isSafeInteger(autoPid)&&autoPid>0,'Only one owned desktop child expected');
    const work=cold.value.receipt.result.id;assert(!db.prepare('SELECT name FROM download_queues WHERE name=?').get('Must not exist'));mark('cold launch, JSON response, timeout cancellation');
    const follow=(await command(['queue','create','Follow'])).id;
    const rule=await command(['automation','route','ZIP route','--host','127.0.0.1','--extension','zip','--queue','Work','--speed-kib','32']);
    const request=randomUUID();const a=await command(['add',`${url}/file.zip`,'--request-id',request]);const again=await command(['add',`${url}/file.zip`,'--request-id',request]);assert.deepEqual(a,again);
    assert.equal(a[0].queue,work);assert.equal(db.prepare('SELECT bytes_per_second AS n FROM bandwidth_downloads WHERE job_id=?').get(a[0].id).n,32768);
    const mismatch=await cli(['add',`${url}/other.zip`,'--request-id',request],1);assert.match(mismatch.value.error,/another command/);
    const invalid=await cli(['add',`${url}/should-not-exist.bin`,'file:///bad'],1);assert.equal(invalid.value.receipt.state,'failed');assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n,1);
    await cli(['queue','start','Missing queue'],1);await cli(['list','--folder','unexpected'],1);mark('routing, speed cap, idempotent retry, validation and atomic batch rejection');
    const batch=path.join(home,'batch.txt');fs.writeFileSync(batch,`\ufeff${url}/one.bin\n\n${url}/two.bin\n`);await command(['batch',batch,'--queue','Follow']);
    const chain=await command(['automation','chain','Follow Work','--source','Work','--queue','Follow']);
    await command(['queue','limit','Work','1']);const started=Date.now();await command(['queue','start','Work']);
    await until(()=>db.prepare("SELECT COUNT(*) AS n FROM jobs WHERE json_extract(data,'$.status')<>'complete'").get().n===0,'routed and chained downloads complete');
    assert(Date.now()-started>=6000,'The routing speed limit must affect the actual transfer');
    for(const {data}of db.prepare('SELECT data FROM jobs').all()){const job=JSON.parse(data);assert.equal(sha(fs.readFileSync(job.saved_path)),sha(bytes.subarray(0,256*1024)));}
    let history=(await cli(['automation','history'])).value.result;assert.equal(history.length,2);assert(history.every(r=>r.success));
    await command(['queue','pause','Follow']);await stop();launch();
    await command(['queue','pause','Work']);assert.equal(db.prepare('SELECT running FROM download_queues WHERE id=?').get(follow).running,0);assert.equal((await cli(['automation','history'])).value.result.length,2);mark('loopback bytes, completion chain, persisted history and no restart replay');
    await command(['automation','disable',rule.id]);const disabled=(await command(['add',`${url}/disabled.zip`]))[0];assert.equal(disabled.queue,'main');
    await command(['automation','enable',rule.id]);await command(['automation','remove',chain.id]);assert.equal((await cli(['automation','list'])).value.result.length,1);
    const slow=(await command(['add',`${url}/slow.bin`]))[0];await command(['queue','start','main']);
    await until(()=>row(slow.id).downloaded>65536,'active progress');await command(['pause',slow.id]);await until(()=>row(slow.id).status==='paused','settled pause');
    const paused=row(slow.id).downloaded;await delay(200);assert.equal(row(slow.id).downloaded,paused);await command(['resume',slow.id]);
    await until(()=>row(slow.id).status==='complete','resume completes',45000);assert.equal(sha(fs.readFileSync(row(slow.id).saved_path)),sha(bytes));
    const status=(await cli(['status',slow.id])).value.result;assert.equal(status.status,'complete');mark('rule controls, actual active-worker pause/resume and byte-for-byte recovery');
    await command(['pause-all']);await cli(['pause',randomUUID()],1);
    fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify({binary,home,checks,requests,success:true},null,2));console.log(JSON.stringify({home,checks,success:true},null,2));
  } finally {await stop();db?.close();server.closeAllConnections();server.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});

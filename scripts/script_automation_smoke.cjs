// Real desktop, CLI, Lua, PowerShell, loopback downloads and cancellation.
// Each invocation owns its profile, server and child desktop; no user data is used.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const http=require('node:http'),{spawn}=require('node:child_process'),{once}=require('node:events');
const {randomUUID,createHash}=require('node:crypto'),{DatabaseSync}=require('node:sqlite');
const {setTimeout:delay}=require('node:timers/promises');
const root=path.resolve(__dirname,'..'),binary=path.resolve(process.argv[2]||path.join(root,'dist/IDM-EX-UnsignedDev'));
const home=path.join(root,'artifacts',`script-automation-smoke-${randomUUID().slice(0,8)}`);
fs.mkdirSync(home,{recursive:true});const env={...process.env,DOWNLOAD_ENGINE_HOME:home,IDM_EX_HOME:home};
for(const key of Object.keys(env))if(key.startsWith('IDM_EX_TEST_')||key.startsWith('IDM_EX_SCRIPT_')||key.includes('SCREENSHOT_TO'))delete env[key];
const checks=[];let app,db;const childPids=new Set();
const bytes=Buffer.alloc(128*1024);for(let i=0;i<bytes.length;i++)bytes[i]=(i*31+7)%251;
const hash=b=>createHash('sha256').update(b).digest('hex');
const server=http.createServer((req,res)=>{
  if(req.url.includes('/missing/')){res.writeHead(404,{'Content-Length':0});res.end();return;}
  const m=/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||''),start=m?+m[1]:0,end=m&&m[2]?Math.min(+m[2],bytes.length-1):bytes.length-1;
  const headers={'Content-Length':end-start+1,'Accept-Ranges':'bytes',ETag:'"script-fixture"','Content-Type':'application/octet-stream'};
  if(m)headers['Content-Range']=`bytes ${start}-${end}/${bytes.length}`;
  res.writeHead(m?206:200,headers);res.end(req.method==='HEAD'?undefined:bytes.subarray(start,end+1));
});
async function until(fn,label,ms=35000){const deadline=Date.now()+ms;while(Date.now()<deadline){const v=await fn();if(v)return v;await delay(80);}throw Error(`Timed out: ${label}`);}
async function cli(args,expected=0){
  const child=spawn(path.join(binary,'de-cli.exe'),[...args,'--no-start'],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
  let stdout='',stderr='';child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
  const started=Date.now(),timer=setTimeout(()=>child.kill(),20000);
  try{const [code,signal]=await once(child,'exit');const detail=`${JSON.stringify(args)} (${Date.now()-started} ms)\nstdout: ${stdout}\nstderr: ${stderr}`;assert.equal(signal,null,detail);assert.equal(code,expected,detail);const value=JSON.parse(stdout||stderr);return value.receipt?value.receipt.result:value.result;}finally{clearTimeout(timer);}
}
async function save(body,trigger={kind:'manual'},enabled=false,name='Script test',timeout_seconds=30,language,change_files){
  const script={id:randomUUID(),revision:0,name,body,enabled,trigger,timeout_seconds,language,change_files};
  const file=path.join(home,`${script.id}.json`);fs.writeFileSync(file,JSON.stringify(script));
  return cli(['scripts','save',file]);
}
const saveLua=(body,trigger={kind:'manual'},enabled=false,name='Lua test',timeout=30)=>save(body,trigger,enabled,name,timeout,'lua');
const runs=id=>db.prepare('SELECT data FROM script_runs WHERE script_id=? ORDER BY rowid DESC').all(id).map(r=>JSON.parse(r.data));
const run=id=>JSON.parse(db.prepare('SELECT data FROM script_runs WHERE id=?').get(id).data);
const output=id=>db.prepare('SELECT text FROM script_logs WHERE run_id=? ORDER BY seq').all(id).map(l=>l.text).join('');
const terminal=r=>!['queued','running'].includes(r.state);
async function finished(id){return until(()=>{const r=run(id);return terminal(r)&&r;},`run ${id}`);}
function mark(label){checks.push(label);console.log(label);}
async function launch(){
  const log=fs.openSync(path.join(home,'desktop-process.log'),'a');
  try { app=spawn(path.join(binary,'de.exe'),[],{env,windowsHide:true,stdio:['ignore',log,log]}); } finally {fs.closeSync(log);}
  const owned=app;owned.once('exit',(code,signal)=>fs.appendFileSync(path.join(home,'desktop-lifecycle.log'),JSON.stringify({pid:owned.pid,code,signal,at:new Date().toISOString()})+'\n'));
  await until(()=>{
  assert(owned.exitCode===null && owned.signalCode===null,`Test desktop exited before ready: ${owned.exitCode}, ${owned.signalCode}`);
  const f=path.join(home,'data/logs/desktop.log');return fs.existsSync(f)&&fs.readFileSync(f,'utf8').trim().split(/\r?\n/).map(s=>JSON.parse(s)).some(e=>e.pid===owned.pid&&e.event==='ready');
},'desktop ready');}
async function stop(){if(app&&app.exitCode===null&&app.signalCode===null){const ended=once(app,'exit');app.kill();await ended;}app=null;}
function alive(pid){try{process.kill(pid,0);return true;}catch(e){if(e.code==='ESRCH')return false;throw e;}}
function treeBody(){return `$child = Start-Process -FilePath "$PSHOME\\powershell.exe" -WindowStyle Hidden -ArgumentList '-NoProfile -NonInteractive -Command "Start-Sleep 120"' -PassThru\nWrite-Output "CHILD:$($child.Id)"\nWrite-Output 'waiting'\nStart-Sleep 120`}
async function childOf(id){const pid=await until(()=>{const m=/CHILD:(\d+)/.exec(output(id));return m&&+m[1];},'child PID');childPids.add(pid);assert(alive(pid));return pid;}
async function checkLuaGuide(url) {
  const document=fs.readFileSync(path.join(root,'docs/user-guide.md'),'utf8').replaceAll('\r\n','\n');
  // Site readers come last in the topic; their example is a reader, not a runnable automation script.
  const section=document.split('\n## Lua scripting\n')[1]?.split('\n## ')[0]?.split('\n### Site readers')[0];
  assert(section,'Dedicated Lua topic must exist');
  const examples=new Map();
  for(const match of section.matchAll(/```lua\n([\s\S]*?)```/g)) {
    const name=/^-- Example: ([a-z]+)\n/.exec(match[1])?.[1];
    assert(name&&!examples.has(name),'Each guide example needs a unique name');
    examples.set(name,match[1].trimEnd());
  }
  assert.deepEqual([...examples.keys()],['hello','inspect','download','completion','retry','logging']);
  async function execute(name,body=examples.get(name)) {
    const script=await saveLua(body,{kind:'manual'},false,`Guide ${name}`);
    const receipt=await cli(['scripts','run',script.id]);
    assert.equal((await finished(receipt.id)).state,'succeeded',output(receipt.id));
    return {script,receipt,text:output(receipt.id)};
  }
  const hello=await execute('hello');
  assert(hello.text.includes('Hello from Lua!')&&hello.text.includes('Trigger:\tmanual')&&hello.text.includes('Main queue'));
  const empty=await execute('inspect');assert(empty.text.includes('Completed downloads:\t0'));
  mark('Guide hello and inspection examples run verbatim in the built-in Lua worker');
  const logging=await execute('logging');
  assert(logging.text.includes('Handled:')&&logging.text.includes('Still running'));
  const levels=new Set((await cli(['scripts','logs',logging.receipt.id])).map(line=>line.level));
  for(const level of ['info','error'])assert(levels.has(level),`Missing guide log stream: ${level}`);
  // Lua warnings travel over stdout; the viewer recognises WARNING: after
  // rejoining pipe chunks. Its colour handling is covered by desktop tests.
  assert(logging.text.includes('WARNING: This is a warning\n'));
  assert(logging.text.includes('WARNING: Handled:'));
  mark('Guide logging example succeeds with info, warning and error output plus a handled error');
  const watcher=await saveLua(examples.get('completion'),{kind:'download_completed',queue:null},true,'Guide completion');
  const manual=await cli(['scripts','run',watcher.id]);
  assert.equal((await finished(manual.id)).state,'succeeded',output(manual.id));
  assert(output(manual.id).includes('No completed files in this run'));
  mark('Completion example explains an empty manual event');
  const retried=await execute('retry');assert(retried.text.includes('Failed downloads handled:\t0'),retried.text);
  assert.deepEqual((await cli(['scripts','state',retried.script.id])).map(v=>[v.key,v.value]),[['handled',0]]);
  mark('Retry example runs verbatim and keeps its count with xd.store');
  const publicUrl='https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf';
  assert(examples.get('download').includes(publicUrl));
  // Use the same guide program with only its remote sample URL replaced by our
  // owned HTTP fixture. No public download or user profile is touched by this test.
  const downloadSource=examples.get('download').replace(publicUrl,url);
  const completed=[];
  for(let attempt=0;attempt<2;attempt++) {
    const download=await execute('download',downloadSource);
    const id=/Added download:\t([^\s]+)/.exec(download.text)?.[1];assert(id,download.text);
    const job=await until(()=>{
      const row=db.prepare('SELECT data FROM jobs WHERE id=?').get(id);
      if(!row)return false;const value=JSON.parse(row.data);
      assert.notEqual(value.status,'failed',JSON.stringify(value));
      return value.status==='complete'&&value;
    },'guide sample download completed');
    assert.equal(hash(fs.readFileSync(path.join(job.directory,job.name))),hash(bytes));
    completed.push(id);
    const eventRun=await until(()=>runs(watcher.id).find(r=>r.trigger==='download_completed'&&terminal(r)&&r.context.downloads.some(f=>f.id===id)),'guide completion trigger');
    assert.equal(eventRun.state,'succeeded',output(eventRun.id));
    assert(output(eventRun.id).includes('Finished:')&&output(eventRun.id).includes('Saved to:'));
  }
  assert.notEqual(completed[0],completed[1]);
  assert.equal((await cli(['queues'])).filter(q=>q.name==='Lua practice').length,1);
  const inspected=await execute('inspect');assert(inspected.text.includes('Completed downloads:\t2'));
  await cli(['scripts','disable',watcher.id]);
  mark('Guide download runs twice, reuses its queue, saves exact bytes and triggers the verbatim completion script');
  return {examples:[...examples.keys()],documentSha256:hash(Buffer.from(document)),sampleUrlReplacedWithLoopback:true,downloadSha256:hash(bytes)};
}
(async()=>{
  server.listen(0,'127.0.0.1');await once(server,'listening');const url=`http://127.0.0.1:${server.address().port}/script-fixture.bin`;
  try {
    assert.deepEqual(await cli(['scripts','list']),[]);
    db=new DatabaseSync(path.join(home,'data/queue.sqlite3'));db.exec('PRAGMA busy_timeout=5000;UPDATE download_queues SET running=0;');
    await launch();

    if(process.argv.includes('--guide-only')) {
      const guide=await checkLuaGuide(url);
      const result={success:true,binary,home,checks,guide};
      fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(result,null,2));
      console.log(JSON.stringify(result,null,2));
      return;
    }
    const queue=await cli(['queue','create','Script downloads']);
    const timer=await save("Write-Output 'timer fired'",{kind:'interval',minutes:1},true,'Real one minute timer');
    const startup=await save("Write-Output 'startup fired'",{kind:'startup'},true,'Startup');
    const watcher=await save("foreach ($file in $Event.downloads) { if (-not (Test-Path -LiteralPath $file.saved_path)) { throw 'Missing completed file' }; Write-Output (\"EVENT:{0}:{1}\" -f $Event.trigger, $file.name) }",{kind:'download_completed',queue:queue.id},true,'Completion watcher');
    const queueWatch=await save("Write-Output (\"QUEUE:{0}:{1}\" -f $Event.trigger, $Event.queue)",{kind:'queue_completed',queue:queue.id},true,'Queue watcher');
    const program=await save(`$env:IDM_EX_HOME = Join-Path $env:TEMP 'must-not-be-used-script-profile'\n$newQueue = Invoke-DownloadEngine queue create 'Created by PowerShell'\nif (-not $newQueue.id) { throw 'Missing CLI queue result' }\n$jobs = Invoke-DownloadEngine add '${url}' --queue '${queue.id}'\nInvoke-DownloadEngine queue start '${queue.id}' | Out-Null\ndo { Start-Sleep -Milliseconds 150; $job = Invoke-DownloadEngine status $jobs[0].id } while ($job.status -notin @('complete','failed'))\nif ($job.status -ne 'complete') { throw 'Download failed' }\nWrite-Output ("FINISHED:{0}" -f $job.id)\nWrite-Output 'こんにちは 🌍'`,{kind:'manual'},false,'Real CLI download',60);
    const request=randomUUID();const manual=await cli(['scripts','run',program.id,'--request-id',request]);const retried=await cli(['scripts','run',program.id,'--request-id',request]);assert.equal(retried.id,manual.id);assert.equal(runs(program.id).length,1);assert.equal((await finished(manual.id)).state,'succeeded',output(manual.id));
    const jobs=await cli(['list']);assert.equal(jobs.length,1);assert.equal(jobs[0].queue,queue.id);assert.equal(jobs[0].status,'complete');
    const saved=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(jobs[0].id).data).saved_path;assert.equal(hash(fs.readFileSync(saved)),hash(bytes));
    assert(output(manual.id).includes('こんにちは 🌍'));assert((await cli(['queues'])).some(q=>q.name==='Created by PowerShell'));
    mark('PowerShell loops and CLI helper: create queue, add/start/poll real download, exact bytes, Unicode and exact inherited profile');
    for(const [script,needle] of [[watcher,'EVENT:download_completed:'],[queueWatch,`QUEUE:queue_completed:${queue.id}`]]){
      const r=await until(()=>runs(script.id).find(r=>terminal(r)),'completion trigger');assert.equal(r.state,'succeeded',output(r.id));assert(output(r.id).includes(needle));assert.equal(runs(script.id).length,1);
    }
    mark('Download and queue completion triggers deliver saved-file context once');
    const failed=await save("throw 'expected failure'"),failure=await cli(['scripts','run',failed.id]);assert.equal((await finished(failure.id)).state,'failed');assert(output(failure.id).includes('expected failure'));
    const staleFile=path.join(home,'stale.json');fs.writeFileSync(staleFile,JSON.stringify({...failed,revision:0}));await cli(['scripts','save',staleFile],1);
    const invalid=await save("Invoke-DownloadEngine status 'not-an-id'"),invalidRun=await cli(['scripts','run',invalid.id]);assert.equal((await finished(invalidRun.id)).state,'failed');assert(output(invalidRun.id).includes('Download Engine command failed'));
    mark('Script failures, failed CLI commands and stale edits are visible and rejected');
    const long=await save(treeBody(),{kind:'manual'},false,'Stop tree',180),longRun=await cli(['scripts','run',long.id]);const child=await childOf(longRun.id);
    await cli(['scripts','run',long.id],1);await cli(['scripts','remove',long.id],1);
    await cli(['scripts','stop',longRun.id]);assert.equal((await finished(longRun.id)).state,'stopped');await until(()=>!alive(child),'stopped child');childPids.delete(child);
    mark('Run overlap and active removal rejected; Stop kills PowerShell and its child');
    const timeout=await save(treeBody(),{kind:'manual'},false,'Timeout tree',4),timed=await cli(['scripts','run',timeout.id]);const timedChild=await childOf(timed.id);assert.equal((await finished(timed.id)).state,'timed_out');await until(()=>!alive(timedChild),'timed-out child');childPids.delete(timedChild);
    mark('Timeout terminates the whole process tree');
    const flood=await save("[Console]::Out.Write(('界' * 200000)); exit 9"),floodRun=await cli(['scripts','run',flood.id]);assert.equal((await finished(floodRun.id)).exit_code,9);assert(output(floodRun.id).includes('Output truncated'));assert(Buffer.byteLength(output(floodRun.id))<131300);
    const persisted=await cli(['scripts','logs',floodRun.id]);assert(persisted.some(l=>l.level==='warn'));
    mark('Nonzero exits preserved; excessive UTF-8 output is bounded with one warning');
    const luaQueue=await cli(['queue','create','Lua downloads']);
    const luaTimer=await saveLua("print('Lua timer')",{kind:'interval',minutes:1},true,'Lua timer');
    const luaStartup=await saveLua("print('Lua startup')",{kind:'startup'},true,'Lua startup');
    const luaWatcher=await saveLua("for _, f in ipairs(event.downloads) do assert(f.saved_path); print('LUA EVENT:' .. f.name) end",{kind:'download_completed',queue:luaQueue.id},true,'Lua completion');
    const luaQueueWatch=await saveLua("assert(#event.downloads == 0); print('LUA QUEUE:' .. event.queue)",{kind:'queue_completed',queue:luaQueue.id},true,'Lua queue completion');
    const luaProgram=await saveLua(`local function check(id) return xd.status(id) end\nlocal jobs = xd.add('${url}', {queue='${luaQueue.id}', name='Lua download.bin'})\nxd.start_queue('${luaQueue.id}')\nwhile true do\n local job = check(jobs[1].id)\n if job.status == 'complete' then print('LUA FINISHED:' .. job.name); break end\n assert(job.status ~= 'failed', 'Download failed'); xd.sleep(150)\nend\nassert(#xd.downloads() == 2)\nxd.log('info','こんにちは 🌍'); xd.log('warning','Lua warning'); xd.log('error','Lua error log')\nassert(xd.create_queue('Created by Lua').id)`,{kind:'manual'},false,'Lua real download');
    const luaManual=await cli(['scripts','run',luaProgram.id]);assert.equal((await finished(luaManual.id)).state,'succeeded',output(luaManual.id));assert.equal(run(luaManual.id).language,'lua');
    const luaJob=(await cli(['list'])).find(j=>j.queue===luaQueue.id);assert.equal(luaJob.status,'complete');
    const luaSaved=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(luaJob.id).data).saved_path;assert.equal(hash(fs.readFileSync(luaSaved)),hash(bytes));
    assert(output(luaManual.id).includes('こんにちは 🌍'));assert(output(luaManual.id).includes('WARNING: Lua warning'));assert((await cli(['scripts','logs',luaManual.id])).some(l=>l.level==='error'&&l.text.includes('Lua error log')));
    for(const [script,needle] of [[luaWatcher,'LUA EVENT:'],[luaQueueWatch,'LUA QUEUE:']]){const r=await until(()=>runs(script.id).find(terminal),'Lua completion');assert.equal(r.state,'succeeded',output(r.id));assert(output(r.id).includes(needle));assert.equal(runs(script.id).length,1);}
    mark('Built-in Lua functions/loops and xd API download exact bytes, create queues, emit Unicode/severity logs and receive completion triggers');
    for(const [body,needle] of [["error('Lua expected failure')",'Lua expected failure'],['local =','automation.lua'],["xd.status('invalid')",'Download not found'],["local t={}; for i=1,100 do t[i]=string.rep('x',1000000) end",'memory']]){
      const s=await saveLua(body),r=await cli(['scripts','run',s.id]);assert.equal((await finished(r.id)).state,'failed');assert(output(r.id).toLowerCase().includes(needle.toLowerCase()),output(r.id));
    }
    const handled=await saveLua("assert(not pcall(xd.status, 'invalid')); assert(io == nil and os == nil and require == nil and dofile == nil); print('handled')");
    const handledRun=await cli(['scripts','run',handled.id]);assert.equal((await finished(handledRun.id)).state,'succeeded',output(handledRun.id));
    const luaLoop=await saveLua("print('Lua loop started'); while true do end",{kind:'manual'},false,'Lua infinite loop',120),luaLoopRun=await cli(['scripts','run',luaLoop.id]);
    await until(()=>output(luaLoopRun.id).includes('Lua loop started'),'Lua worker running');await cli(['scripts','stop',luaLoopRun.id]);assert.equal((await finished(luaLoopRun.id)).state,'stopped');
    const luaTimeout=await saveLua("while true do end",{kind:'manual'},false,'Lua timeout',2),luaTimedRun=await cli(['scripts','run',luaTimeout.id]);assert.equal((await finished(luaTimedRun.id)).state,'timed_out');
    const luaFlood=await saveLua("for i=1,2000 do print(string.rep('界',100)) end"),luaFloodRun=await cli(['scripts','run',luaFlood.id]);assert.equal((await finished(luaFloodRun.id)).state,'succeeded');assert(output(luaFloodRun.id).includes('Output truncated'));assert(Buffer.byteLength(output(luaFloodRun.id))<131300);
    mark('Lua syntax/runtime/CLI errors, pcall recovery, memory cap, infinite-loop Stop/timeout and bounded Unicode logs');
    const failQueue=await cli(['queue','create','Failing downloads']);
    const missingUrl=url.replace('script-fixture.bin','missing/broken.bin');
    const failWatch=await saveLua("for _, f in ipairs(event.downloads) do\n print('FAILED:' .. f.name .. ':' .. f.attempts .. ':' .. tostring(f.error ~= nil and f.url == nil))\n local count = (xd.store.get('count') or 0) + 1\n xd.store.set('count', count)\n xd.notify('Download failed', f.name)\n if count == 1 then xd.retry(f.id) end\nend",{kind:'download_failed',queue:failQueue.id},true,'Lua failure watcher');
    const [broken]=await cli(['add',missingUrl,'--queue',failQueue.id]);await cli(['queue','start',failQueue.id]);
    const failRuns=await until(()=>{const r=runs(failWatch.id).filter(terminal);return r.length>=2&&r;},'failure trigger and script retry');
    for(const r of failRuns){assert.equal(r.state,'succeeded',output(r.id));assert(output(r.id).includes('FAILED:broken.bin:1:true'),output(r.id));assert(output(r.id).includes('Notification: Download failed · broken.bin'),output(r.id));}
    await delay(1500);assert.equal(runs(failWatch.id).length,2);
    assert.deepEqual((await cli(['scripts','state',failWatch.id])).map(v=>[v.key,v.value]),[['count',2]]);
    assert.equal(JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(broken.id).data).status,'failed');
    mark('A failed download triggers its Lua script once per failure; xd.retry tries again, xd.store keeps a count and xd.notify is logged');
    const sample=await cli(['scripts','sample',failWatch.id,broken.id]);
    assert.equal((await finished(sample.id)).state,'succeeded',output(sample.id));assert.equal(run(sample.id).context.sample,true);
    for(const needle of ['FAILED:broken.bin:1:true','Would store count (sample run, nothing changed)','Would show the notification Download failed · broken.bin (sample run, nothing changed)'])assert(output(sample.id).includes(needle),output(sample.id));
    assert.deepEqual((await cli(['scripts','state',failWatch.id])).map(v=>v.value),[2]);
    const psFail=await save("foreach ($f in $Event.downloads) { Invoke-DownloadEngine retry $f.id | Out-Null; Write-Output ('SAMPLE:' + $Event.sample) }",{kind:'download_failed',queue:failQueue.id},false,'PowerShell failure sample');
    const psSample=await cli(['scripts','sample',psFail.id,broken.id]);
    assert.equal((await finished(psSample.id)).state,'succeeded',output(psSample.id));
    assert(output(psSample.id).includes('Would retry broken.bin (sample run, nothing changed)')&&output(psSample.id).includes('SAMPLE:True'),output(psSample.id));
    assert.equal(JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(broken.id).data).status,'failed');
    await cli(['scripts','sample',psFail.id],1);await cli(['scripts','sample',psFail.id,luaJob.id],1);
    mark('Sample runs get the real event shape; Lua and PowerShell changes are only logged as Would lines');
    const sorted=path.join(home,'downloads','Sorted','Lua');
    const fileBody=`xd.rename('${luaJob.id}', 'Lua renamed.bin')\nxd.move('${luaJob.id}', [[${sorted}]])\nprint('MOVED')`;
    const locked=await saveLua(fileBody,{kind:'manual'},false,'Files without permission');
    const lockedRun=await cli(['scripts','run',locked.id]);assert.equal((await finished(lockedRun.id)).state,'failed');
    assert(output(lockedRun.id).includes('Can change downloaded files'),output(lockedRun.id));assert(fs.existsSync(luaSaved));
    const mover=await save(fileBody,{kind:'manual'},false,'Files with permission',30,'lua',true);
    const moverRun=await cli(['scripts','run',mover.id]);assert.equal((await finished(moverRun.id)).state,'succeeded',output(moverRun.id));
    const movedJob=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(luaJob.id).data);
    assert.equal(movedJob.name,'Lua renamed.bin');assert.equal(movedJob.saved_path,path.join(sorted,'Lua renamed.bin'));
    assert(!fs.existsSync(luaSaved));assert.equal(hash(fs.readFileSync(movedJob.saved_path)),hash(bytes));
    const escape=await save(`xd.move('${luaJob.id}', [[${path.join(home,'outside')}]])`,{kind:'manual'},false,'Escape attempt',30,'lua',true);
    const escapeRun=await cli(['scripts','run',escape.id]);assert.equal((await finished(escapeRun.id)).state,'failed');
    assert(output(escapeRun.id).includes('only into your download folder'),output(escapeRun.id));assert(fs.existsSync(movedJob.saved_path));
    mark('File actions need the per-script switch, keep exact bytes, update the download and stay inside the download folders');
    const timedRun=await until(()=>runs(timer.id).find(terminal),'real interval trigger',75000);assert.equal(timedRun.state,'succeeded',output(timedRun.id));await cli(['scripts','disable',timer.id]);
    mark('Real one-minute interval fired without clock manipulation');
    const luaTimerRun=await until(()=>runs(luaTimer.id).find(terminal),'real Lua interval',75000);assert.equal(luaTimerRun.state,'succeeded',output(luaTimerRun.id));await cli(['scripts','disable',luaTimer.id]);
    const luaCrash=await cli(['scripts','run',luaLoop.id]);await until(()=>output(luaCrash.id).includes('Lua loop started'),'Lua crash worker running');
    assert.equal(runs(startup.id).length,0);
    const crash=await cli(['scripts','run',long.id]);const crashChild=await childOf(crash.id);await stop();await until(()=>!alive(crashChild),'crash child');childPids.delete(crashChild);
    await launch();await until(()=>run(crash.id).state==='interrupted','crash recovery');
    await until(()=>run(luaCrash.id).state==='interrupted','Lua crash recovery');
    const luaBoot=await until(()=>runs(luaStartup.id).find(terminal),'Lua startup');assert.equal(luaBoot.state,'succeeded',output(luaBoot.id));assert.equal(runs(luaStartup.id).length,1);assert.equal(runs(luaWatcher.id).length,1);assert.equal(runs(luaQueueWatch.id).length,1);assert.equal(runs(luaTimer.id).length,1);
    const boot=await until(()=>runs(startup.id).find(terminal),'startup trigger');assert.equal(boot.state,'succeeded',output(boot.id));
    await delay(1100);assert.equal(runs(startup.id).length,1);assert.equal(runs(watcher.id).length,1);assert.equal(runs(queueWatch.id).length,1);assert.equal(runs(timer.id).length,1);assert.equal(hash(Buffer.from(output(floodRun.id))),hash(Buffer.from(persisted.map(l=>l.text).join(''))),'Retained UTF-8 output hash');
    assert(!fs.existsSync(path.join(home,'data/script-runtime',crash.id)));
    mark('Desktop crash kills children; restart marks interrupted without replay, cleans temporary source and preserves logs; startup fires once');
    await cli(['scripts','remove',failed.id]);assert((await cli(['scripts','runs'])).some(r=>r.id===failure.id));assert((await cli(['scripts','logs',failure.id])).length>0);
    mark('Removing a saved script preserves its run history');
    fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify({success:true,binary,home,checks,downloadSha256:hash(bytes)},null,2));console.log(JSON.stringify({success:true,home,checks},null,2));
  } finally {await stop();for(const pid of childPids)if(alive(pid))process.kill(pid);db?.close();server.closeAllConnections();server.close();}
})().catch(e=>{fs.writeFileSync(path.join(home,'failure.txt'),String(e.stack));console.error(e);process.exitCode=1;});

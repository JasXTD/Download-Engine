// Actual desktop-owned HTTP lifecycle. All processes, data and downloads are isolated.
const assert = require('node:assert/strict'), fs = require('node:fs'), path = require('node:path');
const {spawn} = require('node:child_process'), {once} = require('node:events');
const {randomUUID,createHash} = require('node:crypto'), {DatabaseSync} = require('node:sqlite');
const {setTimeout:delay} = require('node:timers/promises'), http = require('node:http');
const root=path.resolve(__dirname,'..'),binary=path.resolve(process.argv[2]||'dist/IDM-EX-UnsignedDev');
const home=path.join(root,'artifacts',`mcp-desktop-smoke-${randomUUID().slice(0,8)}`);
fs.mkdirSync(home,{recursive:true});
const env={...process.env,IDM_EX_HOME:home};
for(const key of Object.keys(env)) if(key.startsWith('IDM_EX_TEST_')||key.startsWith('IDM_EX_SCRIPT_')||key.includes('SCREENSHOT_TO')) delete env[key];
const version=/^version = "([^"]+)"/m.exec(fs.readFileSync(path.join(root,'Cargo.toml'),'utf8'))[1];
const checks=[],children=[];let db,app,config,port;
const meta={'io.modelcontextprotocol/protocolVersion':'2026-07-28','io.modelcontextprotocol/clientCapabilities':{}};
async function until(fn,label,ms=15000){const end=Date.now()+ms;while(Date.now()<end){if(await fn())return;await delay(40);}throw Error(`Timed out: ${label}`);}
async function command(exe,args){
  const child=spawn(exe,args,{env,windowsHide:true,stdio:['ignore','pipe','pipe']});children.push(child);
  let stdout='',stderr='';child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
  const timer=setTimeout(()=>child.kill(),20000);
  try{const [code,signal]=await once(child,'exit');assert.equal(signal,null);assert.equal(code,0,stderr);return stdout;}finally{clearTimeout(timer);}
}
const cli=args=>command(path.join(binary,'de-cli.exe'),args).then(JSON.parse);
function mark(text){checks.push(text);console.log(text);}
const logs=()=>fs.existsSync(path.join(home,'data/logs/desktop.log'))?fs.readFileSync(path.join(home,'data/logs/desktop.log'),'utf8').trim().split(/\r?\n/).map(JSON.parse):[];
async function launch(){
  app=spawn(path.join(binary,'de.exe'),[],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});children.push(app);
  app.completion=once(app,'exit');app.completion.catch(()=>{});
  const child=app;
  child.output='';child.stdout.on('data',b=>child.output+=b);child.stderr.on('data',b=>child.output+=b);
  await until(()=>{assert.equal(app.exitCode,null,app.output);return logs().some(l=>l.pid===app.pid&&l.event==='ready');},'desktop ready');
}
async function close(){
  const ownedPid=app.pid;
  await command('powershell.exe',['-NoProfile','-NonInteractive','-Command',
    `$ErrorActionPreference='Stop'; $owned=Get-Process -Id ${ownedPid}; $deadline=[DateTime]::UtcNow.AddSeconds(10); do { $owned.Refresh(); if($owned.MainWindowHandle -ne 0){break}; Start-Sleep -Milliseconds 50 } while([DateTime]::UtcNow -lt $deadline); if(-not $owned.CloseMainWindow()){throw 'Could not close the owned test window'}`]);
  await until(()=>app.exitCode!==null||app.signalCode!==null,'desktop clean exit',20000);
  const [code,signal]=await app.completion;assert.equal(code,0,app.output);assert.equal(signal,null);
  assert(logs().some(l=>l.pid===ownedPid&&l.event==='stopped'));
}
async function request(method,params={}){
  return new Promise((resolve,reject)=>{
    const req=http.request({host:'127.0.0.1',port,path:'/mcp',method:'POST',agent:false,headers:{...config.headers,'Content-Type':'application/json',Accept:'application/json, text/event-stream','MCP-Protocol-Version':'2026-07-28','Mcp-Method':method,...(method==='tools/call'?{'Mcp-Name':params.name}:{})}},res=>{
      let raw='';res.setEncoding('utf8');res.on('data',b=>raw+=b);res.on('error',reject);res.on('end',()=>{try{assert.equal(res.statusCode,200);const data=raw.split(/\r?\n/).find(l=>l.startsWith('data: '));resolve(JSON.parse(data?data.slice(6):raw));}catch(e){reject(e);}});
    });req.on('error',reject);req.setTimeout(10000,()=>req.destroy(Error('MCP request timed out')));
    req.end(JSON.stringify({jsonrpc:'2.0',id:1,method,params:{...params,_meta:meta}}));
  });
}
async function tool(name,args={},failed=false){const reply=await request('tools/call',{name,arguments:args});assert.equal(reply.result?.isError,failed,JSON.stringify(reply));return reply.result.structuredContent;}
const body=Buffer.alloc(512*1024,0x5b);
const fixture=http.createServer((req,res)=>{res.writeHead(200,{'Content-Length':body.length,'Content-Type':'application/octet-stream'});res.end(req.method==='HEAD'?undefined:body);});
const blocker=http.createServer((req,res)=>res.end('owned conflict fixture'));
async function listening(server,listenPort=0){server.listen(listenPort,'127.0.0.1');await once(server,'listening');return server.address().port;}
async function stop(server){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
(async()=>{
  try{
    port=await listening(blocker);await stop(blocker);
    config=(await cli(['mcp-config','--port',String(port)])).mcpServers["download-engine"];
    const encryptedHash=()=>createHash('sha256').update(fs.readFileSync(path.join(home,'data/mcp-token.dpapi'))).digest('hex');
    const originalTokenFile=encryptedHash();
    await cli(['queues']);
    db=new DatabaseSync(path.join(home,'data/queue.sqlite3'));db.exec('PRAGMA busy_timeout=5000');
    db.prepare('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0').run();
    db.prepare('UPDATE mcp_settings SET port=?').run(port);
    await launch();
    await assert.rejects(()=>request('tools/list'),{code:'ECONNREFUSED'});
    assert(!logs().some(l=>l.pid===app.pid&&l.event==='mcp_started'));
    await close();mark('Default desktop settings leave MCP off; normal close exits cleanly');

    db.exec('UPDATE mcp_settings SET start_with_app=1,read_only=1');
    await launch();await until(()=>logs().some(l=>l.pid===app.pid&&l.event==='mcp_started'),'saved startup starts MCP');
    let catalog=(await request('tools/list')).result.tools;assert(catalog.every(t=>t.annotations.readOnlyHint));
    await tool('create_queue',{name:'Rejected'},true);assert(!db.prepare("SELECT id FROM download_queues WHERE name='Rejected'").get());
    await close();await assert.rejects(()=>request('tools/list'),{code:'ECONNREFUSED'});
    mark('Saved view-only startup filters tools, rejects changes and releases the port on app exit');

    db.exec('UPDATE mcp_settings SET read_only=0');
    await launch();await until(()=>logs().some(l=>l.pid===app.pid&&l.event==='mcp_started'),'control startup');
    catalog=(await request('tools/list')).result.tools;assert.equal(catalog.length,21);
    assert.equal((await request('server/discover')).result._meta['io.modelcontextprotocol/serverInfo'].version,version);
    const queue=await tool('create_queue',{name:'Desktop MCP test',request_id:randomUUID()});assert.equal(queue.receipt.state,'done');
    const fixturePort=await listening(fixture);
    const added=await tool('add_downloads',{queue:queue.receipt.result.id,urls:[`http://127.0.0.1:${fixturePort}/sample.bin`],request_id:randomUUID()});
    const id=added.receipt.result[0].id;
    await tool('start_queue',{queue:queue.receipt.result.id});
    await until(async()=>(await tool('get_download',{id})).download.status==='complete','download through desktop-owned MCP');
    const job=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data);
    assert.deepEqual(fs.readFileSync(job.saved_path),body);
    await close();
    assert.equal(encryptedHash(),originalTokenFile);
    await launch();await until(()=>logs().some(l=>l.pid===app.pid&&l.event==='mcp_started'),'restart');
    assert.equal((await tool('get_download',{id})).download.status,'complete');
    assert.equal((await cli(['mcp-config','--port',String(port)])).mcpServers["download-engine"].headers.Authorization,config.headers.Authorization);
    await close();mark('Allow changes controls the real scheduler, completes an exact-byte download, and survives restart with the same credentials');

    await listening(blocker,port);
    await launch();await until(()=>logs().some(l=>l.pid===app.pid&&l.event==='mcp_failed'),'port conflict reported');
    assert(!logs().some(l=>l.pid===app.pid&&l.event==='mcp_started'));
    assert.equal(await (await fetch(`http://127.0.0.1:${port}`)).text(),'owned conflict fixture');
    await close();await stop(blocker);
    const allLogs=fs.readFileSync(path.join(home,'data/logs/desktop.log'),'utf8');
    assert(!allLogs.includes(config.headers.Authorization.slice(7)));
    mark('Conflicting listeners stay untouched; app exits normally and logs contain no connection secret');
    const result={success:true,version,binary,home,checks,transport:'desktop-owned localhost Streamable HTTP'};
    fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result,null,2));
  }finally{
    for(const child of children)if(child.exitCode===null&&child.signalCode===null){child.kill();await once(child,'exit');}
    db?.close();for(const server of [fixture,blocker]){server.closeAllConnections();server.close();}
  }
})().catch(error=>{console.error(error);process.exitCode=1;});

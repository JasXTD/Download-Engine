// Independent HTTP client, real desktop/CLI and loopback bytes. Own profile only.
const assert = require('node:assert/strict'), fs = require('node:fs'), path = require('node:path');
const {spawn} = require('node:child_process'), {once} = require('node:events');
const {randomUUID, createHash} = require('node:crypto'), {DatabaseSync} = require('node:sqlite');
const {setTimeout:delay} = require('node:timers/promises'), http = require('node:http');
const root = path.resolve(__dirname, '..'), binary = path.resolve(process.argv[2] || 'dist/IDM-EX-UnsignedDev');
const home = path.join(root, 'artifacts', `mcp-smoke-${randomUUID().slice(0,8)}`);
fs.mkdirSync(home, {recursive:true});
const env = {...process.env, IDM_EX_HOME:home};
for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];
const exe = path.join(binary, 'de-cli.exe'), processes = [], clients = [], checks = [];
const version = /^version = "([^"]+)"/m.exec(fs.readFileSync(path.join(root,'Cargo.toml'),'utf8'))[1];
const meta = {'io.modelcontextprotocol/protocolVersion':'2026-07-28','io.modelcontextprotocol/clientCapabilities':{},'io.modelcontextprotocol/clientInfo':{name:'download-engine-smoke',version:'1'}};
let db, autoPid;
async function until(fn, label, ms=30000) { const deadline=Date.now()+ms; while(Date.now()<deadline){const value=await fn();if(value)return value;await delay(30);}throw Error(`Timed out: ${label}`); }
async function timed(promise,label,ms=25000) { let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error(`Timed out: ${label}`)),ms);})]);}finally{clearTimeout(timer);} }
function mark(text){checks.push(text);console.log(text);}
async function cli(args) {
  const child=spawn(exe,args,{env,windowsHide:true,stdio:['ignore','pipe','pipe']});let stdout='',stderr='';
  child.stdout.on('data',b=>stdout+=b);child.stderr.on('data',b=>stderr+=b);
  try{const [code,signal]=await timed(once(child,'exit'),`CLI ${args}`,20000);assert.equal(signal,null);assert.equal(code,0,stderr);return JSON.parse(stdout);}catch(error){child.kill();throw error;}
}
async function freePort() { const probe=http.createServer();probe.listen(0,'127.0.0.1');await once(probe,'listening');const port=probe.address().port;await new Promise(resolve=>probe.close(resolve));return port; }
async function startMcp(args=['--no-start'], port) {
  port ??= await freePort();
  const config=await cli(['mcp-config','--port',String(port)]), entry=config.mcpServers["download-engine"];
  assert.equal(entry.url,`http://127.0.0.1:${port}/mcp`);assert(!entry.command&&!entry.args);assert.match(entry.headers.Authorization,/^Bearer [a-f0-9]{64}$/);
  const child=spawn(exe,['mcp','--port',String(port),...args],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
  const service={child,port,token:entry.headers.Authorization,exit:once(child,'exit'),stderr:'',stdout:''};service.exit.catch(()=>{});processes.push(service);
  child.stdout.on('data',b=>service.stdout+=b);child.stderr.on('data',b=>service.stderr+=b);
  await until(()=>{if(child.exitCode!==null)throw Error(service.stderr);return service.stderr.includes('listening at');},'HTTP server ready');
  service.stop=async()=>{if(child.exitCode===null&&child.signalCode===null){child.kill();await service.exit;}assert.equal(service.stdout,'','HTTP server does not serve RPC on stdout');};
  return service;
}
function exchange(service, body, headers={}, method='POST', endpoint='/mcp') {
  let req,res,aborted=false,settle;
  const promise=new Promise((resolve,reject)=>{
    settle=resolve;
    req=http.request({host:'127.0.0.1',port:service.port,path:endpoint,method,agent:false,headers:{authorization:service.token,'content-type':'application/json',accept:'application/json, text/event-stream',...headers}},response=>{
      res=response;let raw='';response.setEncoding('utf8');response.on('data',chunk=>raw+=chunk);
      response.on('error',error=>aborted?resolve({aborted:true}):reject(error));
      response.on('end',()=>{
        try {
          let json;
          if(response.headers['content-type']?.startsWith('text/event-stream')) {
            const events=raw.split(/\r?\n\r?\n/).map(v=>v.split(/\r?\n/).filter(l=>l.startsWith('data:')).map(l=>l.slice(5).trimStart()).join('\n')).filter(Boolean);
            assert(events.length<=1,'One final response per tool');if(events.length)json=JSON.parse(events[0]);
          } else if(raw)json=JSON.parse(raw);
          resolve({status:response.statusCode,headers:response.headers,json,raw});
        }catch(error){reject(error);}
      });
    });
    req.setTimeout(40000,()=>req.destroy(Error('HTTP test request timed out')));
    req.on('error',error=>aborted?resolve({aborted:true}):reject(error));
    req.end(typeof body==='string'?body:body===undefined?undefined:JSON.stringify(body));
  });
  promise.catch(()=>{});
  return {promise,abort(){aborted=true;res?.destroy();req.destroy();settle({aborted:true});}};
}
class Client {
  constructor(service,legacy) {this.service=service;this.legacy=legacy;this.next=0;this.pending=new Set();clients.push(this);}
  headers(method,params) {
    if(this.legacy)return {...(this.session?{'mcp-session-id':this.session}:{}),...(method==='initialize'?{}:{'mcp-protocol-version':this.legacy})};
    return {'mcp-protocol-version':meta['io.modelcontextprotocol/protocolVersion'],'mcp-method':method,...(method==='tools/call'?{'mcp-name':params.name}:{})};
  }
  send(method,params={}) {
    const id=++this.next, body={jsonrpc:'2.0',id,method,params:this.legacy?params:{...params,_meta:meta}};
    const request=exchange(this.service,body,this.headers(method,params));request.id=id;this.pending.add(request);
    const raw=request.promise;
    request.promise=raw.then(result=>{this.pending.delete(request);if(result.headers?.['mcp-session-id'])this.session=result.headers['mcp-session-id'];return {...result.json,httpStatus:result.status,aborted:result.aborted};});
    request.promise.catch(()=>{});return request;
  }
  async ask(method,params={}) {return this.send(method,params).promise;}
  async notify(method,params={}) {return exchange(this.service,{jsonrpc:'2.0',method,params:this.legacy?params:{...params,_meta:meta}},this.headers(method,params)).promise;}
  async cancel(request) {if(this.legacy)await this.notify('notifications/cancelled',{requestId:request.id});else request.abort();}
  async tool(name,args={},failed=false) {
    const reply=await this.ask('tools/call',{name,arguments:args});assert(!reply.error,JSON.stringify(reply));assert.equal(reply.httpStatus,200);
    assert.equal(reply.result.isError,failed,JSON.stringify(reply));const value=JSON.parse(reply.result.content[0].text);assert.deepEqual(value,reply.result.structuredContent);
    if(!this.legacy)assert.equal(reply.result.resultType,'complete');return value;
  }
  async init() {const result=await this.ask('initialize',{protocolVersion:this.legacy,capabilities:{},clientInfo:{name:'legacy-smoke',version:'1'}});assert.equal(result.result.protocolVersion,this.legacy);assert(this.session);assert.equal((await this.notify('notifications/initialized')).status,202);}
  async close() {for(const request of this.pending)request.abort();this.pending.clear();if(this.session&&this.service.child.exitCode===null&&this.service.child.signalCode===null){await exchange(this.service,undefined,this.headers('delete',{}),'DELETE').promise;this.session=undefined;}}
}
const bytes=Buffer.alloc(8*1024*1024);for(let i=0;i<bytes.length;i++)bytes[i]=(i*31+7)%251;
const hash=buffer=>createHash('sha256').update(buffer).digest('hex');
const server=http.createServer((req,res)=>{
  const match=/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||''),start=match?+match[1]:0,end=match&&match[2]?Math.min(+match[2],bytes.length-1):bytes.length-1;
  const headers={'Content-Length':end-start+1,'Accept-Ranges':'bytes',ETag:'"mcp-fixture"','Content-Type':'application/octet-stream'};
  if(match)headers['Content-Range']=`bytes ${start}-${end}/${bytes.length}`;
  res.writeHead(match?206:200,headers);if(req.method==='HEAD'){res.end();return;}
  let at=start;const timer=setInterval(()=>{const next=Math.min(at+16384,end+1);res.write(bytes.subarray(at,next));at=next;if(at>end){clearInterval(timer);res.end();}},15);res.once('close',()=>clearInterval(timer));
});
const receipt=id=>db.prepare('SELECT state FROM control_requests WHERE id=?').get(id);
const job=id=>JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data);
(async()=>{
  server.listen(0,'127.0.0.1');await once(server,'listening');
  try {
    let service=await startMcp(),client=new Client(service);
    const discovery=await client.ask('server/discover');assert.equal(discovery.result._meta['io.modelcontextprotocol/serverInfo'].version,version);
    assert.deepEqual(discovery.result.supportedVersions,['2026-07-28','2025-11-25','2025-06-18']);
    const catalog=(await client.ask('tools/list')).result.tools;assert.equal(catalog.length,21);assert.equal(new Set(catalog.map(t=>t.name)).size,catalog.length);
    for(const tool of catalog){assert.equal(tool.inputSchema.type,'object');assert.equal(tool.inputSchema.additionalProperties,false);assert.equal(typeof tool.annotations.readOnlyHint,'boolean');}
    assert.equal(catalog.find(t=>t.name==='run_script').annotations.destructiveHint,true);
    const guide=await client.tool('get_usage_guide');assert.equal(guide.version,version);assert.equal(guide.guide,fs.readFileSync(path.join(root,'docs/mcp-ai-guide.md'),'utf8'));
    assert.equal((await client.tool('list_queues')).total,1);assert(!fs.existsSync(path.join(home,'data/logs/desktop.log')),'Read tools must not launch desktop');
    db=new DatabaseSync(path.join(home,'data/queue.sqlite3'));db.exec('PRAGMA busy_timeout=5000');
    const ping={jsonrpc:'2.0',id:1,method:'ping',params:{_meta:meta}}, pingHeaders=client.headers('ping',{});
    for(const [headers,status] of [[{authorization:''},401],[{authorization:'Bearer wrong'},401],[{origin:'https://evil.example'},403],[{origin:'null'},403],[{host:'evil.example'},403],[{'mcp-method':'tools/list'},400],[{'mcp-protocol-version':'2025-11-25'},400],[{accept:'application/json'},406],[{'content-type':'text/plain'},415]]) {
      const result=await exchange(service,ping,{...pingHeaders,...headers}).promise;assert.equal(result.status,status);
    }
    assert.equal((await exchange(service,ping,{...pingHeaders,origin:`http://127.0.0.1:${service.port}`}).promise).status,200);
    assert.equal((await exchange(service,undefined,{},'GET').promise).status,405);
    assert.equal((await exchange(service,undefined,{},'POST','/not-mcp').promise).status,404);
    assert.equal((await exchange(service,'{invalid json}',pingHeaders).promise).json.error.code,-32700);
    assert.equal((await exchange(service,[],pingHeaders).promise).json.error.code,-32600);
    assert.equal((await exchange(service,'x'.repeat(262145),pingHeaders).promise).status,413);
    const encoded=await exchange(service,{jsonrpc:'2.0',id:5,method:'tools/call',params:{name:'list_queues',arguments:{},_meta:meta}},{...client.headers('tools/call',{name:'list_queues'}),'mcp-name':`=?base64?${Buffer.from('list_queues').toString('base64')}?=`}).promise;assert.equal(encoded.json.result.isError,false);
    const unsupported=await exchange(service,{...ping,params:{_meta:{...meta,'io.modelcontextprotocol/protocolVersion':'2099-01-01'}}},{...pingHeaders,'mcp-protocol-version':'2099-01-01'}).promise;assert.equal(unsupported.status,400);assert.equal(unsupported.json.error.code,-32022);
    assert.equal((await client.ask('unknown')).error.code,-32601);assert.equal((await client.ask('tools/call',{name:'unknown'})).error.code,-32602);
    await client.tool('list_downloads',{limit:0},true);await client.tool('list_downloads',{unexpected:true},true);
    assert.equal((await client.notify('tools/call',{name:'pause_all',arguments:{}})).status,202);
    mark('HTTP config, 21 strict schemas, embedded AI guide, authentication, origin/host/header checks and bounded bodies');

    const readonlyService=await startMcp(['--read-only','--no-start']),readOnly=new Client(readonlyService);
    const reads=(await readOnly.ask('tools/list')).result.tools;assert(reads.every(t=>t.annotations.readOnlyHint));await readOnly.tool('create_queue',{name:'Forbidden'},true);
    assert.equal((await readOnly.tool('list_queues')).total,1);await readOnly.close();await readonlyService.stop();
    for(const protocolVersion of ['2025-11-25','2025-06-18']){
      const legacy=new Client(service,protocolVersion);assert.equal((await legacy.ask('tools/list')).httpStatus,400);await legacy.init();
      assert.equal((await legacy.tool('list_queues')).total,1);
      const requestId=randomUUID(),request=legacy.send('tools/call',{name:'pause_all',arguments:{request_id:requestId,wait_seconds:30}});
      await until(()=>receipt(requestId)?.state==='pending','legacy pending');request.abort();await request.promise;await delay(150);assert.equal(receipt(requestId).state,'pending','Legacy disconnect is not cancellation');
      await legacy.cancel(request);await until(()=>receipt(requestId)?.state==='cancelled','legacy explicit cancellation');
      const session=legacy.session;await legacy.close();assert.equal((await exchange(service,{...ping,params:{}},{'mcp-protocol-version':protocolVersion,'mcp-session-id':session}).promise).status,404);
    }
    mark('Both legacy HTTP handshakes, explicit cancellation and enforced read-only mode');

    const uuid=randomUUID(),cancelled=client.send('tools/call',{name:'create_queue',arguments:{name:'Cancelled',request_id:uuid}});
    await until(()=>receipt(uuid)?.state==='pending','modern pending');await client.cancel(cancelled);await cancelled.promise;await until(()=>receipt(uuid)?.state==='cancelled','modern disconnect cancellation');
    const work=[];for(let i=0;i<4;i++){const requestId=randomUUID();const request=client.send('tools/call',{name:'pause_all',arguments:{request_id:requestId,wait_seconds:30}});work.push({requestId,request});}
    await until(()=>work.every(w=>receipt(w.requestId)?.state==='pending'),'four pending calls');assert.equal((await client.ask('tools/call',{name:'pause_all',arguments:{}})).httpStatus,429);
    for(const w of work){await client.cancel(w.request);await w.request.promise;}await until(()=>work.every(w=>receipt(w.requestId)?.state==='cancelled'),'cancel concurrent calls');
    const timeout=await client.tool('pause_all',{wait_seconds:1},true);assert.equal(timeout.receipt.state,'cancelled');
    const firstPid=service.child.pid;await client.close();client=new Client(service);assert.equal((await client.ask('ping')).httpStatus,200);assert.equal(service.child.pid,firstPid);
    const oldToken=service.token,oldPort=service.port;await service.stop();service=await startMcp(['--no-start'],oldPort);assert.equal(service.token,oldToken);client=new Client(service);assert.equal((await client.ask('ping')).httpStatus,200);
    mark('Modern disconnect cancellation, four-call bound, deadline cancellation, reconnect and server restart with stable token');

    const autoService=await startMcp([]),auto=new Client(autoService),queueRequest=randomUUID();
    const queue=await auto.tool('create_queue',{name:'MCP downloads',request_id:queueRequest});assert.equal(queue.receipt.state,'done');
    const log=path.join(home,'data/logs/desktop.log');await until(()=>fs.existsSync(log)&&fs.readFileSync(log,'utf8').includes('"event":"ready"'),'auto-started desktop');
    const ready=fs.readFileSync(log,'utf8').trim().split(/\r?\n/).map(JSON.parse).find(e=>e.event==='ready');autoPid=ready.pid;assert.equal(ready.version,version);
    assert.deepEqual(await auto.tool('create_queue',{name:'MCP downloads',request_id:queueRequest}),queue);await auto.tool('create_queue',{name:'Wrong retry',request_id:queueRequest},true);
    const queueId=queue.receipt.result.id;await client.tool('set_queue_limit',{queue:queueId,limit:1});assert.equal((await client.tool('list_queues',{limit:1})).next_offset,1);
    const addId=randomUUID(),args={urls:[`http://127.0.0.1:${server.address().port}/download.bin`],queue:queueId,request_id:addId};
    const added=await client.tool('add_downloads',args);assert.deepEqual(await client.tool('add_downloads',args),added);
    await client.tool('add_downloads',{urls:[args.urls[0],'file:///not-allowed'],queue:queueId},true);
    const id=added.receipt.result[0].id;assert.equal((await client.tool('list_downloads',{queue:queueId,status:'queued'})).total,1);
    assert.equal((await client.tool('get_request',{request_id:addId})).receipt.state,'done');assert.equal((await client.tool('cancel_request',{request_id:addId})).receipt.state,'done');
    await client.tool('start_queue',{queue:queueId});await until(()=>job(id).downloaded>65536,'actual transfer progress');await client.tool('pause_download',{id});
    await until(()=>job(id).status==='paused','pause settles');const paused=job(id).downloaded;await delay(150);assert.equal(job(id).downloaded,paused);
    await client.tool('resume_download',{id});await until(()=>job(id).status==='complete','resumed file completes',45000);
    assert.equal(hash(fs.readFileSync(job(id).saved_path)),hash(bytes));assert.equal((await client.tool('get_download',{id})).download.status,'complete');
    await client.tool('pause_queue',{queue:queueId});await client.tool('pause_all');assert.equal((await client.tool('list_automation_rules')).total,0);
    mark('Automatic desktop launch, durable retries, real download pause/resume, queue controls and exact byte hash');

    async function save(body,name){const definition={id:randomUUID(),revision:0,name,language:'lua',body,enabled:false,trigger:{kind:'manual'},timeout_seconds:60};const file=path.join(home,`${definition.id}.json`);fs.writeFileSync(file,JSON.stringify(definition));const response=await cli(['scripts','save',file,'--no-start']);return response.receipt.result;}
    const script=await save("print('MCP Lua works')",'MCP Lua');assert.equal((await client.tool('list_scripts')).items[0].body,undefined);assert.equal((await client.tool('get_script',{id:script.id})).script.body,script.body);
    const run=await client.tool('run_script',{id:script.id}),runId=run.receipt.result.id;
    await until(async()=> (await client.tool('list_script_runs',{script_id:script.id})).items.find(r=>r.id===runId&&r.state==='succeeded'),'Lua run completes');
    assert((await client.tool('get_script_log',{run_id:runId})).lines.map(l=>l.text).join('').includes('MCP Lua works'));
    const long=await save('while true do xd.sleep(20) end','MCP stop'),started=await client.tool('run_script',{id:long.id}),stopId=started.receipt.result.id;
    await until(async()=> (await client.tool('list_script_runs',{script_id:long.id})).items.some(r=>r.id===stopId&&r.state==='running'),'Lua worker running');
    await client.tool('stop_script',{run_id:stopId});await until(async()=> (await client.tool('get_script_log',{run_id:stopId})).state==='stopped','script stopped');
    mark('Saved Lua inspection, execution, persistent logs and active worker stop');
    for(const c of clients)await c.close();
    const result={success:true,transport:'localhost Streamable HTTP',version,binary,home,checks,tools:catalog.map(t=>t.name),downloadSha256:hash(bytes),autoStartedDesktopPid:autoPid};
    fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result,null,2));
  } finally {
    for(const client of clients)for(const request of client.pending)request.abort();
    for(const service of processes)if(service.child.exitCode===null&&service.child.signalCode===null){service.child.kill();await service.exit;}
    if(!autoPid){const log=path.join(home,'data/logs/desktop.log');if(fs.existsSync(log)){autoPid=fs.readFileSync(log,'utf8').trim().split(/\r?\n/).map(JSON.parse).find(e=>e.event==='ready')?.pid;}}
    if(autoPid){try{process.kill(autoPid);}catch(error){if(error.code!=='ESRCH')throw error;}}
    db?.close();server.closeAllConnections();server.close();
  }
})().catch(error=>{console.error(error);process.exitCode=1;});

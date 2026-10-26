// Isolated native screenshots and packaged proxy/browser-handoff verification.
// Uses built-in Node APIs, synthetic data and loopback listeners only.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { spawn, spawnSync } = require('node:child_process');
const { once } = require('node:events');
const { randomUUID, createHash } = require('node:crypto');
const { DatabaseSync } = require('node:sqlite');
const { deflateSync } = require('node:zlib');
const root = path.resolve(__dirname, '..');
const ui = process.argv.includes('--ui');
const binaryRoot = path.resolve(ui ? path.join(root, 'target/debug') : process.argv[2] || path.join(root, 'dist/IDM-EX-UnsignedDev'));
const home = path.join(root, 'artifacts', `proxy-zip-${ui ? 'ui' : 'package'}-${randomUUID().slice(0,8)}`);
fs.mkdirSync(path.join(home,'data'), {recursive:true});
const env = {...process.env, IDM_EX_HOME:home};
for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
const sha = data => createHash('sha256').update(data).digest('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve,ms));
async function until(check,label) {
  const end = Date.now()+30000;
  while (Date.now()<end) { const value = check(); if (value) return value; await delay(40); }
  throw Error(`Timed out: ${label}`);
}
function png(ppm) {
  const data=fs.readFileSync(ppm);let pos=0;
  function line(){const end=data.indexOf(10,pos);assert(end>=0);const value=data.subarray(pos,end).toString();pos=end+1;return value;}
  assert.equal(line(),'P6');const [width,height]=line().split(' ').map(Number);assert.equal(line(),'255');assert.equal(data.length-pos,width*height*3);
  function chunk(type,bytes){const body=Buffer.concat([Buffer.from(type),bytes]),size=Buffer.alloc(4),checksum=Buffer.alloc(4);size.writeUInt32BE(bytes.length);let crc=0xffffffff;
    for(const byte of body){crc^=byte;for(let bit=0;bit<8;bit++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}checksum.writeUInt32BE((crc^0xffffffff)>>>0);return Buffer.concat([size,body,checksum]);}
  const pixels=Buffer.alloc(height*(width*3+1));for(let y=0;y<height;y++)data.copy(pixels,y*(width*3+1)+1,pos+y*width*3,pos+(y+1)*width*3);
  const header=Buffer.alloc(13);header.writeUInt32BE(width);header.writeUInt32BE(height,4);header[8]=8;header[9]=2;
  const target=ppm.replace(/\.ppm$/,'.png');fs.writeFileSync(target,Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(pixels)),chunk('IEND',Buffer.alloc(0))]));return target;
}
function screenshots() {
  const results=[];
  for(const feature of ['proxy','zip-preview','add']) for(const dark of [false,true]) {
    const directory=path.join(home,`${feature}-${dark?'dark-small':'light'}`);fs.mkdirSync(path.join(directory,'data'),{recursive:true});
    const db=new DatabaseSync(path.join(directory,'data/queue.sqlite3'));
    db.exec('CREATE TABLE interface_preferences(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)');
    db.prepare('INSERT INTO interface_preferences VALUES(1,?)').run(JSON.stringify({appearance:dark?'dark':'light'}));db.close();
    const ppm=path.join(directory,'dialog.ppm');
    const result=spawnSync(path.join(binaryRoot,'de.exe'),[],{windowsHide:true,timeout:30000,encoding:'utf8',env:{...env,IDM_EX_HOME:directory,IDM_EX_SCREENSHOT_TO:ppm,IDM_EX_TEST_FEATURE:feature,IDM_EX_TEST_SIZE:dark?'860x600':'1080x800'}});
    assert.equal(result.error,undefined,String(result.error));assert.equal(result.status,0,result.stderr);
    const reopened=new DatabaseSync(path.join(directory,'data/queue.sqlite3'));assert.equal(reopened.prepare('SELECT COUNT(*) AS n FROM jobs').get().n,0);reopened.close();
    results.push({feature,dark,screenshot:png(ppm),jobsCreated:0});
  }
  return {home,binaryRoot,results,prefilledUiOnly:true};
}
async function native(request) {
  const child=spawn(path.join(binaryRoot,'de-host.exe'),[],{env,windowsHide:true,stdio:['pipe','pipe','pipe']});
  const chunks=[],errors=[];child.stdout.on('data',data=>chunks.push(data));child.stderr.on('data',data=>errors.push(data));
  const timer=setTimeout(()=>child.kill(),15000);
  try {const end=once(child,'exit');const bytes=Buffer.from(JSON.stringify(request)),header=Buffer.alloc(4);header.writeUInt32LE(bytes.length);child.stdin.end(Buffer.concat([header,bytes]));
    const [code]=await end;assert.equal(code,0,Buffer.concat(errors).toString());const output=Buffer.concat(chunks);assert.equal(output.readUInt32LE(),output.length-4);return JSON.parse(output.subarray(4));
  } finally {clearTimeout(timer);if(child.exitCode===null&&child.signalCode===null)child.kill();}
}
async function packaged() {
  const body=Buffer.alloc(6*1024*1024);for(let i=0;i<body.length;i++)body[i]=i%251;
  const requests=[],sockets=new Set();let fixtureError,child,db;
  const proxy=http.createServer((req,res)=>{
    try {
      assert.equal(req.url,'http://only-proxy.invalid/fixture.bin');assert.equal(req.headers['proxy-authorization'],undefined);
      requests.push({url:req.url,range:req.headers.range});
      const match=/^bytes=(\d+)-(\d+)$/.exec(req.headers.range||'');assert(match);
      const start=Number(match[1]),end=Number(match[2]);assert(start<=end&&end<body.length);
      res.writeHead(206,{'Content-Length':end-start+1,'Content-Range':`bytes ${start}-${end}/${body.length}`,'ETag':'"proxy-fixture"','Content-Type':'application/octet-stream'});
      res.end(body.subarray(start,end+1));
    } catch(error) {fixtureError=error;res.destroy();}
  });
  proxy.on('connection',socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));});
  async function stop(){if(child&&child.exitCode===null&&child.signalCode===null){const end=once(child,'exit');child.kill();await end;}child=undefined;}
  try {
    proxy.listen(0,'127.0.0.1');await once(proxy,'listening');assert((await native({action:'ping'})).ok);
    db=new DatabaseSync(path.join(home,'data/queue.sqlite3'));db.exec('PRAGMA busy_timeout=5000');
    db.prepare('UPDATE proxy_settings SET data=? WHERE id=1').run(JSON.stringify({mode:'custom',endpoint:`http://127.0.0.1:${proxy.address().port}/`,credential:null,allow_unencrypted_login:false}));
    db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0');
    const request_id=`capture:${randomUUID()}`;
    const prepared=await native({action:'capture_prepare',request_id,url:'http://only-proxy.invalid/fixture.bin',filename:'proxy.bin',size:body.length,mime:'application/octet-stream'});
    assert.equal(prepared.ok,true,prepared.message);assert.equal(prepared.capture_state,'prepared');assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n,0);
    child=spawn(path.join(binaryRoot,'de.exe'),[],{env,windowsHide:true,stdio:'ignore'});
    // A visible main window is created only after the owned desktop holds its
    // single-instance lock. Never let the host spawn an untracked second app.
    await until(()=>{
      assert.equal(child.exitCode,null,'Owned desktop exited early');
      const state=spawnSync('powershell.exe',['-NoProfile','-Command',`(Get-Process -Id ${child.pid} -ErrorAction Stop).MainWindowHandle`],{windowsHide:true,encoding:'utf8',timeout:10000});
      assert.equal(state.error,undefined,String(state.error));assert.equal(state.status,0,state.stderr);
      return Number(state.stdout.trim())!==0;
    },'owned desktop window');
    const committed=await native({action:'capture_commit',request_id});assert.equal(committed.ok,true,committed.message);
    const complete=await until(()=>{if(fixtureError)throw fixtureError;const row=db.prepare('SELECT data FROM jobs WHERE id=?').get(committed.job_id);if(!row)return false;const job=JSON.parse(row.data);assert.notEqual(job.status,'failed',job.error);return job.status==='complete'&&job;},'proxied download');
    assert.equal(sha(fs.readFileSync(complete.saved_path)),sha(body));assert.equal(complete.integrity.actual,sha(body));
    assert(fs.readFileSync(`${complete.saved_path}:Zone.Identifier`,'utf8').includes('ZoneId=3'));assert(requests.length>=4);
    await stop();return {home,binaryRoot,bytes:body.length,sha256:sha(body),requests:requests.length,customProxyRoute:true,nativeBrowserPreflight:true,exactBytes:true,markOfTheWeb:true,settingsSeeded:true};
  } finally {await stop();if(db)db.close();for(const socket of sockets)socket.destroy();proxy.close();}
}
(async()=>{const result=ui?screenshots():await packaged();fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result,null,2));})().catch(error=>{console.error(error);process.exitCode=1;});

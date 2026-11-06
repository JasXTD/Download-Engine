// Framed private worker protocol for isolated local fixtures, not a public API.
'use strict';
const assert = require('node:assert/strict');
const {spawn} = require('node:child_process');
const {once} = require('node:events');
const crypto = require('node:crypto');
function luaBytes(bytes) { return '"' + [...bytes].map(b=>'\\x'+b.toString(16).padStart(2,'0')).join('') + '"'; }
async function withWorker(cli, source, files, run, env = process.env) {
  const child = spawn(cli, ['--provider-worker'], {env, windowsHide:true,stdio:['pipe','pipe','pipe']});
  let pending, buffer = Buffer.alloc(0), stderr = '';
  const fail = error => { if (pending) { const p = pending; pending = null; p.reject(error); } };
  child.on('error', fail);
  child.on('exit', () => fail(Error('Provider exited before replying: ' + stderr.slice(-2048))));
  child.stdin.on('error', fail);
  child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-4096); });
  child.stdout.on('data', bytes => {
    buffer = Buffer.concat([buffer, bytes]);
    if (buffer.length < 4) return;
    const size = buffer.readUInt32LE();
    if (size > 8*1024*1024) { fail(Error('Oversized worker reply')); child.kill(); return; }
    if (buffer.length < 4 + size) return;
    try {
      assert(pending, 'Unexpected worker reply');
      const message = JSON.parse(buffer.subarray(4,4+size)); buffer = buffer.subarray(4+size);
      const p = pending; pending = null;
      if (message.error) p.reject(Error(message.error)); else p.resolve(message.value);
    } catch (error) { fail(error); }
  });
  async function call(message) {
    assert(!pending, 'Concurrent worker requests are unsupported');
    const reply = new Promise((resolve,reject)=>{pending={resolve,reject};});
    const body = Buffer.from(JSON.stringify(message)), prefix = Buffer.alloc(4); prefix.writeUInt32LE(body.length);
    child.stdin.write(Buffer.concat([prefix,body]));
    const timer = setTimeout(()=>fail(Error('Provider worker timed out')),30000);
    try { return await reply; } finally { clearTimeout(timer); }
  }
  async function segment(bytes, {prepare=false,stream=false,track='video',index=1}={}) {
    await call({op:'segment_begin',prepare,stream,context:{content_id:'owned-fixture',track_id:track,kind:track==='video'?'video':'audio',
      segment_id:'part-'+index,segment_index:index,track_offset:0,output_offset:0,size:bytes.length}});
    for(let p=0;p<bytes.length;p+=256*1024) await call({op:'segment_feed',bytes:bytes.subarray(p,p+256*1024).toString('base64')});
    const size = await call({op:'segment_run'}), chunks=[];
    if(prepare) return Buffer.alloc(0);
    for(let offset=0;offset<size;) {
      const part=Buffer.from(await call({op:'segment_read',offset}),'base64');
      assert(part.length>0 && offset+part.length<=size); chunks.push(part); offset+=part.length;
    }
    return Buffer.concat(chunks);
  }
  try {
    await call({op:'init',role:'processor',script_id:crypto.randomUUID(),handoff_scope:crypto.randomUUID(),handoff:null,source,
      extension:{id:'offline.worker',version:'1.0.0',job:'fixture',hosts:[],files,settings:{}}});
    return await run({call,segment});
  } finally {
    if(child.exitCode===null && child.signalCode===null) { const ended=once(child,'exit'); child.kill(); await ended; }
  }
}
module.exports={withWorker,luaBytes};

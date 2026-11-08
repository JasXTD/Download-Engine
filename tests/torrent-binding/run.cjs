'use strict';
// Operates only on the two explicitly named, disposable Docker lab containers.
const {spawn,spawnSync}=require('node:child_process');
const fs=require('node:fs');
const assert=require('node:assert/strict');
const lab='de-torrent-binding-20261108', peer='de-torrent-binding-peer-20261108';
const out='artifacts/torrent-binding-20261108';fs.mkdirSync(out,{recursive:true});
function docker(args){const result=spawnSync('docker',args,{encoding:'utf8',timeout:30000,maxBuffer:8*1024*1024});assert.equal(result.status,0,result.stderr);return result.stdout.trim();}
const binding={interface:'eth1',address:'172.30.241.2',dns:'172.30.241.3'};
const results=[];const live=new Set();
class Rpc {
 constructor(container,args){
  this.pending=[];this.buffer='';this.error='';this.exited=false;
  this.child=spawn('docker',['exec','-i',container,'/work/de-torrent','--lab-allow-loopback-peers',...args],{stdio:['pipe','pipe','pipe']});
  live.add(this);
  this.child.stderr.on('data',data=>{this.error=(this.error+data).slice(-4096)});
  this.child.stdout.on('data',data=>{this.buffer+=data;let end;while((end=this.buffer.indexOf('\n'))>=0){const line=this.buffer.slice(0,end);this.buffer=this.buffer.slice(end+1);const item=this.pending.shift();if(!item)throw Error('Unexpected helper reply');clearTimeout(item.timer);const reply=JSON.parse(line);reply.ok?item.resolve(reply.result):item.reject(Error(reply.error));}});
  this.child.on('error',error=>this.fail(error));
  this.child.on('exit',code=>{this.exited=true;this.code=code;live.delete(this);this.fail(Error(`Helper exited ${code}: ${this.error}`));});
 }
 fail(error){for(const item of this.pending){clearTimeout(item.timer);item.reject(error)}this.pending=[];}
 call(command){return new Promise((resolve,reject)=>{if(this.exited)return reject(Error(this.error));const timer=setTimeout(()=>reject(Error('RPC timeout: '+JSON.stringify(command))),15000);this.pending.push({resolve,reject,timer});this.child.stdin.write(JSON.stringify(command)+'\n');});}
 async close(){if(!this.exited){await this.call({cmd:'exit'});this.child.stdin.end();await until(()=>this.exited,5000);}}
}
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function until(check,limit=30000){const start=Date.now();while(Date.now()-start<limit){const value=await check();if(value)return value;await delay(70);}throw Error('Condition timed out');}
async function stats(rpc,name){const all=await rpc.call({cmd:'stats'});assert.equal(all.managed,1,'Unexpected duplicate torrent');const value=all.torrents[name];assert(value);assert(!value.error,value.error);return value;}
async function progress(rpc,name){return until(async()=>{const s=await stats(rpc,name);return s.progress_bytes>65536&&!s.finished&&s;});}
function hash(folder){return docker(['exec',lab,'sha256sum',folder+'/harmless.fixture']).split(/\s+/)[0];}
const client=()=>new Rpc(lab,['--state-dir','/work/client-state','--no-seed','--peer-encryption','required','--utp','--trackers','--web-seeds','--bind-network',JSON.stringify(binding)]);
async function main(){
 const expected=JSON.parse(docker(['exec',peer,'cat','/work/fixtures/manifest.json'])).sha256;
 const seed=new Rpc(peer,['--port','6881','--upload-limit','524288','--utp','--peer-encryption','required']);
 await seed.call({cmd:'add',name:'seed',torrent:'/work/fixtures/peer.torrent',output:'/work/fixtures',seed_only:true});
 await until(async()=>(await stats(seed,'seed')).finished);
 for(const kind of ['peer','tracker','udp-tracker','webseed']){
  const rpc=client(),folder='/work/output-'+kind;
  await rpc.call({cmd:'add',name:kind,torrent:'/work/fixtures/'+kind+'.torrent',output:folder,peers:kind==='peer'?['172.30.241.3:6881']:[]});
  if(kind!=='webseed'){
   await progress(rpc,kind);
   const details=await rpc.call({cmd:'details',name:kind});
   assert(details.peers.some(p=>p.connection.transport==='tcp'&&p.connection.encrypted===true));
   await rpc.call({cmd:'pause',name:kind});
   const saved=(await stats(rpc,kind)).progress_bytes, before=hash(folder);
   await delay(250);assert.equal(hash(folder),before);assert.equal((await stats(rpc,kind)).progress_bytes,saved);
   await rpc.call({cmd:'resume',name:kind});
  }
  await until(async()=>(await stats(rpc,kind)).finished,45000);
  assert.equal(hash(folder),expected);
  await rpc.call({cmd:'stop_all'});await rpc.close();
  results.push({case:kind,passed:true,sha256:expected});
 }
 // Disconnect just the VPN network while another interface remains reachable.
 let rpc=client();const folder='/work/output-loss';
 await rpc.call({cmd:'add',name:'loss',torrent:'/work/fixtures/peer.torrent',output:folder,peers:['172.30.241.3:6881']});
 const before=await progress(rpc,'loss');
 docker(['network','disconnect','de-torrent-binding-vpn',lab]);
 await until(()=>rpc.exited,15000);assert.match(rpc.error,/interface.*unavailable/i);
 const blocked=client();await assert.rejects(blocked.call({cmd:'stats'}));await until(()=>blocked.exited,5000);
 assert.match(blocked.error,/interface.*unavailable/i);
 // Network recovery must require a new helper; it resumes the same saved files.
 await seed.call({cmd:'pause',name:'seed'});
 docker(['network','connect','--ip','172.30.241.2','de-torrent-binding-vpn',lab]);
 rpc=client();
 await rpc.call({cmd:'add',name:'loss',torrent:'/work/fixtures/peer.torrent',output:folder,peers:['172.30.241.3:6881']});
 await until(async()=>{const s=await stats(rpc,'loss');return s.progress_bytes>=before.progress_bytes;});
 await rpc.call({cmd:'stop_all'});
 const stopped=hash(folder);await delay(250);assert.equal(hash(folder),stopped);
 await seed.call({cmd:'resume',name:'seed'});await rpc.call({cmd:'resume',name:'loss'});
 await until(async()=>(await stats(rpc,'loss')).finished,45000);assert.equal(hash(folder),expected);
 results.push({case:'interface-loss-no-fallback-recovery-stop-all',passed:true,retainedBytes:before.progress_bytes,sha256:expected});
 await rpc.close();await seed.close();
 const traffic=docker(['exec',peer,'cat','/work/traffic.jsonl']).split('\n').filter(Boolean).map(JSON.parse);
 const other=traffic.filter(item=>item.local==='172.30.240.3');
 assert(other.every(item=>item.kind==='http'&&item.detail==='/control'),JSON.stringify(other));
 assert(traffic.some(item=>item.kind==='dns-tcp'));
 assert(traffic.some(item=>item.kind==='tracker-udp'));
 assert(traffic.filter(item=>item.local==='172.30.241.3').every(item=>item.remote==='172.30.241.2'), 'Unexpected source interface');
 fs.writeFileSync(out+'/traffic.json',JSON.stringify(traffic,null,2)+'\n');
 results.push({case:'traffic-audit',passed:true,requests:traffic.length,secondaryInterfaceRequests:other.length,secondaryInterfaceOnlyExplicitControls:true});
}
main().then(()=>{fs.writeFileSync(out+'/results.json',JSON.stringify(results,null,2)+'\n');console.log(JSON.stringify(results));}).catch(error=>{console.error(error.stack);process.exitCode=1;}).finally(async()=>{for(const rpc of [...live]){try{await rpc.close();}catch{rpc.child.stdin.end();}}});

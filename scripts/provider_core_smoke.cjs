// Generated keys and synthetic bytes only. No HTTP requests or service credentials.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs'), path = require('node:path');
const {spawn} = require('node:child_process');
const {once} = require('node:events');
const root = path.resolve(__dirname, '..');
const bin = path.resolve(process.argv[2]);
const evidence = path.resolve(process.argv[3]);
fs.mkdirSync(evidence, {recursive:true});
const checks = [];
function bytes(value) { return '"'+[...value].map(b=>'\\x'+b.toString(16).padStart(2,'0')).join('')+'"'; }
async function worker(source, run) {
  const child = spawn(path.join(bin, 'de-cli'+(process.platform==='win32'?'.exe':'')), ['--provider-worker'], {windowsHide:true,stdio:['pipe','pipe','pipe']});
  let pending, buffer=Buffer.alloc(0), stderr='';
  child.stderr.on('data', b=>stderr+=b);
  child.stdout.on('data', b=>{buffer=Buffer.concat([buffer,b]); if(buffer.length>=4 && buffer.length>=4+buffer.readUInt32LE()) {
    const size=buffer.readUInt32LE(); const message=JSON.parse(buffer.subarray(4,4+size)); buffer=buffer.subarray(4+size);
    const resolve=pending;pending=null;resolve(message);
  }});
  const call = async message => {
    assert(!pending,'Only one request at a time');
    const reply=new Promise(resolve=>{pending=resolve;});const body=Buffer.from(JSON.stringify(message)),head=Buffer.alloc(4);head.writeUInt32LE(body.length);
    child.stdin.write(Buffer.concat([head,body]));
    let timer; try {
      const value=await Promise.race([reply,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Worker timed out: '+stderr)),30000);})]);
      assert(!value.error,value.error); return value.value;
    } finally {clearTimeout(timer);}
  };
  try {
    await call({op:'init',role:'processor',script_id:crypto.randomUUID(),handoff_scope:crypto.randomUUID(),handoff:null,extension:null,source});
    await run(call);
  } finally {
    if(child.exitCode===null && child.signalCode===null){const exit=once(child,'exit');child.kill();await exit;}
  }
}
async function main() {
  const pair=crypto.generateKeyPairSync('rsa',{modulusLength:2048});
  const message=Buffer.from('Owned offline interoperability sample\0\xff','latin1');
  const plaintext=crypto.randomBytes(32);
  for(const [format,type] of [['pem','pkcs1'],['pem','pkcs8'],['der','pkcs1'],['der','pkcs8']]) {
    const key=Buffer.from(pair.privateKey.export({format,type}));
    const ciphertexts=['sha1','sha256'].map(oaepHash=>crypto.publicEncrypt({key:pair.publicKey,padding:crypto.constants.RSA_PKCS1_OAEP_PADDING,oaepHash},plaintext));
    const source=`local key=xd.rsa_private_key(${bytes(key)})
      assert(type(key)=='userdata' and not pcall(xd.to_json,key))
      assert(not pcall(xd.rsa_decrypt,key,'oaep-sha256',string.rep('x',256)))
      assert(not pcall(xd.rsa_sign,key,'unknown','x'))
      assert(#xd.random_bytes(32)==32)
      local message=${bytes(message)}
      local ciphers={${ciphertexts.map(bytes).join(',')}}
      return {resolve=function()end,refresh=function()end,finish=function()return true end,
        transform=function(_,input,offset)
          if offset<3 then return xd.rsa_sign(key,({'pss-sha1','pss-sha256','pkcs1-sha256'})[offset+1],message) end
          local result=xd.rsa_decrypt(key,({'oaep-sha1','oaep-sha256'})[offset-2],ciphers[offset-2])
          return result..string.rep(string.char(0),256-#result)
        end}`;
    await worker(source,async call=>{
      for(let offset=0;offset<5;offset++) {
        const result=Buffer.from(await call({op:'media_transform',context:{content_id:'owned',track_id:'video',kind:'video',segment_id:'one',segment_index:0,track_offset:0,output_offset:0,size:512},bytes:Buffer.alloc(256).toString('base64'),offset}),'base64');
        if(offset<3) assert(crypto.verify(offset===0?'sha1':'sha256',message,{key:pair.publicKey,padding:offset===2?crypto.constants.RSA_PKCS1_PADDING:crypto.constants.RSA_PKCS1_PSS_PADDING,saltLength:offset===0?20:32},result));
        else {assert.deepEqual(result.subarray(0,32),plaintext);assert(result.subarray(32).every(b=>b===0));}
      }
    });
    checks.push(`Native RSA ${format}/${type}: PSS SHA-1/SHA-256, PKCS1 SHA-256, OAEP SHA-1/SHA-256 verified against Node crypto`);
    console.log(checks.at(-1));
  }
}
let success=false;
main().then(()=>{success=true;}).catch(error=>{console.error(error);process.exitCode=1;}).finally(()=>{
  fs.writeFileSync(path.join(evidence,'core-smoke.json'),JSON.stringify({success,checks,network:'none; generated ephemeral RSA keys only',cleaned:true},null,2)+'\n');
});

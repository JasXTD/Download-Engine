const {test}=require("node:test");
const assert=require("node:assert/strict");
require("../extension/core.js");
require("../extension/browser-login.js");
const {Manager,selectCookies,target,permissionOrigin}=globalThis.IDMBrowserLogin;
const ID="00000000-0000-4000-8000-000000000001";
const cookie=(patch={})=>({name:"session",value:"SYNTHETIC-PRIVATE-COOKIE",domain:"site.test",path:"/",hostOnly:true,secure:true,storeId:"container",httpOnly:true,...patch});
function fixture(){
  const f={values:{},reads:[],calls:[],granted:true,tab:{id:7,url:"https://site.test/account",cookieStoreId:"container"},cookies:[cookie()],clock:1000,compatible:true};
  const api={
    tabs:{get:async id=>{assert.equal(id,7);return structuredClone(f.tab);},query:async()=>[structuredClone(f.tab)],create:async value=>{f.opened=value;}},
    runtime:{getURL:path=>"extension://test/"+path,getManifest:()=>require('../extension/manifest.chromium.json')},
    storage:{session:{get:async key=>structuredClone(key===null?f.values:{[key]:f.values[key]}),set:async value=>Object.assign(f.values,structuredClone(value)),remove:async key=>delete f.values[key]}},
    permissions:{contains:async grant=>{f.grant=grant;return f.granted;}},
    cookies:{getAllCookieStores:async()=>[{id:"default",tabIds:[2]},{id:"container",tabIds:[7]}],getAll:async query=>{f.reads.push(query);return structuredClone(f.cookies);}}
  };
  const native=async req=>{
    f.calls.push(structuredClone(req));
    if(req.action==="ping") return {capabilities:f.compatible?["browser_login_v1"]:[]};
    if(f.failNative) throw Error("SYNTHETIC-PRIVATE-COOKIE");
    return {ok:true,job_id:"job"};
  };
  f.manager=new Manager({api,native,uuid:()=>ID,now:()=>f.clock});f.api=api;
  return f;
}
test("cookie matching retains HttpOnly and duplicate names, filters scope, expiry, partition and other stores",()=>{
  const url=target("https://site.test/files/a");
  const result=selectCookies([cookie(),cookie({path:"/files",value:"specific"}),
    cookie({domain:".test",hostOnly:false,value:"parent"}),cookie({domain:"evilsite.test"}),
    cookie({path:"/file"}),cookie({path:"/admin"}),cookie({expirationDate:1}),
    cookie({storeId:"default"}),cookie({partitionKey:{topLevelSite:"https://site.test"}}),
    cookie({firstPartyDomain:"other.test"})],url,"container",100);
  assert.deepEqual(result.map(c=>c.value),["specific","SYNTHETIC-PRIVATE-COOKIE","parent"]);
  assert.equal(result[0].host_only,true);
  assert(!("httpOnly" in result[0])&&!result.some(c=>"storeId" in c));
  assert.equal(permissionOrigin(target("https://site.test:8443/file")),"https://site.test/*");
});
test("unsafe cookie values, giant snapshots and insecure sites are rejected without secrets",()=>{
  for(const patch of [{value:"secret\r\nInjected:1"},{name:"secret;other"},{value:"secret;other"},{value:"x".repeat(16385)}]){
    assert.throws(()=>selectCookies([cookie(patch)],target("https://site.test/file"),"container"),e=>!e.message.includes("secret"));
  }
  assert.throws(()=>selectCookies(Array.from({length:129},()=>cookie()),target("https://site.test/file"),"container"),/too large/);
  for(const url of ["http://site.test/file","https://name:secret@site.test/file","file:///C:/file"]) assert.throws(()=>target(url));
});
test("review is anonymous; explicit permission and native capability gate cookie reads",async()=>{
  const f=fixture();await f.manager.open("https://site.test/file");
  assert.equal(f.calls.length,0);assert.equal(f.reads.length,0);
  assert.equal((await f.manager.get(ID)).origin,"https://site.test");
  f.granted=false;await assert.rejects(f.manager.submit(ID),/Allow cookie/);
  assert.equal(f.reads.length,0);
  f.granted=true;f.compatible=false;await assert.rejects(f.manager.submit(ID),/Update/);
  assert.equal(f.reads.length,0);
  assert(!JSON.stringify(f.values).includes("SYNTHETIC"));
});
test("uses original tab's store, sends cookies only to native, and retries one immutable receipt",async()=>{
  const f=fixture();await f.manager.open("https://site.test/file");
  const [a,b]=await Promise.all([f.manager.submit(ID),f.manager.submit(ID)]);
  assert.equal(a.job_id,b.job_id);
  assert.deepEqual(f.reads,[{url:"https://site.test/file",storeId:"container"}]);
  assert.equal(f.calls.filter(c=>c.action==="add_browser_login").length,1);
  assert.equal(f.calls.at(-1).cookies[0].value,"SYNTHETIC-PRIVATE-COOKIE");
  assert.equal(f.calls.at(-1).request_id,"login:"+ID);
  assert(!JSON.stringify(f.values).includes("SYNTHETIC"));
  await f.manager.submit(ID);assert.equal(f.reads.length,1);
});
test("private tabs, cross-origin links, closed/navigated tabs and expired reviews never read cookies",async()=>{
  const f=fixture();
  await assert.rejects(f.manager.open("https://other.test/file"),/same server/);
  f.tab.incognito=true;await assert.rejects(f.manager.open("https://site.test/file"),/regular tab/);
  f.tab.incognito=false;await f.manager.open("https://site.test/file");
  f.tab.url="https://other.test/";await assert.rejects(f.manager.submit(ID));
  f.tab.url="https://site.test/";f.clock+=600001;await assert.rejects(f.manager.submit(ID),/expired/);
  assert.equal(f.reads.length,0);
});
test("API failures and native errors cannot disclose cookies; retry preserves request id",async()=>{
  const f=fixture();await f.manager.open("https://site.test/file");f.failNative=true;
  await assert.rejects(f.manager.submit(ID),e=>!e.message.includes("SYNTHETIC"));
  f.failNative=false;await f.manager.submit(ID);
  const sent=f.calls.filter(c=>c.action==="add_browser_login");
  assert.equal(sent.length,2);assert.equal(sent[0].request_id,sent[1].request_id);
  assert(!JSON.stringify(f.values).includes("SYNTHETIC"));
});
test("container mismatch and permission revocation before send fail closed",async()=>{
  const f=fixture();await f.manager.open("https://site.test/file");
  f.tab.cookieStoreId="other";await assert.rejects(f.manager.submit(ID));
  assert.equal(f.reads.length,0);
  f.tab.cookieStoreId="container";
  f.api.cookies.getAll=async()=>{f.granted=false;return [cookie()];};
  await assert.rejects(f.manager.submit(ID));
  assert(!f.calls.some(c=>c.action==="add_browser_login"));
});
test("browser login requires a durable job acknowledgement",async()=>{
  await assert.rejects(IDMCore.handoff(async()=>({ok:true,version:1}),{action:"add_browser_login"}),/saved download/);
});

for (const variant of ['chromium', 'firefox']) {
  test(variant + ': login review requests only optional cookie and applicable data consent', async () => {
    const f=fixture();f.api.runtime.getManifest=()=>require('../extension/manifest.'+variant+'.json');
    await f.manager.open('https://site.test/file');
    const model=await f.manager.get(ID);
    assert.deepEqual(model.grant, {permissions:['cookies'], ...(variant==='firefox'?{data_collection:['authenticationInfo']}:{})});
    let consent=false;
    f.api.permissions.contains=async grant=>f.granted && (!grant.data_collection || consent);
    if (variant==='firefox') {
      await assert.rejects(f.manager.submit(ID),/Allow cookie/);
      assert.equal(f.reads.length,0);assert.equal(f.calls.length,0);
      consent=true;
      f.api.cookies.getAll=async query=>{f.reads.push(query);consent=false;return [cookie()];};
      await assert.rejects(f.manager.submit(ID),/No supported browser login/);
      assert(!f.calls.some(request=>request.action==='add_browser_login'));
      consent=true;f.api.cookies.getAll=async()=>[cookie()];
    }
    assert.equal((await f.manager.submit(ID)).job_id,'job');
  });
  test(variant + ': login page asks on approval and revokes login without removing detection access', async () => {
    const fs=require('node:fs'),vm=require('node:vm');
    const f=fixture();f.api.runtime.getManifest=()=>require('../extension/manifest.'+variant+'.json');
    await f.manager.open('https://site.test/file');const model=await f.manager.get(ID);
    const requests=[],removals=[],elements=new Map();
    const element=id=>{if(!elements.has(id))elements.set(id,{dataset:{},listeners:{},addEventListener(event,fn){this.listeners[event]=fn;}});return elements.get(id);};
    const api={runtime:{sendMessage:async message=>message.action==='browser_login_get'?model:{ok:true,message:'Added'}},
      permissions:{request:async grant=>{requests.push(grant);return true;},remove:async grant=>{removals.push(grant);return true;}}};
    const context=vm.createContext({[variant==='firefox'?'browser':'chrome']:api,URL,location:{href:'https://extension.test/login.html?id='+ID},
      document:{querySelector:element,getElementById:id=>element('#'+id)},console});
    await vm.runInContext(fs.readFileSync(require.resolve('../extension/login.js'),'utf8'),context);
    await new Promise(setImmediate);assert.equal(requests.length,0);
    await element('#approve').listeners.click();assert.deepEqual(requests,[model.grant]);
    await element('#forget').listeners.click();assert.deepEqual(removals,[model.grant]);
    assert.equal(removals[0].origins,undefined);assert.match(element('#status').textContent,/all sites/);
  });
}

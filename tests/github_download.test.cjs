const {test}=require("node:test");
const assert=require("node:assert/strict");
const fs=require("node:fs"), vm=require("node:vm");
require("../extension/github-download.js");
const {rawUrl,bind,Manager}=globalThis.IDMGitHubDownload;
const PAGE="https://github.com/mozilla/pdf.js/blob/master/test/pdfs/tracemonkey.pdf";
const RAW="https://github.com/mozilla/pdf.js/raw/master/test/pdfs/tracemonkey.pdf";

function dom() {
  const state={listeners:[],downloads:[],removed:0,appended:0};
  const document={
    addEventListener:(type,fn,capture)=>{assert.equal(type,"click");assert.equal(capture,true);state.listeners.push(fn);},
    removeEventListener:(_type,fn)=>{state.listeners=state.listeners.filter(value=>value!==fn);},
    createElement:tag=>{assert.equal(tag,"a");return{click(){state.downloads.push({url:this.href,download:this.download});},remove(){state.removed++;}};},
    documentElement:{appendChild:()=>state.appended++}
  };
  // Minimal DOM event boundary: assert externally observable cancellation and
  // browser navigation, rather than how the URL helper is implemented.
  state.click=(patch={})=>{
    const event={isTrusted:true,button:0,defaultPrevented:false,
      target:{closest:selector=>{assert.equal(selector,'button[data-testid="download-raw-button"]');return{disabled:false};}},
      preventDefault(){this.defaultPrevented=true;},stopImmediatePropagation(){this.stopped=true;},...patch};
    for(const listener of state.listeners) listener(event);
    return event;
  };
  return {state,document};
}
test("GitHub raw URLs preserve refs and encoded filenames on the exact HTTPS host",()=>{
  assert.equal(rawUrl(PAGE),RAW);
  assert.equal(rawUrl("https://github.com/o/r/blob/feature/topic/a%20b.pdf?download=1#page2"),"https://github.com/o/r/raw/feature/topic/a%20b.pdf?download=1");
  for(const value of [null,"bad","https://github.com.example.test/o/r/blob/main/a.pdf","http://github.com/o/r/blob/main/a.pdf",
    "https://github.com:444/o/r/blob/main/a.pdf","https://user:secret@github.com/o/r/blob/main/a.pdf",
    "https://github.com/o/r/tree/main/a.pdf","https://github.com/o/r/blob/main","https://github.com/o/r/blob/main//a.pdf",PAGE+"a".repeat(16384)]) assert.equal(rawUrl(value),null);
});
test("a trusted raw-button click becomes an ordinary HTTP download exactly once",()=>{
  const {state,document}=dom(), location={href:PAGE};
  const unbind=bind(document,location,()=>true);
  assert.deepEqual(state.downloads,[]);
  const event=state.click();
  assert(event.defaultPrevented && event.stopped);
  assert.deepEqual(state.downloads,[{url:RAW,download:""}]);
  assert.equal(state.appended,1);assert.equal(state.removed,1);
  location.href="https://github.com/o/r/blob/new/second.zip";
  state.click();assert.equal(state.downloads[1].url,"https://github.com/o/r/raw/new/second.zip");
  unbind();state.click();assert.equal(state.downloads.length,2);
});
test("capture off, modified clicks, synthetic events and other buttons retain site behavior",()=>{
  const {state,document}=dom();let enabled=false;const location={href:PAGE};
  bind(document,location,()=>enabled);
  assert.equal(state.click().defaultPrevented,false);enabled=true;
  for(const patch of [{isTrusted:false},{button:1},{ctrlKey:true},{shiftKey:true},{altKey:true},{metaKey:true},
    {target:{closest:()=>null}},{target:{closest:()=>({disabled:true})}}]) {
    const event=state.click(patch);assert.equal(event.defaultPrevented,false);assert.equal(event.stopped,undefined);
  }
  state.click({defaultPrevented:true});
  location.href="https://github.com/o/r/commits/main/file.pdf";assert.equal(state.click().defaultPrevented,false);
  assert.deepEqual(state.downloads,[]);
});
function managerFixture() {
  const state={enabled:true,granted:false,scripts:[],values:{},injected:[]};
  const api={permissions:{contains:async grant=>{assert.deepEqual(grant,{origins:["https://github.com/*"]});return state.granted;}},
    storage:{local:{set:async values=>Object.assign(state.values,values)}},
    scripting:{getRegisteredContentScripts:async()=>state.scripts,unregisterContentScripts:async()=>{state.scripts=[];},
      registerContentScripts:async scripts=>{state.scripts=scripts;},executeScript:async target=>state.injected.push(target)},
    tabs:{query:async query=>{assert.deepEqual(query,{url:"https://github.com/*"});return[{id:7},{id:8,incognito:true},{}];}}};
  return {state,manager:new Manager({api,enabled:async()=>state.enabled})};
}
test("GitHub hook uses existing site access and capture setting, with revocation for open tabs",async()=>{
  const {state,manager}=managerFixture();
  await manager.sync(true);assert.equal(state.scripts.length,0);assert.equal(state.injected.length,0);
  state.granted=true;await manager.sync(true);
  assert.deepEqual(state.scripts[0].matches,["https://github.com/*"]);
  assert.equal(state.scripts[0].allFrames,false);
  assert.deepEqual(state.injected,[{target:{tabId:7,frameIds:[0]},files:["github-download.js"]}]);
  assert.equal(state.values.githubDownloadAccess,true);
  state.enabled=false;await manager.sync();assert.equal(state.scripts.length,0);assert.equal(state.values.githubDownloadAccess,false);
  state.enabled=true;state.granted=false;await manager.sync();assert.equal(state.scripts.length,0);
});
test("state requests exclude private tabs, subframes, wrong origins and disabled capture",async()=>{
  const {state,manager}=managerFixture();state.granted=true;
  const sender={frameId:0,tab:{id:1},url:PAGE};
  assert.equal((await manager.status(sender)).enabled,true);
  for(const patch of [{frameId:1},{tab:{id:1,incognito:true}},{tab:{}},{url:"http://github.com/o/r/blob/main/x.pdf"},{url:"https://github.com.evil.test/"},{url:"bad"}])
    assert.equal((await manager.status({...sender,...patch})).enabled,false);
  state.enabled=false;assert.equal((await manager.status(sender)).enabled,false);
});
test("content hook ignores stale enable replies, duplicate injection and invalidated extensions",async()=>{
  const {state,document}=dom();const pending=[];let changed;
  const chrome={runtime:{id:"extension",sendMessage:request=>{assert.equal(request.action,"github_download_state");return new Promise(resolve=>pending.push(resolve));}},
    storage:{onChanged:{addListener:listener=>changed=listener}}};
  const context=vm.createContext({URL,document,location:new URL(PAGE),chrome,console});
  vm.runInContext("globalThis.top=globalThis",context);
  const source=fs.readFileSync(require.resolve("../extension/github-download.js"),"utf8");
  vm.runInContext(source,context);assert.equal(state.listeners.length,1);
  state.click();assert.equal(state.downloads.length,0);
  changed({autoCapture:{newValue:false}},"local");
  pending[1]({ok:true,enabled:false});await new Promise(resolve=>setImmediate(resolve));
  pending[0]({ok:true,enabled:true});await new Promise(resolve=>setImmediate(resolve));
  state.click();assert.equal(state.downloads.length,0);
  changed({autoCapture:{newValue:true}},"local");pending[2]({ok:true,enabled:true});await new Promise(resolve=>setImmediate(resolve));
  vm.runInContext(source,context);assert.equal(state.listeners.length,1);
  state.click();assert.equal(state.downloads.length,1);
  chrome.runtime.id=undefined;state.click();assert.equal(state.downloads.length,1);
});

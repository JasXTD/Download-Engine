const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
require('../extension/permission-flow.js');
function fixture(variant = 'firefox') {
  const state = { allowed:false, focused:true, active:7, views:[], session:{}, configured:[], reports:[], opened:[], requests:[] };
  const addedListeners = new Set();
  state.grant = () => { state.allowed=true; for(const listener of addedListeners)listener(); };
  state.listenerCount = () => addedListeners.size;
  const api = {
    permissions: {
      onAdded:{addListener:listener => addedListeners.add(listener),removeListener:listener => addedListeners.delete(listener)},
      contains:async () => state.allowed,
      request:grant => { state.requests.push(grant); return new Promise(resolve => { state.answer = value => { state.allowed=value; resolve(value); }; }); }
    },
    windows:{get:async id => ({id,focused:state.focused})},
    tabs:{query:async () => [{id:state.active}]},
    runtime:{}, extension:{getViews:() => state.views},
    storage:{session:{set:async data => Object.assign(state.session,data), remove:async key => {delete state.session[key];}}},
    [variant === 'firefox' ? 'browserAction' : 'action']:{openPopup:async target => { if(state.openError)throw Error('Popup needs a user gesture');state.opened.push(target); }}
  };
  // Chromium reports toolbar popup contexts with windowId -1, even though
  // windows.getCurrent() inside that popup returns its owning browser window.
  if(variant === 'chromium')api.runtime.getContexts=async filter => filter.windowIds ? [] : state.views;
  const setting = {grant:() => ({permissions:['downloads']}), configure:async message => {
    state.configured.push(message);if(state.configureError)throw Error('Desktop is not connected');
    return {ok:true,message:'Automatic capture is on.'};
  },denied:'Permission denied. Capture stays off.'};
  const manager = new IDMPermissionFlow.Manager({api,settings:{capture:setting},report:async (ok,message) => state.reports.push({ok,message}),
    schedule:callback => {state.expire=callback;return 1;},unschedule:() => {state.expire=null;}});
  const message = {setting:'capture',returnTo:{windowId:2,tabId:7}};
  return {state,api,manager,message};
}
for(const variant of ['firefox','chromium']) {
  test(variant+': Allow completes in background and returns once to the original popup',async () => {
    const {state,manager,message}=fixture(variant);
    const result=manager.request(message);
    assert.equal(state.requests.length,1,'Permission API runs synchronously on the click stack');
    message.returnTo.tabId=99;message.setting='mutated by destroyed popup';
    assert.equal(state.configured.length,0);
    state.answer(true);
    assert.equal((await result).ok,true);
    assert.equal(state.configured.length,1);
    assert.equal(state.configured[0].setting,'capture','Completion must own its data');
    assert.deepEqual(state.opened,[{windowId:2}]);
    assert.deepEqual(state.session['permission-popup-return:2'],{setting:'capture',tabId:7});
    assert.equal(manager.pending,null);
  });
  test(variant+': denial or a rejected permission request does not enable or reopen',async () => {
    const {state,api,manager,message}=fixture(variant);
    const denied=manager.request(message);state.answer(false);
    assert.equal((await denied).ok,false);
    assert.deepEqual(state.configured,[]);assert.deepEqual(state.opened,[]);
    api.permissions.request=() => {throw Error('User gesture required');};
    assert.match((await manager.request(message)).message,/User gesture/);
    assert.equal(manager.pending,null);
  });
  test(variant+': already granted access or an open popup never triggers another popup',async () => {
    for(const condition of ['allowed','visible']) {
      const {state,manager,message}=fixture(variant);
      if(condition==='allowed')state.allowed=true;else state.views=[{}];
      const result=manager.request(message);state.answer(true);await result;
      assert.equal(state.configured.length,1);assert.deepEqual(state.opened,[]);assert.deepEqual(state.session,{});
    }
  });
  test(variant+': switching windows or tabs prevents an intrusive return',async () => {
    for(const condition of ['window','tab']) {
      const {state,manager,message}=fixture(variant);
      const result=manager.request(message);
      if(condition==='window')state.focused=false;else state.active=9;
      state.answer(true);assert.equal((await result).ok,true);
      assert.deepEqual(state.opened,[]);
    }
  });
  test(variant+': a later disable or revocation cancels pending enabling and permits retry',async () => {
    const {state,manager,message}=fixture(variant);
    const result=manager.request(message);
    assert.throws(() => manager.request(message),/Finish the open permission/);
    manager.cancel('capture');state.answer(true);
    assert.equal((await result).ok,false);assert.equal(manager.pending,null);
    assert.deepEqual(state.configured,[]);assert.deepEqual(state.opened,[]);
    const retry=manager.request(message);state.answer(true);assert.equal((await retry).ok,true);
    assert.equal(state.configured.length,1);
  });
  test(variant+': failed setup returns to its error; unavailable popup API preserves a saved setting',async () => {
    const failed=fixture(variant);failed.state.configureError=true;
    const failure=failed.manager.request(failed.message);failed.state.answer(true);
    assert.equal((await failure).ok,false);assert.equal(failed.state.opened.length,1);
    assert.match(failed.state.reports.at(-1).message,/Desktop is not connected/);
    const fallback=fixture(variant);fallback.state.openError=true;
    const result=fallback.manager.request(fallback.message);fallback.state.answer(true);
    assert.equal((await result).ok,true);assert.equal(fallback.state.configured.length,1);
    assert.deepEqual(fallback.state.session,{});
    assert.equal(fallback.state.reports.at(-1).ok,true);
    assert.match(fallback.state.reports.at(-1).message,/Reopen Download Engine/);
  });
}
test('invalid permission requests cannot reach the browser API',() => {
  const {state,manager,message}=fixture();
  for(const invalid of [{...message,setting:'__proto__'},{...message,setting:'cookies'},{...message,returnTo:null},
    {...message,returnTo:{windowId:2,tabId:-1}}])assert.throws(() => manager.request(invalid));
  assert.deepEqual(state.requests,[]);assert.equal(manager.pending,null);
});

test('Firefox approval completes and returns after the popup promise disappears',async () => {
  const {state,manager,message}=fixture();
  let visibleRequests=0;
  const result=manager.request(message,grant => {
    assert.deepEqual(grant,{permissions:['downloads']});visibleRequests++;
    return new Promise(() => {}); // Firefox destroys this request's popup context.
  });
  assert.equal(visibleRequests,1);assert.equal(state.requests.length,0,'Prompt stays attached to the visible popup');
  state.grant();assert.equal((await result).ok,true);
  assert.equal(state.configured.length,1);assert.deepEqual(state.opened,[{windowId:2}]);
  assert.equal(state.listenerCount(),0);assert.equal(state.expire,null);
});

test('Firefox abandoned permission requests cannot enable later on an unrelated grant',async () => {
  for(const reason of ['reopened','tab','expired','revoked']) {
    const {state,manager,message}=fixture();
    const result=manager.request(message,() => new Promise(() => {}));
    if(reason==='reopened')manager.popupOpened(message.returnTo);
    if(reason==='tab')manager.tabLeft(7);
    if(reason==='expired')state.expire();
    if(reason==='revoked')manager.cancel('capture');
    assert.equal((await result).ok,false);state.grant();
    assert.deepEqual(state.configured,[]);assert.deepEqual(state.opened,[]);
    assert.equal(state.listenerCount(),0);assert.equal(state.expire,null);
  }
});

test('Firefox resolved denial cleans its observer; cached grants complete without reopening',async () => {
  const denied=fixture();assert.equal((await denied.manager.request(denied.message,async () => false)).ok,false);
  assert.equal(denied.state.listenerCount(),0);assert.equal(denied.state.expire,null);
  const cached=fixture();cached.state.allowed=true;
  assert.equal((await cached.manager.request(cached.message,async () => true)).ok,true);
  assert.equal(cached.state.configured.length,1);assert.deepEqual(cached.state.opened,[]);
  assert.equal(cached.state.listenerCount(),0);assert.equal(cached.state.expire,null);
});

test('default permission timers keep the Window receiver required by Firefox',async () => {
  const context=vm.createContext({structuredClone,console});
  vm.runInContext(`
    globalThis.timerCalls=[];
    globalThis.setTimeout=function(callback,delay){
      if(this!==globalThis)throw TypeError("'setTimeout' called on an object that does not implement interface Window.");
      timerCalls.push(['schedule',delay]);return 42;
    };
    globalThis.clearTimeout=function(id){
      if(this!==globalThis)throw TypeError("'clearTimeout' called on an object that does not implement interface Window.");
      timerCalls.push(['clear',id]);
    };
  `,context);
  vm.runInContext(fs.readFileSync(require.resolve('../extension/permission-flow.js'),'utf8'),context);
  const WindowManager=vm.runInContext('IDMPermissionFlow.Manager',context);
  const {state,api,manager:base,message}=fixture();
  const manager=new WindowManager({api,settings:base.settings,report:base.report});
  const reply=await manager.request(message,async()=>true);
  assert.equal(reply.ok,true,reply.message);
  assert.equal(state.configured.length,1);assert.equal(state.listenerCount(),0);
  assert.deepEqual(JSON.parse(vm.runInContext('JSON.stringify(timerCalls)',context)),[['schedule',300000],['clear',42]]);
});

test('a timer setup failure removes the permission observer and allows retry',async () => {
  const {state,api,manager:base,message}=fixture();
  const manager=new IDMPermissionFlow.Manager({api,settings:base.settings,report:base.report,
    schedule:()=>{throw Error('Fixture timer could not be created');},unschedule:()=>{throw Error('No timer was created');}});
  const reply=await manager.request(message,async()=>true);
  assert.equal(reply.ok,false);assert.match(reply.message,/timer could not be created/);
  assert.equal(state.listenerCount(),0);assert.equal(manager.pending,null);
  state.grant();assert.equal(state.configured.length,0);
  manager.schedule=()=>1;manager.unschedule=()=>{};
  assert.equal((await manager.request(message,async()=>true)).ok,true);
  assert.equal(state.listenerCount(),0);
});

const {test}=require('node:test');
const assert=require('node:assert/strict');
require('../extension/core.js');
require('../extension/capture.js');
require('../extension/link-recovery.js');

function fixture(options={}) {
  const h={now:Date.now(),allowed:true,values:{},calls:[],notes:[],timers:[],...options};
  h.session={id:'intent',job_id:'existing-job',name:'file.zip',page:'https://site.test/files',size:1000,state:'waiting',expires:Math.floor(h.now/1000)+1800};
  h.item={id:7,url:'https://files.test/expired',finalUrl:'https://files.test/fresh?token=new',startTime:new Date(h.now).toISOString(),filename:'C:\\Downloads\\file.zip',referrer:'https://site.test/',mime:'application/zip',totalBytes:1000,state:'in_progress',paused:false,danger:'safe'};
  const clone=v=>structuredClone(v);
  h.api={permissions:{contains:async()=>h.allowed},storage:{local:{
    get:async key=>clone(key===null?h.values:{[key]:h.values[key]}),
    set:async values=>{h.calls.push('save');if(h.storageFail)throw Error('Disk full');Object.assign(h.values,clone(values));},
    remove:async key=>{delete h.values[key];}
  }}};
  const downloads={
    search:async()=>h.item?[clone(h.item)]:[],
    pause:async()=>{h.calls.push('pause');if(h.pauseError)throw Error('Pause failed');h.item.paused=true;h.item.canResume=true;if(h.firefox)h.item.state='interrupted';},
    resume:async()=>{h.calls.push('resume');if(h.resumeError)throw Error('Resume failed');h.item.paused=false;h.item.canResume=false;h.item.state='in_progress';},
    cancel:async()=>{h.calls.push('cancel');if(h.cancelError)throw Error('Cancel failed');h.item.paused=false;h.item.canResume=false;h.item.state='interrupted';}
  };
  Object.defineProperty(h.api,'downloads',{get:()=>h.allowed?downloads:undefined});
  const native=request=>IDMCore.handoff(async(_,req)=>{
    h.calls.push(req.action);await h.before?.(req,h);
    if(h.offline)throw Error('App unavailable');
    if(req.action==='recovery_offer') {
      assert.equal(h.item.paused,true,'a live browser copy must be paused before offering');
      assert.equal(h.values['linkRecovery.7'].phase,'pausing','pause must be journalled');
      if(h.rejectOffer)throw Error('File rejected');
      h.session.state='offered';h.session.candidate=clone(req.file);
      if(h.lostOffer)throw Error('Lost reply after offering');
    }
    if(req.action==='recovery_cancel' && h.session?.state!=='accepted')h.session.state='cancelled';
    const recovery=req.action==='recovery_pending' && !['waiting','offered'].includes(h.session?.state)?null:h.session;
    return {ok:true,version:1,recovery:clone(recovery),...h.reply};
  },request);
  h.manager=()=>new IDMLinkRecovery.Manager({api:h.api,native,report:async(ok,message)=>h.notes.push({ok,message}),now:()=>h.now,schedule:fn=>h.timers.push(fn),wait:async()=>{throw Error('Unexpected metadata wait');}});
  return h;
}
for(const firefox of [false,true]) {
  test((firefox?'Firefox contract':'Chromium contract')+': fresh link updates an existing job only after review',async()=>{
    const h=fixture({firefox});const manager=h.manager();
    assert.equal(await manager.consider(7),true);
    assert.equal(h.session.state,'offered');assert(h.item.paused);assert(!h.calls.includes('cancel'));
    assert.equal(h.session.candidate.url,h.item.finalUrl);
    assert(!h.calls.some(action=>action==='add'||action.startsWith('capture_')));
    h.session.state='accepted';await manager.recover(7);
    assert.equal(h.calls.filter(v=>v==='cancel').length,1);assert.equal(h.item.state,'interrupted');
    assert.deepEqual(h.values,{});
  });
  test((firefox?'Firefox contract':'Chromium contract')+': cancelling review releases the browser copy',async()=>{
    const h=fixture({firefox});const manager=h.manager();await manager.consider(7);
    h.session.state='cancelled';await manager.recover(7);
    assert.equal(h.item.paused,false);assert.equal(h.item.state,'in_progress');assert(!h.calls.includes('cancel'));
    assert.equal(await manager.consider(7),true,'resumed fallback is not automatically captured');
    h.item.paused=true;await manager.recoverAll();assert(h.item.paused,'a later user pause stays paused');
    assert.equal(h.calls.filter(v=>v==='resume').length,1);
  });
}
test('unrelated, older, private, unsafe, and already-paused downloads are untouched',async()=>{
  for(const patch of [{filename:'different.zip'},{totalBytes:999},{referrer:'https://other.test/'},{startTime:'invalid'},
    {startTime:new Date(0).toISOString()},{incognito:true},{byExtensionId:'other'},{paused:true},{danger:'file'},{state:'complete'}]) {
    const h=fixture();Object.assign(h.item,patch);assert.equal(await h.manager().consider(7),false,JSON.stringify(patch));
    assert(!h.calls.includes('pause'));assert(!h.calls.includes('cancel'));assert(!h.calls.includes('resume'));
  }
});
test('duplicate suffixes and canonical Unicode names match without changing the saved name',async()=>{
  const h=fixture();h.item.filename='file (2).zip';assert(await h.manager().consider(7));assert.equal(h.session.name,'file.zip');
});
test('storage failure before pause leaves the file running and never offers a link',async()=>{
  const h=fixture({storageFail:true});assert(await h.manager().consider(7));
  assert(!h.calls.includes('pause'));assert(!h.calls.includes('resume'));assert(!h.calls.includes('recovery_offer'));
});
test('concurrent browser events offer and pause exactly once',async()=>{
  const h=fixture();const manager=h.manager();await Promise.all([manager.consider(7),manager.consider(7),manager.consider(7)]);
  assert.equal(h.calls.filter(v=>v==='pause').length,1);assert.equal(h.calls.filter(v=>v==='recovery_offer').length,1);
});
test('rejected and lost offer responses release the browser copy',async()=>{
  for(const option of ['rejectOffer','lostOffer','pauseError']) {
    const h=fixture({[option]:true});await h.manager().consider(7);
    assert.equal(h.session.state,'cancelled');assert.equal(h.item.paused,false);assert(!h.calls.includes('cancel'));
  }
});
test('worker restart recovers paused intent and resolves accepted receipts',async()=>{
  for(const state of ['offered','accepted','cancelled']) {
    const h=fixture();await h.manager().consider(7);h.session.state=state;
    await h.manager().recoverAll();
    assert.equal(h.item.paused,state==='offered');assert.equal(h.calls.includes('cancel'),state==='accepted');
  }
  const h=fixture();await h.manager().consider(7);h.values['linkRecovery.7'].phase='pausing';await h.manager().recoverAll();
  assert.equal(h.item.paused,false);assert.equal(h.session.state,'cancelled');
});
test('app loss or expiry releases the browser file and never authorizes cancellation',async()=>{
  for(const mode of ['offline','expired']) {
    const h=fixture();await h.manager().consider(7);
    if(mode==='offline')h.offline=true;else h.now+=1801*1000;
    await h.manager().recoverAll();assert.equal(h.item.paused,false);assert(!h.calls.includes('cancel'));
  }
});
test('failed cancellation preserves the browser fallback; failed resume remains recoverable',async()=>{
  const h=fixture({cancelError:true});await h.manager().consider(7);h.session.state='accepted';await h.manager().recoverAll();
  assert.equal(h.item.paused,false);assert(h.notes.some(n=>!n.ok));
  const fallback=fixture({resumeError:true});await fallback.manager().consider(7);fallback.session.state='cancelled';await fallback.manager().recoverAll();
  assert(fallback.item.paused);fallback.resumeError=false;await fallback.manager().recoverAll();assert.equal(fallback.item.paused,false);
});
test('reused browser ids and malformed acknowledgements cannot cancel an unrelated download',async()=>{
  const h=fixture();await h.manager().consider(7);h.session.state='accepted';h.item.url='https://unrelated.test/file';
  await h.manager().recoverAll();assert(!h.calls.includes('cancel'));assert(!h.calls.includes('resume'));
  const bad=fixture();await bad.manager().consider(7);bad.reply={recovery:{...bad.session,id:'other',state:'accepted'}};
  await bad.manager().recoverAll();assert(!bad.calls.includes('cancel'));assert.equal(bad.item.paused,false);
});
test('optional download permission granted after startup works with capture disabled',async()=>{
  const h=fixture({allowed:false});const manager=h.manager();assert.equal(await manager.consider(7),false);
  h.allowed=true;assert(await manager.consider(7));assert(h.item.paused);
});

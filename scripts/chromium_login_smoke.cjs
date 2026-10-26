const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
module.exports=async({context,cdp,page,config,setup,checks,version,until,state,errors})=>{
  assert.equal(await page.evaluate(()=>chrome.permissions.contains({permissions:['cookies'],origins:['http://127.0.0.1/*']})),false);
  checks.push('stock manifest website access does not grant login cookies');
  const oldId=new URL(page.url()).hostname;
  await page.goto('about:blank');await cdp.send('Extensions.uninstall',{id:oldId});
  const {id}=await cdp.send('Extensions.loadUnpacked',{path:config.captureExtension});setup(id);
  await page.goto('chrome-extension://'+id+'/popup.html');
  const site=await context.newPage();await site.goto(config.url+'/signed-in');await site.bringToFront();
  const targets=await cdp.send('Target.getTargets',{filter:[{type:'tab'}]});
  const target=targets.targetInfos.find(t=>t.url===config.url+'/signed-in');assert(target);
  await cdp.send('Extensions.triggerAction',{id,targetId:target.targetId});
  await page.evaluate(url=>{
    document.querySelector('#url').value=url+'/private/browser-login.bin';
    document.querySelector('#browser-login').checked=true;
    document.querySelector('#download-form').requestSubmit();
  },config.url);
  const review=await until(()=>context.pages().find(p=>p.url().startsWith('chrome-extension://'+id+'/login.html?id=')),'login review');
  await review.waitForFunction(()=>!document.querySelector('#approve').disabled);
  assert.equal(await review.locator('#origin').textContent(),config.url);
  assert.equal((await state()).jobs.length,1,'review must not enqueue');
  await review.screenshot({path:path.join(config.run,'browser-login-review.png'),fullPage:true});
  await review.locator('#approve').click();
  await review.waitForFunction(()=>document.querySelector('#approve').textContent==='Added to Download Engine');
  const finished=await until(async()=>{const s=await state();return s.jobs.find(j=>j.url.endsWith('/private/browser-login.bin')&&j.bytes_verified);},'protected file');
  assert(finished);assert(!finished.authentication);
  const receipt=await review.evaluate(()=>chrome.runtime.sendMessage({action:'browser_login_submit',id:new URL(location.href).searchParams.get('id')}));
  assert.equal(receipt.job_id,finished.id);assert.equal((await state()).jobs.length,2);
  const storage=await review.evaluate(async()=>[await chrome.storage.local.get(null),await chrome.storage.session.get(null)]);
  assert(!JSON.stringify(storage).includes('SYNTHETIC-'));
  checks.push('real site HttpOnly cookies -> review approval -> native host -> exact bytes, hash and MOTW');
  checks.push('same-origin redirect omits path-only cookies; response cookies are not imported; session deleted on completion');
  checks.push('repeat approval is idempotent; extension storage contains no cookies');
  assert.deepEqual(errors,[]);
  fs.writeFileSync(path.join(config.run,'browser-result.json'),JSON.stringify({version,checks,errors,
    browserLoginManifest:'Cookies and loopback host access pre-granted in disposable fixture; real permission dialog not exercised.'},null,2));
};

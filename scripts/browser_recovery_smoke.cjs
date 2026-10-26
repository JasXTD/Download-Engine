/* Invoked by the ignored Rust browser-recovery integration test. The real
   browser/native-host APIs use isolated profiles and only loopback file data.
   A unique test-only host registration is removed even when a check fails. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {execFileSync}=require('node:child_process'),{createHash,randomUUID}=require('node:crypto');
const {DatabaseSync}=require('node:sqlite'),{chromium}=require('playwright');
const config=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
const quote=value=>"'"+value.replaceAll("'","''")+"'";
const powershell=script=>execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',script],{encoding:'utf8',windowsHide:true});
const vendor=config.browser==='edge'?'Microsoft\\Edge':'Google\\Chrome';
const hostName='com.xetro.downloadengine.recovery_'+randomUUID().replaceAll('-','');
const key='HKCU:\\Software\\'+vendor+'\\NativeMessagingHosts\\'+hostName;
async function until(check,label) {
  const deadline=Date.now()+50000;
  while(Date.now()<deadline){const result=await check();if(result)return result;await new Promise(r=>setTimeout(r,75));}
  throw Error('Timed out: '+label);
}
async function main(){
  let context,registered=false;const checks=[],errors=[];
  try {
    const extension=path.join(config.run,'extension');fs.mkdirSync(extension);
    for(const file of fs.readdirSync(config.extension))if(!file.startsWith('manifest.'))fs.cpSync(path.join(config.extension,file),path.join(extension,file),{recursive:true});
    const core=path.join(extension,'core.js'),coreSource=fs.readFileSync(core,'utf8');
    assert.equal(coreSource.split('"com.xetro.downloadengine"').length,2,'Replace exactly one host name before the browser test; never contact a personal app');
    fs.writeFileSync(core,coreSource.replace('"com.xetro.downloadengine"',JSON.stringify(hostName)));
    const manifest=JSON.parse(fs.readFileSync(path.join(config.extension,fs.existsSync(path.join(config.extension,'manifest.json'))?'manifest.json':'manifest.chromium.json'),'utf8'));
    if(!manifest.permissions.includes('downloads'))manifest.permissions.push('downloads');manifest.optional_permissions=manifest.optional_permissions.filter(p=>p!=='downloads');
    fs.writeFileSync(path.join(extension,'manifest.json'),JSON.stringify(manifest));
    const host=path.join(config.run,'de-host.exe');fs.copyFileSync(config.host,host);
    fs.copyFileSync(path.join(path.dirname(config.host),'de.exe'),path.join(config.run,'de.exe'));
    fs.writeFileSync(path.join(config.run,'portable.json'),JSON.stringify({data:'data',downloads:'downloads'}));
    context=await chromium.launchPersistentContext(path.join(config.run,'browser-profile'),{
      executablePath:config.browser==='edge'?'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe':'C:/Program Files/Google/Chrome/Application/chrome.exe',
      headless:true,ignoreDefaultArgs:['--disable-extensions'],args:['--enable-unsafe-extension-debugging'],acceptDownloads:true,
      env:{...process.env,IDM_EX_HOME:config.run}
    });
    const cdp=await context.browser().newBrowserCDPSession(),version=await cdp.send('Browser.getVersion');
    // Playwright normally replaces filenames with UUIDs. Preserve the real
    // browser filename here because filename matching is part of recovery.
    const browserDownloads=path.join(config.run,'browser-downloads');fs.mkdirSync(browserDownloads);
    await cdp.send('Browser.setDownloadBehavior',{behavior:'allow',downloadPath:browserDownloads,eventsEnabled:true});
    const {id}=await cdp.send('Extensions.loadUnpacked',{path:extension});
    const hostManifest=path.join(config.run,'native-test.json');
    fs.writeFileSync(hostManifest,JSON.stringify({name:hostName,description:'Isolated Download Engine recovery test',path:host,type:'stdio',allowed_origins:['chrome-extension://'+id+'/']}));
    registered=true;
    powershell(`$ErrorActionPreference='Stop'; if(Test-Path -LiteralPath ${quote(key)}){throw 'Test host already exists'}; New-Item -Path ${quote(key)} -Force | Out-Null; Set-Item -LiteralPath ${quote(key)} -Value ${quote(hostManifest)}`);
    const popup=await context.newPage();popup.on('pageerror',e=>errors.push(e.message));
    const site=await context.newPage();
    await site.route(config.page,route=>route.fulfill({contentType:'text/html',body:`<!doctype html><a id="file" href="${config.url}">Get the file again</a>`}));
    await site.goto(config.page);
    const db=new DatabaseSync(path.join(config.run,'data','queue.sqlite3'),{readOnly:true});
    try {
      for(let i=0;i<2;i++) {
        await popup.goto('chrome-extension://'+id+'/popup.html');
        const session=await until(()=>popup.evaluate(async job=>{
          const reply=await chrome.runtime.sendMessage({action:'link_recovery_status'});
          return reply.recovery?.job_id===job?reply.recovery:null;
        },config.jobs[i]),'recovery intent '+i);
        await popup.evaluate(()=>refreshBrowserRecovery());
        assert(await popup.locator('#link-recovery').isVisible());assert.equal(await popup.locator('#auto-capture').isChecked(),false);
        await popup.screenshot({path:path.join(config.run,'recovery-popup-'+i+'.png'),fullPage:true});
        const downloadEvent=site.waitForEvent('download');await site.locator('#file').click();await downloadEvent;
        const item=await until(()=>popup.evaluate(async previous=>{
          const items=await chrome.downloads.search({});return items.find(d=>d.paused && !previous.includes(d.id));
        },checks.map(c=>c.browserId)),'browser pause '+i);
        fs.writeFileSync(path.join(config.run,'paused-'+i),session.id);
        if(i===0) {
          const kept=await until(()=>popup.evaluate(async id=>{const [d]=await chrome.downloads.search({id});return d?.state==='complete'?d:null;},item.id),'cancelled recovery resumes browser copy');
          assert.equal(createHash('sha256').update(fs.readFileSync(kept.filename)).digest('hex'),config.sha256);
          const job=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(config.jobs[i]).data);
          assert.equal(job.status,'failed');assert(job.url.includes('expired'));
          checks.push({scenario:'cancel resumes exact browser file and preserves existing partial download',browserId:item.id});
        } else {
          await until(()=>popup.evaluate(async id=>{const [d]=await chrome.downloads.search({id});return d?.state==='interrupted'&&!d.canResume;},item.id),'accepted browser cancellation');
          const job=await until(()=>{const j=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(config.jobs[i]).data);return j.status==='complete'?j:null;},'existing job completion');
          assert.equal(createHash('sha256').update(fs.readFileSync(job.saved_path)).digest('hex'),config.sha256);
          assert.equal(db.prepare('SELECT count(*) n FROM jobs').get().n,2);
          checks.push({scenario:'real native handoff, saved-byte check, confirmed resume, exact file, no duplicate job',browserId:item.id});
        }
      }
      assert.deepEqual(errors,[]);
      fs.writeFileSync(path.join(config.run,'verification.json'),JSON.stringify({version,checks,errors,scope:'Real browser extension, native host and Rust scheduler. Desktop dialog separately tested. Disposable extension uses a unique test-only native-host name and pre-granted downloads permission. No real permission prompt, Firefox or shutdown test.'},null,2));
      console.log(JSON.stringify({browser:config.browser,checks},null,2));
    }finally{db.close();}
  }finally {
    try {if(context)await context.close();} finally {
      if(registered) {
        powershell(`$ErrorActionPreference='Stop'; if(Test-Path -LiteralPath ${quote(key)}){Remove-Item -LiteralPath ${quote(key)}}`);
      }
    }
  }
}
main().catch(error=>{console.error(error);process.exitCode=1;});

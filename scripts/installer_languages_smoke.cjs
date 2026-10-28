const openProfileDatabase = require('./profile_database.cjs');
// Isolated real Setup install/upgrade/uninstall. Never registers browsers or launches protection/shutdown actions.
const fs=require('node:fs');
const path=require('node:path');
const assert=require('node:assert/strict');
const {spawnSync}=require('node:child_process');
const {randomUUID,createHash}=require('node:crypto');
const root=path.resolve(__dirname,'..');
const languages=JSON.parse(fs.readFileSync(path.join(root,'locales/languages.json'),'utf8')).map(language=>language.code);
const version=/^version = "([^"]+)"/m.exec(fs.readFileSync(path.join(root,'Cargo.toml'),'utf8'))[1];
const installer=path.join(root,`dist/DownloadEngine-${version}-TestSetup.exe`);
assert(fs.existsSync(installer),'Build the isolated test installer first');
const output=path.join(root,'artifacts','installer-locales-'+randomUUID().slice(0,8));
fs.mkdirSync(output,{recursive:true});
const identity='HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IDM_EX.DownloadManager.SmokeTest_is1';
const keys=['Google\\Chrome','Microsoft\\Edge','Chromium','Mozilla'].flatMap(s=>['com.xetro.downloadengine','com.idmex.downloader'].map(host=>'HKCU\\Software\\'+s+'\\NativeMessagingHosts\\'+host));
const reg=(key,view)=>{const r=spawnSync('reg.exe',['query',key,`/reg:${view}`],{encoding:'utf8',windowsHide:true,timeout:10000});return r.status===0?r.stdout.replace(/\r\n/g,'\n'):null;};
const snapshot=()=>Object.fromEntries(keys.flatMap(key=>[32,64].map(view=>[`${key}:${view}`,reg(key,view)])));
assert([32,64].every(view=>reg(identity,view)===null),'A prior test install exists; inspect it before continuing');
// The test Setup cleans "Start with Windows" in this isolated key; the real Run key must not change.
const startupKey='HKCU\\Software\\IDM_EX\\Tests\\InstallerStartup',startupKeys=[startupKey+'\\Run',startupKey+'\\StartupApproved'];
const startupValue=key=>{const r=spawnSync('reg.exe',['query',key,'/v','XDownloader'],{encoding:'utf8',windowsHide:true,timeout:10000});return r.status===0?(/XDownloader\s+REG_\w+\s+(.*)/.exec(r.stdout)?.[1]??'').trim():null;};
const realStartup=()=>['HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run','HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run'].map(startupValue);
const plantStartup=command=>{
  for(const [key,type,data] of [[startupKeys[0],'REG_SZ',command],[startupKeys[1],'REG_BINARY','030000000000000000000000']]) assert.equal(spawnSync('reg.exe',['add',key,'/v','XDownloader','/t',type,'/d',data,'/f'],{windowsHide:true,timeout:10000}).status,0);
  assert.equal(startupValue(startupKeys[0]),command);
};
assert.equal(reg(startupKey,64),null,'A prior startup cleanup fixture exists; inspect it before continuing');
const before=snapshot(),startupBefore=realStartup(),checks=[];
const digest=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
function run(exe,args,env=process.env,timeout=120000){const r=spawnSync(exe,args,{env,windowsHide:true,encoding:'utf8',timeout});if(r.error)throw r.error;return r;}
function removeIsolated(target){const resolved=path.resolve(target);assert(resolved.startsWith(output+path.sep)&&resolved!==output);fs.rmSync(resolved,{recursive:true,force:true});}
// The installed first-run language path deliberately ignores explicit profile overrides.
// LOCALAPPDATA isolates the database; Windows Known Folders ignore USERPROFILE.
// No test payload is written there. Remove only a newly created EMPTY app folder.
const folderQuery=run('reg.exe',['query','HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders','/v','{374DE290-123F-4565-9164-39C4925E467B}']);
assert.equal(folderQuery.status,0,'Cannot snapshot Windows Downloads location');
const folderValue=/REG_(?:EXPAND_)?SZ\s+([^\r\n]+)/.exec(folderQuery.stdout)?.[1];assert(folderValue);
const downloadsRoot=folderValue.trim().replace(/%([^%]+)%/g,(_,name)=>process.env[name]??'%'+name+'%');
assert(/^[A-Za-z]:\\/.test(downloadsRoot)&&!downloadsRoot.includes('%'),'Installer smoke needs a local, resolved Downloads folder');
const defaultDownloadFolder=path.resolve(downloadsRoot,'Download Engine');
const defaultFolderExisted=fs.existsSync(defaultDownloadFolder);
const cleanupDefaultFolder=()=>{
  if(!defaultFolderExisted&&fs.existsSync(defaultDownloadFolder)) {
    assert(fs.lstatSync(defaultDownloadFolder).isDirectory()&&!fs.lstatSync(defaultDownloadFolder).isSymbolicLink());
    assert.equal(fs.readdirSync(defaultDownloadFolder).length,0,'New default folder is not empty; preserving its contents');
    fs.rmdirSync(defaultDownloadFolder); // Never recursively remove a user Downloads directory.
  }
};
let installed=null;
try {
  for(const [index,lang] of languages.entries()) {
    const workspace=path.join(output,lang),program=path.join(workspace,'Program files'),profile=path.join(workspace,'profile');
    fs.mkdirSync(workspace,{recursive:true});
    const env={...process.env,LOCALAPPDATA:profile,USERPROFILE:profile};delete env.IDM_EX_HOME;delete env.DOWNLOAD_ENGINE_HOME;
    const setup=(selected,label)=>run(installer,['/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/NOICONS','/TASKS=','/NOCLOSEAPPLICATIONS',`/LANG=${selected.replace('-','_')}`,`/DIR=${program}`,`/LOG=${path.join(output,label+'.log')}`]);
    const result=setup(lang,lang+'-install');assert.equal(result.status,0,result.stderr);installed={program,env};
    assert.equal(JSON.parse(fs.readFileSync(path.join(program,'installer-language.json'),'utf8')),lang);
    for(const bin of ['de.exe','de-host.exe','de-cli.exe']) assert.equal(digest(path.join(program,bin)),digest(path.join(root,'target/release',bin)));
    for(const browser of ['chrome','edge','brave','firefox']) {
      for(const locale of languages.map(language=>language.replace('-','_')))assert(fs.existsSync(path.join(program,'extensions',browser,'_locales',locale,'messages.json')));
      // Check the installed bytes, including files imported indirectly by the background/popup scripts.
      for(const file of fs.readdirSync(path.join(root,'extension')).filter(name=>/\.(js|css|html)$/.test(name)))
        assert.equal(digest(path.join(program,'extensions',browser,file)),digest(path.join(root,'extension',file)),`Missing or stale ${browser} runtime file: ${file}`);
    }
    for(const guide of ['mega.md','wetransfer.md']) assert.equal(digest(path.join(program,'docs',guide)),digest(path.join(root,'docs',guide)));
    const cli=path.join(program,'de-cli.exe');
    const queues=run(cli,['queues'],env,15000);assert.equal(queues.status,0,queues.stderr);
    const reply=JSON.parse(queues.stdout);assert.equal(reply.ok,true);assert(Array.isArray(reply.result));assert(reply.result.some(queue=>queue.id==='main'));
    const database=path.join(profile,'Download Engine','queue.sqlite3');
    let db=openProfileDatabase(database);
    const preferences=()=>JSON.parse(db.prepare('SELECT data FROM interface_preferences WHERE id=1').get().data);
    assert.equal(preferences().language,lang,'Fresh profile follows Setup');
    const saved={...preferences(),language:'system',compact_rows:true,localization_sentinel:'preserve'};
    db.prepare('UPDATE interface_preferences SET data=?').run(JSON.stringify(saved));
    db.close();
    const testDownloads=path.join(profile,'test-downloads');fs.mkdirSync(testDownloads);
    const downloaded=path.join(testDownloads,'keep.bin');fs.writeFileSync(downloaded,'Harmless installer persistence fixture.');
    const userFile=path.join(program,'keep-user-file.md');fs.writeFileSync(userFile,'Preserve this untracked file.');
    const next=languages[(index+1)%languages.length];
    assert.equal(setup(next,lang+'-upgrade').status,0);
    assert.equal(JSON.parse(fs.readFileSync(path.join(program,'installer-language.json'),'utf8')),next);
    assert.equal(run(cli,['queues'],env,15000).status,0);
    db=openProfileDatabase(database);assert.deepEqual(preferences(),saved);db.close();
    // First install: its own entry is removed. Second: another copy's entry is kept.
    const startupCheck=index<2;
    if(startupCheck) plantStartup(`"${index===0?path.join(program,'de.exe'):path.join(workspace,'Other copy','de.exe')}" --autostart`);
    const uninstall=run(path.join(program,'unins000.exe'),['/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',`/LOG=${path.join(output,lang+'-uninstall.log')}`],env);
    assert.equal(uninstall.status,0,uninstall.stderr);installed=null;
    assert(!fs.existsSync(path.join(program,'de.exe')));
    assert(fs.existsSync(database));assert(fs.existsSync(downloaded));assert(fs.existsSync(userFile));
    assert.deepEqual(snapshot(),before,'Native browser registrations must remain unchanged');
    if(startupCheck) {
      const kept=startupKeys.map(key=>startupValue(key)!==null);
      assert.deepEqual(kept,[index===1,index===1],'Uninstall must remove only its own startup entry');
      if(index===1) assert.equal(spawnSync('reg.exe',['delete',startupKey,'/f'],{windowsHide:true,timeout:10000}).status,0);
    }
    assert.deepEqual(realStartup(),startupBefore,'The real Run key must remain unchanged');
    checks.push({language:lang,installed:true,upgradePreservedPreferences:true,uninstallPreservedData:true,extensionCatalogs:languages.length*4,
      ...(startupCheck?{startupEntry:index===0?'own entry removed':'other copy entry kept'}:{})});
    // Only remove our verified isolated fixture, after recording preservation checks.
    removeIsolated(workspace);
    cleanupDefaultFolder();
  }
  assert([32,64].every(view=>reg(identity,view)===null),'Test uninstall registration remains');
  assert.equal(reg(startupKey,64),null,'Startup cleanup fixture remains');
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({version,checks,browserRegistrationUnchanged:true,realStartupUnchanged:true,isolatedFixturesCleaned:true},null,2));
  console.log(JSON.stringify({passed:checks.length,output}));
}finally{
  if(installed) {
    const cleanup=run(path.join(installed.program,'unins000.exe'),['/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART'],installed.env);
    if(cleanup.status!==0) console.error('Test uninstall needs attention:',installed.program);
  }
  if(reg(startupKey,64)!==null) spawnSync('reg.exe',['delete',startupKey,'/f'],{windowsHide:true,timeout:10000});
  cleanupDefaultFolder();
  assert.deepEqual(snapshot(),before,'Native browser registrations changed');
  assert.deepEqual(realStartup(),startupBefore,'The real Run key changed');
}

const openProfileDatabase = require('./profile_database.cjs');
// Capture the real egui renderer against isolated, paused queues. No network or packages.
// Build first: cargo build --offline --locked --features screenshot --bin de
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { randomUUID } = require('node:crypto');
const { spawnSync } = require('node:child_process');
const { deflateSync } = require('node:zlib');
const root = path.resolve(__dirname, '..');
const output = path.join(root, 'artifacts', 'download-engine-native-ui', `capture-${randomUUID().slice(0, 8)}`);
fs.mkdirSync(output, { recursive: true });

function chunk(type, bytes) {
  const data = Buffer.concat([Buffer.from(type), bytes]);
  let crc = 0xffffffff;
  for (const byte of data) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  const length = Buffer.alloc(4), checksum = Buffer.alloc(4);
  length.writeUInt32BE(bytes.length); checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([length, data, checksum]);
}
function png(ppm) {
  const data = fs.readFileSync(ppm);
  const match = /^P6\n(\d+) (\d+)\n255\n/.exec(data.subarray(0, 64).toString());
  assert(match, 'Expected the native PPM capture');
  const width = +match[1], height = +match[2], offset = match[0].length;
  assert.equal(data.length - offset, width * height * 3);
  const scanlines = Buffer.alloc(height * (width * 3 + 1));
  for (let y = 0; y < height; y++) data.copy(scanlines, y * (width * 3 + 1) + 1, offset + y * width * 3, offset + (y + 1) * width * 3);
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 2;
  const destination = ppm.replace(/\.ppm$/, '.png');
  fs.writeFileSync(destination, Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk('IHDR',header), chunk('IDAT',deflateSync(scanlines)), chunk('IEND',Buffer.alloc(0))]));
  return destination;
}
function capture(name, dark, size, view = 'downloads', empty = false, legacy = false, compact = false, toolbar = null) {
  const home = path.join(output, name), downloads = path.join(home, 'downloads');
  fs.mkdirSync(path.join(home, 'data'), { recursive: true });
  fs.mkdirSync(downloads);
  const db = openProfileDatabase(path.join(home, 'data', 'queue.sqlite3'));
  db.exec(`CREATE TABLE jobs(id TEXT PRIMARY KEY,request_id TEXT NOT NULL UNIQUE,created INTEGER NOT NULL,data TEXT NOT NULL);
    CREATE TABLE download_queues(id TEXT PRIMARY KEY,name TEXT NOT NULL UNIQUE COLLATE NOCASE,max_active INTEGER NOT NULL CHECK(max_active BETWEEN 1 AND 8),running INTEGER NOT NULL,scheduled_at INTEGER);
    INSERT INTO download_queues VALUES('main','Main queue',3,0,NULL);
    CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY CHECK(id=1),close_to_tray INTEGER NOT NULL,notify_complete INTEGER NOT NULL,notify_failed INTEGER NOT NULL);
    INSERT INTO desktop_preferences VALUES(1,0,0,0);
    CREATE TABLE interface_preferences(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL);`);
  if (['queues','scheduler'].includes(view)) db.exec("INSERT INTO download_queues VALUES('scheduled','Scheduled downloads',2,0,NULL)");
  const preferences = { language: toolbar?.language || 'en', show_details: true, compact_rows: compact, sidebar_labels: true };
  // Omit appearance for the default-theme capture to exercise real startup defaults.
  if (dark !== null) preferences.appearance = dark ? 'dark' : 'light';
  if (toolbar?.dropTarget) preferences.drop_target = true;
  if (toolbar?.details === false) preferences.show_details = false;
  if (toolbar?.ownTheme) {
    // A synthetic white PNG checks contrast at maximum imported-image strength.
    // No personal image or profile is read by this capture.
    const header=Buffer.alloc(13);header.writeUInt32BE(1);header.writeUInt32BE(1,4);header[8]=8;header[9]=2;
    const bytes=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(Buffer.from([0,255,255,255]))),chunk('IEND',Buffer.alloc(0))]);
    const digest=require('node:crypto').createHash('sha256').update(bytes).digest();
    preferences.appearance='custom';
    preferences.custom_theme={preset:'own_image',strength:100,accent:[195,165,255],image:[...digest]};
    if(toolbar.ownTheme==='white') {
      fs.mkdirSync(path.join(home,'data','themes'),{recursive:true});
      fs.writeFileSync(path.join(home,'data','themes',digest.toString('hex')+'.png'),bytes);
    }
  }
  if (legacy) {
    preferences.columns = [true,true,true,true,true,true,true];
    preferences.table_layout = {order:['number','name','status','progress','size','speed','eta','added','actions'],widths:[null,360,null,null,null,null,null,null,null]};
  }
  db.prepare('INSERT INTO interface_preferences VALUES(1,?)').run(JSON.stringify(preferences));
  if(['connections','offline-website','offline-website-report'].includes(view)) {
    db.exec('CREATE TABLE connection_settings(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)');
    db.prepare('INSERT INTO connection_settings VALUES(1,?)').run(JSON.stringify({default_connections:4,sites:[{host:'downloads.example.com',subdomains:true,connections:1},{host:'cdn.example.com',subdomains:false,connections:8}]}));
  }
  if(view.startsWith('mcp')) {
    db.exec('CREATE TABLE mcp_settings(id INTEGER PRIMARY KEY CHECK(id=1),port INTEGER NOT NULL,read_only INTEGER NOT NULL,start_with_app INTEGER NOT NULL)');
    db.prepare('INSERT INTO mcp_settings VALUES(1,?,?,?)').run(toolbar?.mcpPort || 17842,toolbar?.mcpReadOnly ?? (view==='mcp-running'?0:1),view==='mcp'?0:1);
    if(view==='mcp-error') fs.writeFileSync(path.join(home,'data','mcp-token.dpapi'),'corrupt fixture');
  }
  const rows = empty ? [] : [
    ['Blender-4.5-windows-x64.msi','paused',412,257], ['Nature-documentary-4K.mp4','queued',2100,0],
    ['Project-assets.zip','paused',842,379], ['Ubuntu-desktop-amd64.iso','queued',4800,0],
    ['Product-reference.pdf','complete',12.4,12.4], ['Ambient-session.flac','complete',84.6,84.6],
    ['Textures-collection.zip','failed',356,52], ['Desktop-wallpaper.png','complete',8.2,8.2],
  ];
  if (name.startsWith('stop-') && rows.length) rows[0][1] = 'queued';
  for (const [index,[name,status,size,done]] of rows.entries()) {
    const id=randomUUID(), url=`https://example.invalid/${name}`, total=Math.round(size*1048576);
    const saved = status === 'complete' ? path.join(downloads,name) : null;
    if(saved) fs.writeFileSync(saved,'Offline native UI fixture.\n');
    const job = { id,request_id:id,name,url,directory:downloads,status,total,downloaded:Math.round(done*1048576),
      plan:status==='paused'?{size:total,etag:'"offline-fixture"',final_url:url,segments:[{start:0,end:total-1}]}:null,
      saved_path:saved,error:status==='failed'?'Connection lost. Resume to try again.':null,created:Math.floor(Date.now()/1000)+index };
    db.prepare('INSERT INTO jobs VALUES(?,?,?,?)').run(id,id,job.created,JSON.stringify(job));
  }
  if (view.startsWith('automation') && view !== 'automation-empty') {
    db.exec(`CREATE TABLE automation_rules(id TEXT PRIMARY KEY,data TEXT NOT NULL,generation INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE automation_runs(id INTEGER PRIMARY KEY AUTOINCREMENT,at INTEGER NOT NULL,rule_name TEXT NOT NULL,success INTEGER NOT NULL,message TEXT NOT NULL);
      INSERT INTO download_queues VALUES('work','Work downloads',2,0,NULL);`);
    for (const rule of [
      {id:randomUUID(),name:'Work archives',enabled:true,workflow:{when:'download_added',host:'example.com',extension:'zip',queue:'work',bytes_per_second:1048576}},
      {id:randomUUID(),name:'Continue with work downloads',enabled:false,workflow:{when:'queue_completed',source:'main',start:'work'}},
    ]) db.prepare('INSERT INTO automation_rules(id,data) VALUES(?,?)').run(rule.id,JSON.stringify(rule));
    for (const [name,success,message] of [['Work archives',1,'Routed download to Work downloads and applied its speed limit.'],['Continue with work downloads',0,'Destination queue was removed. Edit this rule.']]) {
      db.prepare('INSERT INTO automation_runs(at,rule_name,success,message) VALUES(?,?,?,?)').run(Math.floor(Date.now()/1000),name,success,message);
    }
    if(name.includes('history-long')) db.prepare('UPDATE automation_runs SET rule_name=?').run('A long automation rule name '.repeat(4).trim());
  }
  if(view.startsWith('automation-scripts') && view !== 'automation-scripts-empty') {
    db.exec(`CREATE TABLE automation_scripts(id TEXT PRIMARY KEY,data TEXT NOT NULL,cursor INTEGER NOT NULL,next_at INTEGER);
      CREATE TABLE script_runs(id TEXT PRIMARY KEY,script_id TEXT NOT NULL,data TEXT NOT NULL,body TEXT NOT NULL,timeout INTEGER NOT NULL,state TEXT NOT NULL,stop_requested INTEGER NOT NULL DEFAULT 0,log_bytes INTEGER NOT NULL DEFAULT 0,truncated INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE script_logs(seq INTEGER PRIMARY KEY AUTOINCREMENT,run_id TEXT NOT NULL REFERENCES script_runs(id) ON DELETE CASCADE,at INTEGER NOT NULL,level TEXT NOT NULL,text TEXT NOT NULL);`);
    const script={id:randomUUID(),revision:1,name:'Report completed downloads',language:'lua',body:'-- Write your own Lua with the xd app API.\nfor _, file in ipairs(event.downloads) do\n    print("Finished:", file.name or file.id)\nend\nlocal queues = xd.queues()\nprint("Queues:", #queues)\n',enabled:false,trigger:{kind:'download_completed',queue:null},timeout_seconds:300};
    if(name.includes('powershell')) { script.language='power_shell'; script.body='foreach ($file in $Event.downloads) {\n    Write-Output ("Finished: {0}" -f $file.name)\n}\n$queues = Invoke-DownloadEngine queues\nWrite-Output ("Queues: {0}" -f $queues.Count)\n'; }
    if(name.includes('scripts-live-error')) script.body = script.language === 'lua'
      ? 'local files = xd.downloads()\nfor _, file in ipairs(files) do\n    print(file.name)\nend)\n'
      : 'Write-Output "Hello"\nif ($true {\n    Write-Output "Missing closing parenthesis"\n}\n';
    if(name.includes('scripts-completion')) script.body = 'local queues = xd.que';
    if(name.includes('scripts-core-reader') || name.includes('workspace-full-reader')) { script.name='Custom site reader'; script.trigger={kind:'site_reader',pages:['example.com/*']}; script.body=fs.readFileSync(path.join(root,'examples/custom-site-reader.lua'),'utf8'); }
    if(name.includes('scripts-manual')) { script.name='New script'; script.trigger={kind:'manual'}; }
    if(name.includes('scripts-startup')) script.trigger={kind:'startup'};
    if(name.includes('scripts-interval')) script.trigger={kind:'interval',minutes:17};
    if(name.includes('scripts-queue')) script.trigger={kind:'queue_completed',queue:'work'};
    if(name.includes('scripts-missing')) script.trigger={kind:'download_completed',queue:'removed-queue'};
    if(name.includes('scripts-long')) {
      script.name='Long script '+'W'.repeat(48);
      script.trigger={kind:'download_completed',queue:'work'};
      db.prepare('UPDATE download_queues SET name=? WHERE id=?').run('Long queue '+'W'.repeat(48),'work');
    }
    const failure=name.includes('scripts-failed');
    if(failure) {
      script.name='Retry failed downloads';
      script.body='for _, file in ipairs(event.downloads) do\n    print(("Failed: %s (%s)"):format(file.name, file.error or ""))\n    xd.retry(file.id)\n    xd.notify("Download retried", file.name)\nend\n';
      script.trigger={kind:'download_failed',queue:null};
      script.change_files=true;
      db.exec('CREATE TABLE script_state(script_id TEXT NOT NULL,key TEXT NOT NULL,value TEXT NOT NULL,updated INTEGER NOT NULL,PRIMARY KEY(script_id,key))');
      for(const [key,value] of [['retried:3f2a9c',2],['last_report','2026-10-22'],['seen_hosts',['cdn.example.com','files.example.org']]]) {
        db.prepare('INSERT INTO script_state VALUES(?,?,?,?)').run(script.id,key,JSON.stringify(value),Math.floor(Date.now()/1000));
      }
    }
    db.prepare('INSERT INTO automation_scripts VALUES(?,?,0,NULL)').run(script.id,JSON.stringify(script));
    if(process.argv.includes('--script-workspace')) {
      for(const [name,language,trigger] of [
        ['Custom site reader','lua',{kind:'site_reader',pages:['example.com/*']}],
        ['Evening report','power_shell',{kind:'manual'}],
        ['Retry failed downloads','lua',{kind:'download_failed',queue:null}],
      ]) {
        const item={...script,id:randomUUID(),name,language,trigger,enabled:false};
        db.prepare('INSERT INTO automation_scripts VALUES(?,?,0,NULL)').run(item.id,JSON.stringify(item));
      }
    }
    const run={id:randomUUID(),script_id:script.id,name:script.name,revision:1,language:script.language,trigger:failure?'download_failed':'manual',context:failure?{trigger:'download_failed',downloads:[{id:randomUUID(),name:'Textures-collection.zip'}],sample:true}:{trigger:'manual',downloads:[]},queued_at:Math.floor(Date.now()/1000),started_at:Math.floor(Date.now()/1000),finished_at:Math.floor(Date.now()/1000),state:name.includes('scripts-error')?'failed':'succeeded',error:name.includes('scripts-error')?'Script exited with 1':null,exit_code:name.includes('scripts-error')?1:0};
    db.prepare('INSERT INTO script_runs(id,script_id,data,body,timeout,state) VALUES(?,?,?,?,?,?)').run(run.id,script.id,JSON.stringify(run),script.body,300,run.state);
    const lines=failure?[['info','Failed: Textures-collection.zip (Connection lost. Resume to try again.)\nWould retry Textures-collection.zip (sample run, nothing changed)\nWould show the notification Download retried · Textures-collection.zip (sample run, nothing changed)\n']]:[['info','Finished: Sample video.mp4\nQueues: 2\n'],['warn','WARNING: One file was moved after completion.\n'],...(run.error?[['error','Cannot find the requested file. Check its saved path.\n']]:[])];
    for(const [level,text] of lines) {
      db.prepare('INSERT INTO script_logs(run_id,at,level,text) VALUES(?,?,?,?)').run(run.id,run.queued_at,level,text);
    }
  }
  db.close();
  if (['diagnostics','logs-empty','logs-error'].includes(view)) {
    const logDir = path.join(home, 'data', 'logs');
    fs.mkdirSync(logDir, { recursive: true });
    const session = randomUUID();
    const events = [
      ['desktop','info','started',{}], ['desktop','info','ready',{}],
      ['native_host','info','native_request_received',{action:'review_video',request:1}],
      ['desktop','info','video_lookup_started',{}], ['desktop','info','video_lookup_succeeded',{elapsed_ms:1240}],
      ['native_host','info','native_request_succeeded',{action:'review_video',request:1}],
      ['desktop','info','download_started',{job:randomUUID()}],
      ['desktop','warn','download_retry_scheduled',{attempt:1}],
      ['desktop','error','download_failed',{failure:{kind:'network',http_status:403}}],
      ['desktop','info','download_completed',{bytes:63753421,elapsed_ms:8230}],
    ];
    for(const [index,[component,level,event,details]] of events.entries()) {
      const record={time:new Date(Date.now()-(events.length-index)*60000).toISOString(),version:'0.1.0',component,pid:1234,session,level,event,details};
      fs.appendFileSync(path.join(logDir, component === 'desktop' ? 'desktop.log' : 'native-host.log'),JSON.stringify(record)+'\n');
    }
    if(view === 'logs-error') fs.appendFileSync(path.join(logDir,'native-host.log'),'x'.repeat(4097));
    if(name.startsWith('logs-notices-')) {
      const record={time:new Date(Date.now()-3600000).toISOString(),version:'0.1.0',component:'native_host',pid:1234,session,level:'info',event:'native_request_received',details:{}};
      fs.appendFileSync(path.join(logDir,'native-host.log'),(JSON.stringify(record)+'\n').repeat(2050)+'interrupted entry\n');
    }
  }
  const ppm = path.join(home,'native-ui.ppm'), env={...process.env};
  for(const key of Object.keys(env)) if(key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
  Object.assign(env,{DOWNLOAD_ENGINE_HOME:home,IDM_EX_HOME:home,IDM_EX_SCREENSHOT_TO:ppm,IDM_EX_TEST_SIZE:size,IDM_EX_TEST_VIEW:view});
  if(view.startsWith('status-') || view.startsWith('scan-batch') || view === 'scan-single') env.IDM_EX_TEST_FEATURE = view;
  if(toolbar?.label) Object.assign(env,{IDM_EX_TEST_TOOLBAR:toolbar.label,IDM_EX_TEST_TOOLBAR_STATE:toolbar.state});
  if(toolbar?.menu) env.IDM_EX_TEST_MENU=toolbar.menu;
  if(toolbar?.mcpBottom) env.IDM_EX_TEST_MCP_BOTTOM='1';
  if(toolbar?.startup) env.IDM_EX_TEST_STARTUP=toolbar.startup;
  if(toolbar?.customTheme) env.IDM_EX_TEST_CUSTOM_THEME=toolbar.customTheme;
  if(view === 'preferences') env.IDM_EX_TEST_PREFERENCES = '1';
  if(['connections','offline-website','offline-website-report'].includes(view)) env.IDM_EX_TEST_FEATURE=view;
  if(view.startsWith('scan-')) env.IDM_EX_TEST_FEATURE=view;
  if(view === 'clipboard-offer') env.IDM_EX_TEST_FEATURE=view;
  if(['list-export','list-import','list-import-details'].includes(view)) env.IDM_EX_TEST_FEATURE=view;
  if(view.startsWith('completion-')) env.IDM_EX_TEST_FEATURE=view;
  if(toolbar?.feature) env.IDM_EX_TEST_FEATURE=toolbar.feature;
  if(process.argv.includes('--script-editor')) env.IDM_EX_TEST_EDITOR=name.includes('scripts-completion')?'completion':'editor';
  if(process.argv.includes('--core-api')) env.IDM_EX_TEST_CORE_API='1';
  if(name.includes('workspace-full')) env.IDM_EX_TEST_FULL_EDITOR='1';
  if(name.includes('workspace-full-logs')) env.IDM_EX_TEST_FULL_LOGS='1';
  if(name.includes('workspace-list')) env.IDM_EX_TEST_SCRIPT_LIBRARY='1';
  if(['add','add-error','filtered-empty','add-advanced','add-login','add-negotiate','add-token','add-login-error','add-login-saving','batch','batch-error','batch-pattern','batch-pattern-error','folders','speed-limits','columns','remove-confirmation','delete-file-confirmation','delete-files-confirmation','multi-selection','browser-videos','browser-videos-empty','youtube-qualities','youtube-preview','youtube-error','proxy','proxy-enterprise','quotas','ftps-login','scan-settings','scan-result','checksum','zip-preview','hls-audio','grabber-setup','grabber-project','grabber-schedule','grabber-discoveries','rename-queue','schedule-queue','update-link'].includes(view)) env.IDM_EX_TEST_FEATURE = view;
  const args = view === 'browser-setup' ? ['--browser-setup'] : [];
  let result;
  const coldReview = view === 'browser-videos-cold-start';
  if(coldReview) {
    const request = {action:'review_browser_video',request_id:'video:'+randomUUID(),review:{
      page_url:'https://example.invalid/watch',page_title:'Native handoff fixture',truncated:false,
      items:[{url:'https://example.invalid/clip.mp4',filename:'Clip.mp4',label:'Direct video',kind:'file',size:1048576},
        {url:'https://example.invalid/master.m3u8',filename:'Stream',label:'Choose a video quality',kind:'hls',size:null}]}};
    const data=Buffer.from(JSON.stringify(request)),header=Buffer.alloc(4);header.writeUInt32LE(data.length);
    result=spawnSync(path.join(root,'target','debug','de-host.exe'),[],{env,windowsHide:true,input:Buffer.concat([header,data]),timeout:30000});
    assert.equal(result.error,undefined,String(result.error));assert.equal(result.status,0,String(result.stderr));
    const reply=result.stdout;assert(reply.length>=4);assert.equal(reply.readUInt32LE(0),reply.length-4);
    const receipt=JSON.parse(reply.subarray(4));assert(receipt.ok,receipt.message);assert.equal(receipt.preview_id,request.request_id);
    assert(!receipt.job_id && !receipt.job_ids?.length);fs.writeFileSync(path.join(home,'native-reply.json'),JSON.stringify(receipt,null,2));
    const desktopLog=path.join(home,'data','logs','desktop.log'),deadline=Date.now()+30000;
    while(!fs.existsSync(ppm) || !fs.existsSync(desktopLog) || !fs.readFileSync(desktopLog,'utf8').includes('"event":"stopped"')) {
      assert(Date.now()<deadline,'Cold desktop did not capture and exit');Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,80);
    }
  } else result=spawnSync(path.join(root,'target','debug','de.exe'),args,{env,windowsHide:true,encoding:'utf8',timeout:30000});
  assert.equal(result.error,undefined,String(result.error)); assert.equal(result.status,0,result.stderr); assert(fs.existsSync(ppm),result.stderr);
  const check=openProfileDatabase(path.join(home,'data','queue.sqlite3'));
  assert.equal(check.prepare('SELECT COUNT(*) n FROM jobs').get().n,rows.length);
  if(coldReview) assert.equal(check.prepare('SELECT COUNT(*) n FROM browser_video_reviews WHERE data IS NOT NULL').get().n,1);
  assert.equal(check.prepare('SELECT COUNT(*) n FROM download_queues WHERE running<>0').get().n,0);
  const actualPreferences=JSON.parse(check.prepare('SELECT data FROM interface_preferences WHERE id=1').get().data);
  for(const [key,value] of Object.entries(preferences)) assert.deepEqual(actualPreferences[key],value);
  check.close();
  if(view.startsWith('delete-file')) for(const [name,status] of rows) {
    if(status === 'complete') assert.equal(fs.readFileSync(path.join(downloads,name),'utf8'),'Offline native UI fixture.\n');
  }
  const screenshot=png(ppm);
  const diagnostics = fs.readFileSync(path.join(home,'data','logs','desktop.log'),'utf8').trim().split(/\r?\n/).map(line => JSON.parse(line));
  for (const event of ['started','ready','stopped']) assert(diagnostics.some(record => record.event === event), `Missing ${event} log`);
  console.log(JSON.stringify({name,screenshot}));
  return {name,screenshot,size,view,dark,legacy,compact,toolbar,queuedNetworkWork:false};
}
if(process.argv.includes('--release-docs')) {
  const results=[capture('public-moonveil',true,'1280x860','downloads',false,false,false,{details:false,customTheme:'moonveil'})];
  results.push(capture('guide-lua-860x600',true,'860x600','guide-lua'));
  results.push(capture('guide-lua-1280x860',true,'1280x860','guide-lua'));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Moonveil public documentation image without local paths, and the updated offline Lua guide at two window sizes. Isolated paused profiles; no live downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--readme')) {
  // Public README/website images hide the details pane, so no local profile path appears.
  const results=[capture('readme-downloads-dark',true,'1280x800','downloads',false,false,false,{details:false})];
  results.push(capture('readme-custom-theme',true,'1280x800','downloads',false,false,false,{details:false,customTheme:'moonveil'}));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'README images: standard dark and Moonveil custom theme, 1280x800, details pane hidden. Isolated paused profiles; no live downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--custom-themes')) {
  const results=[];
  for(const preset of ['neon','reactor','aurora','moonveil']) results.push(capture(`theme-${preset}`,true,'1280x860','downloads',false,false,false,{customTheme:preset}));
  for(const size of ['860x600','1280x860']) results.push(capture(`theme-gallery-${size}`,true,size,'preferences',false,false,false,{customTheme:'moonveil'}));
  for(const language of ['de','ar']) results.push(capture(`theme-gallery-${language}`,true,'860x600','preferences',false,false,false,{customTheme:'aurora',language}));
  results.push(capture('theme-add-dialog',true,'860x600','add',false,false,false,{customTheme:'reactor'}));
  results.push(capture('theme-automation',true,'1280x860','automation-scripts',false,false,false,{customTheme:'neon'}));
  for(const ownTheme of ['white','missing']) results.push(capture(`theme-own-${ownTheme}`,true,'860x600','downloads',false,false,false,{ownTheme}));
  results.push(capture('theme-gallery-own',true,'860x600','preferences',false,false,false,{ownTheme:'white'}));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Custom theme gallery (presets and an imported image), four presets across the whole window and opaque editor/dialog surfaces. Isolated paused profiles; no live downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--updates')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) {
    for(const feature of ['updates-available','updates-current','updates-error']) {
      results.push(capture(feature+'-'+(dark?'dark':'light')+'-'+size,dark,size,'downloads',false,false,false,{feature}));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Update dialog: available, current and error states in both themes and sizes. Offline fixtures, no GitHub requests.'},null,2));
  console.log('Verification: '+path.join(output,'verification.json'));process.exit(0);
}
if(process.argv.includes('--link-fallback')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) {
    for(const feature of ['link-fallback','link-fallback-empty','link-fallback-long']) {
      results.push(capture(`${feature}-${dark?'dark':'light'}-${size}`,dark,size,'downloads',false,false,false,{feature}));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Automatic link fallback review: populated, empty and long/truncated lists, both themes and window sizes. Isolated paused stores; no network work.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--localization') || process.argv.includes('--localization-layout')) {
  const results=[];
  const views=process.argv.includes('--localization-layout') ? ['settings','diagnostics','mcp'] : ['settings','add-advanced','new-queue','queues','guide','diagnostics','automation-scripts','mcp','scheduler','help'];
  const languages=JSON.parse(fs.readFileSync(path.join(root,'locales/languages.json'),'utf8')).map(language=>language.code);
  for(const language of languages) {
    for(const view of views) {
      results.push(capture('locale-'+language+'-'+view,true,'860x600',view,false,false,false,{language}));
    }
    results.push(capture('locale-'+language+'-wide-light',false,'1280x860','settings',false,false,false,{language}));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,languages,scope:'Real native renderer, minimum and wide windows, dark and light. Isolated paused stores; no live network.'},null,2));
  console.log('Verification: '+path.join(output,'verification.json'));process.exit(0);
}
const diagnosticsOnly = process.argv.includes('--diagnostics');
if(process.argv.includes('--batch-malwarebytes')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['860x600','1280x860']) results.push(capture(`batch-malwarebytes-${theme}-${size}`,dark,size,'scan-batch-review'));
    results.push(capture(`batch-malwarebytes-last-${theme}`,dark,'860x600','scan-batch-review-last'));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Malwarebytes batch review pause and last-file controls, both themes and minimum/wide windows. Illustrative isolated profiles; no installed antivirus runs.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--batch-antivirus')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const view of ['scan-single','scan-batch','scan-batch-results']) {
      results.push(capture(`batch-antivirus-${theme}-${view}`,dark,'860x600',view));
    }
    results.push(capture(`batch-antivirus-empty-${theme}`,dark,'860x600','scan-batch',true));
    results.push(capture(`batch-antivirus-tools-${theme}`,dark,'860x600','downloads',false,false,false,{menu:'Tools|Antivirus Scan'}));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native shared dialog, illustrative result rows, empty state and Tools menu in both themes at minimum size. Isolated fixture profiles; no installed antivirus runs.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--grabber-toolbar')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['860x600','1280x860']) results.push(capture(`grabber-toolbar-${theme}-${size}`,dark,size));
    for(const [label,state] of [['Grabber','hover'],['Logs','focus']]) {
      results.push(capture(`grabber-toolbar-${theme}-${label.toLowerCase()}-${state}`,dark,'860x600','downloads',false,false,false,{label,state}));
    }
    for(const view of ['grabber-setup','diagnostics']) results.push(capture(`grabber-toolbar-${theme}-${view}`,dark,'860x600',view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Grabber beside Automation; Logs beside Settings. Native dark/light, minimum/wide toolbar, hover, keyboard focus and destination views. Isolated paused profiles; no network scans or downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--stop-controls')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['860x600','1280x860']) results.push(capture(`stop-${theme}-${size}`,dark,size));
    for(const label of ['Stop','Stop all']) results.push(capture(`stop-${theme}-${label.replaceAll(' ','-')}-hover`,dark,'860x600','downloads',false,false,false,{label,state:'hover'}));
    for(const menu of ['Tasks|Stop all','Download context|Stop','Download actions|Stop']) results.push(capture(`stop-${theme}-${menu.split('|')[0].replaceAll(' ','-')}`,dark,'860x600','downloads',false,false,false,{menu}));
    results.push(capture(`stop-empty-${theme}`,dark,'860x600','downloads',true));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Stop and Stop all toolbar, hover, Tasks, context and three-dot menus, disabled selection; both themes, minimum/wide windows. Isolated paused profiles; no live downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--download-status')) {
  const results = [];
  const views=process.argv.includes('--native-only')?['status-native','status-native-media','status-native-complete']:['status-downloading','status-paused','status-complete','status-error','status-unknown','status-details','status-long','status-media','status-media-unknown','status-native','status-native-media','status-native-complete'];
  for(const dark of [true,false]) for(const view of views) {
    results.push(capture(`${view}-${dark?'dark':'light'}`,dark,'860x600',view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Isolated fixture states rendered by the native GPU renderer, including a separate native status viewport. No live downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
const videosOnly = process.argv.includes('--browser-videos');
if(process.argv.includes('--enterprise')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) for(const view of ['proxy-enterprise','add-negotiate']) {
    results.push(capture(`${view}-${dark?'dark':'light'}-${size}`,dark,size,view));
  }
  for(const dark of [true,false]) results.push(capture(`combined-mcp-${dark?'dark':'light'}`,dark,'860x600','mcp'));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Enterprise proxy and file login UI, isolated profiles and synthetic masked credentials. No downloads or external servers.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--download-controls')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) for(const view of ['connections','update-link']) {
    results.push(capture(`${view}-${dark?'dark':'light'}-${size}`,dark,size,view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Connection settings and browser recovery dialog rendered in native egui. No browser opened, no downloads started, no shutdown action armed or executed.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--mcp-access')) {
  const results=[];
  for(const dark of [true,false]) {
    for(const size of ['860x600','1280x860']) results.push(capture(`mcp-access-${dark?'dark':'light'}-${size}`,dark,size,'mcp',false,false,false,{mcpBottom:true,mcpReadOnly:0}));
    results.push(capture(`mcp-access-ar-${dark?'dark':'light'}`,dark,'860x600','mcp',false,false,false,{mcpBottom:true,mcpReadOnly:0,language:'ar'}));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Editable MCP capabilities, light/dark, minimum/wide windows and Arabic. Isolated paused profiles; server stays stopped.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  return;
}
if(process.argv.includes('--mcp')) {
  (async()=>{
    const net=require('node:net');
    const availablePort=()=>new Promise((resolve,reject)=>{const server=net.createServer();server.once('error',reject);server.listen(0,'127.0.0.1',()=>{const port=server.address().port;server.close(()=>resolve(port));});});
    const results=[];
    for(const dark of [true,false]) {
      for(const size of ['860x600','1280x860']) {
        for(const view of ['mcp','mcp-running']) results.push(capture(`${view}-${dark?'dark':'light'}-${size}`,dark,size,view,false,false,false,{mcpPort:await availablePort()}));
        results.push(capture(`guide-mcp-${dark?'dark':'light'}-${size}`,dark,size,'guide-mcp'));
      }
      results.push(capture(`mcp-error-${dark?'dark':'light'}`,dark,'860x600','mcp-error',false,false,false,{mcpPort:await availablePort()}));
      for(const view of ['mcp','mcp-running']) results.push(capture(`${view}-bottom-${dark?'dark':'light'}`,dark,'860x600',view,false,false,false,{mcpPort:await availablePort(),mcpBottom:true}));
      results.push(capture(`mcp-hover-${dark?'dark':'light'}`,dark,'860x600','downloads',false,false,false,{label:'MCP',state:'hover'}));
      results.push(capture(`mcp-preferences-${dark?'dark':'light'}`,dark,'860x600','preferences-mcp'));
    }
    fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'MCP popup and Preferences entry, real managed server startup and invalid-token errors; dark/light, minimum/wide windows, scrolling and toolbar hover. Isolated paused profiles; loopback only.'},null,2));
    console.log(`Verification: ${path.join(output,'verification.json')}`);
  })().catch(error=>{console.error(error);process.exitCode=1;});
  return;
} else if(process.argv.includes('--automation-controls')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const variant of ['manual','startup','interval','queue','missing','long']) {
      results.push(capture(`scripts-${variant}-${theme}`,dark,'860x600','automation-scripts'));
    }
    results.push(capture(`history-long-${theme}`,dark,'860x600','automation-history'));
    results.push(capture(`scripts-error-powershell-${theme}`,dark,'860x600','automation-scripts-logs'));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Automation settings alignment, all trigger types, removed queues, long names and failed PowerShell output in both themes. Isolated paused profiles; automatic scripts disabled.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--offline-website')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) for(const view of ['offline-website','offline-website-report']) {
    results.push(capture(`${view}-${dark?'dark':'light'}-${size}`,dark,size,view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Offline website setup and saved-copy report; native egui, dark/light, minimum/wide windows. Isolated profiles; no network capture.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--navigation')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['860x600','1280x860']) {
      for(const view of ['downloads','automation-scripts','diagnostics']) {
        results.push(capture(`navigation-${theme}-${size}-${view}`,dark,size,view));
      }
    }
    for(const [label,state] of [['Scripting','hover'],['Logs','focus']]) {
      results.push(capture(`navigation-${theme}-${label.toLowerCase()}-${state}`,dark,'860x600','downloads',false,false,false,{label,state}));
    }
    for(const view of ['guide','changelog']) results.push(capture(`navigation-${theme}-${view}`,dark,'860x600',view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Toolbar navigation on downloads, scripts, logs, Guide and Changelog; dark/light, minimum/wide windows, hover and keyboard focus. Isolated paused queues only.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--automation')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['860x600','1280x860']) {
      results.push(capture(`automation-rules-${theme}-${size}`,dark,size,'automation'));
      results.push(capture(`automation-scripts-${theme}-${size}`,dark,size,'automation-scripts'));
    }
    for(const view of ['automation-empty','automation-history','automation-edit','automation-error']) {
      results.push(capture(`${view}-${theme}`,dark,'860x600',view));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native Automation rules, WHEN->THEN flow pills, run history cards, rule editor dialog, and Scripts IDE in both themes and window sizes.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--script-editor')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) {
    for(const variant of ['valid','live-error','completion','live-error-powershell']) {
      results.push(capture(`scripts-${variant}-${dark?'dark':'light'}-${size}`,dark,size,'automation-scripts'));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Live Lua/PowerShell syntax diagnostics, completion popup, line numbers and highlighting; dark/light, minimum/wide windows. Isolated paused profiles; no script execution.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--core-api')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) for(const role of ['automation','reader']) {
    results.push(capture(`scripts-core-${role}-${dark?'dark':'light'}-${size}`,dark,size,'automation-scripts'));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Shared core API reference and search, automation/site-reader roles, light/dark and minimum/wide windows; no script execution.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--script-workspace')) {
  const results=[];
  for(const dark of [true,false]) for(const size of ['860x600','1280x860']) {
    for(const mode of ['editor','full','list','full-logs']) {
      results.push(capture(`scripts-workspace-${mode}-${dark?'dark':'light'}-${size}`,dark,size,'automation-scripts'));
    }
  }
  for(const dark of [true,false]) for(const kind of ['reader','powershell']) {
    results.push(capture(`scripts-workspace-full-${kind}-${dark?'dark':'light'}`,dark,'860x600','automation-scripts'));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Loaded script library and full editor, both themes/window sizes; isolated disabled scripts with no code execution.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--scripts')) {
  const results=[];
  for(const dark of [true,false]) {
    for(const size of ['860x600','1280x860']) results.push(capture(`scripts-${dark?'dark':'light'}-${size}`,dark,size,'automation-scripts'));
    results.push(capture(`scripts-powershell-${dark?'dark':'light'}`,dark,'1280x860','automation-scripts'));
    for(const view of ['automation-scripts-empty','automation-scripts-error','automation-scripts-logs']) results.push(capture(`${view}-${dark?'dark':'light'}`,dark,'860x600',view==='automation-scripts-error'?'automation-scripts-logs':view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native Scripts editor, trigger settings, empty/error states and coloured logs in both themes; minimum and wide windows; paused isolated profiles.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--changelog')) {
  const results=[];
  for(const dark of [true,false]) {
    for(const size of ['860x600','1280x860']) {
      for(const view of ['changelog','help']) results.push(capture(`${view}-${dark?'dark':'light'}-${size}`,dark,size,view));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native Changelog and About version in both themes and minimum/large window sizes. Isolated paused profiles.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--guide')) {
  const results=[];
  for(const dark of [true,false]) {
    for(const size of ['860x600','1280x860']) {
      results.push(capture(`guide-${dark?'dark':'light'}-${size}`,dark,size,'guide'));
      results.push(capture(`guide-cli-${dark?'dark':'light'}-${size}`,dark,size,'guide-cli'));
      results.push(capture(`guide-lua-${dark?'dark':'light'}-${size}`,dark,size,'guide-lua'));
      results.push(capture(`guide-mcp-${dark?'dark':'light'}-${size}`,dark,size,'guide-mcp'));
    }
    results.push(capture(`guide-automation-${dark?'dark':'light'}`,dark,'860x600','guide-automation'));
    results.push(capture(`guide-troubleshooting-${dark?'dark':'light'}`,dark,'860x600','guide-troubleshooting'));
    results.push(capture(`guide-editor-${dark?'dark':'light'}`,dark,'860x600','automation-scripts'));
    results.push(capture(`guide-editor-powershell-${dark?'dark':'light'}`,dark,'860x600','automation-scripts'));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Offline Guide topics, copyable Lua/CLI examples and language-specific editor help links, both themes and minimum/large window sizes. Isolated paused profiles.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--preferences-popup')) {
  const results=[];
  for(const dark of [true,false]) {
    for(const category of ['appearance','downloads','connection','browser','antivirus','startup']) {
      results.push(capture(`preferences-${category}-${dark?'dark':'light'}`,dark,'860x600',`preferences-${category}`,false,false,false,{language:'en'}));
    }
    results.push(capture(`preferences-wide-${dark?'dark':'light'}`,dark,'1280x860','preferences-appearance',false,false,false,{language:'en'}));
    results.push(capture(`preferences-mcp-${dark?'dark':'light'}`,dark,'860x600','preferences-mcp',false,false,false,{language:'en'}));
  }
  for(const language of ['zh-CN','ms','de','ar']) {
    results.push(capture(`preferences-${language}`,true,'860x600','preferences-appearance',false,false,false,{language}));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native Preferences popup, six categories, nested MCP, both themes, minimum and wide windows, representative languages. Isolated paused profiles; no Run key changes.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--startup')) {
  // Screenshot builds use an in-memory startup entry; the Run key is never read or written.
  const results=[];
  for(const state of ['off','on','other','blocked','missing']) {
    for(const dark of [true,false]) results.push(capture(`startup-en-${state}-${dark?'dark':'light'}`,dark,'860x600','settings-startup',false,false,false,{language:'en',startup:state}));
  }
  for(const language of ['de','ru']) {
    for(const state of ['on','off','other','blocked']) results.push(capture(`startup-${language}-${state}-dark`,true,'860x600','settings-startup',false,false,false,{language,startup:state}));
    results.push(capture(`startup-${language}-wide-light`,false,'1280x860','settings-startup',false,false,false,{language,startup:'on'}));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Settings STARTUP section in English, German and Russian with in-memory startup states (off, on, other copy, Task Manager, missing program). Isolated paused profiles; no Run key access.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--cards')) {
  const results=[];
  for(const dark of [true,false]) {
    for(const size of ['1280x860','860x600']) {
      for(const view of ['queues','settings','add-login']) results.push(capture(`${view}-${dark?'dark':'light'}-${size}`,dark,size,view));
    }
    for(const view of ['settings-bottom','add','add-token','add-login-error','add-login-saving']) results.push(capture(`${view}-${dark?'dark':'light'}`,dark,'860x600',view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native queue cards, settings groups and inline login, including error and saving states. Isolated paused fixture queues; no network or real credentials.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--queue-start')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const menu of ['Download context|Start immediately','Download actions|Start immediately']) {
      results.push(capture(`start-${theme}-${menu.startsWith('Download context')?'context':'actions'}`,dark,'860x600','downloads',false,false,false,{menu}));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Start immediately in native right-click and three-dot menus, both themes at minimum size. Isolated paused queues; no shutdown action or live downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--about-legal')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['860x600','1280x860']) {
      for(const view of ['help','legal','legal-notices']) results.push(capture(`about-${view}-${theme}-${size}`,dark,size,view));
    }
  }
  for(const language of ['de','zh-CN','ar']) results.push(capture(`about-legal-${language}`,true,'860x600','legal',false,false,false,{language}));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'About and offline legal documents, both themes and window sizes plus German, Chinese and Arabic controls. Isolated paused profiles; no live downloads.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--about-close')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['860x600','1280x860']) results.push(capture(`about-close-${theme}-${size}`,dark,size,'help'));
    for(const state of ['hover','pressed','focus']) {
      results.push(capture(`about-close-${theme}-${state}`,dark,'860x600','help',false,false,false,{label:'Close About',state}));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'About close icon, native dark/light at minimum and wide sizes, hover, press and keyboard focus. Isolated paused queues; no network.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);process.exit(0);
}
if(process.argv.includes('--branding')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    for(const size of ['1280x860','860x600']) {
      results.push(capture(`about-${theme}-${size}`,dark,size,'help'));
      results.push(capture(`search-${theme}-${size}`,dark,size,'filtered-empty'));
    }
    results.push(capture(`columns-${theme}`,dark,'860x600','columns'));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native About card, search controls and table-column footer in both themes and representative window sizes. Isolated paused queues; no live network.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--toolbar')) {
  const results=[];
  for(const dark of [true,false]) {
    const theme=dark?'dark':'light';
    results.push(capture(`toolbar-${theme}-idle`,dark,'1280x860'));
    for(const [label,state] of [['Add URL','hover'],['Open folder','hover'],['Settings','hover'],['Add URL','pressed'],['Add URL','focus'],['Pause','hover']]) {
      results.push(capture(`toolbar-${theme}-${label.toLowerCase().replaceAll(' ','-')}-${state}`,dark,'1280x860','downloads',false,false,false,{label,state}));
    }
  }
  results.push(capture('toolbar-minimum-hover',true,'860x600','downloads',false,false,false,{label:'Settings',state:'hover'}));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native toolbar idle, hover, pressed, keyboard focus and disabled states in both themes. Test input stays inside isolated egui contexts; queues remain paused.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--delete-files')) {
  const results = [
    capture('delete-file-dark',true,'1280x860','delete-file-confirmation'),
    capture('delete-file-light',false,'1280x860','delete-file-confirmation'),
    capture('delete-file-minimum',true,'860x600','delete-file-confirmation'),
    capture('delete-files-minimum',true,'860x600','delete-files-confirmation'),
  ];
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native deletion confirmations only; isolated saved fixture files and history are preserved.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--rows')) {
  const results=[];
  for(const dark of [true,false]) for(const compact of [false,true]) {
    const name=`rows-${dark?'dark':'light'}-${compact?'compact':'comfortable'}`;
    results.push(capture(name,dark,'1280x860','downloads-focused',false,false,compact));
    results.push(capture(`${name}-all-columns`,dark,'1440x860','downloads-focused',false,true,compact));
  }
  results.push(capture('rows-minimum',true,'860x600','downloads-focused'));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native focused download-row rendering in both themes and densities, including every optional column. Isolated paused stores; no live network.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--drop-target')) {
  // Screenshot builds never open the native box or its OLE registration; the box itself,
  // its right-click menu and the tray menu are captured from the real app separately.
  const codes=JSON.parse(fs.readFileSync(path.join(root,'locales/languages.json'),'utf8')).map(language=>language.code);
  const catalogs=['desktop','additional'].map(name=>JSON.parse(fs.readFileSync(path.join(root,'locales',`${name}.json`),'utf8')));
  const label=(language,text)=>{const i=codes.indexOf(language);return i<=0?text:i<4?catalogs[0][text][i-1]:catalogs[1][text][i-4];};
  const results=[];
  for(const language of ['en','de','ru','ar']) for(const dark of [true,false]) {
    const tag=`${language}-${dark?'dark':'light'}`;
    const menu=`${label(language,'View')}|${label(language,'Floating drop box')}`;
    results.push(capture(`drop-target-view-menu-${tag}`,dark,'860x600','downloads',false,false,false,{language,menu,dropTarget:true}));
    if(language==='en') results.push(capture(`drop-target-view-menu-off-${tag}`,dark,'860x600','downloads',false,false,false,{language,menu}));
    for(const feature of ['drop-target-one','drop-target-many']) {
      results.push(capture(`${feature}-${tag}`,dark,'860x600','downloads',false,false,false,{language,feature}));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'View → Floating drop box (on and off) and what a drop of one link (Add download filled in) or several (Add batch) opens, in English, German, Russian and Arabic, both themes. Isolated paused profiles; nothing is added or started.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--browsers')) {
  // Browser setup reads this user's real registration to show its status; it never changes it.
  const results=[];
  for(const language of ['en','de','ru','ar']) for(const dark of [true,false]) {
    const tag=`${language}-${dark?'dark':'light'}`;
    for(const browser of ['chrome','brave']) {
      results.push(capture(`browsers-${browser}-${tag}`,dark,'860x600',`browsers-${browser}`,false,false,false,{language}));
    }
    results.push(capture(`browsers-settings-${tag}`,dark,'860x600','settings-startup',false,false,false,{language}));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Browser integration on the Chrome and Brave tabs and its Settings entry at the minimum window size, in English, German, Russian and Arabic, both themes. Registration status is read but never changed; isolated paused profiles.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--power')) {
  // Screenshot builds cannot shut down, sleep or hibernate the computer and play no sounds.
  // Menu paths name translated labels, so read them from the catalogs.
  const desktop=JSON.parse(fs.readFileSync(path.join(root,'locales/desktop.json'),'utf8'));
  const additional=JSON.parse(fs.readFileSync(path.join(root,'locales/additional.json'),'utf8'));
  const codes=JSON.parse(fs.readFileSync(path.join(root,'locales/languages.json'),'utf8')).map(language=>language.code);
  const t=(language,key)=>{const i=codes.indexOf(language);return i===0?key:i<4?desktop[key][i-1]:additional[key][i-4];};
  const results=[];
  for(const language of ['en','de','ar']) for(const dark of [true,false]) {
    const tag=`${language}-${dark?'dark':'light'}`;
    const menu=['Queue actions',t(language,'When queue finishes'),t(language,'Sleep computer')].join('|');
    results.push(capture(`power-menu-${tag}`,dark,'1080x800','queues',false,false,false,{language,menu}));
    results.push(capture(`power-menu-no-hibernate-${tag}`,dark,'1080x800','queues',false,false,false,{language,menu,feature:'queues-no-hibernate'}));
    for(const view of ['status-completion','status-completion-no-hibernate','completion-sleep','completion-hibernate','preferences']) {
      results.push(capture(`power-${view}-${tag}`,dark,'860x600',view,false,false,false,{language}));
    }
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Sleep and Hibernate choices in the queue menu and When done tab (available and hibernation unavailable), the sleep and hibernate countdowns and the completion sound setting, in English, German and Arabic, both themes. Isolated paused profiles; no power request or sound.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--list-transfer')) {
  // Fixture dialogs only: the review comes from an in-memory sample list, and
  // capture() checks that no download was added and no queue started.
  const results=[];
  const codes=JSON.parse(fs.readFileSync(path.join(root,'locales/languages.json'),'utf8')).map(language=>language.code);
  const catalogs=['desktop','additional'].map(name=>JSON.parse(fs.readFileSync(path.join(root,'locales',`${name}.json`),'utf8')));
  const label=(language,text)=>{const i=codes.indexOf(language);return i<=0?text:i<4?catalogs[0][text][i-1]:catalogs[1][text][i-4];};
  for(const language of ['en','de','ar']) for(const dark of [true,false]) {
    const tag=`${language}-${dark?'dark':'light'}`;
    results.push(capture(`list-menu-${tag}`,dark,'860x600','downloads',false,false,false,{language,menu:`${label(language,'Tasks')}|${label(language,'Import list…')}`}));
    for(const view of ['list-export','list-import','list-import-details']) results.push(capture(`${view}-${tag}`,dark,'860x600',view,false,false,false,{language}));
  }
  results.push(capture('list-import-details-wide-dark',true,'1280x860','list-import-details'));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Tasks menu, Export list and Import list review dialogs in English, German and Arabic, both themes, minimum and wide windows. Isolated paused profiles; nothing is written, imported or started.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--clipboard')) {
  // The card comes from a fixture link. Screenshot builds never register a clipboard
  // listener, so the user's clipboard is neither read nor changed.
  const results=[];
  for(const language of ['en','de','ru']) for(const dark of [true,false]) {
    const tag=`${language}-${dark?'dark':'light'}`;
    results.push(capture(`clipboard-settings-${tag}`,dark,'860x600','settings-bottom',false,false,false,{language}));
    results.push(capture(`clipboard-offer-${tag}`,dark,'860x600','clipboard-offer',false,false,false,{language}));
  }
  results.push(capture('clipboard-offer-wide-dark',true,'1280x860','clipboard-offer'));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Clipboard links setting row and copied-link card in English, German and Russian, both themes, minimum and wide windows. Isolated paused profiles; no clipboard access.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
if(process.argv.includes('--batch-pattern')) {
  const results=[];
  for(const language of ['en','de','ar']) {
    for(const size of ['860x600','1280x800']) results.push(capture(`batch-pattern-${language}-${size}`,true,size,'batch-pattern',false,false,false,{language}));
    results.push(capture(`batch-pattern-error-${language}`,true,'860x600','batch-pattern-error',false,false,false,{language}));
  }
  results.push(capture('batch-pattern-en-light',false,'860x600','batch-pattern'));
  results.push(capture('batch-list-en',true,'860x600','batch'));
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Add batch: From a pattern preview and over-limit error in English, German and Arabic at minimum and wide sizes, plus Link list. Isolated paused stores; nothing is added.'},null,2));
  console.log('Verification: '+path.join(output,'verification.json'));process.exit(0);
}
if(process.argv.includes('--youtube')) {
  const results=[];
  for(const dark of [true,false]) for(const view of ['youtube-qualities','youtube-preview','youtube-error']) {
    results.push(capture(`${view}-${dark?'dark':'light'}`,dark,'860x600',view));
  }
  fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native GPU rendering with isolated paused fixture stores; no live network requests.'},null,2));
  console.log(`Verification: ${path.join(output,'verification.json')}`);
  process.exit(0);
}
const results = diagnosticsOnly || videosOnly ? [] : [
  capture('default-theme',null,'1280x860'),
  capture('classic-dark',true,'1280x860'), capture('classic-light',false,'1280x860'),
  capture('minimum-dark',true,'860x600'), capture('minimum-light',false,'860x600'),
  capture('about-dark',true,'860x600','help'), capture('queues-dark',true,'1080x800','queues'),
  capture('about-light',false,'860x600','help'),
  capture('preferences-dark',true,'860x600','preferences'), capture('preferences-light',false,'860x600','preferences'),
  capture('browser-setup-dark',true,'860x600','browser-setup'), capture('browser-setup-light',false,'860x600','browser-setup'),
  capture('settings-light',false,'860x600','settings'), capture('empty-dark',true,'860x600','downloads',true),
  capture('saved-columns-dark',true,'1280x860','downloads',false,true),
  capture('add-dark',true,'860x600','add'), capture('add-light',false,'860x600','add'),
  capture('add-error-dark',true,'860x600','add-error'), capture('add-error-light',false,'860x600','add-error'),
  capture('filtered-empty-dark',true,'860x600','filtered-empty'), capture('filtered-empty-light',false,'860x600','filtered-empty'),
  capture('empty-light',false,'860x600','downloads',true),
  capture('queues-minimum-dark',true,'860x600','queues'), capture('queues-minimum-light',false,'860x600','queues'),
];
for (const dark of diagnosticsOnly || videosOnly ? [] : [true, false]) {
  for (const view of ['settings','add-advanced','folders','speed-limits','columns','remove-confirmation','multi-selection']) {
    results.push(capture(`${view}-polish-${dark ? 'dark' : 'light'}`,dark,'860x600',view));
  }
}
for (const dark of videosOnly ? [] : [true, false]) results.push(capture(`diagnostics-${dark ? 'dark' : 'light'}`,dark,'860x600','diagnostics'));
if(diagnosticsOnly) {
  results.push(capture('logs-wide',true,'1280x860','diagnostics'));
  results.push(capture('logs-empty',true,'860x600','logs-empty'));
  results.push(capture('logs-error-dark',true,'860x600','logs-error'));
  results.push(capture('logs-error-light',false,'860x600','logs-error'));
  for(const dark of [true,false]) results.push(capture(`logs-notices-${dark?'dark':'light'}`,dark,'860x600','diagnostics'));
}
for (const dark of diagnosticsOnly ? [] : [true,false]) {
  for (const view of ['browser-videos','browser-videos-empty']) results.push(capture(`${view}-${dark ? 'dark' : 'light'}`,dark,'860x600',view));
}
if(!diagnosticsOnly) results.push(capture('browser-videos-cold-start',true,'860x600','browser-videos-cold-start'));
fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({results,scope:'Native GPU rendering and isolated paused fixture stores. No live downloads or installed-app state changed.'},null,2));
console.log(`Verification: ${path.join(output,'verification.json')}`);

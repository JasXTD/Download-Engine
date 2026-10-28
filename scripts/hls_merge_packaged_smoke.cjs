const openProfileDatabase = require('./profile_database.cjs');
// Real packaged app + pinned media tools, isolated data, and generated loopback
// media. Review/add clicks are covered by Rust UI tests; this seeds reviewed jobs.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { randomUUID, createHash } = require('node:crypto');
const { spawn, spawnSync } = require('node:child_process');
const { once } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');

(async () => {
  const root = path.resolve(__dirname, '..');
  const packageRoot = path.resolve(process.argv[2] || path.join(root, 'dist/IDM-EX-UnsignedDev'));
  const ffmpeg = path.join(packageRoot, 'tools/ffmpeg/ffmpeg.exe');
  const ffprobe = path.join(root, 'artifacts/ffmpeg-tools/ffprobe.exe');
  const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
  const media = JSON.parse(fs.readFileSync(path.join(root, 'tools/ffmpeg/media-manifest.json'), 'utf8'));
  assert.equal(sha(ffmpeg), media.files.find(file => file.path === 'ffmpeg.exe').sha256);
  assert.equal(sha(ffprobe), media.ffprobeSha256);
  const home = path.join(root, 'artifacts', `hls-merge-package-${randomUUID().slice(0,8)}`);
  fs.mkdirSync(path.join(home, 'data'), { recursive:true });
  const env = {...process.env, IDM_EX_HOME:home};
  for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
  function execute(exe,args,cwd=home) {
    const result=spawnSync(exe,args,{cwd, windowsHide:true, encoding:'utf8',timeout:20000,maxBuffer:4*1024*1024});
    assert.equal(result.error,undefined,String(result.error));
    assert.equal(result.status,0,result.stderr);
    return result.stdout;
  }
  const probe = file => JSON.parse(execute(ffprobe,['-v','error','-show_streams','-show_packets','-show_entries','stream=index,codec_type,codec_name:packet=stream_index,pts_time,dts_time','-of','json',file]));
  const routes=new Map(), requests=[];
  const server=http.createServer((req,res)=> {
    requests.push({url:req.url,authenticated:Boolean(req.headers.cookie||req.headers.authorization)});
    const body=routes.get(req.url);
    res.writeHead(body ? 200 : 404,{'Content-Length':body?.length||0});res.end(body);
  });
  let db,child,childError;
  async function stop() {if(!child?.pid||child.exitCode!==null||child.signalCode!==null)return;const ended=once(child,'exit');child.kill();await ended;child=undefined;}
  try {
  server.listen(0,'127.0.0.1'); await once(server,'listening');
  const base=`http://127.0.0.1:${server.address().port}`;
  const cases=[];
  for (const mp4 of [false,true]) {
    const name=mp4?'fmp4':'ts', folder=path.join(home,name);fs.mkdirSync(folder);
    for (const audio of [false,true]) {
      const track=audio?'audio':'video';
      execute(ffmpeg,['-hide_banner','-nostdin','-loglevel','error','-f','lavfi','-i',audio?'sine=frequency=660:sample_rate=48000:duration=2.5':'testsrc2=size=160x90:rate=25:duration=2.5',
        ...(audio?['-c:a','aac','-b:a','96k','-output_ts_offset','0.32']:['-c:v','libx264','-threads','1','-g','25','-bf','2']),
        '-f','hls','-hls_playlist_type','vod','-hls_time','1','-start_number',audio?'41':'7','-hls_segment_type',mp4?'fmp4':'mpegts',
        ...(mp4?['-hls_fmp4_init_filename',`${track}-init.mp4`]:[]),'-hls_segment_filename',`${track}-%d.${mp4?'m4s':'ts'}`,`${track}.m3u8`],folder);
    }
    for (const file of fs.readdirSync(folder)) routes.set(`/${name}/${file}`,fs.readFileSync(path.join(folder,file)));
    function playlist(track) {
      const text=fs.readFileSync(path.join(folder,`${track}.m3u8`),'utf8');
      const files=[...text.matchAll(/^([^#\s].+)$/gm)].map(m=>m[1].trim());
      const init=text.match(/#EXT-X-MAP:URI="([^"]+)"/)?.[1];
      const parts=[...(init?[init]:[]),...files].map((file,index)=>({url:`${base}/${name}/${file}`,range:null,initialization:Boolean(init&&index===0)}));
      const joined=path.join(folder,`joined-${track}.${mp4?'mp4':'ts'}`);
      fs.writeFileSync(joined,Buffer.concat(parts.map(part=>fs.readFileSync(path.join(folder,new URL(part.url).pathname.split('/').pop())))));
      return {url:`${base}/${name}/${track}.m3u8`, duration_ms:[...text.matchAll(/#EXTINF:([0-9.]+)/g)].reduce((sum,m)=>sum+Math.round(Number(m[1])*1000),0),
        sequence:Number(text.match(/#EXT-X-MEDIA-SEQUENCE:(\d+)/)[1]),format:mp4?'fragmented_mp4':'transport_stream',parts,joined};
    }
    const video=playlist('video'),audio=playlist('audio');
    const {joined:videoFile,...v}=video,{joined:audioFile,...a}=audio;
    const url=`${base}/${name}/master.m3u8`,id=randomUUID(),created=Math.floor(Date.now()/1000);
    const job={id,request_id:randomUUID(),url,name:`Reviewed ${name}.mp4`,server_name_pending:false,directory:path.join(home,'downloads'),status:'queued',downloaded:0,total:null,plan:null,error:null,saved_path:null,created,
      hls:{version:2,completed:[],playlist:{...v,source:url,quality:'160 × 90 · local fixture',bandwidth:800000,audio:{...a,name:'English',language:'en'}}}};
    cases.push({name,job,videoFile,audioFile});
  }
  db=openProfileDatabase(path.join(home,'data/queue.sqlite3')); db.exec('PRAGMA busy_timeout=5000');
  function launch() {childError=undefined;child=spawn(path.join(packageRoot,'de.exe'),[],{env,windowsHide:true,stdio:'ignore'});child.on('error',e=>childError=e);}
  async function until(check,message) {
    const deadline=Date.now()+30000;
    while(Date.now()<deadline) {assert.equal(childError,undefined,String(childError));if(child)assert.equal(child.exitCode,null,'desktop exited');const value=check();if(value)return value;await delay(40);}
    throw new Error(message);
  }
    launch();await until(()=>db.prepare("SELECT COUNT(*) AS n FROM sqlite_master WHERE name='hls_previews'").get().n,'database did not initialize');await stop();
    for (const [position,{job}] of cases.entries()) {
      db.prepare('INSERT INTO jobs(id,request_id,created,data) VALUES(?,?,?,?)').run(job.id,job.request_id,job.created,JSON.stringify(job));
      db.prepare("INSERT INTO job_queues(job_id,queue_id,position) VALUES(?,'main',?)").run(job.id,position);
    }
    launch();
    const results=[];
    for(const fixture of cases) {
      const complete=await until(()=> {const job=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(fixture.job.id).data);assert.notEqual(job.status,'failed',job.error);return job.status==='complete'&&job;},'merge did not complete');
      const merged=probe(complete.saved_path), originalVideo=probe(fixture.videoFile),originalAudio=probe(fixture.audioFile);
      assert.deepEqual(merged.streams.map(s=>s.codec_type),['video','audio']);
      assert.deepEqual(merged.streams.map(s=>s.codec_name),['h264','aac']);
      const videoPackets=merged.packets.filter(p=>p.stream_index===0),audioPackets=merged.packets.filter(p=>p.stream_index===1);
      assert.equal(videoPackets.length,originalVideo.packets.length);assert.equal(audioPackets.length,originalAudio.packets.length);
      const shift=Number(videoPackets[0].dts_time)-Number(originalVideo.packets[0].dts_time);
      for(const [original,actual] of [[originalVideo.packets,videoPackets],[originalAudio.packets,audioPackets]]) {
        original.forEach((packet,i)=> {for(const key of ['pts_time','dts_time'])assert(Math.abs(Number(actual[i][key])-Number(packet[key])-shift)<0.002,`Changed timestamp: ${key}`);});
      }
      execute(ffmpeg,['-hide_banner','-nostdin','-loglevel','error','-xerror','-i',complete.saved_path,'-map','0:v:0','-map','0:a:0','-f','null','-']);
      assert.equal(sha(complete.saved_path),complete.integrity.actual);
      assert.equal(fs.statSync(complete.saved_path).size,complete.downloaded);
      assert.equal(complete.downloaded,complete.total);
      assert.match(fs.readFileSync(`${complete.saved_path}:Zone.Identifier`,'utf8'),/ZoneId=3/);
      assert.equal(complete.hls.completed.length,fixture.job.hls.playlist.parts.length+fixture.job.hls.playlist.audio.parts.length);
      results.push({name:fixture.name,savedPath:complete.saved_path,bytes:complete.downloaded,sha256:complete.integrity.actual,videoPackets:videoPackets.length,audioPackets:audioPackets.length,sharedTimestampShift:shift,decodes:true,markOfTheWeb:true});
    }
    await stop();
    assert(requests.every(r=>!r.authenticated));
    const expected=cases.flatMap(({job})=>{const p=job.hls.playlist;return [p.url,p.audio.url,...p.parts.map(p=>p.url),...p.audio.parts.map(p=>p.url)].map(url=>new URL(url).pathname);}).sort();
    assert.deepEqual(requests.map(r=>r.url).sort(),expected);
    const report={home,packageRoot,helperSHA256:sha(ffmpeg),results,requests,limitations:'Generated local TS/fMP4 media and actual packaged engine verified. Live websites and registered browser interaction were not exercised. Review/add is covered by Rust UI tests.'};
    fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
  } finally {await stop();db?.close();server.closeAllConnections();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});

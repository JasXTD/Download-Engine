// Actual packaged app + native host, generated local MP4 tracks and isolated DB.
// Review/add clicks are covered by Rust UI tests; transfer seeds that reviewed state.
const assert=require('node:assert/strict');const fs=require('node:fs');const path=require('node:path');const http=require('node:http');
const {spawn,spawnSync}=require('node:child_process');const {once}=require('node:events');const {randomUUID,createHash}=require('node:crypto');const {DatabaseSync}=require('node:sqlite');
const root=path.resolve(__dirname,'..'),packageRoot=path.resolve(process.argv[2]||path.join(root,'dist/IDM-EX-UnsignedDev'));
const home=path.join(root,'artifacts','dash-package-'+randomUUID().slice(0,8));fs.mkdirSync(path.join(home,'data'),{recursive:true});
const env={...process.env,IDM_EX_HOME:home};for(const key of Object.keys(env))if(key.startsWith('IDM_EX_TEST_')||key.includes('SCREENSHOT_TO'))delete env[key];
const ffmpeg=path.join(packageRoot,'tools/ffmpeg/ffmpeg.exe'),ffprobe=path.join(root,'artifacts/ffmpeg-tools/ffprobe.exe');
const sha=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const media=JSON.parse(fs.readFileSync(path.join(root,'tools/ffmpeg/media-manifest.json'),'utf8'));
assert.equal(sha(ffmpeg),media.files.find(file=>file.path==='ffmpeg.exe').sha256);
assert.equal(sha(ffprobe),media.ffprobeSha256);
function execute(exe,args,cwd=home){const result=spawnSync(exe,args,{cwd,env,windowsHide:true,encoding:'utf8',timeout:20000,maxBuffer:4*1024*1024});assert.equal(result.error,undefined,String(result.error));assert.equal(result.status,0,result.stderr);return result.stdout;}
const probe=file=>JSON.parse(execute(ffprobe,['-v','error','-show_streams','-show_packets','-show_data_hash','SHA256','-show_entries','stream=index,codec_type,codec_name:packet=stream_index,pts_time,dts_time,data_hash','-of','json',file]));
const routes=new Map(),requests=[];
const server=http.createServer((req,res)=>{
  requests.push({url:req.url,range:req.headers.range||null,authenticated:Boolean(req.headers.cookie||req.headers.authorization)});
  const body=routes.get(req.url);
  if(!body){res.writeHead(404,{'Content-Length':0});res.end();return;}
  const headers={'Content-Length':body.length,ETag:'"fixture-v1"'};
  if(req.headers.range){
    const match=/^bytes=(\d+)-(\d+)$/.exec(req.headers.range),start=Number(match?.[1]),end=Number(match?.[2]);
    if(!match||start>end||end>=body.length){res.writeHead(416,{'Content-Length':0});res.end();return;}
    res.writeHead(206,{...headers,'Content-Length':end-start+1,'Content-Range':`bytes ${start}-${end}/${body.length}`});res.end(body.subarray(start,end+1));
  }else{res.writeHead(200,headers);res.end(body);}
});
const db=new DatabaseSync(path.join(home,'data/queue.sqlite3'));db.exec('PRAGMA busy_timeout=5000');let child;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,label){const end=Date.now()+30000;while(Date.now()<end){const result=fn();if(result)return result;await delay(40);}throw Error('Timed out: '+label);}
function launch(){child=spawn(path.join(packageRoot,'de.exe'),[],{env,windowsHide:true,stdio:'ignore'});}
async function stop(){if(child?.pid&&child.exitCode===null&&child.signalCode===null){const ended=once(child,'exit');child.kill();await ended;}child=null;}
async function native(request){const host=spawn(path.join(packageRoot,'de-host.exe'),[],{env,windowsHide:true,stdio:['pipe','pipe','pipe']});const output=[],errors=[];host.stdout.on('data',v=>output.push(v));host.stderr.on('data',v=>errors.push(v));const timer=setTimeout(()=>host.kill(),15000);try{const ended=once(host,'exit');const data=Buffer.from(JSON.stringify(request)),header=Buffer.alloc(4);header.writeUInt32LE(data.length);host.stdin.end(Buffer.concat([header,data]));const [code]=await ended;assert.equal(code,0,Buffer.concat(errors).toString());const body=Buffer.concat(output);assert.equal(body.readUInt32LE(),body.length-4);return JSON.parse(body.subarray(4));}finally{clearTimeout(timer);if(host.exitCode===null&&host.signalCode===null)host.kill();}}
(async()=>{server.listen(0,'127.0.0.1');await once(server,'listening');const base=`http://127.0.0.1:${server.address().port}`;try{
  const tracks={},indexedTracks={};
  for(const audio of [false,true]){
    const name=audio?'audio':'video',offset=audio?20.32:10;
    execute(ffmpeg,['-hide_banner','-nostdin','-loglevel','error','-f','lavfi','-i',audio?'sine=frequency=660:sample_rate=48000:duration=2.5':'testsrc2=size=160x90:rate=25:duration=2.5',
      ...(audio?['-c:a','aac','-b:a','96k']:['-c:v','libx264','-threads','1','-g','25','-bf','2']),'-output_ts_offset',String(offset),
      '-f','hls','-hls_playlist_type','vod','-hls_time','1','-hls_segment_type','fmp4','-hls_fmp4_init_filename',`${name}-init.mp4`,'-hls_segment_filename',`${name}-%d.m4s`,`${name}.m3u8`]);
    const text=fs.readFileSync(path.join(home,`${name}.m3u8`),'utf8'),files=[`${name}-init.mp4`,...[...text.matchAll(/^([^#\s].+)$/gm)].map(m=>m[1].trim())];
    const joined=path.join(home,`${name}-joined.mp4`);fs.writeFileSync(joined,Buffer.concat(files.map(file=>{const body=fs.readFileSync(path.join(home,file));routes.set('/'+file,body);return body;})));
    tracks[name]={parts:files.map((file,i)=>({url:base+'/'+file,range:null,initialization:i===0})),joined,original:probe(joined),offset:audio?20:10};
    const indexedFile=path.join(home,`${name}-indexed.mp4`);
    execute(ffmpeg,['-hide_banner','-nostdin','-loglevel','error','-f','lavfi','-i',audio?'sine=frequency=660:sample_rate=48000:duration=2.5':'testsrc2=size=160x90:rate=25:duration=2.5',
      ...(audio?['-c:a','aac','-b:a','96k']:['-c:v','libx264','-threads','1','-g','25','-bf','2']),'-movflags','+dash+global_sidx',indexedFile]);
    const bytes=fs.readFileSync(indexedFile),boxes=[];
    for(let start=0;start<bytes.length;){const length=bytes.readUInt32BE(start);assert(length>=8&&start+length<=bytes.length);boxes.push({kind:bytes.toString('ascii',start+4,start+8),start,length});start+=length;}
    const index=boxes.find(b=>b.kind==='sidx');assert(index);
    const url=`${base}/${name}-indexed.mp4`,range={start:index.start,length:index.length};routes.set(`/${name}-indexed.mp4`,bytes);
    // Derive expected fragment ranges from actual moof/mdat boxes, independently
    // of the application's SIDX parser. The generated fixture has one pair per reference.
    const media=boxes.flatMap((box,i)=>{
      if(box.kind!=='moof')return [];
      assert.equal(boxes[i+1]?.kind,'mdat');
      return [{url,range:{start:box.start,length:box.length+boxes[i+1].length},initialization:false}];
    });assert(media.length);
    indexedTracks[name]={parts:[{url,range:{start:0,length:index.start},initialization:true},...media],joined:indexedFile,original:probe(indexedFile),offset:0,
      index:{url,range,sha256:createHash('sha256').update(bytes.subarray(index.start,index.start+index.length)).digest('hex'),total:bytes.length,etag:'"fixture-v1"',last_modified:null}};
  }
  const cases=[];
  for(const indexed of [false,true])for(const audio of [true,false]){
    const selected=indexed?indexedTracks:tracks;
    const name=(indexed?'sidx-':'')+(audio?'merged':'video-only'),url=`${base}/${name}.mpd`;
    const set=(kind,id,language,bandwidth)=>{
      const track=selected[kind];
      const addressing=indexed
        ?`<BaseURL>${kind}-indexed.mp4</BaseURL><SegmentBase indexRange="${track.index.range.start}-${track.index.range.start+track.index.range.length-1}" indexRangeExact="true"><Initialization range="0-${track.index.range.start-1}"/></SegmentBase>`
        :`<SegmentList timescale="1000" presentationTimeOffset="${track.offset*1000}"><Initialization sourceURL="${kind}-init.mp4"/>${track.parts.slice(1).map(p=>`<SegmentURL media="${new URL(p.url).pathname.slice(1)}"/>`).join('')}</SegmentList>`;
      return `<AdaptationSet mimeType="${kind}/mp4"${language?' lang="en"':''}><Representation id="${id}" bandwidth="${bandwidth}"${kind==='video'?' width="160" height="90"':''}><Label>${kind==='video'?'Video':'English'}</Label>${addressing}</Representation></AdaptationSet>`;
    };
    routes.set(`/${name}.mpd`,Buffer.from(`<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT3S"><Period>${set('video','v',null,800000)}${audio?set('audio','en','en',96000):''}</Period></MPD>`));
    const created=Math.floor(Date.now()/1000),id=randomUUID();
    const playlist={source:url,url,quality:'160 × 90 · 0.8 Mb/s',bandwidth:audio?896000:800000,duration_ms:3000,sequence:0,format:'fragmented_mp4',parts:selected.video.parts,
      ...(audio?{audio:{name:'English',language:'en',url,duration_ms:3000,sequence:0,format:'fragmented_mp4',parts:selected.audio.parts}}:{}),
      dash:{video:'0/v',audio:audio?'1/en':null,video_offset_us:selected.video.offset*1000000,audio_offset_us:audio?selected.audio.offset*1000000:0,
        ...(indexed?{video_index:selected.video.index,...(audio?{audio_index:selected.audio.index}:{})}:{})}};
    cases.push({name,audio,indexed,tracks:selected,job:{id,request_id:randomUUID(),url,name:`Reviewed ${name}.mp4`,server_name_pending:false,directory:path.join(home,'downloads'),status:'queued',downloaded:0,total:null,plan:null,error:null,saved_path:null,created,hls:{version:indexed?4:3,completed:[],playlist}}});
  }
  launch();await until(()=>db.prepare("SELECT COUNT(*) AS n FROM sqlite_master WHERE name='hls_previews'").get().n,'database');
  assert((await native({action:'ping'})).capabilities.includes('video_preview_v1'));
  for(const fixture of [cases[0],cases[2]]){
    // A second receipt must not replace an open review. Reopen the isolated app
    // between fixtures so each request can be presented without discarding it.
    if(fixture!==cases[0]){await stop();launch();}
    const request={action:'preview_video',request_id:'hls:'+randomUUID(),url:fixture.job.url};const receipt=await native(request);assert.equal(receipt.ok,true,receipt.message);assert.equal(receipt.preview_id,request.request_id);
    await until(()=>db.prepare('SELECT url FROM hls_previews WHERE request_id=?').get(request.request_id)?.url===null,'desktop preview');
    assert.equal((await native(request)).preview_id,request.request_id);assert.equal((await native({...request,parts:[]})).ok,false);assert.equal((await native({action:'add',request_id:randomUUID(),url:request.url})).ok,false);
  }
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n,0);assert.deepEqual(requests,[]);await stop();
  for(const [index,{job}]of cases.entries()){db.prepare('INSERT INTO jobs(id,request_id,created,data) VALUES(?,?,?,?)').run(job.id,job.request_id,job.created,JSON.stringify(job));db.prepare("INSERT INTO job_queues(job_id,queue_id,position) VALUES(?,'main',?)").run(job.id,index);}
  launch();const results=[];
  for(const {name,audio,indexed,tracks:selected,job} of cases){
    const complete=await until(()=>{const current=JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(job.id).data);assert.notEqual(current.status,'failed',current.error);return current.status==='complete'&&current;},name);
    const merged=probe(complete.saved_path);assert.deepEqual(merged.streams.map(s=>s.codec_type),audio?['video','audio']:['video']);
    let shift;const counts={};
    for(const kind of audio?['video','audio']:['video']){
      const track=selected[kind],actual=merged.packets.filter(p=>p.stream_index===(kind==='video'?0:1));assert.equal(actual.length,track.original.packets.length);counts[kind]=actual.length;
      shift??=Number(actual[0].dts_time)-(Number(track.original.packets[0].dts_time)-track.offset);
      track.original.packets.forEach((packet,i)=>{assert.equal(actual[i].data_hash,packet.data_hash);for(const field of ['dts_time','pts_time'])assert(Math.abs(Number(actual[i][field])-(Number(packet[field])-track.offset)-shift)<0.002,`DASH ${kind} ${field} timing`);});
    }
    execute(ffmpeg,['-hide_banner','-nostdin','-loglevel','error','-xerror','-i',complete.saved_path,'-map','0:v:0',...(audio?['-map','0:a:0']:[]),'-f','null','-']);
    assert.equal(sha(complete.saved_path),complete.integrity.actual);assert.equal(fs.statSync(complete.saved_path).size,complete.downloaded);assert.match(fs.readFileSync(complete.saved_path+':Zone.Identifier','utf8'),/ZoneId=3/);
    results.push({name,indexed,path:complete.saved_path,counts,shift,packetHashesMatch:true,decoded:true,sha256:complete.integrity.actual,bytes:complete.downloaded});
  }
  await stop();assert(requests.every(r=>!r.authenticated));const report={home,packageRoot,nativePreview:true,previewJobs:0,previewMediaRequests:0,results,requests,limitations:'Local generated MP4, actual package and native protocol. Review/add clicks are covered by Rust UI tests; live websites and browser permission prompts are not tested.'};fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}finally{await stop();db.close();server.closeAllConnections();server.close();}})().catch(error=>{console.error(error);process.exitCode=1;});

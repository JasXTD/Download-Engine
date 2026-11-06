// Build a shareable example using only installed tools and generated media.
// No website, account, external download, dependency install or real profile.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createHash, createCipheriv} = require('node:crypto');
const {spawnSync} = require('node:child_process');
const root = path.resolve(__dirname, '..');
const source = path.join(root, 'examples/offline-episode');
const PUBLIC_KEY = '603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4';
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

function run(executable, args, options = {}) {
  const result = spawnSync(executable, args, {windowsHide:true, encoding:'utf8', timeout:60000, maxBuffer:1024*1024, ...options});
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr || result.stdout);
  return result.stdout;
}
function fragments(bytes) {
  const starts = [];
  for (let offset = 0; offset < bytes.length;) {
    assert(offset + 8 <= bytes.length, 'Truncated generated MP4 box');
    let size = bytes.readUInt32BE(offset);
    if (size === 1) { assert(offset + 16 <= bytes.length); size = Number(bytes.readBigUInt64BE(offset+8)); }
    if (size === 0) size = bytes.length - offset;
    assert(Number.isSafeInteger(size) && size >= 8 && offset + size <= bytes.length);
    if (bytes.toString('ascii', offset+4, offset+8) === 'moof') starts.push(offset);
    offset += size;
  }
  assert(starts.length >= 2 && starts.length <= 24, 'Expected a short fragmented MP4');
  return {init:starts[0], pieces:starts.map((offset, index) => ({offset, end:starts[index+1] ?? bytes.length}))};
}
function buildDemo(output, options = {}) {
  output = path.resolve(output);
  assert(!fs.existsSync(output), 'Choose a new output folder; existing examples are never overwritten');
  const artifacts = path.join(root, 'artifacts');
  fs.mkdirSync(artifacts, {recursive:true});
  const scratch = fs.mkdtempSync(path.join(artifacts, 'episode-build-'));
  const stage = path.join(scratch, 'bundle');
  const ffmpeg = path.resolve(options.ffmpeg || path.join(root, 'tools/ffmpeg/ffmpeg.exe'));
  const cli = path.resolve(options.cli || path.join(root, 'dist/IDM-EX-UnsignedDev/de-cli.exe'));
  try {
    fs.mkdirSync(stage);
    fs.cpSync(path.join(source, 'extension'), path.join(stage, 'extension'), {recursive:true, dereference:false});
    for (const file of ['README.md', 'NOTICE.md', 'Start-Server.ps1']) fs.copyFileSync(path.join(source,file), path.join(stage,file));
    fs.copyFileSync(path.join(root,'LICENSE'),path.join(stage,'LICENSE'));
    const routes = [], catalog = {video:{}, audio:{}, subtitles:{}, by_id:{}};
    const fixtureRoot = path.join(stage, 'fixtures');
    const write = (relative, encrypted, clear, processing) => {
      const destination = path.join(fixtureRoot, relative);
      fs.mkdirSync(path.dirname(destination), {recursive:true});
      fs.writeFileSync(destination, encrypted);
      const input_sha256 = sha256(encrypted), output_sha256 = sha256(clear);
      routes.push({url:'/'+relative, file:relative, size:encrypted.length, sha256:input_sha256});
      return {size:encrypted.length, input_sha256, output_sha256, processing};
    };
    const definitions = [
      {id:'video-144', kind:'video', quality:'144p', dimensions:'256x144', title:'Generated test pattern (144p)', language:'und'},
      {id:'video-90', kind:'video', quality:'90p', dimensions:'160x90', title:'Generated test pattern (90p)', language:'und'},
      {id:'audio-ja', kind:'audio', locale:'ja-JP', tone:880, title:'Japanese selection - synthetic 880 Hz tone', language:'jpn'},
      {id:'audio-en', kind:'audio', locale:'en-US', tone:440, title:'English selection - synthetic 440 Hz tone', language:'eng'},
    ];
    for (const definition of definitions) {
      const file = path.join(scratch, definition.id+'.mp4'), isVideo = definition.kind === 'video';
      run(ffmpeg, ['-hide_banner','-nostdin','-loglevel','error','-f','lavfi','-i',
        isVideo ? `testsrc2=size=${definition.dimensions}:rate=20:duration=4` : `sine=frequency=${definition.tone}:sample_rate=48000:duration=4`,
        ...(isVideo ? ['-c:v','mpeg4','-q:v','8','-threads','1','-g','10'] : ['-c:a','aac','-b:a','64k']),
        '-movflags','frag_keyframe+empty_moov+default_base_moof','-frag_duration','500000',file]);
      const clear = fs.readFileSync(file), split = fragments(clear);
      // A per-content, per-track public counter avoids reusing a key/counter pair
      // for different generated media, even if another FFmpeg version is used.
      const counter = createHash('sha256').update(definition.id).update(clear).digest().subarray(0,16);
      const cipher = createCipheriv('aes-256-ctr', Buffer.from(PUBLIC_KEY,'hex'), counter);
      const encrypted = Buffer.concat([cipher.update(clear),cipher.final()]);
      const item = {
        id:definition.id, kind:definition.kind, format:'mp4', language:definition.language, title:definition.title,
        counter:counter.toString('hex'), original_sha256:sha256(clear),
        initialization:'/assets/$RepresentationID$/init.mp4', media:'/assets/$RepresentationID$/$Number%05d$.enc',
        start_number:1, timeline:[{r:split.pieces.length-1}],
        init:write(`assets/${definition.id}/init.mp4`,clear.subarray(0,split.init),clear.subarray(0,split.init),'copy'),
        segments:split.pieces.map((part,index) => write(`assets/${definition.id}/${String(index+1).padStart(5,'0')}.enc`,
          encrypted.subarray(part.offset,part.end),clear.subarray(part.offset,part.end),'lua')),
      };
      catalog.by_id[item.id] = item;
      if (isVideo) catalog.video[definition.quality] = item;
      else catalog.audio[definition.locale] = item;
    }
    for (const [locale,language,label,line] of [
      ['en-US','eng','English sample subtitles','Generated offline episode. No website was contacted.'],
      ['ja-JP','jpn','Japanese sample subtitles','これはローカルで生成したサンプルです。'],
    ]) {
      const id = 'subs-'+language, relative = `assets/${id}/subtitles.srt`;
      const bytes = Buffer.from(`1\n00:00:00,000 --> 00:00:03,500\n${line}\n\n`);
      const item = {id,kind:'subtitle',format:'srt',language,title:label,path:'/'+relative,...write(relative,bytes,bytes,'copy')};
      catalog.subtitles[locale] = item;
      catalog.by_id[id] = item;
    }
    catalog.content_id = 'offline-episode-'+sha256(JSON.stringify(catalog)).slice(0,32);
    fs.writeFileSync(path.join(stage,'extension/catalog.lua'), '-- Generated metadata; no website data or private keys.\nreturn xd.json([==['+JSON.stringify(catalog)+']==])\n');
    fs.writeFileSync(path.join(fixtureRoot,'routes.json'), JSON.stringify(routes,null,2)+'\n');
    fs.writeFileSync(path.join(stage,'catalog.json'), JSON.stringify(catalog,null,2)+'\n');
    const profile = path.join(scratch,'pack-profile');
    const env = {...process.env, DOWNLOAD_ENGINE_HOME:profile, IDM_EX_HOME:profile};
    for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_')) delete env[key];
    run(cli, ['extensions','pack',path.join(stage,'extension'),'--out',path.join(stage,'offline-episode.de-extension.json'),'--no-start'], {env,cwd:scratch});
    const files = [];
    function inventory(directory) {
      for (const entry of fs.readdirSync(directory,{withFileTypes:true}).sort((a,b)=>a.name.localeCompare(b.name))) {
        const file = path.join(directory,entry.name);
        assert(!entry.isSymbolicLink(), 'Links are not permitted in the demo');
        if (entry.isDirectory()) inventory(file);
        else files.push(`${sha256(fs.readFileSync(file))}  ${path.relative(stage,file).replaceAll('\\','/')}`);
      }
    }
    inventory(stage);
    fs.writeFileSync(path.join(stage,'SHA256SUMS'), files.join('\n')+'\n');
    fs.mkdirSync(path.dirname(output),{recursive:true});
    fs.renameSync(stage,output);
    return {output,package:path.join(output,'offline-episode.de-extension.json'),routes:routes.length,bytes:routes.reduce((n,r)=>n+r.size,0)};
  } finally {
    assert(path.dirname(scratch) === artifacts && path.basename(scratch).startsWith('episode-build-'));
    fs.rmSync(scratch,{recursive:true,force:true,maxRetries:8,retryDelay:150});
  }
}
module.exports = {buildDemo, PUBLIC_KEY, sha256};
if (require.main === module) {
  try { console.log(JSON.stringify(buildDemo(process.argv[2] || path.join(root,'dist/examples/Offline-Episode-1.0.0'),
    {ffmpeg:process.env.MEDIA_TEST_FFMPEG,cli:process.env.MEDIA_DEMO_CLI}),null,2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}

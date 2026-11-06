// Original synthetic CENC fixture writer. Node encrypts, Lua decrypts and FFmpeg
// validates the reconstructed clear packets/frames. No external media.
'use strict';
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const {spawnSync} = require('node:child_process');
const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32BE(n); return b; };
const u16 = n => { const b = Buffer.alloc(2); b.writeUInt16BE(n); return b; };
const cat = (...parts) => Buffer.concat(parts);
const box = (type, ...parts) => { const body = cat(...parts); return cat(u32(body.length + 8), Buffer.from(type), body); };
const full = (type, flags, ...parts) => box(type, u32(flags), ...parts);
const digest = b => crypto.createHash('sha256').update(b).digest('hex');
function boxes(b, start = 0, end = b.length) {
  const out = [];
  while (start < end) {
    assert(end - start >= 8); const size = b.readUInt32BE(start);
    assert(size >= 8 && size <= end - start);
    out.push({start, end:start + size, type:b.toString('ascii', start + 4, start + 8)}); start += size;
  }
  assert.equal(start, end); return out;
}
function one(b, type, start = 0, end = b.length) {
  const found = boxes(b, start, end).filter(x => x.type === type); assert.equal(found.length, 1, type); return found[0];
}
function child(b, parent, type) { return one(b, type, parent.start + 8, parent.end); }
function rewriteInit(clear, keyId, ivSize, video) {
  const containers = new Set(['moov','trak','mdia','minf','stbl']);
  function visit(start, end) {
    return cat(...boxes(clear, start, end).map(item => {
      if (containers.has(item.type)) {
        const children = visit(item.start + 8, item.end);
        // A private fixture SystemID, deliberately unrelated to any real DRM.
        const pssh = full('pssh', 0x01000000, Buffer.from('da0ee3eb1c544d29a61998504a8e4cc3','hex'), u32(1), keyId, u32(0));
        return box(item.type, children, ...(item.type === 'moov' ? [pssh] : []));
      }
      if (item.type !== 'stsd') return clear.subarray(item.start, item.end);
      assert.equal(clear.readUInt32BE(item.start + 12), 1);
      const entry = boxes(clear, item.start + 16, item.end)[0];
      assert.equal(entry.type, video ? 'avc1' : 'mp4a');
      const sinf = box('sinf', box('frma', Buffer.from(entry.type)),
        full('schm', 0, Buffer.from('cenc'), u32(65536)),
        box('schi', full('tenc', 0, Buffer.from([0,0,1,ivSize]), keyId)));
      return full('stsd', 0, u32(1), box(video ? 'encv' : 'enca', clear.subarray(entry.start + 8, entry.end), sinf));
    }));
  }
  const encrypted = visit(0, clear.length), expected = Buffer.from(encrypted);
  const moov = one(expected, 'moov');
  const trak = child(expected, moov, 'trak');
  const stsd = child(expected, child(expected, child(expected, child(expected, trak, 'mdia'), 'minf'), 'stbl'), 'stsd');
  const entry = boxes(expected, stsd.start + 16, stsd.end)[0];
  expected.write(video ? 'avc1' : 'mp4a', entry.start + 4, 'ascii');
  const sinf = one(expected, 'sinf', entry.start + (video ? 86 : 36), entry.end);
  expected.write('free', sinf.start + 4, 'ascii');
  expected.write('free', child(expected, moov, 'pssh').start + 4, 'ascii');
  return {encrypted, expected};
}
function sampleInfo(fragment) {
  const moof = one(fragment, 'moof'), traf = child(fragment, moof, 'traf');
  const tfhd = child(fragment, traf, 'tfhd'), trun = child(fragment, traf, 'trun');
  const flags = fragment.readUInt32BE(tfhd.start + 8);
  assert(flags & 0x20000); assert.equal(flags & 1, 0);
  let p = tfhd.start + 16, defaultSize = 0;
  if (flags & 8) p += 4;
  if (flags & 16) { defaultSize = fragment.readUInt32BE(p); p += 4; }
  if (flags & 32) p += 4;
  assert.equal(p, tfhd.end);
  const tf = fragment.readUInt32BE(trun.start + 8), count = fragment.readUInt32BE(trun.start + 12);
  assert(tf & 1); let offset = moof.start + fragment.readInt32BE(trun.start + 16);
  p = trun.start + 20 + (tf & 4 ? 4 : 0);
  const samples = [];
  for (let i = 0; i < count; i++) {
    if (tf & 0x100) p += 4;
    let size = defaultSize;
    if (tf & 0x200) { size = fragment.readUInt32BE(p); p += 4; }
    if (tf & 0x400) p += 4;
    if (tf & 0x800) p += 4;
    assert(size > 0 && offset + size <= fragment.length);
    samples.push({offset, size}); offset += size;
  }
  assert.equal(p, trun.end); return {moof, traf, trun, samples};
}
function encryptFragment(clear, key, ivSize, video, counter) {
  const info = sampleInfo(clear), mdat = one(clear, 'mdat');
  const encryptedPayload = Buffer.from(clear.subarray(mdat.start + 8, mdat.end));
  const entries = [], sizes = [];
  for (const sample of info.samples) {
    const iv = Buffer.alloc(ivSize); iv.writeBigUInt64BE(BigInt(counter.value++));
    const cipher = crypto.createCipheriv('aes-128-ctr', key, cat(iv, Buffer.alloc(16 - ivSize)));
    const pairs = [];
    if (video) {
      let p = sample.offset;
      while (p < sample.offset + sample.size) {
        const length = clear.readUInt32BE(p);
        assert(length > 0 && p + 4 + length <= sample.offset + sample.size);
        // Keep the NAL length and header clear. Encrypted ranges share one CTR
        // state within this sample; clear bytes do not advance that state.
        pairs.push({clear:5, encrypted:length - 1}); p += 4 + length;
      }
    } else pairs.push({clear:0, encrypted:sample.size});
    let p = sample.offset;
    for (const pair of pairs) {
      p += pair.clear;
      cipher.update(clear.subarray(p, p + pair.encrypted)).copy(encryptedPayload, p - mdat.start - 8);
      p += pair.encrypted;
    }
    assert.equal(cipher.final().length, 0);
    const entry = video ? cat(iv, u16(pairs.length), ...pairs.map(x => cat(u16(x.clear), u32(x.encrypted)))) : iv;
    assert(entry.length <= 255); entries.push(entry); sizes.push(entry.length);
  }
  const senc = full('senc', video ? 2 : 0, u32(entries.length), ...entries);
  const saiz = full('saiz', 0, Buffer.from([0]), u32(entries.length), Buffer.from(sizes));
  const saio = full('saio', 0, u32(1), u32(0));
  const extra = senc.length + saiz.length + saio.length;
  const children = boxes(clear, info.traf.start + 8, info.traf.end).map(x => {
    const b = Buffer.from(clear.subarray(x.start, x.end));
    if (x.type === 'trun') b.writeInt32BE(b.readInt32BE(16) + extra, 16);
    return b;
  });
  const newTraf = box('traf', ...children, senc, saiz, saio);
  const newMoof = box('moof', ...boxes(clear, info.moof.start + 8, info.moof.end)
    .map(x => x.type === 'traf' ? newTraf : clear.subarray(x.start, x.end)));
  const traf = child(newMoof, one(newMoof, 'moof'), 'traf');
  const s = child(newMoof, traf, 'senc'), a = child(newMoof, traf, 'saio');
  newMoof.writeUInt32BE(s.start + 16, a.start + 16);
  const encrypted = cat(newMoof, box('mdat', encryptedPayload));
  const expected = cat(Buffer.from(newMoof), clear.subarray(mdat.start, mdat.end));
  for (const name of ['senc','saiz','saio']) expected.write('free', child(expected, child(expected, one(expected,'moof'),'traf'), name).start + 4, 'ascii');
  return {encrypted, expected, samples:entries.length, subsamples:video};
}
function execute(program, args, cwd) {
  const result = spawnSync(program, args, {cwd, windowsHide:true, encoding:'utf8', timeout:60000, maxBuffer:8*1024*1024});
  assert.ifError(result.error); assert.equal(result.status, 0, result.stderr); return result.stdout;
}
function generate(directory, ffmpeg) {
  const tracks = [], routes = new Map();
  for (const [id, video, ivSize, language] of [['video',true,16,'und'],['audio',false,8,'eng']]) {
    const clearPath = path.join(directory, id + '-clear.mp4');
    execute(ffmpeg, ['-hide_banner','-nostdin','-loglevel','error','-f','lavfi','-i',
      video ? 'testsrc2=size=160x90:rate=20:duration=3' : 'sine=frequency=440:sample_rate=48000:duration=3',
      ...(video ? ['-c:v','libx264','-threads','1','-g','10','-bf','0'] : ['-c:a','aac','-b:a','64k']),
      '-movflags','frag_keyframe+empty_moov+default_base_moof','-frag_duration','500000',clearPath], directory);
    const clear = fs.readFileSync(clearPath), top = boxes(clear);
    const first = top.find(x => x.type === 'moof'); assert(first);
    const key = crypto.randomBytes(16), kid = crypto.randomBytes(16);
    const parts = [rewriteInit(clear.subarray(0, first.start), kid, ivSize, video)];
    const counter = {value:1};
    for (let i = 0; i < top.length; i++) if (top[i].type === 'moof') {
      assert.equal(top[i + 1].type, 'mdat');
      parts.push(encryptFragment(clear.subarray(top[i].start, top[i + 1].end), key, ivSize, video, counter));
    }
    const segments = parts.map((part, index) => {
      const url = '/' + id + '/' + index;
      routes.set(url, part.encrypted);
      return {id:'part-' + index, url, size:part.encrypted.length, processing:'lua_stream',
        input_sha256:digest(part.encrypted), output_sha256:digest(part.expected)};
    });
    const encryptedPath = path.join(directory, id + '-cenc.mp4');
    fs.writeFileSync(encryptedPath, cat(...parts.map(p => p.encrypted)));
    tracks.push({id, kind:video ? 'video' : 'audio', format:'mp4', language, default:true,
      init_segment:'part-0', segments, key, kid, parts, clearPath, encryptedPath});
  }
  const ass = Buffer.from('[Script Info]\nScriptType: v4.00+\nPlayResX: 160\nPlayResY: 90\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,12,&H0000FFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,5,5,5,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:02.50,Default,,0,0,0,,{\\i1}Offline CENC sample{\\i0}\n');
  routes.set('/subtitles', ass);
  const plan = {content_id:'offline-cenc-' + crypto.randomUUID(), container:'mkv', tracks:tracks.map(t => {
    const {key,kid,parts,clearPath,encryptedPath,...publicTrack} = t; return publicTrack;
  }).concat([{id:'subtitles',kind:'subtitle',format:'ass',language:'eng',default:true,
    segments:[{id:'one',url:'/subtitles',size:ass.length,processing:'copy',input_sha256:digest(ass),output_sha256:digest(ass)}]}])};
  return {tracks, routes, plan};
}
function issue(request, publicKey, content, tracks) {
  assert.equal(request.subarray(0,5).toString(), 'DECR1');
  const nonce = request.subarray(5,37), contentHash = request.subarray(37,69), count = request.readUInt16BE(69);
  assert.deepEqual(contentHash, crypto.createHash('sha256').update(content).digest());
  assert.equal(count, tracks.length);
  for (let i = 0; i < count; i++) assert.deepEqual(request.subarray(71 + i*16, 87 + i*16), tracks[i].kid);
  const signedEnd = 71 + count*16, signatureLength = request.readUInt16BE(signedEnd);
  assert.equal(request.length, signedEnd + 2 + signatureLength);
  assert(crypto.verify('sha256', request.subarray(0,signedEnd), {key:publicKey,
    padding:crypto.constants.RSA_PKCS1_PSS_PADDING,saltLength:32}, request.subarray(signedEnd + 2)));
  const session = crypto.randomBytes(32), iv = crypto.randomBytes(16);
  const wrapped = crypto.publicEncrypt({key:publicKey,padding:crypto.constants.RSA_PKCS1_OAEP_PADDING,oaepHash:'sha256'},session);
  const cipher = crypto.createCipheriv('aes-256-ctr',session,iv);
  const encrypted = cat(cipher.update(cat(u16(count),...tracks.map(t=>cat(t.kid,t.key)))),cipher.final());
  const signed = cat(Buffer.from('DECL1'),nonce,contentHash,u16(wrapped.length),wrapped,iv,u32(encrypted.length),encrypted);
  return cat(signed,crypto.createHmac('sha256',session).update(signed).digest());
}
module.exports = {generate,issue,boxes,one,child,digest,execute};

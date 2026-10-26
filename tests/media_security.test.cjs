// Exercise the actual bundled parser with small, local MP4 fixtures. The PSSH
// cases are a few kilobytes, not stack-exhaustion or memory-pressure payloads.
const assert = require('node:assert/strict');
const { test, before, after } = require('node:test');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const helper = process.env.XDOWNLOADER_MEDIA_HELPER || path.join(root, 'tools/ffmpeg/ffmpeg.exe');
let fixture, progressive, fragmented;

function run(args) {
  const output = spawnSync(helper, ['-hide_banner', '-nostdin', '-v', 'error', ...args], {
    cwd: fixture, windowsHide: true, timeout: 15000, maxBuffer: 512 * 1024, encoding: 'utf8',
  });
  assert.equal(output.error, undefined, String(output.error));
  assert.equal(output.signal, null, output.stderr);
  return output;
}
function box(kind, payload) {
  const header = Buffer.alloc(8);
  header.writeUInt32BE(8 + payload.length);
  header.write(kind, 4, 'ascii');
  return Buffer.concat([header, payload]);
}
function boxes(data) {
  const result = [];
  for (let offset = 0; offset < data.length;) {
    assert(offset + 8 <= data.length, 'truncated generated box header');
    const size = data.readUInt32BE(offset);
    assert(size >= 8 && offset + size <= data.length, 'unexpected generated box size');
    result.push({ kind: data.toString('ascii', offset + 4, offset + 8),
      bytes: data.subarray(offset, offset + size), payload: data.subarray(offset + 8, offset + size) });
    offset += size;
  }
  return result;
}
function appendInside(data, kind, extra) {
  let found = false;
  const changed = boxes(data).map(entry => {
    if (entry.kind !== kind || found) return entry.bytes;
    found = true;
    return box(kind, Buffer.concat([entry.payload, extra]));
  });
  assert(found, `missing generated ${kind}`);
  return Buffer.concat(changed);
}
function beforeFragment(data, extra) {
  let found = false;
  const changed = boxes(data).map(entry => {
    if (entry.kind !== 'moof' || found) return entry.bytes;
    found = true;
    // The generated fragments use moof-relative data offsets, so inserting
    // before the complete moof/mdat pair preserves their encoded packets.
    return Buffer.concat([extra, entry.bytes]);
  });
  assert(found, 'missing generated movie fragment');
  return Buffer.concat(changed);
}
function parse(bytes, name) {
  const input = path.join(fixture, name);
  fs.writeFileSync(input, bytes);
  return run(['-xerror', '-max_alloc', '268435456', '-protocol_whitelist', 'file', '-f', 'mov',
    '-i', input, '-map', '0:v:0', '-c', 'copy', '-f', 'framehash', '-hash', 'sha256', '-']);
}

before(() => {
  fixture = fs.mkdtempSync(path.join(root, 'artifacts', 'media-security-'));
  for (const fragment of [false, true]) {
    const output = path.join(fixture, fragment ? 'fragmented.mp4' : 'progressive.mp4');
    const generated = run(['-f', 'lavfi', '-i', 'testsrc2=size=64x64:rate=5:duration=0.6',
      '-c:v', 'libx264', '-threads', '1', '-g', '1', '-bf', '0',
      ...(fragment ? ['-movflags', 'empty_moov+frag_keyframe+default_base_moof'] : []), output]);
    assert.equal(generated.status, 0, generated.stderr);
    if (fragment) fragmented = fs.readFileSync(output);
    else progressive = fs.readFileSync(output);
  }
});
after(() => {
  if (!fixture) return;
  assert.equal(path.dirname(fixture), path.join(root, 'artifacts'));
  assert(path.basename(fixture).startsWith('media-security-'));
  fs.rmSync(fixture, { recursive: true });
});

test('media helper preserves clear progressive and fragmented MP4 packets', () => {
  for (const [name, bytes] of [['progressive', progressive], ['fragmented', fragmented]]) {
    const output = parse(bytes, `${name}-clear.mp4`);
    assert.equal(output.status, 0, output.stderr);
    assert.equal(output.stdout.split('\n').filter(line => /^0,/.test(line)).length, 3);
  }
});

test('media helper rejects PSSH in movie and fragment metadata', () => {
  // A version-0 full box with a 16-byte system ID and zero private data.
  const pssh = box('pssh', Buffer.alloc(24));
  const repeated = Buffer.concat(Array.from({ length: 64 }, () => pssh));
  const accepted = [];
  for (const [name, bytes] of [
    ['movie', appendInside(progressive, 'moov', pssh)],
    ['repeated', appendInside(progressive, 'moov', repeated)],
    ['fragment', beforeFragment(fragmented, pssh)],
  ]) {
    const output = parse(bytes, `${name}-pssh.mp4`);
    if (output.status === 0) accepted.push(name);
    // The pinned FFmpeg build (tools/ffmpeg) still prints its pre-2.0 product name.
    else assert.match(output.stderr, /XDownloader does not support encrypted \/ DRM media \(pssh\)/);
  }
  assert.deepEqual(accepted, [], 'PSSH inputs must be rejected');
});

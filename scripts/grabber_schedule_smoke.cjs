const openProfileDatabase = require('./profile_database.cjs');
// Offline native screenshots using isolated databases; no packages required.
// Build first: cargo build --offline --locked --features screenshot
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { randomUUID } = require('node:crypto');
const { spawnSync } = require('node:child_process');
const { deflateSync } = require('node:zlib');

const root = path.resolve(__dirname, '..');
const dash = process.argv.includes('--dash');
const hls = process.argv.includes('--hls') || dash;
const output = path.join(root, 'artifacts', `${dash ? 'dash' : hls ? 'hls' : 'scheduled-grabber'}-ui-${randomUUID().slice(0, 8)}`);
fs.mkdirSync(output, { recursive: true });
function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
function chunk(type, bytes) {
  const data = Buffer.concat([Buffer.from(type), bytes]);
  const length = Buffer.alloc(4), checksum = Buffer.alloc(4);
  length.writeUInt32BE(bytes.length); checksum.writeUInt32BE(crc32(data));
  return Buffer.concat([length, data, checksum]);
}
function png(ppm) {
  const data = fs.readFileSync(ppm);
  let position = 0;
  function line() {
    const end = data.indexOf(10, position);
    assert(end >= 0);
    const text = data.subarray(position, end).toString();
    position = end + 1;
    return text;
  }
  assert.equal(line(), 'P6');
  const [width, height] = line().split(' ').map(Number);
  assert.equal(line(), '255');
  assert.equal(data.length - position, width * height * 3);
  const scanlines = Buffer.alloc(height * (width * 3 + 1));
  for (let y = 0; y < height; y++) data.copy(scanlines, y * (width * 3 + 1) + 1, position + y * width * 3, position + (y + 1) * width * 3);
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 2;
  const file = ppm.replace(/\.ppm$/, '.png');
  fs.writeFileSync(file, Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header), chunk('IDAT', deflateSync(scanlines)), chunk('IEND', Buffer.alloc(0))]));
  return file;
}

function capture(name, feature, width, height, dark, existing) {
  const home = existing || path.join(output, name);
  fs.mkdirSync(path.join(home, 'data'), { recursive: true });
  if (!existing) {
    const db = openProfileDatabase(path.join(home, 'data', 'queue.sqlite3'));
    db.exec('CREATE TABLE interface_preferences(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)');
    db.prepare('INSERT INTO interface_preferences VALUES(1,?)').run(JSON.stringify({ appearance: dark ? 'dark' : 'light', compact_rows: false, show_details: true, sidebar_labels: true }));
    db.close();
  }
  const ppm = path.join(home, `${name}.ppm`);
  const env = { ...process.env };
  for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
  Object.assign(env, { IDM_EX_HOME: home, IDM_EX_SCREENSHOT_TO: ppm, IDM_EX_TEST_SIZE: `${width}x${height}` });
  if (feature) env.IDM_EX_TEST_FEATURE = feature;
  else env.IDM_EX_TEST_VIEW = 'scheduler';
  const result = spawnSync(path.join(root, 'target', 'debug', 'de.exe'), [], { env, windowsHide: true, encoding: 'utf8', timeout: 30000 });
  assert.equal(result.error, undefined, String(result.error));
  assert.equal(result.status, 0, result.stderr);
  assert(fs.existsSync(ppm), result.stderr);
  const db = openProfileDatabase(path.join(home, 'data', 'queue.sqlite3'));
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 0);
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM grabber_projects').get().n, hls ? 0 : 1);
  const pending = db.prepare('SELECT COUNT(*) AS n FROM grabber_pending').get().n;
  if (hls) assert.equal(pending, 0);
  else if (feature !== 'grabber-schedule') assert.equal(pending, 3);
  db.close();
  return { name, home, screenshot: png(ppm), width, height, dark, pending, jobs: 0 };
}
const results = dash ? [
  capture('qualities-light', 'dash-qualities', 1080, 800, false),
  capture('qualities-dark-small', 'dash-qualities', 860, 600, true),
  capture('preview-light', 'dash', 1280, 860, false),
  capture('preview-dark-small', 'dash', 860, 600, true),
] : hls ? [
  capture('qualities-light', 'hls-qualities', 1080, 800, false),
  capture('qualities-dark-small', 'hls-qualities', 860, 600, true),
  capture('preview-light', 'hls', 1280, 860, false),
  capture('preview-dark-small', 'hls', 860, 600, true),
  capture('audio-qualities-light', 'hls-audio-qualities', 1080, 800, false),
  capture('audio-qualities-dark-small', 'hls-audio-qualities', 860, 600, true),
  capture('audio-preview-light', 'hls-audio', 1280, 860, false),
  capture('audio-preview-dark-small', 'hls-audio', 860, 600, true),
] : [
  capture('schedule-light', 'grabber-schedule', 1080, 800, false),
  capture('schedule-dark-small', 'grabber-schedule', 860, 600, true),
  capture('preview-light', 'grabber-discoveries', 1280, 860, false),
  capture('preview-dark-small', 'grabber-discoveries', 860, 600, true),
  capture('scheduler', 'grabber-scheduler', 1080, 800, false),
];
if (!hls) results.push(capture('reopened-scheduler', null, 1080, 800, false, results[2].home));
fs.writeFileSync(path.join(output, 'verification.json'), JSON.stringify(results, null, 2));
console.log(JSON.stringify({ output, results }, null, 2));

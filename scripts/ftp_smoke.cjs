const openProfileDatabase = require('./profile_database.cjs');
// Isolated native screenshots or packaged FTP transfer/restart smoke test.
// No installed packages, personal data, external servers or OS trust changes.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const tls = require('node:tls');
const { spawn, spawnSync } = require('node:child_process');
const { once } = require('node:events');
const { randomUUID, createHash } = require('node:crypto');
const { deflateSync } = require('node:zlib');

const root = path.resolve(__dirname, '..');
const ui = process.argv.includes('--ui');
const packageRoot = path.resolve(ui ? path.join(root, 'target/debug') : process.argv[2] || path.join(root, 'dist/IDM-EX-UnsignedDev'));
const home = path.join(root, 'artifacts', `ftp-${ui ? 'ui' : 'package'}-${randomUUID().slice(0, 8)}`);
fs.mkdirSync(path.join(home, 'data'), { recursive: true });
const env = { ...process.env, IDM_EX_HOME: home };
for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
async function until(check, label) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) { const result = check(); if (result) return result; await delay(40); }
  throw Error(`Timed out: ${label}`);
}
function png(ppm) {
  const data = fs.readFileSync(ppm); let pos = 0;
  function line() { const end = data.indexOf(10, pos); assert(end >= 0); const value = data.subarray(pos, end).toString(); pos = end + 1; return value; }
  assert.equal(line(), 'P6'); const [width, height] = line().split(' ').map(Number); assert.equal(line(), '255');
  assert.equal(data.length - pos, width * height * 3);
  function chunk(type, bytes) {
    const body = Buffer.concat([Buffer.from(type), bytes]), size = Buffer.alloc(4), checksum = Buffer.alloc(4);
    size.writeUInt32BE(bytes.length); let crc = 0xffffffff;
    for (const byte of body) { crc ^= byte; for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0); }
    checksum.writeUInt32BE((crc ^ 0xffffffff) >>> 0); return Buffer.concat([size, body, checksum]);
  }
  const pixels = Buffer.alloc(height * (width * 3 + 1));
  for (let y = 0; y < height; y++) data.copy(pixels, y * (width * 3 + 1) + 1, pos + y * width * 3, pos + (y + 1) * width * 3);
  const header = Buffer.alloc(13); header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 2;
  const target = ppm.replace(/\.ppm$/, '.png');
  fs.writeFileSync(target, Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk('IHDR', header), chunk('IDAT', deflateSync(pixels)), chunk('IEND', Buffer.alloc(0))]));
  return target;
}
function screenshots() {
  const results = [];
  for (const feature of ['ftp-add', 'ftps-add', 'ftps-login']) for (const dark of [false, true]) {
    const directory = path.join(home, `${feature}-${dark ? 'dark-small' : 'light'}`);
    fs.mkdirSync(path.join(directory, 'data'), { recursive: true });
    const db = openProfileDatabase(path.join(directory, 'data/queue.sqlite3'));
    db.exec('CREATE TABLE interface_preferences(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)');
    db.prepare('INSERT INTO interface_preferences VALUES(1,?)').run(JSON.stringify({ appearance: dark ? 'dark' : 'light' })); db.close();
    const ppm = path.join(directory, 'dialog.ppm');
    const result = spawnSync(path.join(packageRoot, 'de.exe'), [], { windowsHide: true, timeout: 30000, encoding: 'utf8', env: {
      ...env, IDM_EX_HOME: directory, IDM_EX_SCREENSHOT_TO: ppm, IDM_EX_TEST_FEATURE: feature, IDM_EX_TEST_SIZE: dark ? '860x600' : '1080x800'
    } });
    assert.equal(result.error, undefined, String(result.error)); assert.equal(result.status, 0, result.stderr);
    const reopened = openProfileDatabase(path.join(directory, 'data/queue.sqlite3'));
    assert.equal(reopened.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 0); reopened.close();
    results.push({ feature, dark, screenshot: png(ppm), jobs: 0 });
  }
  return results;
}
async function native(request) {
  const child = spawn(path.join(packageRoot, 'de-host.exe'), [], { env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const chunks = [], errors = []; child.stdout.on('data', data => chunks.push(data)); child.stderr.on('data', data => errors.push(data));
  const timeout = setTimeout(() => child.kill(), 15000);
  try {
    const ended = once(child, 'exit'); const data = Buffer.from(JSON.stringify(request)), header = Buffer.alloc(4); header.writeUInt32LE(data.length);
    child.stdin.end(Buffer.concat([header, data])); const [code] = await ended; assert.equal(code, 0, Buffer.concat(errors).toString());
    const output = Buffer.concat(chunks); assert.equal(output.readUInt32LE(), output.length - 4); return JSON.parse(output.subarray(4));
  } finally { clearTimeout(timeout); if (child.exitCode === null && child.signalCode === null) child.kill(); }
}
async function packaged() {
  const body = Buffer.alloc(4 * 1024 * 1024); for (let n = 0; n < body.length; n++) body[n] = n % 251;
  const commands = [], sockets = new Set(), listeners = new Set(); let pacing = 15, fixtureError;
  function track(socket) { sockets.add(socket); socket.on('error', () => {}); socket.on('close', () => sockets.delete(socket)); return socket; }
  const server = net.createServer(socket => {
    track(socket); socket.write('220 Local fixture\r\n'); let input = '', passive, accepted, offset = 0, work = Promise.resolve();
    socket.on('close', () => { if (passive) passive.close(); });
    async function command(line) {
      commands.push(line); const space = line.indexOf(' '), verb = space < 0 ? line : line.slice(0, space), arg = space < 0 ? '' : line.slice(space + 1);
      const send = text => socket.write(text + '\r\n');
      switch (verb) {
        case 'USER': assert.equal(arg, 'anonymous'); return send('331 Password');
        case 'PASS': return send('230 Welcome');
        case 'TYPE': case 'OPTS': return send('200 Accepted');
        case 'SIZE': return send(`213 ${body.length}`);
        case 'MDTM': return send('213 20261011120000.000');
        case 'EPSV': {
          passive = net.createServer(); listeners.add(passive); passive.on('close', () => listeners.delete(passive));
          accepted = new Promise(resolve => passive.once('connection', data => resolve(track(data))));
          passive.listen(0, '127.0.0.1'); await once(passive, 'listening'); return send(`229 Passive (|||${passive.address().port}|)`);
        }
        case 'REST': offset = Number(arg); assert(Number.isSafeInteger(offset) && offset >= 0 && offset <= body.length); return send('350 Restart');
        case 'RETR': {
          send('150 Sending'); const data = await accepted; passive.close();
          for (let n = offset; n < body.length && !data.destroyed && !socket.destroyed; n += 16384) {
            data.write(body.subarray(n, n + 16384)); if (pacing) await delay(pacing);
          }
          data.end(); if (!socket.destroyed) send('226 Complete'); return;
        }
        default: throw Error(`Unexpected fixture command: ${verb}`);
      }
    }
    socket.on('data', data => {
      input += data.toString(); let end;
      while ((end = input.indexOf('\r\n')) >= 0) { const line = input.slice(0, end); input = input.slice(end + 2); work = work.then(() => command(line)).catch(error => { fixtureError ||= error; socket.destroy(); }); }
    });
  });
  let tlsConnections = 0, tlsCommands = '';
  const secure = tls.createServer({ pfx: fs.readFileSync(path.join(root, 'tests/fixtures/ftp/server.pfx')), passphrase: 'fixture', minVersion: 'TLSv1.2' }, socket => {
    track(socket); socket.write('220 Untrusted local fixture\r\n'); socket.on('data', data => { tlsCommands += data.toString(); });
  });
  secure.on('connection', socket => { tlsConnections++; track(socket); }); secure.on('tlsClientError', () => {});
  let child, db;
  async function stop() { if (child?.pid && child.exitCode === null && child.signalCode === null) { const ended = once(child, 'exit'); child.kill(); await ended; } child = undefined; }
  function launch() { child = spawn(path.join(packageRoot, 'de.exe'), [], { env, windowsHide: true, stdio: 'ignore' }); }
  try {
    server.listen(0, '127.0.0.1'); await once(server, 'listening'); secure.listen(0, '127.0.0.1'); await once(secure, 'listening');
    assert((await native({ action: 'ping' })).ok);
    const reply = await native({ action: 'add', request_id: randomUUID(), url: `ftp://127.0.0.1:${server.address().port}/fixture.bin` });
    assert.equal(reply.ok, false); assert(reply.message.includes('HTTP'));
    db = openProfileDatabase(path.join(home, 'data/queue.sqlite3')); db.exec('PRAGMA busy_timeout=5000');
    db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0; UPDATE download_queues SET max_active=1;');
    const jobs = [['ftp', '127.0.0.1', server.address().port, 'plain'], ['ftps', 'localhost', secure.address().port, 'implicit']].map(([scheme, host, port, security], i) => {
      const id = randomUUID(), created = Math.floor(Date.now() / 1000);
      const job = { id, request_id: randomUUID(), url: `${scheme}://${host}:${port}/fixture.bin`, name: `${scheme}.bin`, directory: path.join(home, 'downloads'), status: 'queued', downloaded: 0, total: null, plan: null, ftp: { version: 1, security }, integrity: { expected: sha(body), actual: null }, error: null, saved_path: null, created };
      db.prepare('INSERT INTO jobs(id,request_id,created,data) VALUES(?,?,?,?)').run(id, job.request_id, created, JSON.stringify(job));
      db.prepare("INSERT INTO job_queues(job_id,queue_id,position) VALUES(?,'main',?)").run(id, i); return job;
    });
    const read = id => { if (fixtureError) throw fixtureError; return JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data); };
    launch(); await until(() => read(jobs[0].id).downloaded >= 65536, 'durable partial progress'); await stop();
    const part = path.join(home, 'data/parts', jobs[0].id, 'ftp.part'), saved = fs.statSync(part).size;
    assert(saved > 0 && saved < body.length); assert(fs.readFileSync(part).equals(body.subarray(0, saved)));
    pacing = 0; launch();
    const paused = await until(() => { const job = read(jobs[0].id); return job.status === 'paused' && job; }, 'interrupted FTP stays paused after reopen');
    const rejected = await until(() => { const job = read(jobs[1].id); return job.status === 'failed' && job; }, 'untrusted FTPS rejection');
    await stop();
    // Simulate the reviewed Resume action in the isolated DB while no worker
    // owns the job. Actual button/command interactions have Rust UI coverage.
    paused.status='queued';paused.error=null;
    db.prepare('UPDATE jobs SET data=? WHERE id=?').run(JSON.stringify(paused),paused.id);
    launch();
    const complete = await until(() => { const job = read(jobs[0].id); assert.notEqual(job.status,'failed',job.error); return job.status === 'complete' && job; }, 'resumed FTP completion');
    assert.equal(sha(fs.readFileSync(complete.saved_path)), sha(body)); assert.equal(complete.integrity.actual, sha(body));
    assert(commands.includes(`REST ${saved}`)); assert(fs.readFileSync(`${complete.saved_path}:Zone.Identifier`, 'utf8').includes('ZoneId=3'));
    assert(rejected.error.includes('TLS handshake failed')); assert.equal(rejected.saved_path, null); assert(tlsConnections > 0); assert(!/USER|PASS/.test(tlsCommands));
    assert(!fs.existsSync(path.join(home, 'downloads/ftps.bin')));
    await stop(); return { home, packageRoot, bytes: body.length, resumedAt: saved, sha256: sha(body), exactBytes: true, markOfTheWeb: true, restartResume: true, interruptedJobPaused: true, resumeActionSeeded: true, untrustedFtpsRejected: true, browserFtpRejected: true };
  } finally {
    await stop(); if (db) db.close(); for (const socket of sockets) socket.destroy(); for (const listener of listeners) listener.close(); server.close(); secure.close();
  }
}
(async () => {
  const result = ui ? screenshots() : await packaged();
  fs.writeFileSync(path.join(home, 'verification.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
})().catch(error => { console.error(error); process.exitCode = 1; });

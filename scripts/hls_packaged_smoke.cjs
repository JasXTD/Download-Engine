// Actual packaged executables, isolated data and loopback fixtures. The UI's
// review/add action is tested in Rust; the transfer below seeds that reviewed state.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { randomUUID, createHash } = require('node:crypto');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');
const { DatabaseSync } = require('node:sqlite');

(async () => {
  const root = path.resolve(__dirname, '..');
  const packageRoot = path.resolve(process.argv[2] || path.join(root, 'dist', 'IDM-EX-UnsignedDev'));
  const home = path.join(root, 'artifacts', `hls-package-${randomUUID().slice(0, 8)}`);
  fs.mkdirSync(path.join(home, 'data'), { recursive: true });
  const env = { ...process.env, IDM_EX_HOME: home };
  for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
  const segment = number => {
    const bytes = Buffer.alloc(188 * 10, number);
    for (let offset = 0; offset < bytes.length; offset += 188) bytes[offset] = 0x47;
    return bytes;
  };
  const parts = [segment(11), segment(22)];
  const expected = Buffer.concat(parts);
  const manifest = '#EXTM3U\n#EXT-X-TARGETDURATION:5\n#EXTINF:5,\none.ts\n#EXTINF:5,\ntwo.ts\n#EXT-X-ENDLIST\n';
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push({ method: req.method, url: req.url, authenticated: Boolean(req.headers.cookie || req.headers.authorization) });
    const body = req.url === '/index.m3u8' ? Buffer.from(manifest) : req.url === '/one.ts' ? parts[0] : req.url === '/two.ts' ? parts[1] : null;
    res.writeHead(body ? 200 : 404, { 'Content-Type': req.url.endsWith('.m3u8') ? 'application/vnd.apple.mpegurl' : 'video/mp2t', 'Content-Length': body?.length || 0 });
    res.end(body);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const base = `http://127.0.0.1:${server.address().port}`;
  const url = `${base}/index.m3u8`;
  const db = new DatabaseSync(path.join(home, 'data', 'queue.sqlite3'));
  db.exec('PRAGMA busy_timeout=5000');
  let child, childError;
  function launch() {
    childError = undefined;
    child = spawn(path.join(packageRoot, 'de.exe'), [], { env, windowsHide: true, stdio: 'ignore' });
    child.on('error', error => { childError = error; });
  }
  async function stop() {
    if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return;
    const ended = once(child, 'exit');
    child.kill();
    await ended;
    child = undefined;
  }
  async function until(check, message) {
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
      assert.equal(childError, undefined, String(childError));
      if (child) assert.equal(child.exitCode, null, 'desktop exited unexpectedly');
      const result = check();
      if (result) return result;
      await delay(30);
    }
    throw new Error(message);
  }
  async function native(request) {
    const host = spawn(path.join(packageRoot, 'de-host.exe'), [], { env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    const output = [], errors = [];
    host.stdout.on('data', data => output.push(data));
    host.stderr.on('data', data => errors.push(data));
    const timeout = setTimeout(() => host.kill(), 15000);
    try {
      const ended = once(host, 'exit');
      const bytes = Buffer.from(JSON.stringify(request)), header = Buffer.alloc(4);
      header.writeUInt32LE(bytes.length);
      host.stdin.end(Buffer.concat([header, bytes]));
      const [code] = await ended;
      assert.equal(code, 0, Buffer.concat(errors).toString());
      const response = Buffer.concat(output);
      assert(response.length > 4, 'native host returned no frame');
      assert.equal(response.readUInt32LE(), response.length - 4);
      return JSON.parse(response.subarray(4).toString());
    } finally {
      clearTimeout(timeout);
      if (host.exitCode === null && host.signalCode === null) host.kill();
    }
  }
  try {
    launch();
    await until(() => db.prepare("SELECT COUNT(*) AS n FROM sqlite_master WHERE type='table' AND name='hls_previews'").get().n, 'desktop database did not initialize');
    const request = { action: 'preview_hls', request_id: `hls:${randomUUID()}`, url };
    const receipt = await native(request);
    assert.equal(receipt.ok, true, receipt.message);
    assert.equal(receipt.preview_id, request.request_id);
    assert.equal(receipt.job_id, undefined);
    await until(() => db.prepare('SELECT url FROM hls_previews WHERE request_id=?').get(request.request_id)?.url === null, 'desktop did not open the HLS preview');
    assert.equal((await native(request)).preview_id, request.request_id, 'lost replies must replay the same preview receipt');
    assert.equal((await native({ action: 'add', request_id: randomUUID(), url })).ok, false, 'raw playlists must not bypass review');
    assert.equal((await native({ ...request, segments: [`${base}/one.ts`] })).ok, false, 'native preview must reject injected segments');
    assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 0);
    assert.deepEqual(requests, [], 'opening a preview must not fetch or queue anything automatically');
    await stop();

    const id = randomUUID(), created = Math.floor(Date.now() / 1000);
    const job = {
      id, request_id: randomUUID(), url, name: 'Reviewed fixture.ts', server_name_pending: false,
      directory: path.join(home, 'downloads'), status: 'queued', downloaded: 0, total: null,
      plan: null, error: null, saved_path: null, created,
      hls: { version: 1, completed: [], playlist: {
        source: url, url, quality: 'Original quality', bandwidth: null, duration_ms: 10000,
        sequence: 0, format: 'transport_stream',
        parts: ['one.ts', 'two.ts'].map(name => ({ url: `${base}/${name}`, range: null, initialization: false }))
      } }
    };
    db.prepare('INSERT INTO jobs(id,request_id,created,data) VALUES(?,?,?,?)').run(id, job.request_id, created, JSON.stringify(job));
    db.prepare('INSERT INTO job_queues(job_id,queue_id,position) VALUES(?,\'main\',0)').run(id);
    launch();
    const complete = await until(() => {
      const current = JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data);
      assert.notEqual(current.status, 'failed', current.error);
      return current.status === 'complete' && current;
    }, 'reviewed fixture did not complete');
    assert.deepEqual(fs.readFileSync(complete.saved_path), expected);
    assert.equal(complete.integrity.actual, createHash('sha256').update(expected).digest('hex'));
    assert.equal(complete.hls.completed.length, 2);
    assert.match(fs.readFileSync(`${complete.saved_path}:Zone.Identifier`, 'utf8'), /ZoneId=3/);
    assert.deepEqual(requests, ['/index.m3u8', '/one.ts', '/two.ts'].map(url => ({ method: 'GET', url, authenticated: false })));
    await stop();
    const report = { home, packageRoot, nativePreviewReceipt: true, previewCreatedJobs: 0,
      previewNetworkRequests: 0, rawAndForgedRequestsRejected: true,
      reviewedFixture: { savedPath: complete.saved_path, bytes: expected.length, sha256: complete.integrity.actual, markOfTheWeb: true },
      requests, limitations: 'Native protocol and actual packaged transfer tested. Review/add click is covered by Rust UI tests; synthetic TS packets do not test codec playback or live browser registration.' };
    fs.writeFileSync(path.join(home, 'verification.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally {
    await stop();
    db.close();
    server.closeAllConnections();
    server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });

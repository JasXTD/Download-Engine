const openProfileDatabase = require('./profile_database.cjs');
// Packaged Windows memory/handle soak. Only loopback traffic and a fresh profile.
// Retained download/run history is counted explicitly, not mistaken for a leak.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const {spawn} = require('node:child_process');
const {once} = require('node:events');
const {randomUUID, createHash} = require('node:crypto');
const {setTimeout: delay} = require('node:timers/promises');
const root = path.resolve(__dirname, '..');
const binary = path.resolve(process.argv[2] || path.join(root, 'dist/IDM-EX-UnsignedDev'));
const home = path.join(root, 'artifacts', `memory-packaged-${randomUUID().slice(0, 8)}`);
fs.mkdirSync(home, {recursive: true});
const env = {...process.env, IDM_EX_HOME: home};
for (const key of Object.keys(env)) {
  if (key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];
}
const children = new Set(), samples = [], checkpoints = [], payloads = new Set();
const counts = {complete: 0, failed: 0, resumed: 0, lua: 0, powershell: 0, stopped: 0, mcp: 0, statusWindows: 0};
const body = Buffer.alloc(1024 * 1024 + 37);
for (let i = 0; i < body.length; i++) body[i] = (i * 17 + 3) % 251;
const expectedHash = createHash('sha256').update(body).digest('hex');
let app, db, monitor, port, config, version, phase = 'initialization', serial = 0;
function child(exe, args, options = {}) {
  const p = spawn(exe, args, {env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], ...options});
  children.add(p); p.once('exit', () => children.delete(p)); return p;
}
async function command(exe, args) {
  const p = child(exe, args); let out = '', err = '';
  p.stdout.on('data', b => out += b); p.stderr.on('data', b => err += b);
  const timer = setTimeout(() => p.kill(), 30000);
  try { const [code, signal] = await once(p, 'exit'); assert.equal(signal, null, err); assert.equal(code, 0, err || out); return out; }
  finally { clearTimeout(timer); }
}
async function cli(args, noStart = true) {
  return JSON.parse(await command(path.join(binary, 'de-cli.exe'), [...args, ...(noStart ? ['--no-start'] : [])]));
}
async function action(args) { return (await cli(args)).receipt.result; }
async function until(fn, label, timeout = 45000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const value = await fn(); if (value) return value; await delay(60); }
  throw Error(`Timeout: ${label}`);
}
function job(id) { return JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data); }
function run(id) { return JSON.parse(db.prepare('SELECT data FROM script_runs WHERE id=?').get(id).data); }
async function rpc(name, args = {}) {
  counts.mcp++;
  const response = await fetch(`http://127.0.0.1:${port}/mcp`, {method: 'POST', signal: AbortSignal.timeout(25000), headers: {
    ...config.headers, 'Content-Type': 'application/json', Accept: 'application/json, text/event-stream',
    'MCP-Protocol-Version': '2026-07-28', 'Mcp-Method': 'tools/call', 'Mcp-Name': name,
  }, body: JSON.stringify({jsonrpc: '2.0', id: ++serial, method: 'tools/call', params: {name, arguments: args,
    _meta: {'io.modelcontextprotocol/protocolVersion': '2026-07-28', 'io.modelcontextprotocol/clientCapabilities': {}}}})});
  assert.equal(response.status, 200);
  let raw = await response.text();
  if (response.headers.get('content-type')?.startsWith('text/event-stream')) raw = raw.split(/\r?\n/).filter(l => l.startsWith('data:')).map(l => l.slice(5).trimStart()).join('\n');
  const value = JSON.parse(raw); assert(!value.error && !value.result?.isError, raw);
  return value.result.structuredContent;
}
const fixture = http.createServer((req, res) => {
  if (req.url.includes('/missing')) { res.writeHead(404, {'Content-Length': 0}); res.end(); return; }
  const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range || '');
  const start = range ? +range[1] : 0, end = range?.[2] ? Math.min(+range[2], body.length - 1) : body.length - 1;
  const headers = {'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes', ETag: '"memory-soak"', 'Content-Type': 'application/octet-stream'};
  if (range) headers['Content-Range'] = `bytes ${start}-${end}/${body.length}`;
  res.writeHead(range ? 206 : 200, headers);
  if (req.method === 'HEAD') { res.end(); return; }
  if (!req.url.includes('/slow')) { res.end(body.subarray(start, end + 1)); return; }
  (async () => {
    for (let at = start; at <= end && !res.destroyed; at += 16384) {
      res.write(body.subarray(at, Math.min(at + 16384, end + 1))); await delay(20);
    }
    if (!res.destroyed) res.end();
  })().catch(error => res.destroy(error));
});
async function listen(server) { server.listen(0, '127.0.0.1'); await once(server, 'listening'); return server.address().port; }
async function save(body, language, name) {
  const script = {id: randomUUID(), revision: 0, name, language, body, enabled: false, trigger: {kind: 'manual'}, timeout_seconds: 30};
  const file = path.join(home, `${script.id}.json`); fs.writeFileSync(file, JSON.stringify(script));
  return action(['scripts', 'save', file]);
}
async function completed(id) {
  const value = await until(() => { const j = job(id); assert.notEqual(j.status, 'failed', j.error); return j.status === 'complete' && j; }, 'complete file');
  assert.equal(createHash('sha256').update(fs.readFileSync(value.saved_path)).digest('hex'), expectedHash);
  payloads.add(value.saved_path); counts.complete++;
}
async function checkpoint(label, seconds = 8) {
  phase = label; await delay(seconds * 1000);
  assert.equal(app.exitCode, null);
  const recent = samples.filter(s => s.phase === label).slice(-5);
  assert(recent.length >= 3, 'Memory monitor has not supplied stable samples');
  const median = key => [...recent].sort((a,b) => a[key] - b[key])[Math.floor(recent.length / 2)][key];
  const point = {label, at: Date.now(), jobs: db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n,
    runs: db.prepare('SELECT COUNT(*) AS n FROM script_runs').get().n,
    ...Object.fromEntries(['privateBytes','workingSet','handles','threads','gdi','user'].map(k => [k, median(k)]))};
  checkpoints.push(point); fs.writeFileSync(path.join(home, 'checkpoints.json'), JSON.stringify(checkpoints, null, 2));
  console.log(`${label}: private ${(point.privateBytes / 1048576).toFixed(1)} MiB, handles ${point.handles}, threads ${point.threads}, jobs ${point.jobs}`);
}
async function closeApp() {
  if (!app || app.exitCode !== null || app.signalCode !== null) return;
  await command('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command',
    `$ErrorActionPreference='Stop'; $owned=Get-Process -Id ${app.pid}; if(-not $owned.CloseMainWindow()){throw 'Could not close owned memory-test app'}`]);
  await until(() => app.exitCode !== null || app.signalCode !== null, 'normal app exit', 25000);
  assert.equal(app.exitCode, 0); assert.equal(app.signalCode, null);
}
async function windows(action = 'List') {
  return JSON.parse(await command('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
    '-File', path.join(root, 'scripts/status_windows.ps1'), '-WindowProcess', String(app.pid), '-Action', action]));
}
async function nativeAdd(url) {
  const p = child(path.join(binary, 'de-host.exe'), [], {stdio: ['pipe', 'pipe', 'pipe']});
  const chunks = []; let err = '';
  p.stdout.on('data', b => chunks.push(b)); p.stderr.on('data', b => err += b);
  const bytes = Buffer.from(JSON.stringify({action: 'add', request_id: randomUUID(), url}));
  const prefix = Buffer.alloc(4); prefix.writeUInt32LE(bytes.length);
  p.stdin.end(Buffer.concat([prefix, bytes]));
  const timer = setTimeout(() => p.kill(), 20000);
  try {
    const [code, signal] = await once(p, 'exit'); assert.equal(signal, null); assert.equal(code, 0, err);
    const response = Buffer.concat(chunks); assert.equal(response.readUInt32LE(), response.length - 4);
    const value = JSON.parse(response.subarray(4)); assert(value.ok, value.message); return value.job_id;
  } finally { clearTimeout(timer); }
}
(async () => {
  const started = Date.now(); let success = false, failure;
  try {
    version = (await command(path.join(binary, 'de-cli.exe'), ['--version'])).trim();
    const origin = `http://127.0.0.1:${await listen(fixture)}`;
    const probe = http.createServer(); port = await listen(probe); await new Promise(r => probe.close(r));
    config = (await cli(['mcp-config', '--port', String(port)], false)).mcpServers["download-engine"];
    await cli(['queues'], false);
    db = openProfileDatabase(path.join(home, 'data/queue.sqlite3')); db.exec('PRAGMA busy_timeout=5000');
    db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0');
    db.prepare('UPDATE interface_preferences SET data=?').run(JSON.stringify({auto_download_status: true}));
    db.prepare('UPDATE mcp_settings SET port=?,start_with_app=1,read_only=0').run(port);
    app = child(path.join(binary, 'de.exe'), [], {stdio: 'ignore'});
    await until(() => { const file = path.join(home, 'data/logs/desktop.log'); return fs.existsSync(file) && fs.readFileSync(file, 'utf8').includes('"ready"'); }, 'desktop ready');
    monitor = child('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(root, 'scripts/memory_sample.ps1'), '-ProcessId', String(app.pid)]);
    let buffer = ''; monitor.stderr.on('data', b => fs.appendFileSync(path.join(home, 'monitor-errors.log'), b));
    monitor.stdout.on('data', b => {
      buffer += b; let end;
      while ((end = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, end).trim(); buffer = buffer.slice(end + 1);
        if (!line) continue;
        const point = {...JSON.parse(line), phase}; samples.push(point);
        fs.appendFileSync(path.join(home, 'samples.jsonl'), JSON.stringify(point) + '\n');
      }
    });
    await action(['queue', 'limit', 'main', '4']);
    const lua = await save('local v = {}; for i=1,10000 do v[i] = string.rep("x",100) end; print("finished")', 'lua', 'Memory Lua');
    const ps = await save('$v = 1..10000 | ForEach-Object { "x" * 100 }; Write-Output "finished"', 'power_shell', 'Memory PowerShell');
    const stop = await save('print("running"); while true do xd.sleep(50) end', 'lua', 'Memory cancellation');
    await checkpoint('startup');
    for (let cycle = 0; cycle < (process.argv.includes('--windows-only') ? 0 : 14); cycle++) {
      phase = `work-${cycle}`;
      const added = await rpc('add_downloads', {urls: Array.from({length: 8}, (_,i) => `${origin}/file-${cycle}-${i}.bin`), request_id: randomUUID()});
      const ids = added.receipt.result.map(j => j.id);
      await Promise.all(ids.map(completed));
      const missing = (await action(['add', `${origin}/missing-${cycle}.bin`]))[0];
      await until(() => job(missing.id).status === 'failed', '404 failure'); counts.failed++;
      const slow = (await action(['add', `${origin}/slow-${cycle}.bin`]))[0];
      await until(() => job(slow.id).downloaded > 0, 'saved partial bytes');
      await action(['pause', slow.id]);
      await until(() => job(slow.id).status === 'paused', 'paused transfer');
      await action(['resume', slow.id]); await completed(slow.id); counts.resumed++;
      for (const script of [lua, ps]) {
        const r = await action(['scripts', 'run', script.id]);
        const final = await until(() => { const v = run(r.id); return !['running','queued'].includes(v.state) && v; }, 'script completion');
        assert.equal(final.state, 'succeeded', JSON.stringify(final)); counts[script.language === 'power_shell' ? 'powershell' : 'lua']++;
      }
      const stopped = await action(['scripts', 'run', stop.id]);
      await until(() => run(stopped.id).state === 'running', 'running script');
      await action(['scripts', 'stop', stopped.id]);
      await until(() => run(stopped.id).state === 'stopped', 'stopped script'); counts.stopped++;
      // Stay below the server's four active-tool slots; overload rejection is
      // covered by MCP tests and is not a memory-growth failure.
      for (let batch = 0; batch < 50; batch++) await Promise.all(Array.from({length: 3}, () => rpc('list_downloads', {limit: 10})));
      await checkpoint(cycle < 4 ? `warmup-${cycle}` : `settled-${cycle}`);
    }
    for (let cycle = 0; cycle < 24; cycle++) {
      phase = `status-window-${cycle}`;
      const id = await nativeAdd(`${origin}/window-${cycle}.bin`);
      await completed(id);
      await until(async () => (await windows()).some(w => w.Title !== 'Download Engine'), 'native status window visible');
      await windows('CloseStatus');
      await until(async () => (await windows()).every(w => w.Title === 'Download Engine'), 'native status window released');
      counts.statusWindows++;
      if (cycle % 6 === 5) await checkpoint(`windows-settled-${cycle}`);
    }
    // Hold history constant so request-related retention can be distinguished
    // from the deliberate growth in the download list above.
    for (let cycle = 0; cycle < 6; cycle++) {
      phase = `fixed-history-work-${cycle}`;
      for (let batch = 0; batch < 50; batch++) await Promise.all(Array.from({length: 3}, () => rpc('list_downloads', {limit: 10})));
      await checkpoint(`fixed-history-settled-${cycle}`, 5);
    }
    await checkpoint('idle-final', 90);
    assert.equal(db.prepare("SELECT COUNT(*) AS n FROM script_runs WHERE state IN ('queued','running')").get().n, 0);
    assert.equal(db.prepare("SELECT COUNT(*) AS n FROM control_requests WHERE state IN ('pending','running')").get().n, 0);
    await closeApp(); success = true;
  } catch (error) { failure = error.stack; console.error(error); }
  finally {
    try { await closeApp(); } catch (error) { success = false; failure = (failure || '') + '\nCleanup: ' + error.stack; }
    for (const p of [...children]) if (p.exitCode === null && p.signalCode === null) { const ended = once(p, 'exit'); p.kill(); await ended; }
    db?.close(); fixture.closeAllConnections(); await new Promise(r => fixture.close(r));
    let removedBytes = 0;
    const base = fs.realpathSync(home) + path.sep;
    for (const file of payloads) {
      const resolved = fs.realpathSync(file); assert(resolved.startsWith(base), 'Payload outside isolated memory-test profile');
      removedBytes += fs.statSync(resolved).size; fs.unlinkSync(resolved);
    }
    const result = {success, failure, version, binary, home, elapsedSeconds: (Date.now() - started) / 1000, counts, checkpoints,
      peakPrivateBytes: Math.max(...samples.map(s => s.privateBytes)), removedPayloadBytes: removedBytes,
      limitations: ['Bounded process-counter soak, not exhaustive heap allocation tracing', 'Download and script history intentionally retained and counted', 'No browser extension or personal profile used']};
    fs.writeFileSync(path.join(home, 'verification.json'), JSON.stringify(result, null, 2));
    console.log(JSON.stringify({success, home, counts, elapsedSeconds: result.elapsedSeconds}));
    if (!success) process.exitCode = 1;
  }
})().catch(error => { console.error(error); process.exitCode = 1; });

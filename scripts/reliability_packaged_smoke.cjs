const openProfileDatabase = require('./profile_database.cjs');
// Real packaged processes, isolated profiles and generated loopback bytes.
// --large writes a 4 GiB + 64 MiB + 17 byte file (up to twice that during assembly).
// No remote downloads, global settings, or personal profiles are used.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const {spawn} = require('node:child_process');
const {once} = require('node:events');
const {randomUUID, createHash} = require('node:crypto');
const {setTimeout: delay, setImmediate: yieldTurn} = require('node:timers/promises');
const root = path.resolve(__dirname, '..');
const binary = path.resolve(process.argv[2] || 'dist/IDM-EX-UnsignedDev');
const mode = process.argv.includes('--large') ? 'large' : 'combined';
const home = path.join(root, 'artifacts', `reliability-${mode}-${randomUUID().slice(0, 8)}`);
fs.mkdirSync(home, {recursive: true});
const env = {...process.env, IDM_EX_HOME: home};
for (const key of Object.keys(env)) {
  if (key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];
}
const MiB = 1024 ** 2, GiB = 1024 ** 3;
const size = mode === 'large' ? 4 * GiB + 64 * MiB + 17 : 16 * MiB + 17;
const checks = [], requests = [], processes = new Set();
let app, db, phase = 0, origin, service, rpcId = 0;
const dockerOrigin = process.env.IDM_EX_RELIABILITY_FIXTURE;
async function setPhase(next) {
  phase = next;
  if (dockerOrigin) {
    const response = await fetch(`${dockerOrigin}/phase?value=${next}`, {method: 'POST'});
    assert.equal(response.status, 204);
  }
}
const pattern = Buffer.alloc(MiB);
for (let i = 0; i < pattern.length; i++) pattern[i] = (i * 31 + Math.floor(i / 251)) % 251;
function block(index) {
  const bytes = Buffer.from(pattern);
  // Absolute offsets embedded throughout each block catch reordering and 32-bit wraparound.
  for (let i = 0; i < bytes.length; i += 4096) bytes.writeBigUInt64LE(BigInt(index * MiB + i), i);
  return bytes;
}
async function expectedHash() {
  const hash = createHash('sha256');
  for (let at = 0; at < size; at += MiB) {
    hash.update(block(at / MiB).subarray(0, Math.min(MiB, size - at)));
    if (at % (16 * MiB) === 0) await yieldTurn();
  }
  return hash.digest('hex');
}
async function fileHash(file) {
  const hash = createHash('sha256');
  for await (const bytes of fs.createReadStream(file)) hash.update(bytes);
  return hash.digest('hex');
}
function child(exe, args, options = {}) {
  const process = spawn(exe, args, {env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], ...options});
  processes.add(process);
  process.once('exit', () => processes.delete(process));
  return process;
}
async function until(test, label, ms = 30000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const value = await test();
    if (value) return value;
    await delay(40);
  }
  throw Error(`Timed out: ${label}`);
}
async function cli(args, allowBusy = false) {
  const process = child(path.join(binary, 'de-cli.exe'), args);
  let out = '', err = '';
  process.stdout.on('data', bytes => out += bytes);
  process.stderr.on('data', bytes => err += bytes);
  const timer = setTimeout(() => process.kill(), 25000);
  try {
    const [code, signal] = await once(process, 'exit');
    assert.equal(signal, null, `CLI timeout: ${args}`);
    const result = JSON.parse(out || err);
    if (allowBusy && code !== 0) {
      assert.match(result.error || result.receipt?.error || '', /Pause the download before changing it/);
      return {busy: true};
    }
    assert.equal(code, 0, JSON.stringify({args, out, err}));
    return result;
  } finally { clearTimeout(timer); }
}
async function command(args) { return (await cli([...args, '--no-start'])).receipt.result; }
function job(id) { return JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(id).data); }
function mark(text) { checks.push(text); console.log(text); }
async function stopApp() {
  if (app && app.exitCode === null && app.signalCode === null) {
    const exited = once(app, 'exit'); app.kill(); await exited;
  }
  app = null;
}
async function launch() {
  app = child(path.join(binary, 'de.exe'), [], {stdio: 'ignore'});
  await command(['queue', 'limit', 'main', '2']);
}
const server = http.createServer((req, res) => {
  const match = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range || '');
  const start = match ? Number(match[1]) : 0;
  const end = match?.[2] ? Math.min(Number(match[2]), size - 1) : size - 1;
  const record = {phase, start, end, sent: 0, url: req.url}; requests.push(record);
  if (start > end || start >= size) { res.writeHead(416, {'Content-Range': `bytes */${size}`}); res.end(); return; }
  const headers = {'Content-Length': end - start + 1, 'Accept-Ranges': 'bytes', ETag: '"reliability-v1"', 'Content-Type': 'application/octet-stream'};
  if (match) headers['Content-Range'] = `bytes ${start}-${end}/${size}`;
  res.writeHead(match ? 206 : 200, headers);
  if (req.method === 'HEAD') { res.end(); return; }
  (async () => {
    let at = start;
    while (at <= end && !res.destroyed) {
      const gate = mode === 'large' ? (phase === 0 ? 32 * MiB : phase === 1 ? 4 * GiB + 8 * MiB : size) : (phase === 0 ? MiB : size);
      if (at >= gate) { await delay(20); continue; }
      const offset = at % MiB, bytes = block(Math.floor(at / MiB));
      const count = Math.min(MiB - offset, end - at + 1, gate - at);
      const ready = res.write(bytes.subarray(offset, offset + count));
      record.sent += count; at += count;
      if (!ready) await new Promise(resolve => {
        const done = () => { res.off('drain', done); res.off('close', done); resolve(); };
        res.once('drain', done); res.once('close', done);
      });
    }
    if (!res.destroyed) res.end();
  })().catch(error => res.destroy(error));
});
async function startMcp() {
  const probe = http.createServer(); probe.listen(0, '127.0.0.1'); await once(probe, 'listening');
  const port = probe.address().port; await new Promise(resolve => probe.close(resolve));
  const config = (await cli(['mcp-config', '--port', String(port)])).mcpServers["download-engine"];
  const process = child(path.join(binary, 'de-cli.exe'), ['mcp', '--port', String(port), '--no-start']);
  let stderr = ''; process.stderr.on('data', data => stderr += data); process.stdout.resume();
  await until(() => { assert.equal(process.exitCode, null, stderr); return stderr.includes('listening at'); }, 'MCP listener');
  service = {process, port, token: config.headers.Authorization};
}
async function tool(name, args = {}, allowBusy = false) {
  const version = '2026-07-28';
  const body = {jsonrpc: '2.0', id: ++rpcId, method: 'tools/call', params: {name, arguments: args,
    _meta: {'io.modelcontextprotocol/protocolVersion': version, 'io.modelcontextprotocol/clientCapabilities': {}, 'io.modelcontextprotocol/clientInfo': {name: 'reliability-test', version: '1'}}}};
  const result = await new Promise((resolve, reject) => {
    const request = http.request({host: '127.0.0.1', port: service.port, path: '/mcp', method: 'POST', headers: {
      authorization: service.token, 'content-type': 'application/json', accept: 'application/json, text/event-stream',
      'mcp-protocol-version': version, 'mcp-method': 'tools/call', 'mcp-name': name,
    }}, response => {
      let raw = ''; response.setEncoding('utf8'); response.on('data', data => raw += data); response.on('error', reject);
      response.on('end', () => { try {
        assert.equal(response.statusCode, 200, raw);
        if (response.headers['content-type']?.startsWith('text/event-stream')) raw = raw.split(/\r?\n/).filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart()).join('\n');
        resolve(JSON.parse(raw));
      } catch (error) { reject(error); } });
    });
    request.setTimeout(30000, () => request.destroy(Error('MCP timeout'))); request.on('error', reject); request.end(JSON.stringify(body));
  });
  assert(!result.error, JSON.stringify(result));
  const value = result.result.structuredContent;
  if (allowBusy && result.result.isError) {
    assert.match(value.error || value.receipt?.error || '', /Pause the download before changing it/); return {busy: true};
  }
  assert(!result.result.isError, JSON.stringify(value)); return value;
}
async function saveScript(body, language) {
  const script = {id: randomUUID(), revision: 0, name: `Concurrent ${language}`, language, body, enabled: false, trigger: {kind: 'manual'}, timeout_seconds: 60};
  const file = path.join(home, `${script.id}.json`); fs.writeFileSync(file, JSON.stringify(script));
  return command(['scripts', 'save', file]);
}
async function large() {
  assert(fs.statfsSync(home).bavail * fs.statfsSync(home).bsize > 12 * GiB, 'Need 12 GiB free for isolated large-file test');
  const added = (await command(['add', `${origin}/large.bin`]))[0];
  await until(() => job(added.id).downloaded >= 32 * MiB, 'first saved checkpoint', 120000);
  await command(['pause', added.id]);
  await until(() => job(added.id).status === 'paused', 'settled pause');
  const paused = job(added.id).downloaded;
  assert(paused > 0 && paused < size); await setPhase(1);
  await command(['resume', added.id]);
  let last = -1;
  await until(() => {
    const current = job(added.id); assert.notEqual(current.status, 'failed', current.error);
    const progress = Math.floor(current.downloaded / (256 * MiB));
    if (progress !== last) { last = progress; console.log(`Large transfer: ${Math.floor(current.downloaded / MiB)} / ${Math.ceil(size / MiB)} MiB`); }
    return current.downloaded >= 4 * GiB + 8 * MiB;
  }, 'saved progress beyond 32-bit boundary', 300000);
  if (dockerOrigin) requests.splice(0, requests.length, ...await (await fetch(`${dockerOrigin}/requests`)).json());
  assert(requests.some(r => r.phase === 1 && r.start === paused));
  await stopApp();
  const beforeRestart = job(added.id), parts = path.join(home, 'data/parts', added.id);
  const partBytes = fs.readdirSync(parts).filter(name => /^segment-\d+\.part$/.test(name)).reduce((sum, name) => sum + fs.statSync(path.join(parts, name)).size, 0);
  assert(partBytes > 4 * GiB, 'More than 4 GiB must actually exist in partial files');
  assert(beforeRestart.downloaded > 4 * GiB);
  mark('Actual transfer paused/resumed, then process terminated after more than 4 GiB was saved');
  await launch(); await setPhase(2);
  await command(['resume', added.id]);
  await until(() => { const current = job(added.id); assert.notEqual(current.status, 'failed', current.error); return current.status === 'complete'; }, 'large recovery and publication', 300000);
  const final = job(added.id);
  if (dockerOrigin) requests.splice(0, requests.length, ...await (await fetch(`${dockerOrigin}/requests`)).json());
  assert(requests.some(r => r.phase === 2 && r.start === partBytes && r.start > 4 * GiB), 'Resume must use actual >32-bit saved offset');
  assert.equal(final.total, size); assert.equal(final.downloaded, size); assert.equal(fs.statSync(final.saved_path).size, size);
  const actual = await fileHash(final.saved_path), expected = await expectedHash();
  assert.equal(actual, expected); assert.equal(final.integrity.actual, expected);
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 1);
  await until(() => !fs.existsSync(parts), 'completion cache cleanup');
  assert.deepEqual(fs.readdirSync(path.dirname(final.saved_path)), [path.basename(final.saved_path)]);
  mark('Exact SHA-256 and byte count verified after >4 GiB resume; one job, one file, no partial cache');
  // Only the verified generated payload is removed; retain reports and job history.
  assert(path.resolve(final.saved_path).startsWith(path.resolve(home, 'downloads') + path.sep));
  fs.unlinkSync(final.saved_path);
  return {bytes: size, pausedBytes: paused, crashSavedBytes: partBytes, sha256: actual, generatedPayloadRemoved: true};
}
async function combined() {
  await startMcp();
  const request = randomUUID(), url = `${origin}/combined.bin`;
  const [viaCli, viaMcp] = await Promise.all([
    command(['add', url, '--request-id', request]),
    tool('add_downloads', {urls: [url], request_id: request}),
  ]);
  const id = viaCli[0].id; assert.equal(viaMcp.receipt.result[0].id, id);
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 1);
  await until(() => job(id).downloaded >= MiB, 'combined download checkpoint');
  const lua = await saveScript(`for i=1,12 do\n xd.pause('${id}')\n local ok, err = pcall(xd.resume, '${id}')\n if not ok then assert(tostring(err):find('Pause the download before changing it',1,true), tostring(err)) end\n xd.sleep(30)\nend\nprint('combined Lua done')`, 'lua');
  const ps = await saveScript(`for ($i=0; $i -lt 12; $i++) {\n Invoke-DownloadEngine pause '${id}' | Out-Null\n try { Invoke-DownloadEngine resume '${id}' | Out-Null } catch { if ($_.ToString() -notlike '*Pause the download before changing it*') { throw } }\n Start-Sleep -Milliseconds 30\n}\nWrite-Output 'combined PowerShell done'`, 'power_shell');
  const [a, b] = await Promise.all([command(['scripts', 'run', lua.id]), command(['scripts', 'run', ps.id])]);
  let busyRejections = 0;
  for (let i = 0; i < 12; i++) {
    const results = await Promise.all([
      cli(['resume', id, '--no-start'], true), tool('pause_download', {id}),
      tool('resume_download', {id}, true), command(['pause', id]),
    ]);
    busyRejections += results.filter(result => result.busy).length;
    assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 1);
  }
  for (const run of [a, b]) await until(() => {
    const current = JSON.parse(db.prepare('SELECT data FROM script_runs WHERE id=?').get(run.id).data);
    assert(!['failed','timed_out','interrupted'].includes(current.state), JSON.stringify(current));
    return current.state === 'succeeded';
  }, 'both concurrent scripts finish');
  await tool('pause_all'); await until(() => job(id).status === 'paused', 'final pause barrier');
  const checkpoint = job(id).downloaded; await delay(600); assert.equal(job(id).downloaded, checkpoint);
  assert.equal(db.prepare("SELECT COUNT(*) AS n FROM control_requests WHERE state IN ('pending','running')").get().n, 0);
  mark('CLI, MCP, Lua and PowerShell concurrent Pause/Resume settle; same request ID adds only one job');
  await setPhase(1); await tool('resume_download', {id});
  await until(() => { const current=job(id); assert.notEqual(current.status,'failed',current.error); return current.status==='complete'; }, 'combined file completes');
  const final=job(id), actual=await fileHash(final.saved_path); assert.equal(actual,await expectedHash());
  await until(() => !fs.existsSync(path.join(home,'data/parts',id)), 'combined cache cleaned');
  assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n,1);
  assert.deepEqual(fs.readdirSync(path.dirname(final.saved_path)),[path.basename(final.saved_path)]);
  mark('Completion after conflicting controls has the exact checksum, one file and no stuck pending commands');
  return {bytes:size,sha256:actual,busyRejections,controlRequests:db.prepare('SELECT COUNT(*) AS n FROM control_requests').get().n};
}
(async () => {
  if (dockerOrigin) {
    origin=dockerOrigin;
    const health=await (await fetch(`${origin}/health`)).json(); assert.equal(health.size,size); assert.equal(health.mode,mode);
    await setPhase(0);
  } else {
    server.listen(0,'127.0.0.1'); await once(server,'listening'); origin=`http://127.0.0.1:${server.address().port}`;
  }
  try {
    await cli(['list']);
    db=openProfileDatabase(path.join(home,'data/queue.sqlite3')); db.exec('PRAGMA busy_timeout=5000');
    db.exec(`CREATE TABLE IF NOT EXISTS connection_settings(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL)`);
    db.prepare('INSERT OR REPLACE INTO connection_settings VALUES(1,?)').run(JSON.stringify({default_connections:1,sites:[]}));
    await launch();
    const result=mode==='large'?await large():await combined();
    if(dockerOrigin) requests.splice(0,requests.length,...await (await fetch(`${origin}/requests`)).json());
    const report={success:true,binary,home,mode,server:dockerOrigin?'Docker Linux fixture; native Windows client':'native loopback fixture',checks,result,requests};
    fs.writeFileSync(path.join(home,'verification.json'),JSON.stringify(report,null,2));
    console.log(JSON.stringify({success:true,home,checks,result},null,2));
  } catch(error) {
    fs.writeFileSync(path.join(home,'failure.json'),JSON.stringify({error:error.stack,checks,requests},null,2)); throw error;
  } finally {
    const exits=[];
    for(const process of processes) if(process.exitCode===null && process.signalCode===null) {exits.push(once(process,'exit'));process.kill();}
    await Promise.allSettled(exits); app=null; db?.close();
    if(server.listening) {server.closeAllConnections(); await new Promise(resolve=>server.close(resolve));}
  }
})().catch(error=>{console.error(error); process.exitCode=1;});

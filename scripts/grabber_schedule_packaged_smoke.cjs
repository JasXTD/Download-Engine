// Exercise the actual portable desktop against loopback HTML and isolated data.
// Stops only the child processes it starts; the intentional stop tests restart durability.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { randomUUID } = require('node:crypto');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');
const { DatabaseSync } = require('node:sqlite');

(async () => {
  const root = path.resolve(__dirname, '..');
  const packageRoot = path.resolve(process.argv[2] || path.join(root, 'dist', 'IDM-EX-UnsignedDev'));
  const home = path.join(root, 'artifacts', `scheduled-grabber-package-${randomUUID().slice(0, 8)}`);
  fs.mkdirSync(path.join(home, 'data'), { recursive: true });
  let html = "<a href='one.pdf'>one</a><a href='two.pdf'>two</a>";
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push({ method: req.method, url: req.url, authenticated: Boolean(req.headers.cookie || req.headers.authorization) });
    res.writeHead(200, { 'Content-Type': 'text/html', 'Content-Length': Buffer.byteLength(html) });
    res.end(html);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const db = new DatabaseSync(path.join(home, 'data', 'queue.sqlite3'));
  db.exec(`PRAGMA busy_timeout=5000;
    CREATE TABLE grabber_projects(id TEXT PRIMARY KEY,name TEXT NOT NULL UNIQUE COLLATE NOCASE,revision INTEGER NOT NULL,settings TEXT NOT NULL);
    CREATE TABLE grabber_schedules(project_id TEXT PRIMARY KEY REFERENCES grabber_projects(id) ON DELETE CASCADE,enabled INTEGER NOT NULL DEFAULT 0,next_at INTEGER,every_hours INTEGER,run_id TEXT,last_at INTEGER,message TEXT NOT NULL DEFAULT '');`);
  const project = randomUUID();
  db.prepare('INSERT INTO grabber_projects VALUES(?,?,1,?)').run(project, 'Packaged scan fixture', JSON.stringify({ url: `http://127.0.0.1:${server.address().port}/`, extensions: 'pdf', depth: 0, page_limit: 1, queue: 'main', destination: 'downloads' }));
  db.prepare('INSERT INTO grabber_schedules(project_id,enabled,next_at,every_hours) VALUES(?,1,?,1)').run(project, Math.floor(Date.now() / 1000) - 1);
  let child, childError;
  function launch() {
    const env = { ...process.env, IDM_EX_HOME: home };
    for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.includes('SCREENSHOT_TO')) delete env[key];
    child = spawn(path.join(packageRoot, 'de.exe'), [], { env, windowsHide: true, stdio: 'ignore' });
    childError = undefined;
    child.on('error', error => { childError = error; });
  }
  async function stop() {
    if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return;
    const ended = once(child, 'exit');
    child.kill();
    await ended;
    child = undefined;
  }
  async function pending(count) {
    const deadline = Date.now() + 15000;
    while (Date.now() < deadline) {
      assert.equal(childError, undefined, String(childError));
      assert.equal(child.exitCode, null, 'desktop exited unexpectedly');
      const exists = db.prepare("SELECT COUNT(*) AS n FROM sqlite_master WHERE type='table' AND name='grabber_pending'").get().n;
      if (exists && db.prepare('SELECT COUNT(*) AS n FROM grabber_pending').get().n === count) return;
      await delay(25);
    }
    throw new Error(`Timed out waiting for ${count} pending discoveries`);
  }
  try {
    launch();
    await pending(2);
    assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 0);
    const first = db.prepare('SELECT id FROM grabber_pending ORDER BY id').all().map(row => row.id);
    await stop();
    html += "<a href='three.pdf'>three</a><a href='one.pdf#duplicate'>same</a>";
    db.prepare('UPDATE grabber_schedules SET next_at=? WHERE project_id=?').run(Math.floor(Date.now() / 1000) - 1, project);
    launch();
    await pending(3);
    const second = db.prepare('SELECT id FROM grabber_pending ORDER BY id').all().map(row => row.id);
    assert(first.every(id => second.includes(id)), 'old pending discoveries were replaced');
    assert.equal(db.prepare('SELECT COUNT(*) AS n FROM grabber_seen').get().n, 3);
    assert.equal(db.prepare('SELECT COUNT(*) AS n FROM jobs').get().n, 0);
    assert.deepEqual(requests, [{ method: 'GET', url: '/', authenticated: false }, { method: 'GET', url: '/', authenticated: false }]);
    const schedule = db.prepare('SELECT next_at,run_id,message FROM grabber_schedules').get();
    assert(schedule.next_at > Math.floor(Date.now() / 1000));
    assert.equal(schedule.run_id, null);
    assert(schedule.message.includes('1 new files'));
    const report = { home, packageRoot, pending: 3, seen: 3, jobs: 0, requests, restartPreservedPending: true, schedule };
    fs.writeFileSync(path.join(home, 'verification.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally {
    await stop();
    db.close();
    server.closeAllConnections();
    server.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });

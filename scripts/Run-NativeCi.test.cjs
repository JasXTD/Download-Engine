'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const wrapper = path.join(__dirname, 'Run-NativeCi.cjs');

function fixture(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'download-engine-ci-report-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test('successful commands leave no failure artifact', t => {
  const dir = fixture(t);
  const result = spawnSync(process.execPath, [wrapper, 'success', dir, process.execPath, '-e', 'console.log("build passed")'], { encoding: 'utf8' });
  assert.equal(result.status, 0);
  assert.match(result.stdout, /build passed/);
  assert.deepEqual(fs.readdirSync(dir), []);
});

test('failure preserves exit code and redacts report and forwarded output', t => {
  const dir = fixture(t);
  const samples = [
    'error: sample-secret-12345',
    'contact test.person@example.invalid',
    '/Users/runner/work/project/src/main.rs:3',
    '/private/var/folders/aa/tmp123/config.log',
    'https://fixture:sample-pass@invalid.example/build',
    'Authorization: Bearer sample-bearer-value',
    'token=sample-assigned-token',
    '-----BEGIN PRIVATE KEY-----', 'sample-private-material', '-----END PRIVATE KEY-----',
    'error: useful compiler information',
  ];
  const code = `for (const line of ${JSON.stringify(samples)}) console.error(line); process.exitCode=7;`;
  const result = spawnSync(process.execPath, [wrapper, 'failure', dir, process.execPath, '-e', code], {
    encoding: 'utf8', env: { ...process.env, CI_SAMPLE_SECRET: 'sample-secret-12345', GITHUB_WORKSPACE: '/Users/runner/work/project', GITHUB_ACTIONS: 'true' },
  });
  assert.equal(result.status, 7);
  const raw = fs.readFileSync(path.join(dir, 'failure.json'), 'utf8');
  const report = JSON.parse(raw);
  assert.equal(report.exitCode, 7);
  for (const secret of ['sample-secret-12345', 'test.person@example.invalid', '/Users/runner', '/private/var/folders', 'fixture:sample-pass', 'sample-bearer-value', 'sample-assigned-token', 'sample-private-material']) {
    assert.ok(!raw.includes(secret), `report leaked ${secret}`);
    assert.ok(!result.stdout.includes(secret), `output leaked ${secret}`);
  }
  assert.match(raw, /useful compiler information/);
  assert.match(result.stdout, /::stop-commands::native-ci-/);
});

test('large and unterminated output remains bounded', t => {
  const dir = fixture(t);
  const code = 'console.log("x".repeat(100000)); for(let i=0;i<500;i++) console.log("error: fixture "+i+" "+"y".repeat(900)); process.stderr.write("final error"); process.exitCode=9;';
  const result = spawnSync(process.execPath, [wrapper, 'bounded', dir, process.execPath, '-e', code], { encoding: 'utf8', maxBuffer: 2 * 1024 * 1024 });
  assert.equal(result.status, 9);
  const raw = fs.readFileSync(path.join(dir, 'bounded.json'), 'utf8');
  assert.ok(Buffer.byteLength(raw) < 70000);
  assert.match(raw, /final error/);
  assert.doesNotMatch(result.stdout, /x{100}/);
});

test('a missing command produces a failed report', t => {
  const dir = fixture(t);
  const result = spawnSync(process.execPath, [wrapper, 'spawn-error', dir, path.join(dir, 'missing-command')], { encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.equal(JSON.parse(fs.readFileSync(path.join(dir, 'spawn-error.json'), 'utf8')).exitCode, 1);
});

test('interruption saves a report before the child close event', t => {
  const dir = fixture(t);
  const report = path.join(dir, 'interrupted.json');
  const code = `
    process.argv = [process.execPath, ${JSON.stringify(wrapper)}, 'interrupted', ${JSON.stringify(dir)}, process.execPath, '-e', 'setInterval(() => {}, 1000)'];
    require(${JSON.stringify(wrapper)});
    process.emit('SIGTERM');
    const report = JSON.parse(require('node:fs').readFileSync(${JSON.stringify(report)}, 'utf8'));
    require('node:assert/strict').equal(report.signal, 'SIGTERM');
    require('node:assert/strict').equal(report.exitCode, 1);
  `;
  const result = spawnSync(process.execPath, ['-e', code], { encoding: 'utf8', timeout: 10000 });
  assert.ifError(result.error);
  assert.equal(result.status, 1);
  assert.equal(result.stderr, '');
  assert.equal(JSON.parse(fs.readFileSync(report, 'utf8')).signal, 'SIGTERM');
});

test('a silent stalled command fails with a bounded diagnostic', t => {
  const dir = fixture(t);
  const result = spawnSync(process.execPath, [wrapper, 'stalled', dir, process.execPath, '-e', 'setInterval(() => {}, 1000)'], {
    encoding: 'utf8', timeout: 10000, env: { ...process.env, DOWNLOAD_ENGINE_CI_IDLE_TIMEOUT_MS: '1000' },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 1);
  const report = JSON.parse(fs.readFileSync(path.join(dir, 'stalled.json'), 'utf8'));
  assert.equal(report.signal, 'SIGTERM');
  assert.ok(report.output.some(item => item.stream === 'watchdog' && item.text.includes('No command output')));
});

test('ongoing command output refreshes the inactivity limit', t => {
  const dir = fixture(t);
  const code = 'let n=0; const timer=setInterval(() => {console.log(++n); if(n===5) clearInterval(timer);}, 500);';
  const result = spawnSync(process.execPath, [wrapper, 'active', dir, process.execPath, '-e', code], {
    encoding: 'utf8', timeout: 10000, env: { ...process.env, DOWNLOAD_ENGINE_CI_IDLE_TIMEOUT_MS: '1500' },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0);
  assert.deepEqual(fs.readdirSync(dir), []);
});

'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { fixturePaths } = require('./Run-MacTests.cjs');

function fixture(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'download-engine-mac-test-')));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const workspace = path.join(root, 'checkout');
  const runnerTemp = path.join(root, 'runner');
  const target = path.join(workspace, 'target', 'debug');
  fs.mkdirSync(target, { recursive: true });
  fs.mkdirSync(runnerTemp);
  const keychain = path.join(runnerTemp, 'download-engine-tests.keychain-db');
  fs.writeFileSync(keychain, 'synthetic fixture');
  const executable = path.join(target, 'test-program');
  fs.writeFileSync(executable, 'synthetic fixture', { mode: 0o700 });
  return { workspace, runnerTemp, target, executable, keychain };
}

test('only existing compiled binaries in this checkout receive fixture access', t => {
  const f = fixture(t);
  const result = fixturePaths(f.workspace, f.runnerTemp, [f.executable, f.executable]);
  assert.equal(result.keychain, fs.realpathSync(f.keychain));
  assert.deepEqual(result.programs, [fs.realpathSync(f.executable)]);
  assert.throws(() => fixturePaths(f.workspace, f.runnerTemp, []), /number/);
  assert.throws(() => fixturePaths(f.workspace, f.runnerTemp, [path.join(f.target, 'missing')]), /ENOENT/);
});

test('other executables and prefix-collision paths cannot access the test key', t => {
  const f = fixture(t);
  const outside = path.join(f.workspace, 'target', 'debug-other');
  fs.mkdirSync(outside);
  const binary = path.join(outside, 'another-program');
  fs.writeFileSync(binary, 'synthetic fixture', { mode: 0o700 });
  assert.throws(() => fixturePaths(f.workspace, f.runnerTemp, [binary]), /Only binaries/);
  assert.throws(() => fixturePaths(f.workspace, f.runnerTemp, [f.target]), /regular file/);
});

test('a missing dedicated test keychain never falls back to the user keychain', t => {
  const f = fixture(t);
  fs.unlinkSync(f.keychain);
  assert.throws(() => fixturePaths(f.workspace, f.runnerTemp, [f.executable]), /ENOENT/);
});

test('linked executables and keychains are rejected', { skip: process.platform === 'win32' }, t => {
  const f = fixture(t);
  const link = path.join(f.target, 'linked-program');
  fs.symlinkSync(f.executable, link);
  assert.throws(() => fixturePaths(f.workspace, f.runnerTemp, [link]), /regular file/);
  fs.unlinkSync(f.keychain);
  fs.symlinkSync(f.executable, f.keychain);
  assert.throws(() => fixturePaths(f.workspace, f.runnerTemp, [f.executable]), /regular test keychain/);
});

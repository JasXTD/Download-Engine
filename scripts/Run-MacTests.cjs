#!/usr/bin/env node
'use strict';

// CI only: Cargo test executables are separate, ad-hoc signed applications.
// Share one synthetic master key among those exact programs without a GUI
// Keychain approval prompt. Never change the application's normal access policy.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const { randomBytes } = require('node:crypto');
const { spawn, spawnSync } = require('node:child_process');

const account = 'master-key-v1';
const service = 'com.xetro.downloadengine.protected-data';
const testArgs = ['test', '--locked', '--offline', '--all-targets', '-j', '2'];

function within(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative !== '' && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function fixturePaths(workspace, runnerTemp, executables) {
  const root = fs.realpathSync(workspace);
  const target = fs.realpathSync(path.join(root, 'target', 'debug'));
  assert(within(root, target), 'Test target must stay inside the checkout');
  const temporary = fs.realpathSync(runnerTemp);
  const keychain = path.join(temporary, 'download-engine-tests.keychain-db');
  assert(fs.lstatSync(keychain).isFile(), 'Expected the dedicated regular test keychain');
  assert.equal(fs.realpathSync(keychain), keychain, 'Test keychain must not be a link');
  const programs = [...new Set(executables)].map(filename => {
    assert.equal(typeof filename, 'string');
    assert(path.isAbsolute(filename) && !/[\x00-\x1f\x7f]/.test(filename), 'Invalid test executable path');
    assert(fs.lstatSync(filename).isFile(), 'Test executable must be a regular file');
    const resolved = fs.realpathSync(filename);
    assert.equal(resolved, filename, 'Test executable must not be a link');
    assert(within(target, resolved), 'Only binaries built in this test target may access the fixture');
    fs.accessSync(resolved, fs.constants.X_OK);
    return resolved;
  });
  assert(programs.length > 0 && programs.length <= 128, 'Unexpected number of compiled test programs');
  return { keychain, programs };
}

async function compileTests() {
  const programs = [];
  const child = spawn('cargo', [...testArgs, '--no-run', '--message-format=json-render-diagnostics'], {
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  let invalid = false;
  const lines = readline.createInterface({ input: child.stdout });
  lines.on('line', line => {
    try {
      assert(line.length < 4 * 1024 * 1024);
      const message = JSON.parse(line);
      if (message.reason === 'compiler-artifact' && message.executable) programs.push(message.executable);
      if (message.reason === 'compiler-message' && message.message?.rendered) process.stderr.write(message.message.rendered);
    } catch {
      invalid = true;
      child.kill('SIGTERM');
    }
  });
  await new Promise((resolve, reject) => {
    child.on('error', reject);
    child.on('close', code => {
      if (invalid) reject(new Error('Invalid Cargo test-build record'));
      else if (code !== 0) reject(new Error(`Cargo test build failed (${code})`));
      else resolve();
    });
  });
  return programs;
}

function security(args) {
  // Do not print command arguments: creating this fixture passes a random key.
  const result = spawnSync('/usr/bin/security', args, { stdio: ['ignore', 'ignore', 'pipe'], encoding: 'utf8', timeout: 30000 });
  assert(!result.error && result.status === 0, `Test keychain operation failed (${result.status ?? 'launch/timeout'})`);
}

async function main() {
  assert(process.platform === 'darwin' && process.env.GITHUB_ACTIONS === 'true', 'This helper runs only in the disposable macOS CI job');
  assert(process.env.RUNNER_TEMP && process.env.GITHUB_WORKSPACE, 'Missing CI scratch directories');
  assert.equal(fs.realpathSync(process.cwd()), fs.realpathSync(process.env.GITHUB_WORKSPACE));
  const { keychain, programs } = fixturePaths(process.cwd(), process.env.RUNNER_TEMP, await compileTests());
  const key = randomBytes(32);
  try {
    // No -A (allow every application) or -U (overwrite an existing item).
    security(['add-generic-password', '-a', account, '-s', service,
      ...programs.flatMap(program => ['-T', program]), '-X', key.toString('hex'), keychain]);
  } finally {
    key.fill(0);
  }
  console.log(`Prepared isolated Keychain access for ${programs.length} compiled test programs.`);
  try {
    const result = spawnSync('cargo', [...testArgs, '--', '--test-threads=2'], { stdio: 'inherit' });
    if (result.error) throw new Error('Could not launch native regression tests');
    process.exitCode = result.status === 0 ? 0 : 1;
  } finally {
    // These disposable profiles are not reused by the package/rendering checks.
    // The workflow also deletes the entire test keychain on interruption.
    security(['delete-generic-password', '-a', account, '-s', service, keychain]);
    console.log('Removed the synthetic CI master key.');
  }
}

module.exports = { fixturePaths };
if (require.main === module) main().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});

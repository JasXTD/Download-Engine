'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const project = path.resolve(__dirname, '..');
const read = name => fs.readFileSync(path.join(project, name), 'utf8').replace(/\r\n/g, '\n');
const mac = read('.github/workflows/macos-preview.yml');
const linux = read('.github/workflows/linux-release.yml');
const linuxScript = read('scripts/Run-LinuxCi.sh');

// Inspect specific workflow steps, not arbitrary YAML or GitHub expressions.
function step(workflow, name) {
  const result = workflow.split(/\n(?=      - (?:name|uses):)/)
    .find(block => block.startsWith(`      - name: ${name}\n`));
  assert.ok(result, `Missing workflow step: ${name}`);
  return result;
}
function cachePaths(block) {
  const match = block.match(/^          path: \|\n((?: {12}[^\n]+\n?)+)/m);
  assert.ok(match, 'Cache paths must be explicitly listed');
  return match[1].trim().split('\n').map(line => line.trim());
}
const cleanCommands = step(mac, 'Prepare Rust dependency cache').split('\n')
  .map(line => line.trim()).filter(line => line.startsWith('cargo clean '));

test('cache scope excludes credentials, profiles, packages and app executables', () => {
  for (const workflow of [mac, linux]) {
    const restore = step(workflow, 'Restore Rust dependencies');
    const save = step(workflow, 'Save Rust dependencies');
    const paths = cachePaths(restore);
    assert.deepEqual(cachePaths(save), paths);
    assert.equal(new Set(paths).size, paths.length);
    const normalized = paths.map(entry => entry
      .replace(/^~\/\.cargo\/registry\//, 'registry/')
      .replace(/^\$\{\{ runner.temp \}\}\/rust-cache\/cargo-registry\//, 'registry/')
      .replace(/^\$\{\{ runner.temp \}\}\/rust-cache\//, '')
      .replace(/^torrent-target\//, 'torrent/target/'));
    for (const entry of normalized) {
      assert.match(entry, /^(?:registry\/(?:cache|index|src)|(?:torrent\/)?target\/(?:debug|release)\/(?:\.fingerprint|build|deps))$/);
    }
    for (const required of ['registry/cache', 'registry/src', 'target/debug/deps', 'target/release/deps', 'torrent/target/debug/deps']) {
      assert.ok(normalized.includes(required), `Missing dependency cache: ${required}`);
    }
    for (const block of [restore, save]) {
      assert.match(block, /uses: actions\/cache\/(?:restore|save)@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6\.1\.0/);
      assert.doesNotMatch(block, /enableCrossOsArchive: true|fail-on-cache-miss: true/);
    }
    assert.match(save, /if: success\(\) && github.ref == 'refs\/heads\/main'/);
    assert.doesNotMatch(workflow, /pull_request|workflow_run|contents: write/);
    assert.match(workflow, /persist-credentials: false/);
  }
});

test('cache hits retain all test gates, native identities and Linux isolation', () => {
  for (const name of ['Native toolchain', 'Torrent regression tests', 'Native regression tests', 'Package preview', 'Build native UI test executable', 'Native UI rendering checks']) {
    assert.doesNotMatch(step(mac, name), /cache-hit|cache-matched-key/);
    assert.ok(mac.indexOf(`name: ${name}`) < mac.indexOf('name: Save Rust dependencies'));
  }
  assert.match(step(mac, 'Save Rust dependencies'), /validation == 'full'/);
  assert.match(step(mac, 'Rust cache identity'), /rustc -vV/);
  assert.match(step(mac, 'Rust cache identity'), /xcodebuild -version/);
  assert.match(step(mac, 'Rust cache identity'), /xcrun clang --version/);
  assert.match(step(mac, 'Restore Rust dependencies'), /matrix.runner.*runner.arch.*outputs.toolchain/);
  assert.match(step(linux, 'Rust cache identity'), /RUST_IMAGE/);
  assert.match(step(linux, 'Restore Rust dependencies'), /linux-ci-inputs.lock/);
  assert.match(read('scripts/Run-MacTests.cjs'), /'--all-targets'/);
  assert.match(linux, /docker network disconnect bridge/);
  assert.match(linux, /--cap-drop ALL --security-opt no-new-privileges/);
  assert.doesNotMatch(step(linux, 'Isolated Debian build and package checks').match(/^        if:.*$/m)?.[0] || '', /cache/);
  assert.match(linuxScript, /cargo test --locked --offline --manifest-path torrent\/Cargo.toml/);
  assert.match(linuxScript, /bash scripts\/Test-NativePackage.sh/);
  assert.match(linuxScript, /mkdir -p \/source \/output/);
  assert.equal(cleanCommands.length, 3);
  for (const command of cleanCommands) assert.ok(linuxScript.includes(command));
});

test('a warm dependency cache rebuilds current app code and still catches failing tests', { timeout: 180_000 }, t => {
  // A real, dependency-free Cargo fixture. Never touch the project's target or
  // install/fetch anything; exercise both target directories and build profiles.
  const temporary = fs.realpathSync(os.tmpdir());
  const root = fs.mkdtempSync(path.join(temporary, 'de-rust-cache-'));
  t.after(() => {
    assert.equal(path.dirname(root), temporary);
    fs.rmSync(root, { recursive: true, force: true });
  });
  const write = (name, value) => {
    const file = path.join(root, name);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, value);
  };
  write('Cargo.toml', '[package]\nname = "download-engine"\nversion = "0.1.0"\nedition = "2024"\n[dependencies]\nde-torrent = { path = "torrent" }\n');
  write('torrent/Cargo.toml', '[package]\nname = "de-torrent"\nversion = "0.1.0"\nedition = "2024"\n[dependencies]\ncache-dependency = { path = "../dependency" }\n');
  write('dependency/Cargo.toml', '[package]\nname = "cache-dependency"\nversion = "0.1.0"\nedition = "2024"\n');
  write('dependency/src/lib.rs', 'pub fn value() -> u8 { 42 }\n');
  write('torrent/src/lib.rs', 'pub fn value() -> u8 { cache_dependency::value() }\n#[test] fn torrent_test() { assert!(value() > 0); }\n');
  const app = expected => `pub fn value() -> u8 { de_torrent::value() + 1 }\n#[test] fn current_source() { assert_eq!(value(), ${expected}); }\n`;
  write('src/lib.rs', app(43));
  const cargo = (args, expected = 0) => {
    const isTorrent = args.includes('torrent/Cargo.toml');
    const result = spawnSync('cargo', [...args, '--target-dir', path.join(root, isTorrent ? 'torrent/target' : 'target')], {
      cwd: root, encoding: 'utf8', timeout: 45_000,
      env: { ...process.env, CARGO_NET_OFFLINE: 'true', CARGO_INCREMENTAL: '0', CARGO_PROFILE_DEV_DEBUG: '0' },
    });
    assert.ifError(result.error);
    const output = result.stdout + result.stderr;
    assert.equal(result.status, expected, output);
    return output;
  };
  // The initial test commands create their local lockfiles offline.
  cargo(['test', '--offline', '--lib', '--verbose']);
  cargo(['test', '--offline', '--manifest-path', 'torrent/Cargo.toml', '--lib']);
  cargo(['build', '--locked', '--offline', '--release', '--lib']);
  const profiles = ['target/debug', 'target/release', 'torrent/target/debug'];
  const dependencyFiles = profiles.flatMap(profile => fs.readdirSync(path.join(root, profile, 'deps'))
    .filter(name => /^libcache_dependency-.*\.rlib$/.test(name))
    .map(name => path.join(root, profile, 'deps', name)));
  assert.equal(dependencyFiles.length, 3);
  const timestamps = dependencyFiles.map(file => fs.statSync(file).mtimeMs);
  for (const command of cleanCommands) cargo(command.split(/\s+/).slice(1));
  for (const profile of profiles) {
    assert.ok(!fs.readdirSync(path.join(root, profile, 'deps')).some(name => /^(?:lib)?(?:download_engine|de_torrent)-/.test(name)), `App artifacts retained in ${profile}`);
    assert.ok(!fs.readdirSync(path.join(root, profile, '.fingerprint')).some(name => /^(?:download-engine|de-torrent)-/.test(name)), `App fingerprints retained in ${profile}`);
  }
  assert.deepEqual(dependencyFiles.map(file => fs.statSync(file).mtimeMs), timestamps);
  const warm = cargo(['test', '--locked', '--offline', '--lib', '--verbose']);
  assert.match(warm, /Fresh cache-dependency/);
  assert.match(warm, /Compiling download-engine/);
  assert.match(warm, /1 passed/);
  const manifest = path.join(root, 'Cargo.toml');
  const oldLock = fs.readFileSync(path.join(root, 'Cargo.lock'), 'utf8');
  fs.writeFileSync(manifest, fs.readFileSync(manifest, 'utf8').replace('version = "0.1.0"', 'version = "0.2.0"'));
  const lockedFailure = cargo(['test', '--locked', '--offline', '--lib'], 101);
  assert.match(lockedFailure, /lock file/);
  assert.match(lockedFailure, /--locked/);
  assert.equal(fs.readFileSync(path.join(root, 'Cargo.lock'), 'utf8'), oldLock);
  const newVersion = cargo(['test', '--offline', '--lib', '--verbose']);
  assert.match(newVersion, /Fresh cache-dependency/);
  assert.match(newVersion, /1 passed/);
  write('src/lib.rs', app(44));
  assert.match(cargo(['test', '--locked', '--offline', '--lib'], 101), /current_source.*FAILED/);
  // Changed local dependency source must also invalidate its cached build.
  write('dependency/src/lib.rs', 'pub fn value() -> u8 { 77 }\n');
  write('src/lib.rs', app(78));
  assert.match(cargo(['test', '--locked', '--offline', '--lib', '--verbose']), /1 passed/);
});

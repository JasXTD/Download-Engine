'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {createHash} = require('node:crypto');
const {prepare} = require('./prepare_bundled_plugins.cjs');
const root = path.resolve(__dirname, '..');
const artifacts = path.join(root, 'artifacts');
const scratch = fs.mkdtempSync(path.join(artifacts, 'bundled-plugins-test-'));
const cli = process.env.MEDIA_DEMO_CLI || path.join(root, 'target/release/de-cli.exe');
const ffmpeg = process.env.MEDIA_TEST_FFMPEG || path.join(root, 'tools/ffmpeg/ffmpeg.exe');
const hash = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
try {
  const first = prepare(path.join(scratch, 'first'), cli, ffmpeg);
  assert.deepEqual(fs.readdirSync(first.directory).sort(), ['README.md', 'all-debrid.de-extension.json', 'offline-episode', 'offline-episode.de-extension.json', 'premiumize.de-extension.json', 'real-debrid.de-extension.json']);
  for (const slug of ['all-debrid', 'real-debrid', 'premiumize']) {
    assert.equal(hash(path.join(first.directory, slug + '.de-extension.json')),
      hash(path.join(root, 'assets/plugins', slug + '.de-extension.json')));
  }
  assert.equal(JSON.parse(fs.readFileSync(first.package)).id, 'download-engine.offline-episode');
  assert.equal(hash(first.package), hash(path.join(first.directory, 'offline-episode/offline-episode.de-extension.json')));
  const bundle = path.join(first.directory, 'offline-episode');
  const inventory = fs.readFileSync(path.join(bundle, 'SHA256SUMS'), 'utf8').trim().split('\n');
  for (const line of inventory) {
    const [expected, relative] = line.split('  ');
    assert(relative && !relative.includes('..') && !path.isAbsolute(relative));
    assert.equal(hash(path.join(bundle, relative)), expected, relative);
  }
  const custom = path.join(first.directory, 'custom.de-extension.json');
  fs.writeFileSync(custom, 'Preserve this user-supplied package.');
  assert.throws(() => prepare(first.directory, cli, ffmpeg), /fresh staging folder/);
  assert.equal(fs.readFileSync(custom, 'utf8'), 'Preserve this user-supplied package.');
  const second = prepare(path.join(scratch, 'second'), cli, ffmpeg);
  assert.equal(hash(first.package), hash(second.package), 'The same tools and sources must not create same-version package conflicts');
  assert(!fs.existsSync(path.join(second.directory, 'custom.de-extension.json')));
  console.log(JSON.stringify({bundledFiles:inventory.length, deterministic:true, existingFolderProtected:true, cleanPackaging:true}));
} finally {
  assert(path.dirname(scratch) === artifacts && path.basename(scratch).startsWith('bundled-plugins-test-'));
  fs.rmSync(scratch, {recursive:true, force:true, maxRetries:8, retryDelay:150});
}

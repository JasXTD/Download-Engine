'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { readNotices, reviewedSupplement } = require('./LicenseNotices.cjs');

function fixture(t, revision) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'de-license-notices-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  fs.writeFileSync(path.join(directory, '.cargo_vcs_info.json'), JSON.stringify({ git: { sha1: revision } }));
  return directory;
}

test('notice discovery excludes Rust implementations and retains actual notices', t => {
  const directory = fixture(t);
  fs.mkdirSync(path.join(directory, 'src'));
  fs.mkdirSync(path.join(directory, 'fonts'));
  fs.writeFileSync(path.join(directory, 'src/copying.rs'), 'fn copy_object() {}');
  fs.writeFileSync(path.join(directory, 'src/license.rs'), 'fn license_menu() {}');
  fs.writeFileSync(path.join(directory, 'LICENSE-MIT'), 'fixture MIT notice');
  fs.writeFileSync(path.join(directory, 'fonts/Font.txt'), 'fixture font notice');
  const notices = readNotices(directory).map(([name]) => name.replaceAll('\\', '/')).sort();
  assert.deepEqual(notices, ['LICENSE-MIT', 'fonts/Font.txt']);
});

test('legacy block2 gets its exact upstream MIT copyright', t => {
  const directory = fixture(t, '4fc083f1c6d6784577e38b0ee8dbd344481e2fd2');
  const pkg = { name: 'block2', version: '0.5.1', license: 'MIT', repository: 'https://github.com/madsmtm/objc2' };
  const result = reviewedSupplement(pkg, directory);
  assert.equal(result.selectedLicense, 'MIT');
  assert.match(result.notices[0][1].toString('utf8'), /Copyright \(c\) Steven Sheldon/);
  assert.ok(result.notices.some(([name]) => name === 'NOTICE-PROVENANCE.md'));
  assert.throws(() => reviewedSupplement({ ...pkg, version: '0.5.2' }, directory), /Unreviewed/);
  assert.throws(() => reviewedSupplement({ ...pkg, license: 'Apache-2.0' }, directory), /Unexpected.*license/);
  fs.writeFileSync(path.join(directory, '.cargo_vcs_info.json'), JSON.stringify({ git: { sha1: '0'.repeat(40) } }));
  assert.throws(() => reviewedSupplement(pkg, directory), /Unexpected.*revision/);
  assert.equal(reviewedSupplement({ ...pkg, repository: 'https://example.invalid/objc2' }, directory), null);
});

test('modern objc2 retains MIT text and its workspace license explanation', t => {
  const directory = fixture(t, 'd7d2fa23ceaa5e6096c923b081040e5d81b3b9df');
  const result = reviewedSupplement({ name: 'objc2', version: '0.6.5', license: 'MIT', repository: 'https://github.com/madsmtm/objc2' }, directory);
  assert.equal(result.selectedLicense, 'MIT');
  assert.ok(result.notices.some(([name]) => name === 'UPSTREAM-LICENSE.md'));
});

test('trio-licensed frameworks select Apache and retain upstream explanation', t => {
  const directory = fixture(t, '7b1abfd750a2cacaea71d6a56ecfb83cb7de560b');
  const result = reviewedSupplement({ name: 'objc2-app-kit', version: '0.3.2', license: 'Zlib OR Apache-2.0 OR MIT', repository: 'https://github.com/madsmtm/objc2' }, directory);
  assert.equal(result.selectedLicense, 'Apache-2.0');
  assert.match(result.notices[0][1].toString('utf8'), /Apache License/);
  assert.ok(result.notices.some(([name]) => name === 'UPSTREAM-LICENSE.md'));
});

test('legacy dispatch supplemental notice requires the reviewed author and revision', t => {
  const revision = '82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748';
  const directory = fixture(t, revision);
  const pkg = { name: 'dispatch', version: '0.2.0', authors: ['Steven Sheldon'], license: 'MIT', repository: 'http://github.com/SSheldon/rust-dispatch' };
  assert.equal(reviewedSupplement(pkg, directory).selectedLicense, 'MIT');
  assert.throws(() => reviewedSupplement({ ...pkg, authors: ['Unreviewed author'] }, directory), /Unreviewed dispatch authors/);
  fs.writeFileSync(path.join(directory, '.cargo_vcs_info.json'), JSON.stringify({ git: { sha1: revision, dirty: true } }));
  assert.throws(() => reviewedSupplement(pkg, directory), /Unreviewed dispatch source/);
});

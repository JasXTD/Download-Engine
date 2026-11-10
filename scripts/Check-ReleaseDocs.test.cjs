'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { checkReleaseDocs } = require('./Check-ReleaseDocs.cjs');
const root = path.resolve(__dirname, '..');
const originals = Object.fromEntries(['Cargo.toml', 'README.md', 'docs/index.html']
  .map(file => [file, fs.readFileSync(path.join(root, file), 'utf8')]));
const version = originals['Cargo.toml'].match(/^version = "([^"]+)"/m)[1];
const oldVersion = '0.0.0';

function fixture(changes, action) {
  const parent = fs.realpathSync(os.tmpdir());
  const folder = fs.mkdtempSync(path.join(parent, 'de-release-docs-'));
  try {
    fs.mkdirSync(path.join(folder, 'docs'));
    for (const [file, source] of Object.entries(originals)) {
      fs.writeFileSync(path.join(folder, file), changes[file] ?? source);
    }
    return action(folder);
  } finally {
    assert.equal(path.dirname(folder), parent);
    assert.ok(path.basename(folder).startsWith('de-release-docs-'));
    fs.rmSync(folder, { recursive: true });
  }
}

function rejects(file, change, expected) {
  const content = change(originals[file]);
  assert.notEqual(content, originals[file], 'Mutation must change the fixture');
  fixture({ [file]: content }, folder => assert.throws(() => checkReleaseDocs(folder), expected));
}

test('current release documents pass without changing their bytes', () => {
  fixture({}, folder => {
    const result = checkReleaseDocs(folder);
    assert.equal(result.version, version);
    assert.equal(result.linkedAssets, 10);
    for (const [file, source] of Object.entries(originals)) {
      assert.equal(fs.readFileSync(path.join(folder, file), 'utf8'), source);
    }
  });
});

test('README current download label, tag and presence are checked independently', () => {
  rejects('README.md', s => s.replace(`[Download ${version}]`, `[Download ${oldVersion}]`), /README.md: Download label/);
  rejects('README.md', s => s.replace(`/tag/v${version}`, `/tag/v${oldVersion}`), /README.md:.*release link/);
  rejects('README.md', s => s.replace(`[Download ${version}]`, '[Get the app]'), /expected one current Download/);
  rejects('README.md', s => s.replace(`**New in ${version}:**`, `**New in ${oldVersion}:**`), /current-release highlights/);
  rejects('README.md', s => s.replace('<!-- current-release:start -->', ''), /marked current-release section/);
});

test('every marked current website label rejects a stale version', async t => {
  const labels = [...originals['docs/index.html'].matchAll(/<([a-z][a-z0-9]*)\b[^>]*\sdata-release-version(?=\s|>)[^>]*>([\s\S]*?)<\/\1\s*>/gi)];
  assert.ok(labels.length > 0);
  for (const [i, match] of labels.entries()) {
    await t.test(`label ${i + 1}`, () => {
      const replacement = match[0].replaceAll(version, oldVersion);
      rejects('docs/index.html', s => s.slice(0, match.index) + replacement + s.slice(match.index + match[0].length), /current release label/);
    });
  }
  rejects('docs/index.html', s => s.replaceAll(' data-release-version', ''), /missing data-release-version/);
});

test('all ten asset links reject stale tags, stale filenames and missing downloads', async t => {
  const urls = [...new Set(originals['docs/index.html'].match(/https:\/\/github\.com\/JasXTD\/Download-Engine\/releases\/download\/[^"<>]+/g))];
  assert.equal(urls.length, 10);
  for (const url of urls) {
    await t.test(url.slice(url.lastIndexOf('/') + 1), () => {
      rejects('docs/index.html', s => s.replaceAll(url, url.replace(`/v${version}/`, `/v${oldVersion}/`)), /invalid release asset/);
      const split = url.lastIndexOf('/') + 1;
      rejects('docs/index.html', s => s.replaceAll(url, url.slice(0, split) + url.slice(split).replace(version, oldVersion)), /invalid release asset/);
      rejects('docs/index.html', s => s.replaceAll(url, '#download'), /missing release download/);
    });
  }
});

test('metadata must have valid JSON and the current version and download URL', () => {
  rejects('docs/index.html', s => s.replace(`"softwareVersion": "${version}"`, `"softwareVersion": "${oldVersion}"`), /softwareVersion/);
  rejects('docs/index.html', s => s.replace(`"downloadUrl": "https://github.com/JasXTD/Download-Engine/releases/tag/v${version}"`, '"downloadUrl": "https://example.invalid/"'), /structured downloadUrl/);
  rejects('docs/index.html', s => s.replace('"softwareVersion":', '"softwareVersion"!'), /invalid JSON-LD/);
  rejects('docs/index.html', s => s.replace('"@type": "SoftwareApplication"', '"@type": "Thing"'), /expected one SoftwareApplication/);
});

test('unknown assets and old README release assets are rejected', () => {
  rejects('docs/index.html', s => s.replace(`SHA256SUMS-${version}`, `SHA256SUMS-${version}.zip`), /invalid release asset/);
  rejects('README.md', s => `${s}\n[Old installer](https://github.com/JasXTD/Download-Engine/releases/download/v${oldVersion}/DownloadEngine-${oldVersion}-UnsignedSetup.exe)\n`, /README.md: invalid release asset/);
});

test('historical feature versions remain allowed', () => {
  fixture({
    'README.md': originals['README.md'] + '\nAdded in 1.2.3: a historical feature.\n',
    'docs/index.html': originals['docs/index.html'].replace('</main>', '<p>Added in 1.2.3: a historical feature.</p></main>'),
  }, folder => assert.equal(checkReleaseDocs(folder).version, version));
});

test('a future Cargo bump fails until current docs and filenames change together', () => {
  const next = '99.98.97';
  rejects('Cargo.toml', s => s.replace(`version = "${version}"`, `version = "${next}"`), /Release documentation check failed/);
  fixture(Object.fromEntries(Object.entries(originals).map(([file, source]) => [file, source.replaceAll(version, next)])), folder => {
    assert.equal(checkReleaseDocs(folder).version, next);
  });
});

test('the CLI works from another directory and fails on stale or missing files', () => {
  fixture({}, folder => {
    const run = () => spawnSync(process.execPath, [path.join(__dirname, 'Check-ReleaseDocs.cjs'), folder], { cwd: os.tmpdir(), encoding: 'utf8', windowsHide: true });
    assert.equal(run().status, 0);
    fs.writeFileSync(path.join(folder, 'README.md'), originals['README.md'].replace(`[Download ${version}]`, `[Download ${oldVersion}]`));
    assert.equal(run().status, 1);
    fs.unlinkSync(path.join(folder, 'docs/index.html'));
    const missing = run();
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /ENOENT/);
  });
});

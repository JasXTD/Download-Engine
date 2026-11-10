'use strict';

// Offline release preflight shared by Windows packaging, Unix packaging and CI.
const fs = require('node:fs');
const path = require('node:path');

const releaseBase = 'https://github.com/JasXTD/Download-Engine/releases/';
const semver = /(?<![\d.])\d+\.\d+\.\d+(?!\d|\.\d)/g;

function checkReleaseDocs(root) {
  const read = file => fs.readFileSync(path.join(root, file), 'utf8');
  const cargo = read('Cargo.toml').match(/^\[package\]\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m)?.[1];
  const version = cargo?.match(/^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$/m)?.[1];
  if (!version) throw new Error('Cargo.toml: expected a numeric package release version');
  const readme = read('README.md');
  const html = read('docs/index.html');
  const errors = [];
  const expect = (condition, message) => { if (!condition) errors.push(message); };
  const releaseUrl = `${releaseBase}tag/v${version}`;
  const readmeDownloads = [...readme.matchAll(/\[Download (\d[^\]]*)\]\(([^)]+)\)/g)];
  expect(readmeDownloads.length === 1, 'README.md: expected one current Download version link');
  for (const [, label, url] of readmeDownloads) {
    expect(label === version && url === releaseUrl, `README.md: Download label and release link must use ${version}`);
  }
  const readmeHighlights = [...readme.matchAll(/<!-- current-release:start -->([\s\S]*?)<!-- current-release:end -->/g)];
  expect(readmeHighlights.length === 1, 'README.md: expected one marked current-release section');
  for (const [, content] of readmeHighlights) {
    const versions = content.match(semver) || [];
    expect(versions.length > 0 && versions.every(value => value === version),
      `README.md: current-release highlights must use ${version}`);
  }

  // Mark only current-release copy, so historical feature versions remain valid.
  const labels = [...html.matchAll(/<([a-z][a-z0-9]*)\b[^>]*\sdata-release-version(?=\s|>)[^>]*>([\s\S]*?)<\/\1\s*>/gi)];
  expect(labels.length > 0, 'docs/index.html: missing data-release-version labels');
  for (const [, , content] of labels) {
    const text = content.replace(/<[^>]*>/g, '').trim();
    const versions = text.match(semver) || [];
    expect(versions.length === 1 && versions[0] === version,
      `docs/index.html: current release label must use ${version}: ${text}`);
  }

  const blocks = [...html.matchAll(/<script\b[^>]*\btype\s*=\s*["']application\/ld\+json["'][^>]*>([\s\S]*?)<\/script\s*>/gi)];
  let applications = 0;
  for (const [, content] of blocks) {
    let data;
    try { data = JSON.parse(content); } catch { errors.push('docs/index.html: invalid JSON-LD'); continue; }
    if (data['@type'] !== 'SoftwareApplication') continue;
    applications++;
    expect(data.softwareVersion === version, `docs/index.html: softwareVersion must be ${version}`);
    expect(data.downloadUrl === releaseUrl, `docs/index.html: structured downloadUrl must use ${version}`);
  }
  expect(applications === 1, 'docs/index.html: expected one SoftwareApplication record');

  const assets = [
    [`DownloadEngine-${version}-UnsignedSetup.exe`, `DownloadEngine-${version}-Setup.exe`],
    [`DownloadEngine-${version}-Unsigned-windows-x64.zip`, `DownloadEngine-${version}-windows-x64.zip`],
    [`DownloadEngine-${version}-macos-arm64.zip`], [`DownloadEngine-${version}-macos-x64.zip`],
    [`DownloadEngine-${version}-linux-x64.tar.gz`],
    ...['chrome', 'edge', 'firefox'].map(browser => [`download-engine-${browser}-${version}.zip`]),
    [`DownloadEngine-${version}-Source.zip`], [`SHA256SUMS-${version}`],
  ];
  const allowedAssets = new Set(assets.flat());
  const linkedAssets = new Set();
  for (const [file, content] of [['README.md', readme], ['docs/index.html', html]]) {
    for (const [url] of content.matchAll(/https:\/\/github\.com\/JasXTD\/Download-Engine\/releases\/[^\s<>"')]+/g)) {
      if (url === `${releaseBase}latest`) continue;
      if (url.startsWith(`${releaseBase}tag/`)) {
        expect(url === releaseUrl, `${file}: stale release link: ${url}`);
      } else if (url.startsWith(`${releaseBase}download/`)) {
        const prefix = `${releaseBase}download/v${version}/`;
        const asset = url.slice(prefix.length);
        expect(url.startsWith(prefix) && allowedAssets.has(asset), `${file}: invalid release asset: ${url}`);
        if (file === 'docs/index.html' && url.startsWith(prefix)) linkedAssets.add(asset);
      } else {
        errors.push(`${file}: unexpected release URL: ${url}`);
      }
    }
  }
  for (const choices of assets) {
    expect(choices.some(name => linkedAssets.has(name)), `docs/index.html: missing release download: ${choices[0]}`);
  }
  if (errors.length) throw new Error(`Release documentation check failed:\n- ${errors.join('\n- ')}`);
  return { version, currentLabels: labels.length, linkedAssets: linkedAssets.size };
}

module.exports = { checkReleaseDocs };
if (require.main === module) {
  try {
    if (process.argv.length > 3) throw new Error('Usage: node scripts/Check-ReleaseDocs.cjs [ROOT]');
    console.log(JSON.stringify(checkReleaseDocs(path.resolve(process.argv[2] || path.join(__dirname, '..')))));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

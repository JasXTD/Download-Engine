#!/usr/bin/env node
'use strict';
// Exercise real rendering with synthetic screenshot views. Pixel data stays in
// the temporary runner profile and is deleted; only test results are printed.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { randomUUID } = require('node:crypto');
const [binaryArg] = process.argv.slice(2);
assert(binaryArg, 'Usage: Test-NativeUi.cjs BINARY');
const binary = path.resolve(binaryArg);
const failures = [];
// Hosted Mac desktops clamp large content areas (1080x800 became 1024x653).
// Test two sizes that fit their usable desktop; keep the strict pixel/aspect
// check below. Larger layouts are also covered by headless interaction tests.
const smallMacDesktop = process.platform === 'darwin' && process.env.GITHUB_ACTIONS === 'true';
for (const [view, appearance, width, height] of [
  ['preferences', 'dark', smallMacDesktop ? 960 : 1080, smallMacDesktop ? 640 : 800],
  ['preferences', 'light', 860, 600],
  ['mcp', 'dark', 860, 600],
  ['guide-cli', 'dark', 960, smallMacDesktop ? 640 : 720],
  ['crash-reports', 'dark', 860, 600],
  ['crash-reports', 'light', 960, smallMacDesktop ? 640 : 720],
]) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'download-engine-ui-'));
  const ppm = path.join(profile, 'capture.ppm');
  try {
    if (view === 'crash-reports' && appearance === 'dark') {
      const report = {schema:1,id:randomUUID(),session_started_utc:'2026-11-04T01:00:00.000Z',recorded_utc:'2026-11-04T01:01:00.000Z',
        version:JSON.parse(fs.readFileSync(path.resolve('extension/manifest.chromium.json'),'utf8')).version,
        os:process.platform === 'darwin'?'macos':process.platform === 'win32'?'windows':'linux',architecture:process.arch === 'arm64'?'aarch64':'x86_64',component:'desktop',kind:'rust_panic',
        location:{file:'src/desktop.rs',line:42,column:3}};
      const reports=path.join(profile,'data','logs','crashes');fs.mkdirSync(reports,{recursive:true});
      fs.writeFileSync(path.join(reports,`report-${report.id}.json`),JSON.stringify(report));
    }
    const result = spawnSync(binary, [], {
      encoding: 'utf8', timeout: 60000, maxBuffer: 2 * 1024 * 1024,
      env: { ...process.env, DOWNLOAD_ENGINE_HOME: profile, XDG_CACHE_HOME: path.join(profile, 'cache'), IDM_EX_SCREENSHOT_TO: ppm, IDM_EX_TEST_VIEW: view, IDM_EX_TEST_SIZE: `${width}x${height}`, IDM_EX_TEST_APPEARANCE: appearance },
    });
    if (result.stdout) process.stdout.write(result.stdout);
    if (result.stderr) process.stderr.write(result.stderr);
    if (result.error) throw result.error;
    assert.equal(result.status, 0, `Capture failed: ${view} (${result.signal || result.status})`);
    assert(!fs.existsSync(path.join(profile,'data','logs','crashes','desktop-session.json')), 'Normal desktop exit left a crash marker');
    assert(fs.statSync(ppm).size <= 64 * 1024 * 1024, 'Oversized capture');
    const data = fs.readFileSync(ppm);
    const header = /^P6\n(\d+) (\d+)\n255\n/.exec(data.subarray(0, 80).toString('ascii'));
    assert(header, 'Invalid capture header');
    const actualWidth = Number(header[1]), actualHeight = Number(header[2]);
    assert.equal(data.length - header[0].length, actualWidth * actualHeight * 3);
    // Retina displays legitimately produce more pixels than logical UI points.
    const scale = actualWidth / width;
    assert(scale >= 0.75 && scale <= 4 && Math.abs(actualHeight / height - scale) < 0.02,
      `Unexpected capture dimensions for ${view}, ${appearance}: requested ${width}x${height} points, captured ${actualWidth}x${actualHeight} pixels`);
    const colors = new Set();
    for (let at = header[0].length; at < data.length && colors.size < 32; at += 3) {
      colors.add(data[at] * 65536 + data[at + 1] * 256 + data[at + 2]);
    }
    assert(colors.size >= 16, 'Blank or incomplete UI capture');
    console.log(`Rendered ${view}, ${appearance}, ${actualWidth}x${actualHeight}; app exited normally.`);
  } catch (error) {
    // Inspect every representative size, but still fail the check if any fails.
    console.error(error.message);
    failures.push(error);
  } finally {
    assert(path.dirname(profile) === os.tmpdir());
    fs.rmSync(profile, { recursive: true, force: true });
  }
}
if (failures.length) throw new AggregateError(failures, `${failures.length} native UI capture check(s) failed`);

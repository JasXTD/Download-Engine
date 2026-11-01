#!/usr/bin/env node
'use strict';

// Run a build command without a shell and retain a bounded, redacted failure
// report. No command arguments or environment dumps are written to the report.
const { spawn } = require('node:child_process');
const { randomUUID } = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { StringDecoder } = require('node:string_decoder');

const [stage, directory, command, ...args] = process.argv.slice(2);
if (!/^[a-z0-9-]{1,48}$/.test(stage || '') || !directory || !command) {
  console.error('Usage: node Run-NativeCi.cjs STAGE REPORT_DIRECTORY COMMAND [ARGUMENTS...]');
  process.exit(2);
}

const secrets = Object.entries(process.env)
  .filter(([key, value]) => /token|secret|password|credential|authorization|private.key|access.key|api.key/i.test(key) && value && value.length >= 4)
  .flatMap(([, value]) => [value, encodeURIComponent(value)])
  .sort((a, b) => b.length - a.length);
const prefixes = [
  [process.env.GITHUB_WORKSPACE || process.cwd(), '<workspace>'],
  [process.env.RUNNER_TEMP, '<runner-temp>'],
  [process.env.TMPDIR, '<temp>'],
  [process.env.USERPROFILE || process.env.HOME, '<home>'],
].filter(([value]) => value).sort((a, b) => b[0].length - a[0].length);

function redact(value) {
  let text = value.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '').replace(/[\x00-\x08\x0b-\x1f\x7f]/g, '');
  for (const secret of secrets) text = text.split(secret).join('[redacted]');
  for (const [prefix, replacement] of prefixes) {
    text = text.split(prefix).join(replacement);
    text = text.split(prefix.replaceAll('\\', '/')).join(replacement);
  }
  return text
    .replace(/\b(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,})\b/g, '[redacted]')
    .replace(/\b(?:Bearer|Basic)\s+[A-Za-z0-9+/_.=~-]+/gi, '[authorization redacted]')
    .replace(/(https?:\/\/)[^\s/]+@/gi, '$1[redacted]@')
    .replace(/\b[A-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi, '[email redacted]')
    .replace(/\b(password|token|secret|authorization|api[_-]?key)\s*[:=]\s*.+/gi, '$1=[redacted]')
    .replace(/(?:\/private)?\/(?:tmp|var\/folders|Users)\/[^\s"'<>]+/g, '<temporary-path>')
    .replace(/[A-Z]:[\\/]Users[\\/][^\s"'<>]+/gi, '<user-path>');
}

const tail = [];
const issues = [];
let linesSeen = 0;
let signalReceived;
const stopToken = `native-ci-${randomUUID()}`;
if (process.env.GITHUB_ACTIONS === 'true') console.log(`::stop-commands::${stopToken}`);

function accept(line, stream) {
  const text = redact(line).slice(0, 1600);
  const item = { line: ++linesSeen, stream, text };
  tail.push(item);
  if (tail.length > 100) tail.shift();
  if (/error|failed|panic|not found|undefined|mismatch|checksum|fatal/i.test(text)) {
    issues.push(item);
    if (issues.length > 30) issues.shift();
  }
  process.stdout.write(`${text}\n`);
}

function capture(stream, name) {
  const decoder = new StringDecoder('utf8');
  let pending = '';
  let oversized = false;
  let privateKey = false;
  function consume(chunk, end = false) {
    pending += chunk;
    for (;;) {
      const at = pending.indexOf('\n');
      if (at < 0) break;
      const line = pending.slice(0, at);
      pending = pending.slice(at + 1);
      if (oversized || line.length > 16384) {
        accept('[oversized output line omitted]', name);
        oversized = false;
      } else if (/-----BEGIN .*PRIVATE KEY-----/.test(line)) {
        privateKey = true;
        accept('[private key omitted]', name);
      } else if (privateKey) {
        if (/-----END .*PRIVATE KEY-----/.test(line)) privateKey = false;
      } else {
        accept(line, name);
      }
    }
    if (pending.length > 16384) { pending = ''; oversized = true; }
    if (end && (pending || oversized)) {
      if (oversized) accept('[oversized output line omitted]', name);
      else if (!privateKey) {
        accept(/-----BEGIN .*PRIVATE KEY-----/.test(pending) ? '[private key omitted]' : pending, name);
      }
      pending = '';
    }
  }
  stream.on('data', chunk => consume(decoder.write(chunk)));
  stream.on('end', () => consume(decoder.end(), true));
}

const child = spawn(command, args, {
  shell: false,
  stdio: ['ignore', 'pipe', 'pipe'],
  env: { ...process.env, DOWNLOAD_ENGINE_CI_CAPTURE: '1' },
});
capture(child.stdout, 'stdout');
capture(child.stderr, 'stderr');
child.on('error', error => accept(error.message, 'spawn'));
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => {
  signalReceived = signal;
  child.kill(signal);
});
child.on('close', (code, signal) => {
  const exitCode = code === 0 && !signalReceived ? 0 : Number.isInteger(code) && code > 0 ? code : 1;
  if (process.env.GITHUB_ACTIONS === 'true') console.log(`::${stopToken}::`);
  if (exitCode !== 0) {
    const selected = [...new Map([...issues, ...tail].map(item => [item.line, item])).values()].sort((a, b) => a.line - b.line);
    // Bound the saved payload independently of the subprocess output volume.
    while (Buffer.byteLength(JSON.stringify(selected)) > 60000) selected.shift();
    const report = { stage, platform: process.platform, arch: process.arch, exitCode, signal: signal || signalReceived || null, linesSeen, output: selected };
    fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
    fs.writeFileSync(path.join(directory, `${stage}.json`), JSON.stringify(report, null, 2) + '\n', { mode: 0o600 });
    if (process.env.GITHUB_ACTIONS === 'true') console.log(`::error title=Native CI::${stage} failed (exit ${exitCode}). See the redacted diagnostic artifact.`);
  }
  process.exitCode = exitCode;
});

'use strict';

// Uses only Cargo's locked local cache. No network requests or package execution.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const check = process.argv.includes('--check');
const sha = data => crypto.createHash('sha256').update(data).digest('hex');
const cargo = process.env.CARGO || 'cargo';
const metadata = JSON.parse(execFileSync(cargo, ['metadata', '--offline', '--locked', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc'], { cwd: root, encoding: 'utf8', maxBuffer: 12 * 1024 * 1024, windowsHide: true }));
const lock = fs.readFileSync(path.join(root, 'Cargo.lock'), 'utf8');
const checksums = new Map();
for (const block of lock.split('[[package]]').slice(1)) {
  const name = /^name = "([^"]+)"/m.exec(block)?.[1];
  const version = /^version = "([^"]+)"/m.exec(block)?.[1];
  const checksum = /^checksum = "([a-f0-9]{64})"/m.exec(block)?.[1];
  if (checksum) checksums.set(`${name}-${version}`, checksum);
}
// Locally modified copies of published crates, used through [patch.crates-io].
// Their notices and the included source archive come from the exact upstream
// crate, verified by its crates.io checksum; the changes are described in the
// vendor directory, which is part of the source release.
const patchedCrates = {
  epaint: { version: '0.36.2', path: 'vendor/epaint', checksum: 'a0ec0308dc23130fc1a7d4623a8534c2ed069c6dc84e258ee4f3de99615aa1a3',
    change: 'bidirectional text layout, so Latin text and numbers keep their order inside right-to-left text' }
};
const workspace = new Set(metadata.workspace_members);
const registryPackage = metadata.packages.find(p => p.source?.startsWith('registry+'));
for (const pkg of metadata.packages.filter(p => !p.source && !workspace.has(p.id))) {
  const patched = patchedCrates[pkg.name];
  if (!patched || patched.version !== pkg.version || path.resolve(path.dirname(pkg.manifest_path)) !== path.resolve(root, patched.path)) {
    throw new Error(`Unreviewed local dependency: ${pkg.name} ${pkg.version}`);
  }
  // Read notices from the upstream crate in the same registry cache as other packages.
  const registryDirectory = path.dirname(path.dirname(registryPackage.manifest_path));
  pkg.manifest_path = path.join(registryDirectory, `${pkg.name}-${pkg.version}`, 'Cargo.toml');
  pkg.source = registryPackage.source;
  pkg.patched = patched;
  checksums.set(`${pkg.name}-${pkg.version}`, patched.checksum);
}
for (const name of Object.keys(patchedCrates)) {
  if (!metadata.packages.some(p => p.patched && p.name === name)) throw new Error(`Patched crate is no longer used: ${name}`);
}
const packages = metadata.packages.filter(p => p.source).sort((a, b) => `${a.name}-${a.version}`.localeCompare(`${b.name}-${b.version}`, 'en'));
const output = new Map();
function add(relative, data) { output.set(relative.replaceAll('\\', '/'), Buffer.isBuffer(data) ? data : Buffer.from(data)); }
function readNotices(directory, relative = '') {
  const result = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    if (entry.isSymbolicLink()) throw new Error(`Unexpected symlink in Cargo source: ${entry.name}`);
    const rel = path.join(relative, entry.name), full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!['.git', 'target', 'tests', 'examples', 'benches'].includes(entry.name)) result.push(...readNotices(full, rel));
    } else if (/(^|[-_.])(licen[cs]e|copying|copyright|notice|authors?)([-_.]|$)/i.test(entry.name) || (/^fonts[\\/]/.test(rel) && /\.txt$/i.test(entry.name))) {
      result.push([rel, fs.readFileSync(full)]);
    }
  }
  return result;
}
// These upstream crates omit their workspace-level license files from the
// published crate. Include the declared standard license and the entire crate,
// preserving every original source notice. Apache is selected where offered.
const standardForMissing = {
  accesskit: 'Apache-2.0', 'clipboard-win': 'BSL-1.0', ecolor: 'Apache-2.0',
  eframe: 'Apache-2.0', egui: 'Apache-2.0', 'egui-winit': 'Apache-2.0',
  egui_glow: 'Apache-2.0', emath: 'Apache-2.0', epaint: 'Apache-2.0',
  gl_generator: 'Apache-2.0', khronos_api: 'Apache-2.0',
  'mlua-sys': 'MIT-mlua', profiling: 'Apache-2.0', selectors: 'MPL-2.0'
};
const inventory = [];
const reviewedExpressions = new Set([
  'MIT OR Apache-2.0', 'MIT', 'Unicode-3.0', 'Apache-2.0 OR MIT',
  'MIT/Apache-2.0', 'Apache-2.0', 'MIT OR Apache-2.0 OR Zlib', 'MPL-2.0',
  'ISC', 'Unlicense OR MIT', 'MIT OR Zlib OR Apache-2.0', 'Zlib OR Apache-2.0 OR MIT',
  'BSL-1.0', 'BSD-3-Clause OR Apache-2.0', 'BSD-2-Clause OR Apache-2.0 OR MIT',
  'Zlib', 'Apache-2.0 OR GPL-2.0-only', 'Apache-2.0 AND MIT',
  '0BSD OR MIT OR Apache-2.0', '(MIT OR Apache-2.0) AND Unicode-3.0',
  '(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0', 'Unlicense/MIT'
]);
for (const pkg of packages) {
  if (!pkg.source.startsWith('registry+') || !reviewedExpressions.has(pkg.license)) throw new Error(`Unreviewed dependency source/license: ${pkg.name}`);
  const id = `${pkg.name}-${pkg.version}`, directory = path.dirname(pkg.manifest_path);
  const registryName = path.basename(path.dirname(directory));
  const registryRoot = path.dirname(path.dirname(path.dirname(directory)));
  const archive = fs.readFileSync(path.join(registryRoot, 'cache', registryName, `${id}.crate`));
  const checksum = checksums.get(id);
  if (!checksum || sha(archive) !== checksum) throw new Error(`Cargo archive checksum mismatch: ${id}`);
  const notices = readNotices(directory);
  let supplemental;
  if (!notices.length) {
    supplemental = standardForMissing[pkg.name];
    if (!supplemental) throw new Error(`No license text found for ${id}; review before releasing.`);
    if (supplemental !== 'MIT-mlua' && !pkg.license.includes(supplemental)) throw new Error(`Supplemental license no longer matches ${id}`);
    notices.push([`LICENSE-${supplemental}`, fs.readFileSync(path.join(root, 'docs/licenses/common', supplemental))]);
  }
  for (const [name, data] of notices) add(`docs/licenses/rust/${id}/${name}`, data);
  // MPL source obligations and omitted upstream notices are self-contained;
  // recipients do not depend on a third-party download service remaining online.
  const includeSource = pkg.license.includes('MPL-2.0') || !!supplemental;
  const sourcePath = includeSource ? `docs/licenses/sources/${id}.crate` : null;
  if (sourcePath) add(sourcePath, archive);
  inventory.push({ name: pkg.name, version: pkg.version, license: pkg.license,
    selectedLicense: pkg.name === 'self_cell' ? 'Apache-2.0' : supplemental === 'MIT-mlua' ? 'MIT' : supplemental || pkg.license,
    authors: pkg.authors, repository: pkg.repository, checksum,
    notices: notices.map(([name]) => `rust/${id}/${name.replaceAll('\\', '/')}`),
    source: sourcePath ? sourcePath.slice('docs/licenses/'.length) : null,
    upstreamSource: `https://crates.io/crates/${pkg.name}/${pkg.version}`,
    ...(pkg.patched ? { modifiedCopy: pkg.patched.path, modification: pkg.patched.change } : {}) });
}
add('docs/licenses/rust-dependencies.json', JSON.stringify({ target: 'x86_64-pc-windows-msvc', scope: 'Locked Windows dependency graph, including build and test dependencies; not every listed crate is linked into the runtime.', packages: inventory }, null, 2) + '\n');
const modified = inventory.filter(p => p.modifiedCopy);
let index = `# Rust dependency licenses\n\nThese are locked Cargo dependencies, unmodified except ${modified.map(p => p.name).join(', ')} (see below). Download Engine's MIT license does not replace their licenses. Alternative licenses remain alternatives; self_cell is used under Apache-2.0. This inventory includes build/test dependencies as well as runtime dependencies.\n\n`;
index += 'MPL-covered source and published crates that omit workspace license files are included in `sources/`. A `.crate` file is a gzip-compressed tar archive; extract it with `tar -xzf filename.crate`. The included source is the unmodified upstream crate; its SHA-256 is checked against Cargo.lock, or for a modified crate against the published crates.io checksum. Other source links identify the exact published versions.\n\n';
for (const p of modified) index += `Download Engine builds a modified copy of ${p.name} ${p.version} (${p.modification}). The modified source is \`${p.modifiedCopy}\` in the Download Engine source release; \`${p.modifiedCopy}/XDOWNLOADER-PATCH.md\` lists the changed files.\n\n`;
for (const p of inventory) index += `- **${p.name} ${p.version}** — ${p.selectedLicense}${p.modifiedCopy ? ', modified copy' : ''}. ${p.notices.map(n => `[${path.posix.basename(n)}](${n})`).join(', ')}${p.source ? `; [included source](${p.source})` : ''}.\n`;
add('docs/licenses/RUST-DEPENDENCIES.md', index);
let changed = 0;
for (const [relative, data] of output) {
  const destination = path.join(root, relative);
  if (fs.existsSync(destination) && fs.readFileSync(destination).equals(data)) continue;
  changed++;
  if (check) throw new Error(`License bundle missing or stale: ${relative}`);
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.writeFileSync(destination, data);
}
console.log(JSON.stringify({ checked: check, packages: inventory.length, noticeFiles: inventory.reduce((n, p) => n + p.notices.length, 0), includedSources: inventory.filter(p => p.source).length, changed }));

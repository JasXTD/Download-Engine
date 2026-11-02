'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const common = path.resolve(__dirname, '../docs/licenses/common');

function readNotices(directory, relative = '') {
  const result = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    if (entry.isSymbolicLink()) throw new Error(`Unexpected symlink in Cargo source: ${entry.name}`);
    const rel = path.join(relative, entry.name), full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!['.git', 'target', 'tests', 'examples', 'benches'].includes(entry.name)) result.push(...readNotices(full, rel));
    } else if (!/\.rs$/i.test(entry.name) && (/(^|[-_.])(licen[cs]e|copying|copyright|notice|authors?)([-_.]|$)/i.test(entry.name) || (/^fonts[\\/]/.test(rel) && /\.txt$/i.test(entry.name)))) {
      // objc2-foundation's copying.rs implements object copying; it is not a notice.
      result.push([rel, fs.readFileSync(full)]);
    }
  }
  return result;
}

// Exact published crate revisions whose workspace notices were omitted from the
// archives. Never silently apply these to future releases or other repositories.
const objcRevisions = {
  'block2-0.5.1': '4fc083f1c6d6784577e38b0ee8dbd344481e2fd2',
  'objc-sys-0.3.5': '4fc083f1c6d6784577e38b0ee8dbd344481e2fd2',
  'objc2-0.5.3': '5237cbf081d04ef75a1f6a207b4018e7c1ae7438',
  'objc2-app-kit-0.2.2': 'e282618be4c3a3b9542957e0c8540e9588472ce8',
  'objc2-core-data-0.2.2': 'e282618be4c3a3b9542957e0c8540e9588472ce8',
  'objc2-core-image-0.2.2': 'e282618be4c3a3b9542957e0c8540e9588472ce8',
  'objc2-foundation-0.2.2': 'e282618be4c3a3b9542957e0c8540e9588472ce8',
  'objc2-metal-0.2.2': 'e282618be4c3a3b9542957e0c8540e9588472ce8',
  'objc2-quartz-core-0.2.2': 'e282618be4c3a3b9542957e0c8540e9588472ce8',
  'block2-0.6.2': 'b4167b582b2f75f9a1be75495c41b765344fd03c',
  'objc2-0.6.5': 'd7d2fa23ceaa5e6096c923b081040e5d81b3b9df',
  'objc2-encode-4.1.0': '8d214f5477365ffcbcbb7de058c86ed9a518efb7',
  'objc2-foundation-0.3.2': '7b1abfd750a2cacaea71d6a56ecfb83cb7de560b',
  'objc2-app-kit-0.3.2': '7b1abfd750a2cacaea71d6a56ecfb83cb7de560b',
  'objc2-core-foundation-0.3.2': '7b1abfd750a2cacaea71d6a56ecfb83cb7de560b',
  'objc2-core-graphics-0.3.2': '7b1abfd750a2cacaea71d6a56ecfb83cb7de560b',
  'objc2-io-surface-0.3.2': '7b1abfd750a2cacaea71d6a56ecfb83cb7de560b',
  'dispatch2-0.3.1': '8852b424193ca41602281b3d7540d7c8ed51e49a'
};
const legacyRevisions = new Set([
  '4fc083f1c6d6784577e38b0ee8dbd344481e2fd2',
  '5237cbf081d04ef75a1f6a207b4018e7c1ae7438',
  'e282618be4c3a3b9542957e0c8540e9588472ce8'
]);
const apacheCrates = new Set(['dispatch2-0.3.1', 'objc2-app-kit-0.3.2', 'objc2-core-foundation-0.3.2', 'objc2-core-graphics-0.3.2', 'objc2-io-surface-0.3.2']);

function verifiedNotice(name, expectedHash) {
  const data = fs.readFileSync(path.join(common, name));
  if (crypto.createHash('sha256').update(data).digest('hex') !== expectedHash) throw new Error(`Upstream notice checksum mismatch: ${name}`);
  return data;
}

function reviewedSupplement(pkg, directory) {
  const id = `${pkg.name}-${pkg.version}`;
  if (pkg.repository === 'http://github.com/SSheldon/rust-dispatch') {
    const vcs = JSON.parse(fs.readFileSync(path.join(directory, '.cargo_vcs_info.json'), 'utf8'));
    if (id !== 'dispatch-0.2.0' || pkg.license !== 'MIT' || vcs.git?.sha1 !== '82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748' || vcs.git?.dirty) {
      throw new Error(`Unreviewed dispatch source/license: ${id}`);
    }
    // This older crate omits license text entirely. Its manifest declares MIT
    // and the same copyright holder as the retained upstream MIT notice.
    if (pkg.authors?.length !== 1 || pkg.authors[0] !== 'Steven Sheldon') throw new Error(`Unreviewed dispatch authors: ${id}`);
    return { selectedLicense: 'MIT', notices: [
      ['LICENSE-MIT', verifiedNotice('MIT-objc2', 'e353f37b12aefbb9f9b29490e837cfee05d9bda70804b3562839a3285c1df1e5')],
      ['NOTICE-PROVENANCE.md', fs.readFileSync(path.join(common, 'objc2-PROVENANCE.md'))]
    ] };
  }
  if (pkg.repository !== 'https://github.com/madsmtm/objc2') return null;
  const revision = objcRevisions[id];
  if (!revision) throw new Error(`Unreviewed objc2 notice revision: ${id}`);
  const vcs = JSON.parse(fs.readFileSync(path.join(directory, '.cargo_vcs_info.json'), 'utf8'));
  if (vcs.git?.sha1 !== revision || vcs.git?.dirty) throw new Error(`Unexpected objc2 source revision: ${id}`);
  const selectedLicense = apacheCrates.has(id) ? 'Apache-2.0' : 'MIT';
  const expectedLicense = apacheCrates.has(id) ? 'Zlib OR Apache-2.0 OR MIT' : 'MIT';
  if (pkg.license !== expectedLicense) throw new Error(`Unexpected objc2 license: ${id}`);
  const notices = [[`LICENSE-${selectedLicense}`, selectedLicense === 'MIT'
    ? verifiedNotice('MIT-objc2', 'e353f37b12aefbb9f9b29490e837cfee05d9bda70804b3562839a3285c1df1e5')
    : fs.readFileSync(path.join(common, 'Apache-2.0'))]];
  if (!legacyRevisions.has(revision)) {
    // The later workspace describes its license choices but omits full text.
    // Retain the original MIT copyright where MIT is the declared license.
    notices.push(['UPSTREAM-LICENSE.md', verifiedNotice('objc2-LICENSE.md', '7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54')]);
  }
  notices.push(['NOTICE-PROVENANCE.md', fs.readFileSync(path.join(common, 'objc2-PROVENANCE.md'))]);
  return { selectedLicense, notices };
}

module.exports = { readNotices, reviewedSupplement };

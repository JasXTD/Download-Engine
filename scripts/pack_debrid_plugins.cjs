'use strict';
// Build only these reviewed, self-contained Lua packages. No user plugins.
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '../assets/plugins');
function build(check = false) {
  for (const slug of ['all-debrid', 'real-debrid', 'premiumize']) {
    const folder = path.join(root, slug);
    const manifest = JSON.parse(fs.readFileSync(path.join(folder, 'manifest.json'), 'utf8'));
    manifest.files = {
      'common.lua': fs.readFileSync(path.join(root, 'debrid-common.lua'), 'utf8').replace(/\r\n/g, '\n'),
      'main.lua': fs.readFileSync(path.join(folder, 'main.lua'), 'utf8').replace(/\r\n/g, '\n'),
    };
    const bytes = JSON.stringify(manifest, null, 2) + '\n';
    const output = path.join(root, slug + '.de-extension.json');
    if (check) {
      if (fs.readFileSync(output, 'utf8').replace(/\r\n/g, '\n') !== bytes) throw new Error(slug + ' package is stale');
    } else fs.writeFileSync(output, bytes);
  }
}
module.exports = {build};
if (require.main === module) build(process.argv.includes('--check'));

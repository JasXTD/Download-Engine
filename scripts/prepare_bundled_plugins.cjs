// Only the reviewed packages enter release packages. Never copy a developer's or
// user's live plugins folder into a distributable.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {buildDemo} = require('./build_offline_episode_demo.cjs');

function prepare(output, cli, ffmpeg) {
  output = path.resolve(output);
  assert(!fs.existsSync(output), 'Bundled plugins require a fresh staging folder');
  assert(cli && ffmpeg, 'Pass the built native CLI and approved FFmpeg paths');
  const demo = buildDemo(path.join(output, 'offline-episode'), {cli, ffmpeg});
  fs.copyFileSync(demo.package, path.join(output, 'offline-episode.de-extension.json'));
  require('./pack_debrid_plugins.cjs').build(true);
  for (const slug of ['all-debrid', 'real-debrid', 'premiumize']) {
    fs.copyFileSync(path.join(__dirname, '../assets/plugins', slug + '.de-extension.json'),
      path.join(output, slug + '.de-extension.json'));
  }
  fs.writeFileSync(path.join(output, 'README.md'), `# Application plugins

Put packaged Lua extensions (*.de-extension.json) directly in this folder.
Open Download Engine → Extensions → Installed → From plugins folder.
Choose Refresh after adding a file, then Review install / Review update.
Nothing installs or runs automatically. Custom extensions start disabled.

Only this application's plugins folder is scanned, never a profile folder.
Subfolders are not scanned. Keep one package per extension ID.
Updates need a higher extension version and an explicit permission review.
Removing a file here does not remove the reviewed installed copy.

The offline episode demo is included but disabled until you install and enable it.
See offline-episode/README.md for the generated media, sample server and tutorial.
The server starts only when you run its launcher; it contacts no public site.

All Debrid, Real Debrid and Premiumize.me are separate optional Lua plugins.
Review and install each one you want; these three start enabled after installation.
Account and download actions still require a click. Existing disabled choices stay off.
All Debrid and Real Debrid offer Connect
account in Overview; approve the code on the service website. Premiumize.me
uses its API key, saved in the plugin's protected Settings. Paste a supported
file link and choose Generate links, then review files before adding downloads.
See docs/debrid-plugins.md in the app.
`);
  return {directory:output, package:path.join(output, 'offline-episode.de-extension.json'), files:demo.routes};
}
module.exports = {prepare};
if (require.main === module) {
  try {
    assert.equal(process.argv.length, 5, 'Usage: node scripts/prepare_bundled_plugins.cjs OUTPUT CLI FFMPEG');
    console.log(JSON.stringify(prepare(...process.argv.slice(2)), null, 2));
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}

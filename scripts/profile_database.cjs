// Only for owned smoke-test profiles. Production code uses Store connections.
// Keep these supported versions in sync with src/store/migrations.rs. Explicit
// registration exercises the same persistent guards without disabling them.
const { DatabaseSync } = require('node:sqlite');
module.exports = function openProfileDatabase(file, options = {}) {
  const db = new DatabaseSync(file, options);
  for (const name of ['writer', 'schema', 'data']) {
    db.function(`xdownloader_${name}_version`, { deterministic: true }, () => name === 'schema' ? 1 : 4);
  }
  return db;
};

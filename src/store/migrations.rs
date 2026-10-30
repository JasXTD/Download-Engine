//! Profile upgrades are ordered and atomic. A connection-local writer function
//! also makes already-released binaries fail closed: they cannot satisfy the
//! persistent write guards, even though they do not know about profile_format.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, functions::FunctionFlags};

const SCHEMA_VERSION: i64 = 1;
const DATA_VERSION: i64 = 4;
const WRITER_VERSION: i64 = 4;

pub(super) fn register_writer(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "xdownloader_writer_version",
        0,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |_| Ok(WRITER_VERSION),
    )?;
    for (name, value) in [
        ("xdownloader_schema_version", SCHEMA_VERSION),
        ("xdownloader_data_version", DATA_VERSION),
    ] {
        conn.create_scalar_function(
            name,
            0,
            FunctionFlags::SQLITE_UTF8
                | FunctionFlags::SQLITE_DETERMINISTIC
                | FunctionFlags::SQLITE_INNOCUOUS,
            move |_| Ok(value),
        )?;
    }
    Ok(())
}

fn version(conn: &Connection) -> Result<i64> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='profile_format')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    let (schema, data, writer): (i64, i64, i64) = conn
        .query_row(
            "SELECT schema_version,data_version,min_writer_version FROM profile_format WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("The profile format record is missing or invalid; keep the profile and use a compatible build")?;
    ensure!(
        schema >= 1 && data >= 1 && writer >= 1,
        "Invalid profile format record; the profile was not upgraded"
    );
    ensure!(
        schema <= SCHEMA_VERSION && data <= DATA_VERSION && writer <= WRITER_VERSION,
        "This profile requires a newer XDownloader build (schema {schema}, data {data}, writer {writer}). No profile changes were made."
    );
    Ok(schema)
}

pub(super) fn check_compatible(conn: &Connection) -> Result<()> {
    version(conn).map(|_| ())
}

pub(super) fn initialize(conn: &mut Connection, directory: &std::path::Path) -> Result<()> {
    if version(conn)? == SCHEMA_VERSION {
        return Ok(());
    }
    // journal_mode cannot change inside a transaction, and its lock upgrade
    // need not honor busy_timeout. Serialize first upgrades across processes
    // before touching the journal; SQLite still owns the actual migration.
    let migration_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("profile-migration.lock"))?;
    fs2::FileExt::lock_exclusive(&migration_lock)?;
    if version(conn)? == SCHEMA_VERSION {
        return Ok(());
    }
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // SQLite requires this outside the transaction for the legacy parent-table
    // replacement. Integrity is checked inside the transaction before commit.
    let replacing_queue = crate::queue_store::legacy_queue_limit(conn)?;
    if replacing_queue {
        conn.pragma_update(None, "foreign_keys", false)?;
    }
    let result = migrate(conn);
    if replacing_queue {
        conn.pragma_update(None, "foreign_keys", true)?;
    }
    result
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Recheck after acquiring the lock: another process may have upgraded.
    let current = version(&tx)?;
    if current < 1 {
        baseline(&tx)?;
        tx.execute_batch(
            "CREATE TABLE profile_format(
             id INTEGER PRIMARY KEY CHECK(id=1),schema_version INTEGER NOT NULL,
             data_version INTEGER NOT NULL,min_writer_version INTEGER NOT NULL);
             INSERT INTO profile_format VALUES(1,1,1,1);
             CREATE TABLE profile_migrations(version INTEGER PRIMARY KEY,applied_at INTEGER NOT NULL);
             INSERT INTO profile_migrations VALUES(1,unixepoch());",
        )?;
        install_guards(&tx)?;
        let broken: Option<String> = tx
            .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
            .optional()?;
        ensure!(
            broken.is_none(),
            "Profile migration found an invalid foreign key in {}",
            broken.unwrap_or_default()
        );
    }
    tx.commit()?;
    Ok(())
}

fn baseline(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, created INTEGER NOT NULL, data TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS browser_captures (request_id TEXT PRIMARY KEY, state TEXT NOT NULL, data TEXT, job_id TEXT);",
    )?;
    crate::queue_store::initialize(conn)?;
    crate::queue_start::initialize(conn)?;
    crate::auth::initialize(conn)?;
    crate::bandwidth::initialize(conn)?;
    crate::quota::initialize(conn)?;
    crate::preferences::initialize(conn)?;
    crate::proxy::initialize(conn)?;
    crate::categories::initialize(conn)?;
    crate::browser_batch::initialize(conn)?;
    crate::browser_video::initialize(conn)?;
    crate::grabber_projects::initialize(conn)?;
    crate::grabber_schedule::initialize(conn)?;
    crate::hls::initialize(conn)?;
    crate::antivirus::initialize(conn)?;
    crate::automation::initialize(conn)?;
    crate::script_automation::initialize(conn)?;
    crate::control::initialize(conn)?;
    crate::mcp::initialize(conn)?;
    crate::connections::initialize(conn)?;
    crate::browser_recovery::initialize(conn)?;
    crate::status_requests::initialize(conn)?;
    crate::file_relocation::initialize(conn)?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS jobs_created ON jobs(created);
         CREATE INDEX IF NOT EXISTS jobs_status_created ON jobs(json_extract(data,'$.status'),created);",
    )?;
    Ok(())
}

fn install_guards(conn: &Connection) -> Result<()> {
    let tables = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for table in tables {
        let quoted = table.replace('"', "\"\"");
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            conn.execute_batch(&format!(
                "CREATE TRIGGER \"writer_guard_{quoted}_{operation}\" BEFORE {operation} ON \"{quoted}\"
                 BEGIN SELECT CASE WHEN xdownloader_writer_version() < COALESCE((SELECT min_writer_version FROM profile_format WHERE id=1),9223372036854775807)
                 OR xdownloader_schema_version() < (SELECT schema_version FROM profile_format WHERE id=1)
                 OR xdownloader_data_version() < (SELECT data_version FROM profile_format WHERE id=1)
                 THEN RAISE(ABORT,'This profile requires a newer XDownloader writer') END; END;"
            ))?;
        }
    }
    conn.execute_batch(
        "CREATE TRIGGER profile_format_no_delete BEFORE DELETE ON profile_format
         BEGIN SELECT RAISE(ABORT,'The profile format record cannot be removed'); END;
         CREATE TRIGGER profile_format_no_downgrade BEFORE UPDATE ON profile_format
         WHEN NEW.schema_version<OLD.schema_version OR NEW.data_version<OLD.data_version OR NEW.min_writer_version<OLD.min_writer_version
         BEGIN SELECT RAISE(ABORT,'The profile format cannot be downgraded'); END;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../../tests/support/legacy_profile.rs"]
mod legacy_profile;
#[cfg(test)]
pub(crate) use legacy_profile::unversioned_fixture;

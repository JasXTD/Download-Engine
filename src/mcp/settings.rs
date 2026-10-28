//! Desktop-owned MCP preferences. CLI flags remain independent.
use super::*;
use anyhow::ensure;
use rusqlite::{Connection, params};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub port: u16,
    pub read_only: bool,
    pub start_with_app: bool,
    pub permissions: Permissions,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            read_only: true,
            start_with_app: false,
            permissions: Permissions::default(),
        }
    }
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_settings (
        id INTEGER PRIMARY KEY CHECK(id=1),
        port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
        read_only INTEGER NOT NULL CHECK(read_only IN (0,1)),
        start_with_app INTEGER NOT NULL CHECK(start_with_app IN (0,1)));
        INSERT OR IGNORE INTO mcp_settings VALUES(1,17842,1,0);",
    )?;
    super::access::initialize(conn)?;
    Ok(())
}

impl Store {
    pub fn mcp_settings(&self) -> Result<Settings> {
        let conn = self.connect()?;
        // Read the master switch and capabilities from the same snapshot. Two
        // separate queries could combine permissions from different saves.
        let (port, read_only, start_with_app, data): (u16, bool, bool, String) = conn.query_row(
            "SELECT s.port,s.read_only,s.start_with_app,p.data
             FROM mcp_settings s JOIN mcp_permissions p ON p.id=s.id WHERE s.id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        Ok(Settings {
            port,
            read_only,
            start_with_app,
            permissions: serde_json::from_str(&data)?,
        })
    }

    pub fn save_mcp_settings(&self, settings: Settings) -> Result<()> {
        ensure!(settings.port != 0, "Enter a port from 1 to 65535");
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "UPDATE mcp_settings SET port=?1,read_only=?2,start_with_app=?3 WHERE id=1",
            params![settings.port, settings.read_only, settings.start_with_app],
        )?;
        ensure!(n == 1, "MCP settings are unavailable");
        let n = tx.execute(
            "UPDATE mcp_permissions SET data=?1 WHERE id=1",
            [serde_json::to_string(&settings.permissions)?],
        )?;
        ensure!(n == 1, "MCP permissions are unavailable");
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_settings_reads_never_mix_access_from_different_saves() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        })
        .unwrap();
        let locked = Settings {
            permissions: Permissions {
                powershell: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let controls = Settings {
            port: 17843,
            read_only: false,
            ..Default::default()
        };
        store.save_mcp_settings(locked).unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                for n in 0..200 {
                    store
                        .save_mcp_settings(if n % 2 == 0 { controls } else { locked })
                        .unwrap();
                }
            });
            barrier.wait();
            for _ in 0..400 {
                let saved = store.mcp_settings().unwrap();
                assert!(
                    saved == locked || saved == controls,
                    "Mixed settings: {saved:?}"
                );
            }
        });
    }

    #[test]
    fn settings_are_opt_in_persisted_and_reject_invalid_updates() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        };
        let store = Store::open(paths.clone()).unwrap();
        assert_eq!(store.mcp_settings().unwrap(), Settings::default());
        let settings = Settings {
            port: 17843,
            read_only: false,
            start_with_app: true,
            permissions: Permissions {
                downloads: false,
                queues: true,
                lua: true,
                powershell: false,
            },
        };
        store.save_mcp_settings(settings).unwrap();
        assert!(
            store
                .save_mcp_settings(Settings {
                    port: 0,
                    ..settings
                })
                .is_err()
        );
        assert_eq!(
            Store::open(paths).unwrap().mcp_settings().unwrap(),
            settings
        );
        store
            .connect()
            .unwrap()
            .execute("DELETE FROM mcp_settings", [])
            .unwrap();
        assert!(store.mcp_settings().is_err());
        assert!(store.save_mcp_settings(settings).is_err());
    }
}

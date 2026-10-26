//! Desktop-owned MCP preferences. CLI flags remain independent.
use super::*;
use anyhow::ensure;
use rusqlite::{Connection, params};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub port: u16,
    pub read_only: bool,
    pub start_with_app: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            read_only: true,
            start_with_app: false,
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
    Ok(())
}

impl Store {
    pub fn mcp_settings(&self) -> Result<Settings> {
        Ok(self.connect()?.query_row(
            "SELECT port,read_only,start_with_app FROM mcp_settings WHERE id=1",
            [],
            |r| {
                Ok(Settings {
                    port: r.get(0)?,
                    read_only: r.get(1)?,
                    start_with_app: r.get(2)?,
                })
            },
        )?)
    }

    pub fn save_mcp_settings(&self, settings: Settings) -> Result<()> {
        ensure!(settings.port != 0, "Enter a port from 1 to 65535");
        let n = self.connect()?.execute(
            "UPDATE mcp_settings SET port=?1,read_only=?2,start_with_app=?3 WHERE id=1",
            params![settings.port, settings.read_only, settings.start_with_app],
        )?;
        ensure!(n == 1, "MCP settings are unavailable");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

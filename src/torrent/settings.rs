//! Torrents start switched off, and only a person can turn them on.
use crate::store::Store;
use anyhow::Result;
use rusqlite::{Connection, params};

pub const OFF_MESSAGE: &str = "Torrents are off. Turn them on to continue.";
pub const PROXY_MESSAGE: &str = "Torrents can't start while a proxy is on, because peers and the DHT would connect directly and show your real IP address.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    /// Whether other people can download pieces from this computer while it downloads.
    pub upload: bool,
    pub upload_limit: Option<u32>,
    pub dht: bool,
    pub seed_completed: bool,
    pub incoming_port: Option<u16>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            upload: true,
            upload_limit: None,
            dht: true,
            seed_completed: false,
            incoming_port: None,
        }
    }
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS torrent_settings (
        id INTEGER PRIMARY KEY CHECK(id=1),
        enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
        upload INTEGER NOT NULL CHECK(upload IN (0,1)),
        upload_limit INTEGER CHECK(upload_limit BETWEEN 1024 AND 1073741824),
        dht INTEGER NOT NULL CHECK(dht IN (0,1)));
        INSERT OR IGNORE INTO torrent_settings VALUES(1,0,1,NULL,1);
        CREATE TABLE IF NOT EXISTS torrent_files (
        job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
        data TEXT NOT NULL);",
    )?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS torrent_sharing_settings (
        id INTEGER PRIMARY KEY CHECK(id=1), seed_completed INTEGER NOT NULL CHECK(seed_completed IN (0,1)),
        incoming_port INTEGER CHECK(incoming_port BETWEEN 1024 AND 65535));
        INSERT OR IGNORE INTO torrent_sharing_settings VALUES(1,0,NULL);
        CREATE TABLE IF NOT EXISTS torrent_sharing (
        job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
        enabled INTEGER NOT NULL CHECK(enabled IN (0,1)), error TEXT);")?;
    // Development builds could save torrents before the version guard existed. Upgrade
    // those profiles too, without changing profiles that have never saved a torrent.
    let has_torrents: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE
         json_type(CASE WHEN json_valid(data) THEN data END,'$.torrent')='object')",
        [],
        |row| row.get(0),
    )?;
    if has_torrents {
        protect_profile(conn)?;
    }
    Ok(())
}

pub(super) fn protect_profile(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE profile_format SET data_version=MAX(data_version,6),
         min_writer_version=MAX(min_writer_version,6) WHERE id=1",
        [],
    )?;
    Ok(())
}

impl Store {
    pub fn torrent_settings(&self) -> Result<Settings> {
        Ok(self.connect()?.query_row(
            "SELECT enabled,upload,upload_limit,dht,seed_completed,incoming_port FROM torrent_settings JOIN torrent_sharing_settings USING(id) WHERE id=1",
            [],
            |row| {
                Ok(Settings {
                    enabled: row.get(0)?,
                    upload: row.get(1)?,
                    upload_limit: row.get(2)?,
                    dht: row.get(3)?,
                    seed_completed: row.get(4)?,
                    incoming_port: row.get(5)?,
                })
            },
        )?)
    }

    /// Saves the settings. Send `Command::TorrentSettings` after this, so that a running
    /// helper follows them and turning torrents off stops every torrent.
    pub fn save_torrent_settings(&self, value: Settings) -> Result<()> {
        crate::bandwidth::validate(value.upload_limit.map(u64::from))?;
        anyhow::ensure!(
            value.incoming_port.is_none_or(|port| port >= 1024),
            "Choose an incoming port between 1024 and 65535"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let count = tx.execute(
            "UPDATE torrent_settings SET enabled=?1,upload=?2,upload_limit=?3,dht=?4 WHERE id=1",
            params![value.enabled, value.upload, value.upload_limit, value.dht],
        )?;
        anyhow::ensure!(count == 1, "Torrent settings are unavailable");
        tx.execute(
            "UPDATE torrent_sharing_settings SET seed_completed=?1,incoming_port=?2 WHERE id=1",
            params![value.seed_completed, value.incoming_port],
        )?;
        if value.seed_completed || value.incoming_port.is_some() {
            protect_profile(&tx)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Peers and the DHT can't go through the app's proxy, so a torrent would show the real
    /// address that the proxy is there to hide.
    pub fn proxy_blocks_torrents(&self) -> Result<bool> {
        Ok(match self.proxy_settings()?.mode {
            crate::proxy::Mode::Direct => false,
            crate::proxy::Mode::Custom => true,
            crate::proxy::Mode::System => system_proxy_is_on(),
        })
    }
}

fn system_proxy_is_on() -> bool {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        || windows_proxy_is_on()
}

#[cfg(windows)]
fn windows_proxy_is_on() -> bool {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW},
    };
    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings");
    let enable = wide("ProxyEnable");
    let script = wide("AutoConfigURL");
    let mut enabled = 0u32;
    let mut size = size_of::<u32>() as u32;
    // The buffers live through each call, and the sizes describe them.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            enable.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut enabled as *mut u32).cast(),
            &mut size,
        )
    };
    if status == ERROR_SUCCESS && enabled != 0 {
        return true;
    }
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            script.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    // An empty string still has room for its ending.
    status == ERROR_SUCCESS && size > 2
}

#[cfg(not(windows))]
fn windows_proxy_is_on() -> bool {
    false
}

#[cfg(test)]
mod sharing_tests {
    use super::*;
    #[test]
    fn sharing_and_incoming_are_separate_off_by_default_and_validate_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: dir.path().join("data"),
            downloads: dir.path().join("downloads"),
        })
        .unwrap();
        let initial = store.torrent_settings().unwrap();
        assert!(!initial.seed_completed);
        assert_eq!(initial.incoming_port, None);
        assert!(
            store
                .save_torrent_settings(Settings {
                    enabled: true,
                    seed_completed: true,
                    incoming_port: Some(1023),
                    ..initial
                })
                .is_err()
        );
        assert_eq!(store.torrent_settings().unwrap(), initial);
        let sharing = Settings {
            enabled: true,
            seed_completed: true,
            ..initial
        };
        store.save_torrent_settings(sharing).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), sharing);
        let incoming = Settings {
            seed_completed: false,
            incoming_port: Some(45000),
            ..sharing
        };
        store.save_torrent_settings(incoming).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), incoming);
        store.save_torrent_settings(initial).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), initial);
        assert!(
            !store.torrents.is_running(),
            "saving preferences alone does not start a listener"
        );
    }
}

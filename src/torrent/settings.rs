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
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            upload: true,
            upload_limit: None,
            dht: true,
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
    Ok(())
}

impl Store {
    pub fn torrent_settings(&self) -> Result<Settings> {
        Ok(self.connect()?.query_row(
            "SELECT enabled,upload,upload_limit,dht FROM torrent_settings WHERE id=1",
            [],
            |row| {
                Ok(Settings {
                    enabled: row.get(0)?,
                    upload: row.get(1)?,
                    upload_limit: row.get(2)?,
                    dht: row.get(3)?,
                })
            },
        )?)
    }

    /// Saves the settings. Send `Command::TorrentSettings` after this, so that a running
    /// helper follows them and turning torrents off stops every torrent.
    pub fn save_torrent_settings(&self, value: Settings) -> Result<()> {
        crate::bandwidth::validate(value.upload_limit.map(u64::from))?;
        let count = self.connect()?.execute(
            "UPDATE torrent_settings SET enabled=?1,upload=?2,upload_limit=?3,dht=?4 WHERE id=1",
            params![value.enabled, value.upload, value.upload_limit, value.dht],
        )?;
        anyhow::ensure!(count == 1, "Torrent settings are unavailable");
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

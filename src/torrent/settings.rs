//! Torrents start switched off, and only a person can turn them on.
use super::limits::{self, Limits};
use crate::store::Store;
use anyhow::Result;
use rusqlite::{Connection, params};

pub const OFF_MESSAGE: &str = "Torrents are off. Turn them on to continue.";
pub const PROXY_MESSAGE: &str = "Torrents can't start while a proxy is on, because peers and the DHT would connect directly and show your real IP address.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub binding: Option<de_torrent::network::Binding>,
    pub encryption: de_torrent::mse::Encryption,
    pub utp: bool,
    pub web_seeds: bool,
    pub enabled: bool,
    /// Whether other people can download pieces from this computer while it downloads.
    pub upload: bool,
    pub upload_limit: Option<u32>,
    pub dht: bool,
    pub seed_completed: bool,
    pub incoming_port: Option<u16>,
    /// Sharing a finished torrent stops at this ratio, in hundredths, if set.
    pub share_ratio: Option<u32>,
    /// Sharing a finished torrent stops after this many seconds of sharing, if set.
    pub share_seconds: Option<u32>,
    /// Asks the router to forward the incoming port while torrents run. Only with a port.
    pub upnp: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            binding: None,
            encryption: de_torrent::mse::Encryption::Disabled,
            utp: false,
            web_seeds: false,
            enabled: false,
            upload: false,
            upload_limit: None,
            dht: true,
            seed_completed: false,
            incoming_port: None,
            share_ratio: Some(limits::DEFAULT_RATIO),
            share_seconds: Some(limits::DEFAULT_TIME),
            upnp: false,
        }
    }
}
impl Settings {
    pub fn sharing_limits(&self) -> Limits {
        Limits::new(self.share_ratio, self.share_seconds)
    }
}

/// `CHECK` lists made from the values Preferences offers, so the two can't drift apart.
fn allowed(values: &[u32]) -> String {
    values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS torrent_network_binding (
        id INTEGER PRIMARY KEY CHECK(id=1), config TEXT CHECK(config IS NULL OR length(config)<=1024));
        INSERT OR IGNORE INTO torrent_network_binding VALUES(1,NULL);")?;
    // Separate table keeps older profiles readable without rewriting existing settings.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS torrent_transport_settings (
        id INTEGER PRIMARY KEY CHECK(id=1),
        encryption TEXT NOT NULL CHECK(encryption IN ('disabled','prefer','required')),
        utp INTEGER NOT NULL CHECK(utp IN (0,1)),
        web_seeds INTEGER NOT NULL CHECK(web_seeds IN (0,1)));
        INSERT OR IGNORE INTO torrent_transport_settings VALUES(1,'disabled',0,0);",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS torrent_settings (
        id INTEGER PRIMARY KEY CHECK(id=1),
        enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
        upload INTEGER NOT NULL CHECK(upload IN (0,1)),
        upload_limit INTEGER CHECK(upload_limit BETWEEN 1024 AND 1073741824),
        dht INTEGER NOT NULL CHECK(dht IN (0,1)));
        INSERT OR IGNORE INTO torrent_settings VALUES(1,0,0,NULL,1);
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
    // Trackers the user added in the status window. Older versions ignore this table.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS torrent_trackers (
        job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
        url TEXT NOT NULL CHECK(length(url) BETWEEN 1 AND 2048),
        PRIMARY KEY(job_id, url));",
    )?;
    // Sharing limits, what each torrent has shared so far, the router switch and the
    // person's yes to a DHT search. New tables, so older versions ignore all of them.
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS torrent_sharing_limits (
        id INTEGER PRIMARY KEY CHECK(id=1),
        ratio INTEGER CHECK(ratio IS NULL OR ratio IN ({ratios})),
        seconds INTEGER CHECK(seconds IS NULL OR seconds IN ({times})));
        INSERT OR IGNORE INTO torrent_sharing_limits VALUES(1,{ratio},{time});
        CREATE TABLE IF NOT EXISTS torrent_sharing_totals (
        job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
        uploaded INTEGER NOT NULL CHECK(uploaded >= 0),
        shared_secs INTEGER NOT NULL CHECK(shared_secs >= 0),
        stopped TEXT CHECK(stopped IS NULL OR stopped IN ('ratio','time')),
        unlimited INTEGER NOT NULL DEFAULT 0 CHECK(unlimited IN (0,1)));
        CREATE TABLE IF NOT EXISTS torrent_router_settings (
        id INTEGER PRIMARY KEY CHECK(id=1),
        upnp INTEGER NOT NULL CHECK(upnp IN (0,1)));
        INSERT OR IGNORE INTO torrent_router_settings VALUES(1,0);
        CREATE TABLE IF NOT EXISTS torrent_dht_consent (
        job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
        info_hash TEXT NOT NULL CHECK(length(info_hash)=40));",
        ratios = allowed(&limits::RATIOS),
        times = allowed(&limits::TIMES),
        ratio = limits::DEFAULT_RATIO,
        time = limits::DEFAULT_TIME,
    ))?;
    // Preserve per-download consent while allowing full SHA-256 identities. Serialize
    // migration with other app instances and never expose a half-rebuilt table.
    let consent_tx =
        rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let consent_schema: String = consent_tx.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='torrent_dht_consent'",
        [],
        |row| row.get(0),
    )?;
    if !consent_schema.contains("40,64") {
        consent_tx.execute_batch("ALTER TABLE torrent_dht_consent RENAME TO torrent_dht_consent_v1;
            CREATE TABLE torrent_dht_consent (job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
            info_hash TEXT NOT NULL CHECK(length(info_hash) IN (40,64)));
            INSERT INTO torrent_dht_consent SELECT job_id,info_hash FROM torrent_dht_consent_v1;
            DROP TABLE torrent_dht_consent_v1;")?;
    }
    crate::store::migrations::guard_table(&consent_tx, "torrent_dht_consent")?;
    consent_tx.commit()?;
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
    // These tables are created after the baseline migration installs its guards.
    // An older connection must not overwrite the policy once binding is enabled.
    for table in ["torrent_network_binding", "torrent_settings"] {
        crate::store::migrations::guard_table(conn, table)?;
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
            "SELECT enabled,upload,upload_limit,dht,seed_completed,incoming_port,ratio,seconds,upnp,encryption,utp,web_seeds,config
             FROM torrent_settings JOIN torrent_sharing_settings USING(id)
             JOIN torrent_sharing_limits USING(id) JOIN torrent_router_settings USING(id)
             JOIN torrent_transport_settings USING(id) JOIN torrent_network_binding USING(id) WHERE id=1",
            [],
            |row| {
                Ok(Settings {
                    binding: row.get::<_,Option<String>>(12)?.map(|json| {
                        let binding: de_torrent::network::Binding = serde_json::from_str(&json).map_err(|_| rusqlite::Error::InvalidQuery)?;
                        binding.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                        Ok::<_, rusqlite::Error>(binding)
                    }).transpose()?,
                    enabled: row.get(0)?,
                    upload: row.get(1)?,
                    upload_limit: row.get(2)?,
                    dht: row.get(3)?,
                    seed_completed: row.get(4)?,
                    incoming_port: row.get(5)?,
                    share_ratio: row.get(6)?,
                    share_seconds: row.get(7)?,
                    upnp: row.get(8)?,
                    encryption: match row.get::<_, String>(9)?.as_str() {
                        "disabled" => de_torrent::mse::Encryption::Disabled,
                        "prefer" => de_torrent::mse::Encryption::Prefer,
                        "required" => de_torrent::mse::Encryption::Required,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    },
                    utp: row.get(10)?,
                    web_seeds: row.get(11)?,
                })
            },
        )?)
    }

    /// Saves the settings. Send `Command::TorrentSettings` after this, so that a running
    /// helper follows them and turning torrents off stops every torrent.
    pub fn save_torrent_settings(&self, value: &Settings) -> Result<()> {
        if let Some(binding) = &value.binding {
            binding.validate()?;
        }
        anyhow::ensure!(
            value.binding.is_none() || !value.upnp,
            "Turn off router port mapping before binding a torrent interface"
        );
        crate::bandwidth::validate(value.upload_limit.map(u64::from))?;
        anyhow::ensure!(
            value.incoming_port.is_none_or(|port| port >= 1024),
            "Choose an incoming port between 1024 and 65535"
        );
        anyhow::ensure!(
            value
                .share_ratio
                .is_none_or(|ratio| limits::RATIOS.contains(&ratio))
                && value
                    .share_seconds
                    .is_none_or(|seconds| limits::TIMES.contains(&seconds)),
            "Choose a sharing limit from the list"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE torrent_network_binding SET config=?1 WHERE id=1",
            [value
                .binding
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?],
        )?;
        anyhow::ensure!(changed == 1, "Torrent network settings are unavailable");
        if value.binding.is_some() {
            // Older builds ignore the new setting. Never let them open this profile
            // and start unbound torrents, including after a future setting reset.
            tx.execute("UPDATE profile_format SET data_version=MAX(data_version,10),min_writer_version=MAX(min_writer_version,10) WHERE id=1", [])?;
        }
        let count = tx.execute(
            "UPDATE torrent_settings SET enabled=?1,upload=?2,upload_limit=?3,dht=?4 WHERE id=1",
            params![value.enabled, value.upload, value.upload_limit, value.dht],
        )?;
        anyhow::ensure!(count == 1, "Torrent settings are unavailable");
        tx.execute(
            "UPDATE torrent_sharing_settings SET seed_completed=?1,incoming_port=?2 WHERE id=1",
            params![value.seed_completed, value.incoming_port],
        )?;
        tx.execute(
            "UPDATE torrent_sharing_limits SET ratio=?1,seconds=?2 WHERE id=1",
            params![value.share_ratio, value.share_seconds],
        )?;
        tx.execute(
            "UPDATE torrent_router_settings SET upnp=?1 WHERE id=1",
            [value.upnp],
        )?;
        tx.execute(
            "UPDATE torrent_transport_settings SET encryption=?1,utp=?2,web_seeds=?3 WHERE id=1",
            params![value.encryption.as_str(), value.utp, value.web_seeds],
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
    #[test]
    fn uploading_starts_off_and_existing_choices_survive_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        };
        let store = Store::open(paths.clone()).unwrap();
        assert!(
            !store.torrent_settings().unwrap().upload,
            "Fresh profiles must not upload"
        );
        assert!(
            !Settings::default().upload,
            "Reset must not enable uploading"
        );
        for upload in [true, false] {
            let mut choice = store.torrent_settings().unwrap();
            choice.upload = upload;
            store.save_torrent_settings(&choice).unwrap();
            initialize(&store.connect().unwrap()).unwrap();
            let reopened = Store::open(paths.clone()).unwrap();
            assert_eq!(reopened.torrent_settings().unwrap(), choice);
            assert!(!reopened.torrents.is_running());
        }
    }

    #[test]
    fn binding_round_trip_validation_and_downgrade_guard() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let initial = store.torrent_settings().unwrap();
        assert!(initial.binding.is_none());
        // Saving a temporarily missing interface is allowed, starting it is not.
        let changed = Settings {
            binding: Some(de_torrent::network::Binding {
                interface: "fixture-vpn".into(),
                address: "192.0.2.2".parse().unwrap(),
                dns: "192.0.2.53".parse().unwrap(),
            }),
            ..initial.clone()
        };
        store.save_torrent_settings(&changed).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), changed);
        let mut bad = changed.clone();
        bad.upnp = true;
        assert!(store.save_torrent_settings(&bad).is_err());
        bad.upnp = false;
        bad.binding.as_mut().unwrap().dns = "::1".parse().unwrap();
        assert!(store.save_torrent_settings(&bad).is_err());
        assert_eq!(store.torrent_settings().unwrap(), changed);
        store.save_torrent_settings(&initial).unwrap();
        let conn = store.connect().unwrap();
        let floor: i64 = conn
            .query_row(
                "SELECT min_writer_version FROM profile_format WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            floor, 10,
            "Resetting the preference must not re-enable an unsafe downgrade"
        );
        conn.create_scalar_function(
            "xdownloader_writer_version",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            |_| Ok(9i64),
        )
        .unwrap();
        assert!(
            conn.execute("UPDATE torrent_settings SET enabled=1 WHERE id=1", [])
                .is_err()
        );
    }

    #[test]
    fn transport_settings_start_off_round_trip_and_validate() {
        use de_torrent::mse::Encryption;
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(crate::paths::AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let initial = store.torrent_settings().unwrap();
        assert_eq!(initial.encryption, Encryption::Disabled);
        assert!(!initial.utp && !initial.web_seeds);
        let changed = Settings {
            encryption: Encryption::Required,
            utp: true,
            web_seeds: true,
            ..initial.clone()
        };
        store.save_torrent_settings(&changed).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), changed);
        let conn = store.connect().unwrap();
        for sql in [
            "UPDATE torrent_transport_settings SET encryption='unknown'",
            "UPDATE torrent_transport_settings SET utp=2",
            "UPDATE torrent_transport_settings SET web_seeds=-1",
        ] {
            assert!(conn.execute(sql, []).is_err());
        }
        initialize(&conn).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), changed);
    }
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
                .save_torrent_settings(&Settings {
                    enabled: true,
                    seed_completed: true,
                    incoming_port: Some(1023),
                    ..initial.clone()
                })
                .is_err()
        );
        assert_eq!(store.torrent_settings().unwrap(), initial);
        let sharing = Settings {
            enabled: true,
            seed_completed: true,
            ..initial.clone()
        };
        store.save_torrent_settings(&sharing).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), sharing);
        let incoming = Settings {
            seed_completed: false,
            incoming_port: Some(45000),
            ..sharing
        };
        store.save_torrent_settings(&incoming).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), incoming);
        store.save_torrent_settings(&initial).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), initial);
        assert!(
            !store.torrents.is_running(),
            "saving preferences alone does not start a listener"
        );
    }

    #[test]
    fn sharing_limits_default_to_ratio_two_or_a_week_and_only_listed_values_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: dir.path().join("data"),
            downloads: dir.path().join("downloads"),
        })
        .unwrap();
        let initial = store.torrent_settings().unwrap();
        assert_eq!(initial, Settings::default());
        assert_eq!(initial.share_ratio, Some(200));
        assert_eq!(initial.share_seconds, Some(604_800));
        assert!(!initial.upnp, "router setup starts off");
        for bad in [
            Settings {
                share_ratio: Some(250),
                ..initial.clone()
            },
            Settings {
                share_ratio: Some(0),
                ..initial.clone()
            },
            Settings {
                share_seconds: Some(1),
                ..initial.clone()
            },
            Settings {
                share_seconds: Some(0),
                ..initial.clone()
            },
        ] {
            assert!(store.save_torrent_settings(&bad).is_err(), "{bad:?}");
            assert_eq!(store.torrent_settings().unwrap(), initial);
        }
        let changed = Settings {
            share_ratio: None,
            share_seconds: Some(3_600),
            upnp: true,
            ..initial.clone()
        };
        store.save_torrent_settings(&changed).unwrap();
        assert_eq!(store.torrent_settings().unwrap(), changed);
        assert!(changed.sharing_limits().ratio_hundredths.is_none());
        // The database refuses what the window never offers, even written directly.
        let conn = store.connect().unwrap();
        for sql in [
            "UPDATE torrent_sharing_limits SET ratio=250 WHERE id=1",
            "UPDATE torrent_sharing_limits SET seconds=59 WHERE id=1",
            "UPDATE torrent_router_settings SET upnp=2 WHERE id=1",
        ] {
            assert!(conn.execute(sql, []).is_err(), "{sql}");
        }
        assert_eq!(store.torrent_settings().unwrap(), changed);
    }
}

//! Desktop preferences are separate from worker-owned job snapshots.
use crate::store::Store;
use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub close_to_tray: bool,
    pub notify_complete: bool,
    pub notify_failed: bool,
    /// Show what scripts send with xd.notify.
    pub notify_scripts: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            notify_complete: true,
            notify_failed: true,
            notify_scripts: true,
        }
    }
}
pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS desktop_preferences (
        id INTEGER PRIMARY KEY CHECK(id=1),
        close_to_tray INTEGER NOT NULL CHECK(close_to_tray IN (0,1)),
        notify_complete INTEGER NOT NULL CHECK(notify_complete IN (0,1)),
        notify_failed INTEGER NOT NULL CHECK(notify_failed IN (0,1)));
        INSERT OR IGNORE INTO desktop_preferences(id,close_to_tray,notify_complete,notify_failed) VALUES(1,0,1,1);
        CREATE TABLE IF NOT EXISTS interface_preferences (
        id INTEGER PRIMARY KEY CHECK(id=1), data TEXT NOT NULL);
        INSERT OR IGNORE INTO interface_preferences VALUES(1,'{}');
        CREATE TABLE IF NOT EXISTS windows_startup (
        id INTEGER PRIMARY KEY CHECK(id=1),
        start_hidden INTEGER NOT NULL CHECK(start_hidden IN (0,1)),
        registered TEXT CHECK(registered IS NULL OR length(registered) BETWEEN 1 AND 32767));
        INSERT OR IGNORE INTO windows_startup VALUES(1,1,NULL);",
    )?;
    crate::store::add_column(
        conn,
        "desktop_preferences",
        "notify_scripts",
        "INTEGER NOT NULL DEFAULT 1 CHECK(notify_scripts IN (0,1))",
    )?;
    apply_install_language(conn)
}

pub(crate) fn apply_install_language(conn: &Connection) -> Result<()> {
    // A setup language is only an initial display preference. An explicit saved
    // choice (including System) always wins, also after an installer upgrade.
    if crate::paths::home_override().is_none() {
        let executable = std::env::current_exe()?;
        if let Some(base) = executable.parent()
            && base.join("installed.json").is_file()
            && !base.join("portable.json").exists()
        {
            let saved: String = conn.query_row(
                "SELECT data FROM interface_preferences WHERE id=1",
                [],
                |r| r.get(0),
            )?;
            let needs_language =
                serde_json::from_str::<serde_json::Value>(&saved).is_ok_and(|value| {
                    value
                        .as_object()
                        .is_some_and(|object| !object.contains_key("language"))
                });
            let marker = base.join("installer-language.json");
            // Existing choices and invalid preferences keep their previous load/
            // recovery path; a stale setup marker must not override either.
            if needs_language && marker.is_file() {
                anyhow::ensure!(
                    std::fs::metadata(&marker)?.len() <= 64,
                    "Invalid installer language marker"
                );
                let language = serde_json::from_slice(&std::fs::read(marker)?)?;
                initialize_language(conn, language)?;
            }
        }
    }
    Ok(())
}

fn initialize_language(conn: &Connection, language: crate::i18n::Language) -> Result<()> {
    // json_set keeps every unrelated setting, even fields from a newer build.
    conn.execute(
        "UPDATE interface_preferences SET data=json_set(data,'$.language',?1) WHERE id=1 AND json_type(data,'$.language') IS NULL",
        [serde_json::to_value(language)?.as_str().expect("Language string")],
    )?;
    Ok(())
}
impl Store {
    pub fn preferences(&self) -> Result<Preferences> {
        Ok(self.connect()?.query_row("SELECT close_to_tray,notify_complete,notify_failed,notify_scripts FROM desktop_preferences WHERE id=1",[],|r| Ok(Preferences {
            close_to_tray:r.get(0)?,notify_complete:r.get(1)?,notify_failed:r.get(2)?,notify_scripts:r.get(3)?,
        }))?)
    }
    pub fn save_preferences(&self, value: Preferences) -> Result<()> {
        let n = self.connect()?.execute("UPDATE desktop_preferences SET close_to_tray=?1,notify_complete=?2,notify_failed=?3,notify_scripts=?4 WHERE id=1",
            params![value.close_to_tray,value.notify_complete,value.notify_failed,value.notify_scripts])?;
        anyhow::ensure!(n == 1, "Desktop preferences are unavailable");
        Ok(())
    }
}

/// Per-copy Windows startup choices. The Run entry itself is the source of
/// truth for whether this copy starts with Windows; `registered` only records
/// which executable this copy last registered, so a moved copy can repair it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupPreferences {
    pub start_hidden: bool,
    pub registered: Option<std::path::PathBuf>,
}
impl Default for StartupPreferences {
    fn default() -> Self {
        Self {
            start_hidden: true,
            registered: None,
        }
    }
}
impl Store {
    pub fn startup_preferences(&self) -> Result<StartupPreferences> {
        let (start_hidden, registered): (bool, Option<String>) = self.connect()?.query_row(
            "SELECT start_hidden,registered FROM windows_startup WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(StartupPreferences {
            start_hidden,
            registered: registered.map(Into::into),
        })
    }
    pub fn save_startup_preferences(&self, value: &StartupPreferences) -> Result<()> {
        let registered = match &value.registered {
            Some(path) => Some(
                path.to_str()
                    .ok_or_else(|| anyhow::anyhow!("The program path cannot be saved"))?,
            ),
            None => None,
        };
        let n = self.connect()?.execute(
            "UPDATE windows_startup SET start_hidden=?1,registered=?2 WHERE id=1",
            params![value.start_hidden, registered],
        )?;
        anyhow::ensure!(n == 1, "Startup preferences are unavailable");
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    System,
    Light,
    #[default]
    Dark,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreset {
    NeonCourier,
    ReactorNight,
    /// The scenery preset is the neutral first choice; it is also listed first.
    #[default]
    Aurora,
    #[serde(alias = "fediel")]
    Moonveil,
    OwnImage,
}

impl ThemePreset {
    pub const BUILT_IN: [Self; 4] = [
        Self::Aurora,
        Self::NeonCourier,
        Self::ReactorNight,
        Self::Moonveil,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::NeonCourier => "Neon Courier",
            Self::ReactorNight => "Reactor Night",
            Self::Aurora => "Aurora",
            Self::Moonveil => "Moonveil",
            Self::OwnImage => "Your image",
        }
    }

    pub fn accent(self) -> [u8; 3] {
        match self {
            Self::NeonCourier => [195, 165, 255],
            Self::ReactorNight => [118, 224, 181],
            Self::Aurora => [122, 212, 246],
            Self::Moonveil => [224, 163, 244],
            Self::OwnImage => [163, 191, 255],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomTheme {
    pub preset: ThemePreset,
    pub strength: u8,
    pub accent: [u8; 3],
    /// A profile-local, content-addressed image. Never persist a user file path.
    pub image: Option<[u8; 32]>,
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self {
            preset: ThemePreset::default(),
            strength: 80,
            accent: ThemePreset::default().accent(),
            image: None,
        }
    }
}

impl CustomTheme {
    pub fn validate(self) -> Result<()> {
        anyhow::ensure!(
            self.strength <= 100,
            "Background strength must be between 0 and 100"
        );
        anyhow::ensure!(
            self.preset != ThemePreset::OwnImage || self.image.is_some(),
            "Choose a theme image first"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterfacePreferences {
    pub language: crate::i18n::Language,
    pub appearance: Appearance,
    pub custom_theme: CustomTheme,
    pub compact_rows: bool,
    pub sidebar_labels: bool,
    pub show_details: bool,
    /// Preferred inspector height in logical points. None uses the compact-window default.
    pub details_height: Option<u16>,
    /// Optional toolbar actions, in the stable order defined by the desktop toolbar.
    pub toolbar_pins: [bool; 10],
    pub auto_download_status: bool,
    /// Check for a newer stable release once when this app opens.
    /// Existing profiles without the field use the enabled default.
    pub check_updates_on_launch: bool,
    /// Offer HTTP(S) file links copied in other apps. Saved data without this
    /// field (older versions) turns it on, the default.
    pub clipboard_links: bool,
    /// Play one Windows sound when a batch of downloads finishes. Off by
    /// default, also for older saved data.
    pub completion_sound: bool,
    /// Show the floating drop box. Off by default, also for older saved data.
    pub drop_target: bool,
    /// Its top-left corner in physical screen pixels, once moved.
    pub drop_target_position: Option<[i32; 2]>,
    /// Number, status, progress, size, speed, time left, added.
    pub columns: [bool; 7],
    pub table_layout: crate::table_layout::TableLayout,
}
impl Default for InterfacePreferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            custom_theme: CustomTheme::default(),
            language: crate::i18n::Language::default(),
            compact_rows: false,
            sidebar_labels: true,
            show_details: true,
            details_height: None,
            toolbar_pins: [true; 10],
            auto_download_status: true,
            check_updates_on_launch: true,
            clipboard_links: true,
            completion_sound: false,
            drop_target: false,
            drop_target_position: None,
            columns: [false, true, false, true, true, true, false],
            table_layout: Default::default(),
        }
    }
}
fn write_interface(conn: &Connection, value: InterfacePreferences) -> Result<()> {
    value.table_layout.validate()?;
    value.custom_theme.validate()?;
    let n = conn.execute(
        "UPDATE interface_preferences SET data=?1 WHERE id=1",
        [serde_json::to_string(&value)?],
    )?;
    anyhow::ensure!(n == 1, "Interface preferences are unavailable");
    Ok(())
}
impl Store {
    pub fn interface_preferences(&self) -> Result<InterfacePreferences> {
        let data: String = self.connect()?.query_row(
            "SELECT data FROM interface_preferences WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        let value: InterfacePreferences = serde_json::from_str(&data)?;
        value.table_layout.validate()?;
        value.custom_theme.validate()?;
        Ok(value)
    }
    pub fn save_interface_preferences(&self, value: InterfacePreferences) -> Result<()> {
        write_interface(&self.connect()?, value)
    }
    /// Save both sections together so a rejected appearance write cannot partly apply a dialog.
    pub fn save_desktop_preferences(
        &self,
        behavior: Preferences,
        interface: InterfacePreferences,
    ) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE desktop_preferences SET close_to_tray=?1,notify_complete=?2,notify_failed=?3,notify_scripts=?4 WHERE id=1",
            params![behavior.close_to_tray,behavior.notify_complete,behavior.notify_failed,behavior.notify_scripts],
        )?;
        anyhow::ensure!(n == 1, "Desktop preferences are unavailable");
        write_interface(&tx, interface)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Job, Status},
        paths::AppPaths,
    };
    #[test]
    fn update_checks_default_on_for_existing_profiles_and_preserve_opt_out() {
        let defaults: InterfacePreferences = serde_json::from_str("{}").unwrap();
        assert!(defaults.check_updates_on_launch);
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        write_interface(
            &conn,
            InterfacePreferences {
                check_updates_on_launch: false,
                ..defaults
            },
        )
        .unwrap();
        let saved: String = conn
            .query_row(
                "SELECT data FROM interface_preferences WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let restored: InterfacePreferences = serde_json::from_str(&saved).unwrap();
        assert!(!restored.check_updates_on_launch);
        assert_eq!(restored.appearance, defaults.appearance);
    }
    #[test]
    fn pre_release_theme_key_loads_as_moonveil_and_saves_the_new_name() {
        let preset: ThemePreset = serde_json::from_str(r#""fediel""#).unwrap();
        assert_eq!(preset, ThemePreset::Moonveil);
        assert_eq!(preset.name(), "Moonveil");
        assert_eq!(serde_json::to_string(&preset).unwrap(), r#""moonveil""#);
    }

    #[test]
    fn custom_theme_defaults_roundtrip_and_validation_preserve_previous_preferences() {
        let previous: InterfacePreferences =
            serde_json::from_str(r#"{"appearance":"dark"}"#).unwrap();
        assert_eq!(previous.custom_theme, CustomTheme::default());
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        for preset in ThemePreset::BUILT_IN {
            let value = InterfacePreferences {
                appearance: Appearance::Custom,
                custom_theme: CustomTheme {
                    preset,
                    strength: 65,
                    accent: preset.accent(),
                    image: None,
                },
                ..previous
            };
            write_interface(&conn, value).unwrap();
            let read = || -> InterfacePreferences {
                serde_json::from_str(
                    &conn
                        .query_row::<String, _, _>(
                            "SELECT data FROM interface_preferences",
                            [],
                            |row| row.get(0),
                        )
                        .unwrap(),
                )
                .unwrap()
            };
            assert_eq!(read(), value);
            let mut invalid = value;
            invalid.custom_theme.strength = 101;
            assert!(write_interface(&conn, invalid).is_err());
            invalid.custom_theme.strength = 50;
            invalid.custom_theme.preset = ThemePreset::OwnImage;
            assert!(write_interface(&conn, invalid).is_err());
            assert_eq!(read(), value);
        }
    }
    #[test]
    fn older_profiles_get_toolbar_and_details_defaults_and_preserve_other_settings() {
        let older: InterfacePreferences =
            serde_json::from_str(r#"{"language":"de","compact_rows":true,"show_details":false}"#)
                .unwrap();
        assert_eq!(older.toolbar_pins, [true; 10]);
        assert_eq!(older.details_height, None);
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        let mut changed = older;
        changed.toolbar_pins[6] = false;
        changed.details_height = Some(237);
        write_interface(&conn, changed).unwrap();
        let data: String = conn
            .query_row("SELECT data FROM interface_preferences", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<InterfacePreferences>(&data).unwrap(),
            changed
        );
        assert!(changed.compact_rows && !changed.show_details);
        assert_eq!(changed.language, older.language);
    }
    #[test]
    fn clipboard_links_default_on_including_older_saved_settings_and_persist_off() {
        assert!(InterfacePreferences::default().clipboard_links);
        let older: InterfacePreferences =
            serde_json::from_str(r#"{"compact_rows":true,"language":"de"}"#).unwrap();
        assert!(older.clipboard_links && older.compact_rows);
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        let off = InterfacePreferences {
            clipboard_links: false,
            ..older
        };
        write_interface(&conn, off).unwrap();
        let data: String = conn
            .query_row("SELECT data FROM interface_preferences", [], |r| r.get(0))
            .unwrap();
        assert!(data.contains(r#""clipboard_links":false"#));
        assert_eq!(
            serde_json::from_str::<InterfacePreferences>(&data).unwrap(),
            off
        );
    }
    #[test]
    fn completion_sound_defaults_off_including_older_saved_settings_and_persists_on() {
        assert!(!InterfacePreferences::default().completion_sound);
        let older: InterfacePreferences =
            serde_json::from_str(r#"{"clipboard_links":false,"language":"de"}"#).unwrap();
        assert!(!older.completion_sound && !older.clipboard_links);
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        })
        .unwrap();
        let on = InterfacePreferences {
            completion_sound: true,
            ..older
        };
        store
            .save_desktop_preferences(Preferences::default(), on)
            .unwrap();
        assert_eq!(store.interface_preferences().unwrap(), on);
    }
    #[test]
    fn drop_target_defaults_off_including_older_saved_settings_and_persists_its_position() {
        let default = InterfacePreferences::default();
        assert!(!default.drop_target && default.drop_target_position.is_none());
        let older: InterfacePreferences =
            serde_json::from_str(r#"{"clipboard_links":false,"language":"de"}"#).unwrap();
        assert!(!older.drop_target && older.drop_target_position.is_none());
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        })
        .unwrap();
        assert!(!store.interface_preferences().unwrap().drop_target);
        // Monitors left of or above the primary one have negative coordinates.
        let shown = InterfacePreferences {
            drop_target: true,
            drop_target_position: Some([-1500, -20]),
            ..older
        };
        store.save_interface_preferences(shown).unwrap();
        assert_eq!(store.interface_preferences().unwrap(), shown);
        let hidden = InterfacePreferences {
            drop_target: false,
            ..shown
        };
        store.save_interface_preferences(hidden).unwrap();
        let restored = store.interface_preferences().unwrap();
        assert!(!restored.drop_target);
        assert_eq!(
            restored.drop_target_position,
            Some([-1500, -20]),
            "Hiding keeps the place for next time"
        );
    }
    #[test]
    fn language_migration_and_installer_defaults_preserve_existing_data() {
        use crate::i18n::Language;
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE interface_preferences(id INTEGER PRIMARY KEY,data TEXT NOT NULL); INSERT INTO interface_preferences VALUES(1,'{\"compact_rows\":true,\"future_field\":42}')").unwrap();
        initialize_language(&conn, Language::TraditionalChinese).unwrap();
        let data: String = conn
            .query_row("SELECT data FROM interface_preferences", [], |r| r.get(0))
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(json["future_field"], 42);
        let prefs: InterfacePreferences = serde_json::from_str(&data).unwrap();
        assert!(prefs.compact_rows);
        assert_eq!(prefs.language, Language::TraditionalChinese);
        for language in Language::ALL {
            let expected = InterfacePreferences { language, ..prefs };
            write_interface(&conn, expected).unwrap();
            initialize_language(&conn, Language::Malay).unwrap();
            let data: String = conn
                .query_row("SELECT data FROM interface_preferences", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                serde_json::from_str::<InterfacePreferences>(&data).unwrap(),
                expected
            );
        }
        assert_eq!(
            serde_json::from_str::<InterfacePreferences>("{}")
                .unwrap()
                .language,
            Language::System
        );
    }
    #[test]
    fn automatic_status_defaults_on_and_can_be_saved_off() {
        let legacy: InterfacePreferences = serde_json::from_str("{}").unwrap();
        assert!(legacy.auto_download_status);
        let custom = InterfacePreferences {
            auto_download_status: false,
            ..legacy
        };
        let restored: InterfacePreferences =
            serde_json::from_str(&serde_json::to_string(&custom).unwrap()).unwrap();
        assert!(!restored.auto_download_status);
    }
    #[test]
    fn legacy_database_defaults_migrate_and_preferences_survive_worker_saves() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        };
        paths.prepare().unwrap();
        let connection = Connection::open(paths.data.join("queue.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE jobs(id TEXT PRIMARY KEY,request_id TEXT UNIQUE,created INTEGER,data TEXT);").unwrap();
        drop(connection);
        let store = Store::open(paths.clone()).unwrap();
        assert_eq!(store.preferences().unwrap(), Preferences::default());
        let custom = Preferences {
            close_to_tray: true,
            notify_complete: false,
            notify_failed: false,
            notify_scripts: false,
        };
        store.save_preferences(custom).unwrap();
        let mut job = Job::new(
            "https://example.com/file.zip",
            None,
            paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        store.enqueue(&job).unwrap();
        job.status = Status::Paused;
        job.downloaded = 123;
        store.save(&job).unwrap();
        let reopened = Store::open(paths).unwrap();
        assert_eq!(reopened.preferences().unwrap(), custom);
        assert_eq!(reopened.get(&job.id).unwrap().unwrap().downloaded, 123);
        assert!(
            reopened
                .connect()
                .unwrap()
                .execute("UPDATE desktop_preferences SET notify_complete=7", [])
                .is_err()
        );
        assert_eq!(reopened.preferences().unwrap(), custom);
    }
    #[test]
    fn startup_preferences_default_to_tray_and_round_trip_per_profile() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        };
        paths.prepare().unwrap();
        // Profiles from earlier releases gain the table with the default choices.
        let connection = Connection::open(paths.data.join("queue.sqlite3")).unwrap();
        connection.execute_batch("CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY,close_to_tray INTEGER,notify_complete INTEGER,notify_failed INTEGER); INSERT INTO desktop_preferences VALUES(1,1,0,1);").unwrap();
        drop(connection);
        let store = Store::open(paths.clone()).unwrap();
        assert_eq!(
            store.startup_preferences().unwrap(),
            StartupPreferences::default()
        );
        assert!(StartupPreferences::default().start_hidden);
        let custom = StartupPreferences {
            start_hidden: false,
            registered: Some(r"D:\Portable Download Engine\de.exe".into()),
        };
        store.save_startup_preferences(&custom).unwrap();
        store.save_preferences(Preferences::default()).unwrap();
        drop(store);
        let reopened = Store::open(paths).unwrap();
        assert_eq!(reopened.startup_preferences().unwrap(), custom);
        let conn = reopened.connect().unwrap();
        assert!(
            conn.execute("UPDATE windows_startup SET start_hidden=2", [])
                .is_err()
        );
        assert!(
            conn.execute("UPDATE windows_startup SET registered=''", [])
                .is_err()
        );
        conn.execute("DELETE FROM windows_startup", []).unwrap();
        assert!(reopened.startup_preferences().is_err());
        assert!(
            reopened
                .save_startup_preferences(&StartupPreferences::default())
                .is_err()
        );
    }
    #[test]
    fn missing_preference_row_is_reported_instead_of_acknowledging_a_save() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        store
            .connect()
            .unwrap()
            .execute("DELETE FROM desktop_preferences", [])
            .unwrap();
        assert!(store.preferences().is_err());
        assert!(store.save_preferences(Preferences::default()).is_err());
    }
    #[test]
    fn appearance_migrates_defaults_and_survives_legacy_writers_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        };
        paths.prepare().unwrap();
        let conn = Connection::open(paths.data.join("queue.sqlite3")).unwrap();
        conn.execute_batch("CREATE TABLE desktop_preferences(id INTEGER PRIMARY KEY,close_to_tray INTEGER,notify_complete INTEGER,notify_failed INTEGER); INSERT INTO desktop_preferences VALUES(1,1,0,1);").unwrap();
        drop(conn);
        let store = Store::open(paths.clone()).unwrap();
        assert_eq!(
            store.interface_preferences().unwrap(),
            InterfacePreferences::default()
        );
        assert_eq!(
            store.interface_preferences().unwrap().appearance,
            Appearance::Dark
        );
        assert!(store.preferences().unwrap().close_to_tray);
        let custom = InterfacePreferences {
            appearance: Appearance::System,
            custom_theme: CustomTheme::default(),
            language: crate::i18n::Language::Malay,
            compact_rows: true,
            sidebar_labels: false,
            show_details: false,
            details_height: Some(236),
            toolbar_pins: [
                true, false, true, false, true, false, true, false, true, false,
            ],
            auto_download_status: false,
            check_updates_on_launch: false,
            clipboard_links: false,
            completion_sound: true,
            drop_target: true,
            drop_target_position: Some([-1200, 340]),
            columns: [true, false, true, false, true, false, true],
            table_layout: Default::default(),
        };
        store
            .save_desktop_preferences(Preferences::default(), custom)
            .unwrap();
        let legacy = Preferences {
            close_to_tray: true,
            ..Preferences::default()
        };
        store.save_preferences(legacy).unwrap();
        drop(store);
        let reopened = Store::open(paths).unwrap();
        assert_eq!(reopened.interface_preferences().unwrap(), custom);
        assert_eq!(reopened.preferences().unwrap(), legacy);
    }
    #[test]
    fn combined_preferences_roll_back_if_either_section_cannot_be_saved() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        let behavior = Preferences {
            close_to_tray: true,
            ..Preferences::default()
        };
        let interface = InterfacePreferences {
            appearance: Appearance::Light,
            ..InterfacePreferences::default()
        };
        let conn = store.connect().unwrap();
        conn.execute_batch("CREATE TRIGGER reject_appearance BEFORE UPDATE ON interface_preferences BEGIN SELECT RAISE(ABORT,'fixture save failure'); END;").unwrap();
        assert!(store.save_desktop_preferences(behavior, interface).is_err());
        assert_eq!(store.preferences().unwrap(), Preferences::default());
        assert_eq!(
            store.interface_preferences().unwrap(),
            InterfacePreferences::default()
        );
        conn.execute_batch("DROP TRIGGER reject_appearance; DELETE FROM desktop_preferences;")
            .unwrap();
        assert!(store.save_desktop_preferences(behavior, interface).is_err());
        assert_eq!(
            store.interface_preferences().unwrap(),
            InterfacePreferences::default()
        );
        conn.execute("DELETE FROM interface_preferences", [])
            .unwrap();
        assert!(store.save_interface_preferences(interface).is_err());
    }
    #[test]
    fn partial_appearance_records_use_defaults_and_invalid_records_report_an_error() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        let conn = store.connect().unwrap();
        conn.execute(
            "UPDATE interface_preferences SET data=?1",
            [r#"{"compact_rows":true}"#],
        )
        .unwrap();
        let partial = store.interface_preferences().unwrap();
        assert_eq!(partial.appearance, Appearance::Dark);
        assert!(partial.compact_rows);
        conn.execute(
            "UPDATE interface_preferences SET data=?1",
            [r#"{"appearance":"light"}"#],
        )
        .unwrap();
        assert_eq!(
            store.interface_preferences().unwrap(),
            InterfacePreferences {
                appearance: Appearance::Light,
                ..InterfacePreferences::default()
            }
        );
        conn.execute("UPDATE interface_preferences SET data='invalid'", [])
            .unwrap();
        assert!(store.interface_preferences().is_err());
    }

    #[test]
    fn column_layout_migrates_and_round_trips_with_atomic_validation() {
        use crate::table_layout::{Column, TableLayout};
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        };
        let store = Store::open(paths.clone()).unwrap();
        let conn = store.connect().unwrap();
        conn.execute(
            "UPDATE interface_preferences SET data=?1",
            [r#"{"columns":[true,false,true,true,false,true,true],"appearance":"dark"}"#],
        )
        .unwrap();
        let mut value = store.interface_preferences().unwrap();
        assert_eq!(value.table_layout, TableLayout::default());
        assert!(!value.columns[1]);
        value
            .table_layout
            .move_column(Column::Size, Column::Name, true);
        value.table_layout.widths[Column::Name as usize] = Some(380);
        value.table_layout.widths[Column::Size as usize] = Some(180);
        store.save_interface_preferences(value).unwrap();
        assert_eq!(
            Store::open(paths).unwrap().interface_preferences().unwrap(),
            value
        );
        for corrupt in 0..2 {
            let mut invalid = value;
            if corrupt == 0 {
                invalid.table_layout.order[0] = Column::Actions;
            } else {
                invalid.table_layout.widths[Column::Name as usize] = Some(0);
            }
            assert!(
                store
                    .save_desktop_preferences(
                        Preferences {
                            close_to_tray: true,
                            ..Default::default()
                        },
                        invalid
                    )
                    .is_err()
            );
            assert_eq!(store.interface_preferences().unwrap(), value);
            assert_eq!(store.preferences().unwrap(), Preferences::default());
            conn.execute(
                "UPDATE interface_preferences SET data=?1",
                [serde_json::to_string(&invalid).unwrap()],
            )
            .unwrap();
            assert!(store.interface_preferences().is_err());
            store.save_interface_preferences(value).unwrap();
        }
    }
}

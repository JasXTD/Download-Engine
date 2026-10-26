//! One immutable route snapshot per HTTP transfer. Secrets live only in the
//! Windows vault; a journal reconciles interrupted replacements and removals.
use crate::{credential_vault as vault, store::Store};
use anyhow::{Context, Result, ensure};
use reqwest::{ClientBuilder, Url};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

mod route;
pub use route::Route;

#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod network_tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    System,
    Direct,
    Custom,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System proxy",
            Self::Direct => "Direct connection",
            Self::Custom => "Custom proxy",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authentication {
    #[default]
    Basic,
    Ntlm,
    Negotiate,
}
impl Authentication {
    pub fn label(self) -> &'static str {
        match self {
            Self::Basic => "Username / password",
            Self::Ntlm => "NTLM",
            Self::Negotiate => "Negotiate (Kerberos / NTLM)",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub endpoint: String,
    pub credential: Option<Uuid>,
    pub allow_unencrypted_login: bool,
    pub authentication: Authentication,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.mode != Mode::Custom {
            ensure!(
                self.endpoint.is_empty()
                    && self.credential.is_none()
                    && !self.allow_unencrypted_login
                    && self.authentication == Authentication::Basic,
                "Inactive proxy settings must not contain an endpoint or login"
            );
            return Ok(());
        }
        let url = endpoint(&self.endpoint)?;
        if self.authentication != Authentication::Basic {
            ensure!(
                cfg!(windows),
                "Enterprise proxy authentication requires Windows"
            );
            ensure!(
                url.scheme() == "http",
                "NTLM/Negotiate proxies require an http:// proxy address. HTTPS destination tunnels remain encrypted."
            );
        }
        if self.credential.is_some() {
            ensure!(
                encrypted_or_loopback(&url) || self.allow_unencrypted_login,
                "This proxy does not encrypt its login. Use an HTTPS proxy or explicitly allow an unencrypted proxy login."
            );
        }
        Ok(())
    }
}

pub fn endpoint(value: &str) -> Result<Url> {
    ensure!(
        value.len() <= 2048
            && !value.chars().any(char::is_whitespace)
            && !value.chars().any(char::is_control),
        "Invalid proxy address"
    );
    let url = Url::parse(value).map_err(|_| {
        anyhow::anyhow!("Enter a proxy URL, such as https://proxy.example.com:8443")
    })?;
    ensure!(
        matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") && url.host_str().is_some(),
        "Use an HTTP, HTTPS, SOCKS5 or SOCKS5h proxy"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none() && !value.contains('@'),
        "Enter proxy credentials in the login fields, not the proxy URL"
    );
    ensure!(
        matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port() != Some(0),
        "A proxy address may contain only a scheme, hostname and port"
    );
    Ok(url)
}

pub fn encrypted_or_loopback(url: &Url) -> bool {
    url.scheme() == "https"
        || url
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub(crate) username: String,
    pub(crate) password: String,
}
impl Login {
    pub fn new(username: String, password: String) -> Self {
        Self { username, password }
    }
    fn validate(&self, url: &Url) -> Result<()> {
        ensure!(
            !self.username.is_empty()
                && !self.username.contains(':')
                && self.username.len() <= 512
                && self.password.len() <= 2048,
            "Enter a proxy username without a colon and a password within the size limits"
        );
        ensure!(
            !self
                .username
                .chars()
                .chain(self.password.chars())
                .any(char::is_control),
            "Proxy credentials cannot contain control characters"
        );
        if url.scheme().starts_with("socks") {
            ensure!(
                self.username.len() <= 255
                    && !self.password.is_empty()
                    && self.password.len() <= 255,
                "SOCKS5 usernames and passwords must each contain 1–255 bytes"
            );
        }
        Ok(())
    }
}
impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxyLogin([redacted])")
    }
}
impl Drop for Login {
    fn drop(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLogin {
    endpoint: String,
    login: Login,
}

pub enum LoginChange {
    Keep,
    Remove,
    Replace(Login),
}

static LOCK: Mutex<()> = Mutex::new(());
fn guard() -> Result<std::sync::MutexGuard<'static, ()>> {
    LOCK.lock()
        .map_err(|_| anyhow::anyhow!("Proxy settings were interrupted. Restart Download Engine."))
}
pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS proxy_settings(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL);
        INSERT OR IGNORE INTO proxy_settings VALUES(1,'{}');
        CREATE TABLE IF NOT EXISTS proxy_credential_cleanup(id TEXT PRIMARY KEY);")?;
    Ok(())
}
impl Store {
    pub fn proxy_settings(&self) -> Result<Settings> {
        let data: String =
            self.connect()?
                .query_row("SELECT data FROM proxy_settings WHERE id=1", [], |r| {
                    r.get(0)
                })?;
        let settings: Settings =
            serde_json::from_str(&data).context("Invalid saved proxy settings")?;
        settings.validate()?;
        Ok(settings)
    }

    /// Returns a cleanup warning only after a successful, durable save.
    pub fn save_proxy_settings(
        &self,
        expected: &Settings,
        mut value: Settings,
        change: LoginChange,
    ) -> Result<Option<String>> {
        let _guard = guard()?;
        ensure!(
            &self.proxy_settings()? == expected,
            "Proxy settings changed. Reopen the dialog."
        );
        // Callers cannot inject an arbitrary credential identifier.
        value.credential = None;
        if value.mode != Mode::Custom {
            value = Settings {
                mode: value.mode,
                ..Default::default()
            };
        } else {
            ensure!(
                !value.endpoint.chars().any(char::is_control),
                "Invalid proxy address"
            );
            let url = endpoint(value.endpoint.trim())?;
            value.endpoint = url.to_string();
            match change {
                LoginChange::Keep => {
                    if expected.credential.is_some() {
                        ensure!(
                            expected.endpoint == value.endpoint,
                            "The proxy address changed. Enter a new login or remove the saved login."
                        );
                        value.credential = expected.credential;
                    }
                }
                LoginChange::Remove => {}
                LoginChange::Replace(login) => {
                    login.validate(&url)?;
                    value.credential = Some(Uuid::new_v4());
                    value.validate()?;
                    let id = value.credential.unwrap();
                    let mut blob = Zeroizing::new(serde_json::to_vec(&StoredLogin {
                        endpoint: value.endpoint.clone(),
                        login,
                    })?);
                    ensure!(
                        blob.len() <= 2560,
                        "Proxy credentials are too long for Windows Credential Manager"
                    );
                    retire(&self.connect()?, id)?;
                    if let Err(error) = vault::write(id, &mut blob) {
                        let _ = cleanup_locked(self);
                        return Err(error);
                    }
                }
            }
        }
        let commit = (|| -> Result<()> {
            value.validate()?;
            let mut conn = self.connect()?;
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let data: String =
                tx.query_row("SELECT data FROM proxy_settings WHERE id=1", [], |r| {
                    r.get(0)
                })?;
            ensure!(
                serde_json::from_str::<Settings>(&data)? == *expected,
                "Proxy settings changed. Reopen the dialog."
            );
            if expected.credential != value.credential
                && let Some(id) = expected.credential
            {
                retire(&tx, id)?;
            }
            ensure!(
                tx.execute(
                    "UPDATE proxy_settings SET data=?1 WHERE id=1",
                    [serde_json::to_string(&value)?]
                )? == 1,
                "Proxy settings are unavailable"
            );
            if let Some(id) = value.credential {
                tx.execute(
                    "DELETE FROM proxy_credential_cleanup WHERE id=?1",
                    [id.to_string()],
                )?;
            }
            tx.commit()?;
            Ok(())
        })();
        let cleanup = cleanup_locked(self);
        commit?;
        Ok(cleanup.err().map(|_| {
            "Proxy settings saved. Credential cleanup will retry at the next app start.".into()
        }))
    }
}
fn retire(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO proxy_credential_cleanup(id) VALUES(?1)",
        [id.to_string()],
    )?;
    Ok(())
}
pub(crate) fn cleanup_pending(store: &Store) -> Result<()> {
    let _guard = guard()?;
    cleanup_locked(store)
}
fn cleanup_locked(store: &Store) -> Result<()> {
    let conn = store.connect()?;
    let mut query = conn.prepare("SELECT id FROM proxy_credential_cleanup")?;
    let ids = query
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(query);
    for id in ids {
        let id = Uuid::parse_str(&id).context("Invalid proxy cleanup record")?;
        ensure!(
            store.proxy_settings()?.credential != Some(id),
            "Proxy cleanup conflicts with an active login"
        );
        vault::delete(id)?;
        conn.execute(
            "DELETE FROM proxy_credential_cleanup WHERE id=?1",
            params![id.to_string()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn configure(store: &Store, builder: ClientBuilder) -> Result<ClientBuilder> {
    let route = Route::load(store)?;
    ensure!(
        !route.enterprise(),
        "This caller must use the enterprise HTTP transport"
    );
    route.configure(builder)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: dir.path().join("data"),
            downloads: dir.path().join("files"),
        })
        .unwrap();
        (dir, store)
    }
    fn custom(endpoint: &str) -> Settings {
        Settings {
            mode: Mode::Custom,
            endpoint: endpoint.into(),
            ..Default::default()
        }
    }
    #[test]
    fn migration_roundtrip_and_invalid_routes_fail_closed() {
        let (_dir, store) = store();
        let initial = store.proxy_settings().unwrap();
        assert_eq!(initial, Settings::default());
        for bad in [
            "http://user:secret@proxy.test",
            "http://proxy.test/path",
            "https://proxy.test?x=1",
            "socks4://proxy.test",
            "http://proxy.test:0",
            "http://proxy.test/#fragment",
            "http://proxy.test\r\n",
            "http://@proxy.test",
        ] {
            assert!(
                store
                    .save_proxy_settings(&initial, custom(bad), LoginChange::Remove)
                    .is_err(),
                "{bad}"
            );
            assert_eq!(store.proxy_settings().unwrap(), initial);
        }
        for route in [
            "http://127.0.0.1:8080",
            "https://proxy.test:8443",
            "socks5://127.0.0.1:1080",
            "socks5h://proxy.test:1080",
        ] {
            let original = store.proxy_settings().unwrap();
            store
                .save_proxy_settings(&original, custom(route), LoginChange::Remove)
                .unwrap();
            let reopened = Store::open(store.paths.clone()).unwrap();
            assert_eq!(
                reopened.proxy_settings().unwrap().endpoint,
                endpoint(route).unwrap().to_string()
            );
            assert!(
                configure(&reopened, crate::engine::client_builder())
                    .unwrap()
                    .build()
                    .is_ok()
            );
        }
        store
            .connect()
            .unwrap()
            .execute("UPDATE proxy_settings SET data='{}bad'", [])
            .unwrap();
        assert!(configure(&store, crate::engine::client_builder()).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn enterprise_routes_persist_the_method_and_snapshot_the_matching_vault_login() {
        let (_dir, store) = store();
        for method in [Authentication::Ntlm, Authentication::Negotiate] {
            let initial = store.proxy_settings().unwrap();
            let mut settings = custom("http://127.0.0.1:8080");
            settings.authentication = method;
            store
                .save_proxy_settings(
                    &initial,
                    settings,
                    LoginChange::Replace(Login::new(
                        "FIXTURE\\user".into(),
                        "synthetic-secret".into(),
                    )),
                )
                .unwrap();
            let route = Route::load(&store).unwrap();
            assert!(route.enterprise());
            assert_eq!(route.settings.authentication, method);
            let reopened = Store::open(store.paths.clone()).unwrap();
            assert_eq!(Route::load(&reopened).unwrap().settings, route.settings);
            assert!(!format!("{route:?}").contains("synthetic-secret"));
            store
                .save_proxy_settings(&route.settings, Settings::default(), LoginChange::Remove)
                .unwrap();
            assert!(
                vault::read(route.settings.credential.unwrap())
                    .unwrap()
                    .is_none()
            );
            assert_eq!(route.login.as_ref().unwrap().username, "FIXTURE\\user");
            assert!(Route::load(&store).unwrap().login.is_none());
            for endpoint in ["https://proxy.test:443", "socks5://proxy.test:1080"] {
                let mut invalid = custom(endpoint);
                invalid.authentication = method;
                assert!(invalid.validate().is_err());
            }
        }
    }
    #[cfg(windows)]
    #[test]
    fn vault_binding_replacement_removal_and_rollback_are_transactional() {
        let (_dir, store) = store();
        let original = store.proxy_settings().unwrap();
        let login =
            || LoginChange::Replace(Login::new("proxy-only-user".into(), "secret-canary".into()));
        store
            .save_proxy_settings(&original, custom("https://proxy.test:8443"), login())
            .unwrap();
        let first = store.proxy_settings().unwrap();
        let id = first.credential.unwrap();
        assert!(vault::read(id).unwrap().is_some());
        let data: String = store
            .connect()
            .unwrap()
            .query_row("SELECT data FROM proxy_settings", [], |r| r.get(0))
            .unwrap();
        assert!(!data.contains("proxy-only-user") && !data.contains("secret-canary"));
        assert!(
            store
                .save_proxy_settings(&first, custom("https://other.test:8443"), LoginChange::Keep)
                .is_err()
        );
        store.connect().unwrap().execute_batch("CREATE TRIGGER fail_proxy BEFORE UPDATE ON proxy_settings BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        assert!(
            store
                .save_proxy_settings(&first, first.clone(), login())
                .is_err()
        );
        assert_eq!(store.proxy_settings().unwrap(), first);
        assert!(vault::read(id).unwrap().is_some());
        let pending: i64 = store
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM proxy_credential_cleanup", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pending, 0);
        store
            .connect()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_proxy")
            .unwrap();
        store
            .save_proxy_settings(&first, first.clone(), login())
            .unwrap();
        let second = store.proxy_settings().unwrap();
        assert_ne!(second.credential, first.credential);
        assert!(vault::read(id).unwrap().is_none());
        assert!(
            store
                .save_proxy_settings(&first, first.clone(), LoginChange::Keep)
                .is_err()
        );
        let reopened = Store::open(store.paths.clone()).unwrap();
        assert!(configure(&reopened, crate::engine::client_builder()).is_ok());
        vault::delete(second.credential.unwrap()).unwrap();
        assert!(configure(&store, crate::engine::client_builder()).is_err());
        store
            .save_proxy_settings(
                &second,
                Settings {
                    mode: Mode::Direct,
                    ..Default::default()
                },
                LoginChange::Keep,
            )
            .unwrap();
        assert!(store.proxy_settings().unwrap().credential.is_none());
        assert_eq!(store.proxy_settings().unwrap().mode, Mode::Direct);
        let orphan = Uuid::new_v4();
        retire(&store.connect().unwrap(), orphan).unwrap();
        vault::write(orphan, &mut [1, 2, 3]).unwrap();
        cleanup_pending(&store).unwrap();
        assert!(vault::read(orphan).unwrap().is_none());
    }
    #[test]
    fn plaintext_proxy_auth_requires_explicit_consent_and_secrets_are_redacted() {
        let (_dir, store) = store();
        let initial = store.proxy_settings().unwrap();
        for url in [
            "http://proxy.test:8080",
            "socks5://proxy.test:1080",
            "socks5h://proxy.test:1080",
        ] {
            let error = store
                .save_proxy_settings(
                    &initial,
                    custom(url),
                    LoginChange::Replace(Login::new(
                        "canary-user".into(),
                        "canary-password".into(),
                    )),
                )
                .unwrap_err();
            assert!(error.to_string().contains("does not encrypt"));
            assert!(!format!("{error:?}").contains("canary"));
        }
        assert!(
            !format!(
                "{:?}",
                Login::new("canary-user".into(), "canary-password".into())
            )
            .contains("canary")
        );
        assert!(
            Login::new("a".repeat(256), "password".into())
                .validate(&endpoint("socks5h://proxy.test").unwrap())
                .is_err()
        );
        assert!(
            Login::new("user".into(), "pass\nword".into())
                .validate(&endpoint("https://proxy.test").unwrap())
                .is_err()
        );
    }
}

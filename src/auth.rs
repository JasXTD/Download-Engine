//! Authentication is explicit and per download. Only non-secret binding metadata
//! enters job records; browser snapshots use a separate encrypted table.
//! The vault journal makes interrupted password/token replacement/removal
//! recoverable without enumerating or reading unrelated Windows credentials.
use crate::{
    credential_vault as vault, download_file, engine,
    model::{Job, Status, validate_url},
    store::Store,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{RequestBuilder, Response};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Basic,
    Bearer,
    Browser,
    Ntlm,
    Negotiate,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Basic => "Username and password",
            Self::Bearer => "Bearer token",
            Self::Browser => "Browser login",
            Self::Ntlm => "NTLM",
            Self::Negotiate => "Negotiate (Kerberos / NTLM)",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub id: Uuid,
    pub origin: String,
    pub kind: Kind,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    kind: Kind,
    username: String,
    secret: String,
}
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials([redacted])")
    }
}
impl Drop for Credentials {
    fn drop(&mut self) {
        self.username.zeroize();
        self.secret.zeroize();
    }
}
impl Credentials {
    pub(crate) fn ftp_login(&self) -> Result<(&str, &str)> {
        self.validate()?;
        ensure!(
            self.kind == Kind::Basic,
            "FTPS supports username/password authentication only"
        );
        Ok((&self.username, &self.secret))
    }
    pub fn basic(username: String, password: String) -> Self {
        Self {
            kind: Kind::Basic,
            username,
            secret: password,
        }
    }
    pub fn enterprise(kind: Kind, username: String, password: String) -> Self {
        Self {
            kind,
            username,
            secret: password,
        }
    }
    pub(crate) fn enterprise_login(&self) -> Result<(Kind, &str, &str)> {
        ensure!(
            matches!(self.kind, Kind::Ntlm | Kind::Negotiate),
            "Invalid enterprise authentication method"
        );
        self.validate()?;
        Ok((self.kind, &self.username, &self.secret))
    }
    pub fn bearer(token: String) -> Self {
        Self {
            kind: Kind::Bearer,
            username: String::new(),
            secret: token,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.username.len() <= 512 && self.secret.len() <= 2048,
            "Credentials are too long"
        );
        ensure!(
            !self
                .username
                .chars()
                .chain(self.secret.chars())
                .any(char::is_control),
            "Credentials cannot contain control characters"
        );
        match self.kind {
            Kind::Browser => bail!("Browser login must be sent by the browser extension"),
            Kind::Basic | Kind::Ntlm | Kind::Negotiate => ensure!(
                !self.username.is_empty() && !self.username.contains(':'),
                "Enter a username without a colon"
            ),
            Kind::Bearer => {
                let value = self.secret.trim_end_matches('=');
                ensure!(
                    !value.is_empty()
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b))
                        && self.username.is_empty(),
                    "Enter a bearer token without spaces or a Bearer prefix"
                );
            }
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    binding: Binding,
    credentials: Credentials,
}

#[derive(Debug)]
pub struct Update {
    pub request: Uuid,
    pub job: String,
    pub expected_url: String,
    pub credentials: Option<Credentials>,
    pub restart_confirmed: bool,
    pub resume: bool,
}
#[derive(Debug)]
pub struct Event {
    pub request: Uuid,
    pub result: std::result::Result<Option<String>, String>,
}

/// Inspect an app-owned entry for lifecycle diagnostics without returning its
/// contents. Use the vault's serialized access path even for presence checks.
/// Presence does not prove the server will accept the saved credentials.
pub fn credentials_present(id: Uuid) -> Result<bool> {
    Ok(vault::read(id)?.is_some())
}

pub fn origin(url: &str) -> Result<String> {
    if crate::ftp::is_url(url) {
        let parsed = crate::model::validate_download_url(url)?;
        return crate::ftp::State::new(&parsed)
            .context("Invalid FTP URL")?
            .origin(url);
    }
    Ok(validate_url(url)?.origin().ascii_serialization())
}
pub(crate) fn ftp_credentials(job: &Job) -> Result<Option<Credentials>> {
    let Some(binding) = &job.authentication else {
        return Ok(None);
    };
    let state = job.ftp.as_ref().context("Missing FTP state")?;
    ensure!(
        state.security != crate::ftp::Security::Plain
            && binding.kind == Kind::Basic
            && binding.origin == state.origin(&job.url)?,
        "Saved logins require FTPS and must match this server"
    );
    let blob = vault::read(binding.id)?
        .context("Saved credentials are unavailable. Open Authentication and enter them again.")?;
    let stored: Stored = serde_json::from_slice(&blob)
        .map_err(|_| anyhow::anyhow!("Saved FTPS credentials are invalid. Enter them again."))?;
    ensure!(
        stored.binding == *binding,
        "Saved FTPS credentials do not match this download"
    );
    stored.credentials.ftp_login()?;
    Ok(Some(stored.credentials))
}
pub(crate) fn secure_origin(url: &str) -> Result<String> {
    let url = validate_url(url)?;
    let local = url
        .host_str()
        .and_then(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .is_some_and(|ip| ip.is_loopback());
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && local),
        "Authentication requires HTTPS for remote servers. Plain HTTP is allowed only for a local loopback address."
    );
    Ok(url.origin().ascii_serialization())
}

#[derive(Clone)]
pub(crate) struct DownloadClient {
    client: reqwest::Client,
    authenticated: Option<(String, Arc<Credentials>)>,
    browser: Option<Arc<crate::browser_login::Session>>,
    route: crate::proxy::Route,
    /// The browser page the download came from, sent as an origin-only Referer.
    page: Option<String>,
    public_only: bool,
}
impl DownloadClient {
    pub(crate) fn anonymous(store: &Store) -> Result<Self> {
        let route = crate::proxy::Route::load(store)?;
        let client = route
            .configure(engine::client_builder())?
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            route,
            authenticated: None,
            browser: None,
            page: None,
            public_only: false,
        })
    }
    pub(crate) fn for_job(store: &Store, job: &Job) -> Result<Self> {
        if let Some(binding) = &job.authentication {
            ensure!(
                secure_origin(&job.url)? == binding.origin,
                "Saved authentication belongs to another server. Edit Authentication."
            );
        }
        Self::for_replacement(store, job, &job.url)
    }
    pub(crate) fn with_page(mut self, page: &str) -> Self {
        self.page = Some(page.to_owned());
        self
    }
    pub(crate) fn for_replacement(store: &Store, job: &Job, url: &str) -> Result<Self> {
        if job.public_only {
            ensure!(
                !job.authentication
                    .as_ref()
                    .is_some_and(|binding| matches!(binding.kind, Kind::Ntlm | Kind::Negotiate)),
                "Site reader downloads cannot use NTLM or Negotiate authentication"
            );
        }
        let scope = origin(url)?;
        let binding = job.authentication.as_ref().filter(|b| b.origin == scope);
        let browser = if let Some(binding) = binding.filter(|b| b.kind == Kind::Browser) {
            Some(Arc::new(crate::browser_login::load(store, binding, url)?))
        } else {
            None
        };
        let authenticated = if let Some(binding) = binding.filter(|b| b.kind != Kind::Browser) {
            secure_origin(url)?;
            let blob = vault::read(binding.id)?.context("Saved credentials are unavailable on this Windows account. Open Authentication and enter them again.")?;
            let stored: Stored = serde_json::from_slice(&blob)
                .map_err(|_| anyhow::anyhow!("Saved credentials are invalid. Enter them again."))?;
            ensure!(
                stored.binding == *binding && stored.credentials.kind == binding.kind,
                "Saved credentials do not match this download. Enter them again."
            );
            stored.credentials.validate()?;
            Some((scope, Arc::new(stored.credentials)))
        } else {
            None
        };
        let route = crate::proxy::Route::load(store)?;
        let client = route
            .configure(engine::client_builder())?
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            authenticated,
            browser,
            route,
            page: job.source_page.clone(),
            public_only: job.public_only,
        })
    }
    pub(crate) fn get(&self, url: &str) -> Result<DownloadRequest> {
        let mut request = self.client.get(url);
        if let Some(referer) = self.page.as_deref().and_then(|page| {
            reqwest::Url::parse(url)
                .ok()
                .and_then(|target| crate::model::page_referer(page, &target))
        }) {
            request = request.header(reqwest::header::REFERER, referer);
        }
        if let Some((scope, credentials)) = &self.authenticated {
            ensure!(
                secure_origin(url)? == *scope,
                "Authentication cannot be sent to another server. Use a direct link for the intended server."
            );
            request = match credentials.kind {
                Kind::Basic => request.basic_auth(&credentials.username, Some(&credentials.secret)),
                Kind::Bearer => request.bearer_auth(&credentials.secret),
                Kind::Ntlm | Kind::Negotiate => request,
                Kind::Browser => bail!("Invalid credential kind"),
            };
        }
        Ok(DownloadRequest {
            request,
            client: self.client.clone(),
            browser: self.browser.clone(),
            route: self.route.clone(),
            authenticated: self.authenticated.clone(),
            public_only: self.public_only,
            same_origin: false,
        })
    }
}
pub(crate) struct DownloadRequest {
    same_origin: bool,
    public_only: bool,
    route: crate::proxy::Route,
    authenticated: Option<(String, Arc<Credentials>)>,
    request: RequestBuilder,
    client: reqwest::Client,
    browser: Option<Arc<crate::browser_login::Session>>,
}
impl DownloadRequest {
    pub(crate) fn same_origin(mut self) -> Self {
        self.same_origin = true;
        self
    }
    pub(crate) fn header(
        mut self,
        name: reqwest::header::HeaderName,
        value: impl AsRef<str>,
    ) -> Self {
        self.request = self.request.header(name, value.as_ref());
        self
    }
    pub(crate) async fn send(self) -> Result<Response> {
        // One deadline covers the entire redirect chain. Body read deadlines
        // remain in the engine, so speed limits do not look like stalled reads.
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut request = self.request.build().map_err(crate::retry::network)?;
            for redirects in 0..=10 {
                if let Some(session) = &self.browser {
                    let header = session.header(request.url())?;
                    request.headers_mut().remove(reqwest::header::COOKIE);
                    if let Some(header) = header { request.headers_mut().insert(reqwest::header::COOKIE, header); }
                }
                let mut next = request
                    .try_clone()
                    .context("Cannot retry browser download request")?;
                let native = self.authenticated.as_ref().filter(|(_, c)| matches!(c.kind, Kind::Ntlm | Kind::Negotiate)).map(|(_, c)| c.clone());
                let client = if self.public_only {
                    crate::network_scope::public_client(request.url(), &self.route).await?
                } else { self.client.clone() };
                let response = self.route.execute(&client, request, native).await?;
                if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                    ensure!(redirects < 10, "Too many browser download redirects");
                    let location = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .context("Invalid browser download redirect")?;
                    let url = response
                        .url()
                        .join(location)
                        .map_err(|_| anyhow::anyhow!("Invalid browser download redirect"))?;
                    // Validate before any request to the next origin is made.
                    validate_url(url.as_str())?;
                    if self.same_origin && url.origin() != next.url().origin() {
                        return Err(crate::link_fallback::CrossOriginRedirect.into());
                    }
                    ensure!(next.url().scheme() != "https" || url.scheme() == "https", "Refusing an HTTPS to HTTP redirect");
                    if let Some((scope, _)) = &self.authenticated {
                        ensure!(url.origin().ascii_serialization() == *scope, "Authenticated downloads cannot redirect to another server. Use a direct link for the intended server.");
                    }
                    if let Some(session) = &self.browser { session.header(&url)?; }
                    *next.url_mut() = url;
                    request = next;
                } else {
                    if let Some(session) = &self.browser { session.check_response(&response)?; }
                    return Ok(response);
                }
            }
            bail!("Too many browser download redirects")
        })
        .await
        .map_err(|_| crate::retry::incomplete("The server connection timed out."))?
    }
}
pub(crate) fn check_response(response: &Response) -> Result<()> {
    match response.status().as_u16() {
        407 => {
            bail!(
                "Proxy login was required or rejected (HTTP 407). Check Preferences → Connection → Proxy."
            )
        }
        401 => bail!(
            "Authentication is required or was rejected (HTTP 401). Open Authentication to enter or replace credentials."
        ),
        403 => bail!(
            "Access was refused (HTTP 403). Check Authentication or update an expired download link."
        ),
        _ => Ok(()),
    }
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS credential_cleanup(id TEXT PRIMARY KEY);")?;
    crate::browser_login::initialize(conn)
}
pub(crate) fn retire(conn: &Connection, binding: &Binding) -> Result<()> {
    if binding.kind == Kind::Browser {
        return crate::browser_login::retire(conn, binding.id);
    }
    conn.execute(
        "INSERT OR IGNORE INTO credential_cleanup(id) VALUES (?1)",
        [binding.id.to_string()],
    )?;
    Ok(())
}
// Completion and authentication updates run on different threads. Keep cleanup
// from mistaking an in-flight vault write for an orphan from a previous run.
static VAULT_LOCK: Mutex<()> = Mutex::new(());
fn credential_guard() -> Result<MutexGuard<'static, ()>> {
    VAULT_LOCK.lock().map_err(|_| {
        anyhow::anyhow!(
            "Credential storage was interrupted. Restart the app before changing authentication."
        )
    })
}
pub(crate) fn cleanup_pending(store: &Store) -> Result<()> {
    let _guard = credential_guard()?;
    cleanup_locked(store)
}
fn cleanup_locked(store: &Store) -> Result<()> {
    let conn = store.connect()?;
    let mut stmt = conn.prepare("SELECT id FROM credential_cleanup")?;
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    for id in ids {
        let id = Uuid::parse_str(&id).context("Invalid credential cleanup record")?;
        // Defensive check: a journal error must not delete a live binding.
        ensure!(
            !store
                .list()?
                .iter()
                .any(|job| job.authentication.as_ref().is_some_and(|b| b.id == id)),
            "Credential cleanup conflicts with a live download"
        );
        vault::delete(id)?;
        conn.execute(
            "DELETE FROM credential_cleanup WHERE id=?1",
            [id.to_string()],
        )?;
    }
    Ok(())
}
pub fn needs_restart(store: &Store, job: &Job) -> bool {
    if job.plan.is_some() || job.downloaded != 0 {
        return true;
    }
    let Ok(parts) = store.paths.parts(&job.id) else {
        return true;
    };
    match std::fs::read_dir(parts) {
        Ok(mut entries) => entries.any(|entry| {
            entry
                .and_then(|e| e.metadata())
                .map_or(true, |m| m.len() > 0)
        }),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Called only by the scheduler after excluding workers and link checks.
pub(crate) fn apply(store: &Store, update: Update) -> Result<Option<String>> {
    let _guard = credential_guard()?;
    let original = store
        .get(&update.job)?
        .context("Download no longer exists")?;
    ensure!(
        original.provider.is_none(),
        "Provider downloads do not use saved browser or application logins"
    );
    ensure!(
        original.link_fallback.is_none(),
        "Webpage file discovery uses anonymous access. Add the direct file URL as a new download to use a login."
    );
    ensure!(
        !crate::mega::is_url(&original.url),
        "MEGA public links use their share-link key and do not support saved logins."
    );
    ensure!(
        !crate::wetransfer::is_url(&original.url),
        "WeTransfer share links use anonymous access and do not support saved logins."
    );
    ensure!(
        original.hls.is_none(),
        "HLS previews and downloads currently use anonymous requests"
    );
    ensure!(
        Uuid::parse_str(&original.id).is_ok(),
        "Invalid saved download identifier"
    );
    ensure!(
        original.url == update.expected_url,
        "The download link changed. Open Authentication again."
    );
    ensure!(
        matches!(original.status, Status::Paused | Status::Failed),
        "Pause this download before changing authentication"
    );
    ensure!(
        original.saved_path.is_none(),
        "Resume this download to recover its pending completion first"
    );
    let resetting = needs_restart(store, &original);
    ensure!(
        !resetting || update.restart_confirmed,
        "Confirm restarting this download before changing authentication"
    );
    let new_binding = if let Some(credentials) = update.credentials {
        credentials.validate()?;
        let origin = if let Some(ftp) = &original.ftp {
            ensure!(
                ftp.security != crate::ftp::Security::Plain,
                "Saved logins require FTPS. Add an ftps:// URL; plain FTP supports anonymous downloads only."
            );
            credentials.ftp_login()?;
            ftp.origin(&original.url)?
        } else {
            secure_origin(&original.url)?
        };
        let binding = Binding {
            id: Uuid::new_v4(),
            origin,
            kind: credentials.kind,
        };
        let mut bytes = Zeroizing::new(serde_json::to_vec(&Stored {
            binding: binding.clone(),
            credentials,
        })?);
        ensure!(
            bytes.len() <= 2560,
            "These credentials are too long for Windows Credential Manager"
        );
        // Journal before the external write. A crash before commit leaves only
        // an orphan, which the owning scheduler deletes on the next start.
        retire(&store.connect()?, &binding)?;
        if let Err(error) = vault::write(binding.id, &mut bytes) {
            let _ = cleanup_locked(store); // Failed cleanup remains journaled.
            return Err(error);
        }
        Some(binding)
    } else {
        None
    };
    let commit = (|| -> Result<()> {
        let mut conn = store.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let data: Option<String> = tx
            .query_row("SELECT data FROM jobs WHERE id=?1", [&original.id], |r| {
                r.get(0)
            })
            .optional()?;
        let data = data.context("Download no longer exists")?;
        ensure!(
            serde_json::from_str::<serde_json::Value>(&data)? == serde_json::to_value(&original)?,
            "The download changed. Open Authentication again."
        );
        let mut job = original.clone();
        if resetting {
            download_file::reset_parts(
                &store.paths.data.join("parts"),
                &store.paths.parts(&job.id)?,
            )?;
            let staging = job.directory.join(format!(".idm-ex-{}.assembling", job.id));
            match std::fs::remove_file(staging) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("Cannot reset the temporary download"),
            }
        }
        job.plan = None;
        if let Some(ftp) = &mut job.ftp {
            ftp.reset();
        }
        crate::synchronization::restarting(&mut job);
        job.expected_file = None;
        job.integrity.actual = None;
        job.downloaded = 0;
        job.total = None;
        if let Some(old) = &job.authentication {
            retire(&tx, old)?;
        }
        job.authentication = new_binding.clone();
        job.retry = crate::retry::State::default();
        job.error = None;
        job.status = if update.resume {
            Status::Queued
        } else {
            Status::Paused
        };
        tx.execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            params![serde_json::to_string(&job)?, job.id],
        )?;
        tx.execute(
            "UPDATE job_queues SET paused_by_queue=0 WHERE job_id=?1",
            [&job.id],
        )?;
        crate::queue_start::wake_idle_main(&tx, &job.id)?;
        if let Some(binding) = &new_binding {
            tx.execute(
                "DELETE FROM credential_cleanup WHERE id=?1",
                [binding.id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    })();
    let cleanup = cleanup_locked(store);
    commit?;
    Ok(cleanup.err().map(|_| {
        "Authentication saved. Windows credential cleanup will retry when the app starts.".into()
    }))
}

/// Completion is durable before credentials are deleted. On cancellation/failure
/// the binding is retained so the next worker can resume after an app restart.
pub(crate) fn save_finished(store: &Store, job: &mut Job) -> Result<()> {
    if job.status != Status::Complete || job.authentication.is_none() {
        return store.save(job);
    }
    let _guard = credential_guard()?;
    let mut conn = store.connect()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let binding = job
        .authentication
        .as_ref()
        .context("Missing authentication")?;
    retire(&tx, binding)?;
    let mut completed = job.clone();
    completed.authentication = None;
    let count = tx.execute(
        "UPDATE jobs SET data=?1 WHERE id=?2",
        params![serde_json::to_string(&completed)?, job.id],
    )?;
    ensure!(count == 1, "Download no longer exists");
    tx.commit()?;
    *job = completed;
    Ok(()) // The engine clears partial files before retrying vault cleanup.
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn browser_page_downloads_send_only_the_page_origin_as_referer() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let referer = |client: &DownloadClient, url: &str| {
            client
                .get(url)
                .unwrap()
                .request
                .build()
                .unwrap()
                .headers()
                .get(reqwest::header::REFERER)
                .map(|v| v.to_str().unwrap().to_owned())
        };
        let mut job = store
            .new_download(
                "https://cdn.example.test/file.zip",
                None,
                None,
                "referer-fixture".into(),
            )
            .unwrap();
        let plain = DownloadClient::for_job(&store, &job).unwrap();
        assert_eq!(referer(&plain, &job.url), None);
        job.source_page = Some("https://shop.example.test/item?id=1".into());
        let client = DownloadClient::for_job(&store, &job).unwrap();
        assert_eq!(
            referer(&client, &job.url).as_deref(),
            Some("https://shop.example.test/")
        );
        assert_eq!(referer(&client, "http://cdn.example.test/file.zip"), None);
    }

    #[test]
    fn cleanup_waits_for_a_staged_credential_to_commit() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: temp.path().join("data"),
            downloads: temp.path().join("downloads"),
        })
        .unwrap();
        let mut job = Job::new(
            "https://example.com/file",
            None,
            store.paths.downloads.clone(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = Status::Paused;
        store.enqueue(&job).unwrap();
        let binding = Binding {
            id: Uuid::new_v4(),
            origin: origin(&job.url).unwrap(),
            kind: Kind::Basic,
        };
        struct Cleanup(Uuid);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = vault::delete(self.0);
            }
        }
        let _cleanup = Cleanup(binding.id);
        let guard = credential_guard().unwrap();
        retire(&store.connect().unwrap(), &binding).unwrap();
        let mut blob = Zeroizing::new(b"synthetic staged credential".to_vec());
        vault::write(binding.id, &mut blob).unwrap();
        let cloned = store.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(cleanup_pending(&cloned)).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Bounded non-completion assertion: cleanup must wait at the same lock
        // held across journal -> external vault write -> durable job binding.
        let blocked = matches!(
            done_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        job.authentication = Some(binding.clone());
        let mut conn = store.connect().unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            params![serde_json::to_string(&job).unwrap(), job.id],
        )
        .unwrap();
        tx.execute(
            "DELETE FROM credential_cleanup WHERE id=?1",
            [binding.id.to_string()],
        )
        .unwrap();
        tx.commit().unwrap();
        drop(guard);
        if blocked {
            done_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
        }
        worker.join().unwrap();
        assert!(
            blocked,
            "Cleanup ran while the credential was still being saved"
        );
        assert!(vault::read(binding.id).unwrap().is_some());
        store.remove(&job.id).unwrap();
        assert!(vault::read(binding.id).unwrap().is_none());
    }
}

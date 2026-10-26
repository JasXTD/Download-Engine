//! Explicit, first-party browser cookie snapshots. Only DPAPI ciphertext is
//! persisted; queue metadata and replay receipts never contain cookies.
use crate::{
    auth::{self, Binding, Kind},
    model::validate_url,
    store::Store,
};
use anyhow::{Context, Result, ensure};
use reqwest::{Url, header::HeaderValue};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

pub(crate) const MAX_BYTES: usize = 128 * 1024;
const MAX_HEADER: usize = 32 * 1024;
const LIFETIME: i64 = 24 * 60 * 60;
pub(crate) const EXPIRED: &str = "Browser login expired or was rejected. Sign in to the site, then send the file again using Use this site's login.";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub host_only: bool,
    pub secure: bool,
    pub expires: Option<i64>,
}
impl fmt::Debug for Cookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cookie([redacted])")
    }
}
impl Drop for Cookie {
    fn drop(&mut self) {
        self.name.zeroize();
        self.value.zeroize();
    }
}
impl Cookie {
    fn matches(&self, url: &Url) -> bool {
        let host = url.host_str().unwrap_or("");
        let domain = self.domain.trim_start_matches('.');
        let domain_match = host.eq_ignore_ascii_case(domain)
            || (!self.host_only
                && host.to_ascii_lowercase().ends_with(&format!(".{domain}"))
                && host.parse::<std::net::IpAddr>().is_err());
        let path = url.path();
        domain_match
            && (!self.secure || url.scheme() == "https")
            && (path == self.path
                || (path.starts_with(&self.path)
                    && (self.path.ends_with('/')
                        || path.as_bytes().get(self.path.len()) == Some(&b'/'))))
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 256
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_\x60|~".contains(&b)),
            "Invalid browser cookie name"
        );
        ensure!(
            self.value.len() <= 16 * 1024
                && self.value.bytes().all(|b| b == 0x21
                    || (0x23..=0x2b).contains(&b)
                    || (0x2d..=0x3a).contains(&b)
                    || (0x3c..=0x5b).contains(&b)
                    || (0x5d..=0x7e).contains(&b)),
            "Invalid browser cookie value"
        );
        let domain = self.domain.trim_start_matches('.');
        ensure!(
            !domain.is_empty()
                && self.domain.len() <= 255
                && domain
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
                && domain == domain.to_ascii_lowercase(),
            "Invalid browser cookie domain"
        );
        ensure!(
            self.path.starts_with('/')
                && self.path.len() <= 4096
                && self.path.bytes().all(|b| (0x21..=0x7e).contains(&b)),
            "Invalid browser cookie path"
        );
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    binding: Binding,
    expires: i64,
    cookies: Vec<Cookie>,
}
impl Session {
    fn validate(&self, url: &str) -> Result<()> {
        ensure!(
            self.binding.kind == Kind::Browser && auth::secure_origin(url)? == self.binding.origin,
            "Browser login belongs to another server"
        );
        ensure!(
            !self.cookies.is_empty() && self.cookies.len() <= 128,
            "Invalid browser cookie count"
        );
        let mut length = 0;
        for cookie in &self.cookies {
            cookie.validate()?;
            length += cookie.name.len() + cookie.value.len() + 3;
        }
        ensure!(
            length <= MAX_HEADER,
            "This site's browser login is too large to transfer"
        );
        self.check_time()?;
        Ok(())
    }
    fn check_time(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        ensure!(
            self.expires > now && self.expires <= now + LIFETIME,
            EXPIRED
        );
        Ok(())
    }
    pub(crate) fn header(&self, url: &Url) -> Result<Option<HeaderValue>> {
        self.check_time()?;
        ensure!(
            auth::secure_origin(url.as_str())? == self.binding.origin,
            "Browser login cannot redirect to another server. Download this file in the browser."
        );
        let now = chrono::Utc::now().timestamp();
        ensure!(
            self.cookies
                .iter()
                .any(|c| c.expires.is_none_or(|e| e > now)),
            EXPIRED
        );
        let mut value = Zeroizing::new(String::new());
        for cookie in self
            .cookies
            .iter()
            .filter(|c| c.matches(url) && c.expires.is_none_or(|e| e > now))
        {
            if !value.is_empty() {
                value.push_str("; ");
            }
            value.push_str(&cookie.name);
            value.push('=');
            value.push_str(&cookie.value);
        }
        if value.is_empty() {
            return Ok(None);
        }
        let mut header = HeaderValue::from_str(&value).context("Invalid browser login header")?;
        header.set_sensitive(true);
        Ok(Some(header))
    }
    pub(crate) fn check_response(&self, response: &reqwest::Response) -> Result<()> {
        ensure!(!matches!(response.status().as_u16(), 401 | 403), EXPIRED);
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        ensure!(
            !response.status().is_success()
                || (!mime.eq_ignore_ascii_case("text/html")
                    && !mime.eq_ignore_ascii_case("application/xhtml+xml")),
            "The site returned a web page instead of a file. Your browser login may have expired. Sign in and send a direct file link again."
        );
        Ok(())
    }
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS browser_sessions(id TEXT PRIMARY KEY, sealed BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS browser_login_receipts(request_id TEXT PRIMARY KEY,
            fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32), job_id TEXT NOT NULL);",
    )?;
    Ok(())
}
pub(crate) fn retire(conn: &Connection, id: Uuid) -> Result<()> {
    conn.execute("DELETE FROM browser_sessions WHERE id=?1", [id.to_string()])?;
    Ok(())
}

/// The encrypted session, job and receipt commit together. The desktop can never
/// observe an incomplete binding, and a crash cannot orphan an external secret.
pub(crate) fn enqueue(
    store: &Store,
    request_id: String,
    url: String,
    mut cookies: Vec<Cookie>,
) -> Result<String> {
    let token = request_id
        .strip_prefix("login:")
        .context("Invalid browser login request identifier")?;
    ensure!(
        Uuid::parse_str(token).is_ok(),
        "Invalid browser login request identifier"
    );
    let url = validate_url(&url)?;
    ensure!(
        !crate::hls::is_playlist_url(url.as_str()),
        "HLS / DASH streams need an anonymous video preview; browser login is not supported for streams yet"
    );
    let origin = auth::secure_origin(url.as_str())?;
    let fingerprint = Sha256::digest(url.as_str().as_bytes());
    let mut conn = store.connect()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous: Option<(Vec<u8>, String)> = tx
        .query_row(
            "SELECT fingerprint,job_id FROM browser_login_receipts WHERE request_id=?1",
            [&request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((saved, id)) = previous {
        ensure!(
            saved == fingerprint.as_slice(),
            "Browser login request belongs to a different file"
        );
        return Ok(id);
    }
    let now = chrono::Utc::now().timestamp();
    ensure!(
        !cookies.is_empty() && cookies.len() <= 128,
        "No usable browser login cookies were supplied"
    );
    for cookie in &cookies {
        cookie.validate()?;
        ensure!(
            cookie.matches(&url),
            "Browser cookie does not belong to this file"
        );
        ensure!(cookie.expires.is_none_or(|e| e > now), EXPIRED);
    }
    cookies.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
    let binding = Binding {
        id: Uuid::new_v4(),
        origin,
        kind: Kind::Browser,
    };
    let session = Session {
        binding: binding.clone(),
        expires: now + LIFETIME,
        cookies,
    };
    session.validate(url.as_str())?;
    let mut bytes = Zeroizing::new(serde_json::to_vec(&session)?);
    ensure!(
        bytes.len() <= MAX_BYTES,
        "This site's browser login is too large to transfer"
    );
    let sealed = crate::protected_data::seal(binding.id, &mut bytes)?;
    let receipt_id = request_id.clone();
    let mut job = store.new_download(url.as_str(), None, None, request_id)?;
    job.authentication = Some(binding);
    let id = job.id.clone();
    tx.execute(
        "INSERT INTO browser_sessions(id,sealed) VALUES(?1,?2)",
        params![session.binding.id.to_string(), sealed.as_slice()],
    )?;
    let saved = crate::queue_store::enqueue_batch_in(&tx, &[job], crate::queue::MAIN_QUEUE)?;
    ensure!(
        saved[0].id == id,
        "Browser login request identifier is already in use"
    );
    tx.execute(
        "INSERT INTO browser_login_receipts(request_id,fingerprint,job_id) VALUES(?1,?2,?3)",
        params![receipt_id, fingerprint.as_slice(), id],
    )?;
    tx.commit()?;
    Ok(id)
}
pub(crate) fn load(store: &Store, binding: &Binding, url: &str) -> Result<Session> {
    let sealed: Vec<u8> = store.connect()?.query_row(
        "SELECT sealed FROM browser_sessions WHERE id=?1", [binding.id.to_string()], |r| r.get(0))
        .optional()?.context("Saved browser login is unavailable. Sign in and send this file from the browser again.")?;
    ensure!(
        sealed.len() <= MAX_BYTES + 4096,
        "Invalid protected browser login"
    );
    let bytes = crate::protected_data::open(binding.id, &sealed)?;
    let session: Session = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid protected browser login"))?;
    ensure!(
        session.binding == *binding,
        "Browser login does not match this download"
    );
    session.validate(url)?;
    Ok(session)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            binding: Binding {
                id: Uuid::new_v4(),
                origin: "https://site.test".into(),
                kind: Kind::Browser,
            },
            expires: chrono::Utc::now().timestamp() + 3600,
            cookies: vec![Cookie {
                name: "session".into(),
                value: "synthetic-secret".into(),
                domain: "site.test".into(),
                path: "/files".into(),
                host_only: true,
                secure: true,
                expires: None,
            }],
        }
    }
    #[test]
    fn cookie_headers_are_sensitive_and_follow_domain_path_secure_and_expiry_rules() {
        let mut saved = session();
        let url = Url::parse("https://site.test/files/a").unwrap();
        assert!(saved.header(&url).unwrap().unwrap().is_sensitive());
        for path in ["/files-other/a", "/other", "/Files/a"] {
            assert!(
                saved
                    .header(&Url::parse(&format!("https://site.test{path}")).unwrap())
                    .unwrap()
                    .is_none()
            );
        }
        for target in [
            "http://site.test/files/a",
            "https://other.test/files/a",
            "https://site.test:8443/files/a",
            "https://user:password@site.test/files/a",
        ] {
            assert!(saved.header(&Url::parse(target).unwrap()).is_err());
        }
        saved.cookies[0].expires = Some(1);
        assert!(
            saved
                .header(&url)
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
        saved.cookies[0].expires = None;
        saved.expires = 1;
        assert!(
            saved
                .header(&url)
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
    }
    #[test]
    fn dpapi_large_snapshot_round_trip_rejects_wrong_binding_and_tampering() {
        let mut bytes = Zeroizing::new(vec![b's'; 32000]);
        let id = Uuid::new_v4();
        let mut sealed = crate::protected_data::seal(id, &mut bytes).unwrap();
        assert_ne!(&*sealed, &*bytes);
        assert_eq!(&*crate::protected_data::open(id, &sealed).unwrap(), &*bytes);
        assert!(crate::protected_data::open(Uuid::new_v4(), &sealed).is_err());
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(crate::protected_data::open(id, &sealed).is_err());
    }
}

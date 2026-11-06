//! Explicit credentials for one origin and one Lua run; never browser/profile cookies.
use anyhow::{Result, ensure};
use reqwest::{
    Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
use zeroize::Zeroize;

struct Credentials {
    cookies: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
}
impl Drop for Credentials {
    fn drop(&mut self) {
        for value in self.cookies.values_mut().chain(self.headers.values_mut()) {
            value.zeroize();
        }
    }
}

#[derive(Clone)]
pub struct HttpSession {
    id: Uuid,
    origin: String,
    credentials: Arc<Credentials>,
    jar: Arc<Mutex<super::fetch::cookies::Jar>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    origin: String,
    headers: BTreeMap<String, String>,
    jar: super::fetch::cookies::Jar,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        for value in self.headers.values_mut() {
            value.zeroize();
        }
    }
}
impl std::fmt::Debug for HttpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSession")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
impl mlua::UserData for HttpSession {}
impl HttpSession {
    pub(crate) fn new(
        origin: &str,
        cookies: BTreeMap<String, String>,
        headers: BTreeMap<String, String>,
    ) -> Result<Self> {
        let url = crate::model::validate_url(origin)?;
        ensure!(
            url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
            "HTTP sessions need an origin without a path, query or fragment"
        );
        // Loopback is useful for isolated tests; the ordinary public-address policy
        // still applies at fetch time. This does not grant local network access.
        let loopback = url
            .host_str()
            .and_then(|h| h.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        ensure!(
            url.scheme() == "https" || loopback,
            "Authenticated HTTP sessions require HTTPS"
        );
        ensure!(
            cookies.len() <= 50 && headers.len() <= 32,
            "HTTP session has too many cookies or headers"
        );
        ensure!(
            cookies
                .iter()
                .chain(headers.iter())
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                <= 16384,
            "HTTP session credentials exceed 16 KiB"
        );
        for (name, value) in &cookies {
            ensure!(valid_cookie(name, value), "Invalid HTTP session cookie");
        }
        let mut checked = HeaderMap::new();
        for (name, value) in &headers {
            insert_header(&mut checked, name, value)?;
        }
        Ok(Self {
            id: Uuid::new_v4(),
            origin: url.origin().ascii_serialization(),
            jar: Arc::new(Mutex::new(super::fetch::cookies::Jar::seeded(&cookies))),
            credentials: Arc::new(Credentials { cookies, headers }),
        })
    }
    pub(crate) fn check_origin(&self, url: &Url) -> Result<()> {
        ensure!(
            url.origin().ascii_serialization() == self.origin,
            "An authenticated HTTP session cannot reach another origin"
        );
        Ok(())
    }
    pub(crate) fn jar_id(&self) -> String {
        self.id.to_string()
    }
    pub(crate) fn cookie_header(&self, url: &Url) -> Result<Option<String>> {
        self.check_origin(url)?;
        Ok(self
            .jar
            .lock()
            .map_err(|_| anyhow::anyhow!("HTTP session unavailable"))?
            .header(url))
    }
    pub(crate) fn keep_cookies(&self, url: &Url, values: &[String]) -> Result<()> {
        self.check_origin(url)?;
        self.jar
            .lock()
            .map_err(|_| anyhow::anyhow!("HTTP session unavailable"))?
            .keep(url, values);
        Ok(())
    }
    pub(crate) fn seal(&self, scope: Uuid) -> Result<String> {
        use base64::Engine as _;
        let snapshot = Snapshot {
            origin: self.origin.clone(),
            headers: self.credentials.headers.clone(),
            jar: self
                .jar
                .lock()
                .map_err(|_| anyhow::anyhow!("HTTP session unavailable"))?
                .clone(),
        };
        let mut bytes = zeroize::Zeroizing::new(serde_json::to_vec(&snapshot)?);
        ensure!(
            bytes.len() <= 256 * 1024,
            "HTTP session exceeds handoff limit"
        );
        Ok(base64::engine::general_purpose::STANDARD
            .encode(crate::protected_data::seal(scope, &mut bytes)?))
    }
    pub(crate) fn open(scope: Uuid, token: &str) -> Result<Self> {
        use base64::Engine as _;
        ensure!(token.len() <= 360_000, "HTTP session exceeds handoff limit");
        let sealed = base64::engine::general_purpose::STANDARD.decode(token)?;
        let bytes = crate::protected_data::open(scope, &sealed)?;
        ensure!(
            bytes.len() <= 256 * 1024,
            "HTTP session exceeds handoff limit"
        );
        let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
        snapshot.jar.validate()?;
        let session = Self::new(&snapshot.origin, BTreeMap::new(), snapshot.headers.clone())?;
        *session
            .jar
            .lock()
            .map_err(|_| anyhow::anyhow!("HTTP session unavailable"))? = snapshot.jar.clone();
        Ok(session)
    }
    pub(crate) fn apply_headers(&self, headers: &mut HeaderMap) -> Result<()> {
        for (name, value) in &self.credentials.headers {
            let parsed = HeaderName::from_bytes(name.as_bytes())?;
            ensure!(
                !headers.contains_key(&parsed),
                "A request cannot override an HTTP session header"
            );
            insert_header(headers, name, value)?;
        }
        Ok(())
    }
    pub(crate) fn apply_download_headers(&self, headers: &mut HeaderMap) -> Result<()> {
        // Download clients have a default User-Agent. Explicit session identity
        // replaces that default, but cannot override framing or range headers.
        let agent = headers.remove(reqwest::header::USER_AGENT);
        self.apply_headers(headers)?;
        if let Some(agent) = agent {
            headers.entry(reqwest::header::USER_AGENT).or_insert(agent);
        }
        Ok(())
    }
}
pub(super) fn valid_cookie(name: &str, value: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && value.len() <= 4096
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        && value.bytes().all(|b| {
            b == 0x21
                || (0x23..=0x2b).contains(&b)
                || (0x2d..=0x3a).contains(&b)
                || (0x3c..=0x5b).contains(&b)
                || (0x5d..=0x7e).contains(&b)
        })
}
fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) -> Result<()> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| anyhow::anyhow!("Invalid HTTP session header name"))?;
    ensure!(
        !super::fetch::reserved(name.as_str()),
        "HTTP session cannot set this header"
    );
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| anyhow::anyhow!("Invalid HTTP session header value"))?;
    value.set_sensitive(true);
    ensure!(
        !headers.contains_key(&name),
        "Duplicate HTTP session header"
    );
    headers.insert(name, value);
    Ok(())
}

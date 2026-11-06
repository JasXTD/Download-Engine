//! `xd.fetch` for Lua site readers. It reaches the public internet only (every
//! address, including each redirect, is checked and pinned before connecting),
//! sends no browser cookies or saved logins, and is bounded in requests, size
//! and time. Cookies a site sets are kept in memory for the rest of the run and
//! sent back only to the same origin.
use crate::engine;
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    Method, Url,
    header::{self, HeaderMap, HeaderName, HeaderValue},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};
pub(crate) mod cookies;

pub const MAX_REQUESTS: usize = 50;
pub const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_HEADERS: usize = 32;
const MAX_REDIRECTS: usize = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
/// Headers a script may not set: identity, framing and encoding stay the app's.
const RESERVED: [&str; 12] = [
    "cookie",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "te",
    "trailer",
    "accept-encoding",
    "proxy-authorization",
    "proxy-connection",
];
pub(super) fn reserved(name: &str) -> bool {
    RESERVED.contains(&name) || name.starts_with("proxy-")
}

#[derive(Clone, Debug, Default)]
pub struct FetchRequest {
    pub url: String,
    pub method: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub session: Option<super::HttpSession>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FetchReply {
    pub status: u16,
    /// The final address after redirects.
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

struct Session {
    route: crate::proxy::Route,
    cookies: HashMap<String, cookies::Jar>,
    requests: usize,
    /// An extension's declared hosts. Every request and redirect must match.
    hosts: Option<Vec<String>>,
    #[cfg(test)]
    allow_local: bool,
}

pub struct Fetcher {
    runtime: tokio::runtime::Runtime,
    session: Session,
}

impl Fetcher {
    pub fn new(route: crate::proxy::Route) -> Result<Self> {
        Ok(Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
            session: Session {
                route,
                cookies: HashMap::new(),
                requests: 0,
                hosts: None,
                #[cfg(test)]
                allow_local: false,
            },
        })
    }
    /// Refuses every host outside `hosts`, including those a redirect leads to.
    pub fn limit_hosts(mut self, hosts: Vec<String>) -> Self {
        self.session.hosts = Some(hosts);
        self
    }
    /// Test servers listen on loopback; real readers never reach it.
    #[cfg(test)]
    pub(crate) fn allowing_local(mut self) -> Self {
        self.session.allow_local = true;
        self
    }
    pub fn fetch(&mut self, request: FetchRequest) -> Result<FetchReply> {
        self.runtime.block_on(self.session.fetch(request))
    }
}

impl Session {
    async fn fetch(&mut self, request: FetchRequest) -> Result<FetchReply> {
        let mut method = match request
            .method
            .as_deref()
            .unwrap_or("GET")
            .to_ascii_uppercase()
            .as_str()
        {
            "GET" => Method::GET,
            "POST" => Method::POST,
            "HEAD" => Method::HEAD,
            "PUT" => Method::PUT,
            "PATCH" => Method::PATCH,
            "DELETE" => Method::DELETE,
            "OPTIONS" => Method::OPTIONS,
            _ => bail!("Unsupported HTTP method"),
        };
        let mut body = request.body;
        ensure!(
            !matches!(method, Method::GET | Method::HEAD) || body.is_none(),
            "GET and HEAD requests cannot send a body"
        );
        ensure!(
            body.as_ref().is_none_or(|b| b.len() <= MAX_BODY_BYTES),
            "An xd.fetch body can be at most 1 MiB"
        );
        ensure!(
            request.headers.len() <= MAX_HEADERS,
            "xd.fetch accepts at most {MAX_HEADERS} headers"
        );
        let mut headers = HeaderMap::new();
        for (name, value) in &request.headers {
            let name = HeaderName::from_bytes(name.trim().as_bytes())
                .ok()
                .with_context(|| format!("Invalid header name: {name}"))?;
            ensure!(
                !reserved(name.as_str()),
                "xd.fetch cannot set the {name} header"
            );
            let value = HeaderValue::from_str(value)
                .ok()
                .with_context(|| format!("Invalid value for the {name} header"))?;
            headers.insert(name, value);
        }
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        let mut url = crate::model::validate_url(&request.url)?;
        let jar_id = if let Some(session) = &request.session {
            session.check_origin(&url)?;
            session.apply_headers(&mut headers)?;
            let id = session.jar_id();
            Some(id)
        } else {
            None
        };
        headers
            .entry(header::USER_AGENT)
            .or_insert(HeaderValue::from_static(WEB_UA));
        for hop in 0..=MAX_REDIRECTS {
            if let Some(session) = &request.session {
                session.check_origin(&url)?;
            }
            ensure!(
                self.requests < MAX_REQUESTS,
                "A site reader can make at most {MAX_REQUESTS} requests per run"
            );
            if let Some(hosts) = &self.hosts {
                ensure!(
                    crate::extensions::host_allowed(hosts, &host(&url)),
                    "This extension did not declare {}",
                    host(&url)
                );
            }
            self.requests += 1;
            let client = self.client(&url).await?;
            let mut builder = client
                .request(method.clone(), url.clone())
                .headers(headers.clone());
            let cookie = if let Some(session) = &request.session {
                session.cookie_header(&url)?
            } else {
                self.cookie_header(&url, jar_id.as_deref())
            };
            if let Some(cookie) = cookie {
                let mut cookie =
                    HeaderValue::from_str(&cookie).context("Invalid session cookie header")?;
                cookie.set_sensitive(true);
                builder = builder.header(header::COOKIE, cookie);
            }
            if let Some(body) = &body {
                builder = builder.body(body.clone());
            }
            let (client, built) = builder.build_split();
            let read = async {
                let mut response = self.route.execute(&client, built?, None).await?;
                let status = response.status();
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let set_cookies: Vec<String> = response
                    .headers()
                    .get_all(header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok().map(str::to_owned))
                    .collect();
                if status.is_redirection() && location.is_some() {
                    return Ok::<_, anyhow::Error>((status, location, set_cookies, None));
                }
                if method != Method::HEAD {
                    engine::identity(&response)?;
                }
                ensure!(
                    method == Method::HEAD
                        || response
                            .content_length()
                            .is_none_or(|n| n <= MAX_REPLY_BYTES as u64),
                    "An xd.fetch reply can be at most 8 MiB"
                );
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    ensure!(
                        bytes.len() + chunk.len() <= MAX_REPLY_BYTES,
                        "An xd.fetch reply can be at most 8 MiB"
                    );
                    bytes.extend_from_slice(&chunk);
                }
                let mut names = BTreeMap::new();
                for (name, value) in response.headers() {
                    if name == header::SET_COOKIE {
                        continue;
                    }
                    let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
                    names
                        .entry(name.as_str().to_owned())
                        .and_modify(|v: &mut String| {
                            v.push_str(", ");
                            v.push_str(&value);
                        })
                        .or_insert(value);
                }
                Ok((status, None, set_cookies, Some((names, bytes))))
            };
            let (status, location, set_cookies, reply) =
                tokio::time::timeout(REQUEST_TIMEOUT, read)
                    .await
                    .with_context(|| format!("xd.fetch timed out reading {}", host(&url)))??;
            if let Some(session) = &request.session {
                session.keep_cookies(&url, &set_cookies)?;
            } else {
                self.keep_cookies(&url, jar_id.as_deref(), &set_cookies);
            }
            if let Some((headers, bytes)) = reply {
                return Ok(FetchReply {
                    status: status.as_u16(),
                    url: url.to_string(),
                    headers,
                    body: bytes,
                });
            }
            ensure!(hop < MAX_REDIRECTS, "xd.fetch followed too many redirects");
            let next = crate::model::validate_url(
                url.join(location.as_deref().unwrap_or_default())?.as_str(),
            )?;
            ensure!(
                url.scheme() != "https" || next.scheme() == "https",
                "xd.fetch refused a redirect from HTTPS to plain HTTP"
            );
            // Browsers turn a redirected POST into a GET, except for 307 and 308.
            if (status == reqwest::StatusCode::SEE_OTHER && method != Method::HEAD)
                || (method == Method::POST && matches!(status.as_u16(), 301 | 302))
            {
                method = Method::GET;
                body = None;
            }
            if url.origin() != next.origin() {
                // Any caller header can hold an API key, not just Authorization.
                // Never restore them if a later redirect returns to the origin.
                ensure!(
                    body.is_none(),
                    "xd.fetch refused to forward a request body to another origin"
                );
                headers.clear();
                headers.insert(header::USER_AGENT, HeaderValue::from_static(WEB_UA));
                headers.insert(
                    header::ACCEPT_ENCODING,
                    HeaderValue::from_static("identity"),
                );
            }
            url = next;
        }
        unreachable!("the redirect limit is checked before following")
    }

    /// A client pinned to this host's checked public addresses.
    async fn client(&self, url: &Url) -> Result<reqwest::Client> {
        #[cfg(test)]
        if self.allow_local {
            self.route.require_public_pinning()?;
            return Ok(engine::client_builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()?);
        }
        crate::network_scope::public_client(url, &self.route).await
    }

    fn cookie_header(&self, url: &Url, session: Option<&str>) -> Option<String> {
        let origin = url.origin().ascii_serialization();
        let jar = self.cookies.get(session.unwrap_or(&origin))?;
        jar.header(url)
    }

    fn keep_cookies(&mut self, url: &Url, session: Option<&str>, set_cookies: &[String]) {
        let jar = self
            .cookies
            .entry(
                session
                    .map(str::to_owned)
                    .unwrap_or_else(|| url.origin().ascii_serialization()),
            )
            .or_default();
        jar.keep(url, set_cookies);
    }
}

fn host(url: &Url) -> String {
    url.host_str().unwrap_or("this address").to_owned()
}

#[cfg(test)]
mod tests;

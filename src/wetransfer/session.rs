//! A short-lived anonymous session scoped to one transfer page's API origin.
//! Nothing here imports, stores, or forwards the user's browser cookies.
use anyhow::{Context, Result, ensure};
use reqwest::{RequestBuilder, Url, header};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub(super) struct Session {
    pub api: Url,
    csrf: Option<header::HeaderValue>,
    cookies: BTreeMap<String, Zeroizing<String>>,
}
impl Session {
    pub fn new(page: &Url, transfer: &str, html: &[u8]) -> Result<Self> {
        let html = std::str::from_utf8(html).context("Invalid WeTransfer page encoding")?;
        let document = scraper::Html::parse_document(html);
        let selector =
            scraper::Selector::parse("meta[name='csrf-token']").expect("Static selector");
        let mut tokens = document.select(&selector);
        let csrf = tokens.next().and_then(|node| node.value().attr("content"));
        ensure!(
            tokens.next().is_none(),
            "Ambiguous WeTransfer session token"
        );
        let csrf = csrf
            .map(|value| {
                ensure!(value.len() <= 4096, "WeTransfer session token is too large");
                let mut value = header::HeaderValue::from_str(value)
                    .context("Invalid WeTransfer session token")?;
                value.set_sensitive(true);
                Ok(value)
            })
            .transpose()?;
        Ok(Self {
            api: page.join(&format!("/api/v4/transfers/{transfer}/"))?,
            csrf,
            cookies: BTreeMap::new(),
        })
    }
    pub fn update(&mut self, source: &Url, headers: &header::HeaderMap) -> Result<()> {
        ensure!(
            source.origin() == self.api.origin(),
            "Invalid WeTransfer cookie origin"
        );
        let mut length = 0;
        for header in headers.get_all(header::SET_COOKIE) {
            length += header.as_bytes().len();
            ensure!(length <= 16384, "WeTransfer session cookies are too large");
            let Ok(value) = header.to_str() else { continue };
            let mut fields = value.split(';');
            let Some((name, value)) = fields.next().and_then(|s| s.trim().split_once('=')) else {
                continue;
            };
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                || !value.bytes().all(|b| {
                    b == 0x21
                        || (0x23..=0x2b).contains(&b)
                        || (0x2d..=0x3a).contains(&b)
                        || (0x3c..=0x5b).contains(&b)
                        || (0x5d..=0x7e).contains(&b)
                })
            {
                continue;
            }
            let mut allowed = true;
            let mut expires = false;
            let mut max_age = None;
            // RFC 6265 default path: page-only cookies do not reach /api,
            // while cookies set by an API response can omit Path.
            let default_path = source
                .path()
                .rsplit_once('/')
                .map(|(p, _)| p)
                .filter(|p| !p.is_empty())
                .unwrap_or("/");
            let matches_path = |path: &str| {
                self.api.path().strip_prefix(path).is_some_and(|tail| {
                    path.starts_with('/')
                        && (path.ends_with('/') || tail.is_empty() || tail.starts_with('/'))
                })
            };
            let mut path_matches = matches_path(default_path);
            for field in fields {
                let (key, value) = field.trim().split_once('=').unwrap_or((field.trim(), ""));
                if key.eq_ignore_ascii_case("domain") {
                    let domain = value.trim_start_matches('.');
                    let host = self.api.host_str().unwrap_or_default();
                    allowed &= domain.eq_ignore_ascii_case(host)
                        || (domain.eq_ignore_ascii_case("wetransfer.com")
                            && host.ends_with(".wetransfer.com"));
                } else if key.eq_ignore_ascii_case("path") {
                    path_matches = matches_path(value);
                } else if key.eq_ignore_ascii_case("max-age") {
                    max_age = value.parse::<i64>().ok();
                } else if key.eq_ignore_ascii_case("expires") {
                    expires = httpdate::parse_http_date(value)
                        .is_ok_and(|date| date <= std::time::SystemTime::now());
                }
            }
            if allowed && path_matches {
                if max_age.map_or(expires, |seconds| seconds <= 0) {
                    self.cookies.remove(name);
                } else {
                    self.cookies
                        .insert(name.into(), Zeroizing::new(value.into()));
                }
            }
        }
        ensure!(
            self.cookies.len() <= 64
                && self
                    .cookies
                    .iter()
                    .map(|(k, v)| k.len() + v.len() + 3)
                    .sum::<usize>()
                    <= 16384,
            "WeTransfer session cookies are too large"
        );
        Ok(())
    }
    pub fn apply(&self, url: &Url, request: RequestBuilder) -> Result<RequestBuilder> {
        ensure!(
            url.origin() == self.api.origin() && url.path().starts_with(self.api.path()),
            "Invalid WeTransfer session destination"
        );
        let origin = self.api.origin().ascii_serialization();
        let mut request = request
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .header(header::ORIGIN, &origin)
            .header(header::REFERER, format!("{origin}/"))
            .header("X-Requested-With", "XMLHttpRequest");
        if let Some(csrf) = &self.csrf {
            request = request.header("X-CSRF-Token", csrf.clone());
        }
        if !self.cookies.is_empty() {
            let pairs = Zeroizing::new(
                self.cookies
                    .iter()
                    .map(|(name, value)| format!("{name}={}", value.as_str()))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
            let mut value = header::HeaderValue::from_str(&pairs)
                .context("Invalid WeTransfer cookie header")?;
            value.set_sensitive(true);
            request = request.header(header::COOKIE, value);
        }
        Ok(request)
    }
}

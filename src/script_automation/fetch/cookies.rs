//! Small, run-local cookie jar. Domains never widen the caller's exact origin.
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroize;

#[derive(Clone, Serialize, Deserialize)]
struct Cookie {
    value: String,
    secure: bool,
    expires: Option<SystemTime>,
    order: usize,
}
impl Drop for Cookie {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct Jar {
    #[serde(with = "pairs")]
    values: BTreeMap<(String, String), Cookie>,
    next_order: usize,
}
impl Jar {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.values.len() <= 50, "Too many session cookies");
        let mut total = 0;
        for ((name, path), cookie) in &self.values {
            anyhow::ensure!(
                super::super::http_session::valid_cookie(name, &cookie.value)
                    && path.starts_with('/')
                    && path.len() <= 2048
                    && !path.chars().any(char::is_control),
                "Invalid session cookie"
            );
            total += name.len() + cookie.value.len() + 3;
        }
        anyhow::ensure!(total <= 16384, "Session cookies exceed 16 KiB");
        Ok(())
    }
    pub fn seeded(values: &BTreeMap<String, String>) -> Self {
        let mut jar = Self::default();
        for (name, value) in values {
            jar.values.insert(
                (name.clone(), "/".into()),
                Cookie {
                    value: value.clone(),
                    secure: false,
                    expires: None,
                    order: jar.next_order,
                },
            );
            jar.next_order += 1;
        }
        jar
    }
    pub fn header(&self, url: &Url) -> Option<String> {
        let now = SystemTime::now();
        let mut matching: Vec<_> = self
            .values
            .iter()
            .filter(|((_, path), cookie)| {
                (!cookie.secure || url.scheme() == "https")
                    && cookie.expires.is_none_or(|end| end > now)
                    && (url.path() == path
                        || (url.path().starts_with(path.as_str())
                            && (path.ends_with('/')
                                || url.path().as_bytes().get(path.len()) == Some(&b'/'))))
            })
            .collect();
        matching.sort_by(|((_, a), ac), ((_, b), bc)| {
            b.len().cmp(&a.len()).then(ac.order.cmp(&bc.order))
        });
        (!matching.is_empty()).then(|| {
            matching
                .iter()
                .map(|((name, _), cookie)| format!("{name}={}", cookie.value))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
    pub fn keep(&mut self, url: &Url, lines: &[String]) {
        let now = SystemTime::now();
        self.values
            .retain(|_, c| c.expires.is_none_or(|end| end > now));
        for line in lines.iter().take(50) {
            if line.len() > 8192 {
                continue;
            }
            let mut fields = line.split(';');
            let Some((name, value)) = fields.next().unwrap_or_default().split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            if !super::super::http_session::valid_cookie(name, value) {
                continue;
            }
            let default_path = url
                .path()
                .rsplit_once('/')
                .map(|(p, _)| if p.is_empty() { "/" } else { p })
                .unwrap_or("/");
            let mut path = default_path.to_owned();
            let mut secure = false;
            let mut domain_valid = true;
            let mut expires = None;
            let mut max_age = None;
            for field in fields {
                let (key, value) = field.trim().split_once('=').unwrap_or((field.trim(), ""));
                match key.trim().to_ascii_lowercase().as_str() {
                    "secure" => secure = true,
                    "path"
                        if value.starts_with('/')
                            && value.len() <= 2048
                            && !value.chars().any(char::is_control) =>
                    {
                        path = value.to_owned()
                    }
                    "domain" => {
                        let domain = value.trim().trim_start_matches('.').to_ascii_lowercase();
                        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
                        domain_valid = !domain.is_empty()
                            && (host == domain
                                || (host.parse::<std::net::IpAddr>().is_err()
                                    && host.ends_with(&format!(".{domain}"))));
                    }
                    "max-age" => max_age = value.trim().parse::<i64>().ok(),
                    "expires" => expires = httpdate::parse_http_date(value.trim()).ok(),
                    _ => {}
                }
            }
            if !domain_valid || (secure && url.scheme() != "https") {
                continue;
            }
            if let Some(age) = max_age {
                expires = Some(if age <= 0 {
                    UNIX_EPOCH
                } else {
                    now.checked_add(Duration::from_secs(age as u64))
                        .unwrap_or(now)
                });
            }
            let key = (name.to_owned(), path);
            if expires.is_some_and(|end| end <= now) {
                self.values.remove(&key);
                continue;
            }
            if self.values.len() >= 50 && !self.values.contains_key(&key) {
                continue;
            }
            let bytes: usize = self
                .values
                .iter()
                .filter(|(k, _)| *k != &key)
                .map(|((name, _), c)| name.len() + c.value.len() + 3)
                .sum();
            if bytes + name.len() + value.len() + 3 > 16384 {
                continue;
            }
            let order = self.values.get(&key).map_or(self.next_order, |c| c.order);
            self.next_order += 1;
            self.values.insert(
                key,
                Cookie {
                    value: value.into(),
                    secure,
                    expires,
                    order,
                },
            );
        }
    }
}

// JSON object keys cannot hold a (name, path) tuple. Preserve both explicitly.
mod pairs {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &BTreeMap<(String, String), Cookie>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<(String, String), Cookie>, D::Error> {
        let values = Vec::<((String, String), Cookie)>::deserialize(deserializer)?;
        Ok(values.into_iter().collect())
    }
}

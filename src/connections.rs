//! A persisted connection policy, snapshotted once for each HTTP file transfer.
use crate::store::Store;
use anyhow::{Result, ensure};
use reqwest::Url;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const MAX_CONNECTIONS: u8 = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Site {
    pub host: String,
    pub subdomains: bool,
    pub connections: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub default_connections: u8,
    pub sites: Vec<Site>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            default_connections: 4,
            sites: Vec::new(),
        }
    }
}

pub fn host(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_whitespace),
        "Enter a website hostname, such as example.com"
    );
    ensure!(
        !value.contains(':') || (value.starts_with('[') && value.ends_with(']')),
        "Enter a hostname without a port"
    );
    let url = Url::parse(&format!("https://{value}"))?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "Enter only the hostname, without a scheme, port or path"
    );
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_owned();
    ensure!(
        !host.is_empty() && !host.contains('*'),
        "Enter a valid hostname; use Include subdomains instead of wildcards"
    );
    Ok(host)
}
fn valid_limit(value: u8) -> Result<()> {
    ensure!(
        (1..=MAX_CONNECTIONS).contains(&value),
        "Choose between 1 and 32 connections per file"
    );
    Ok(())
}
impl Settings {
    pub fn normalized(mut self) -> Result<Self> {
        valid_limit(self.default_connections)?;
        ensure!(self.sites.len() <= 100, "Add at most 100 website rules");
        let mut seen = std::collections::HashSet::new();
        for site in &mut self.sites {
            site.host = host(&site.host)?;
            valid_limit(site.connections)?;
            ensure!(
                seen.insert(site.host.clone()),
                "Each website can have only one rule"
            );
            ensure!(
                !site.subdomains
                    || site
                        .host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_err(),
                "IP addresses do not have subdomains"
            );
        }
        Ok(self)
    }
    fn site_limit(&self, url: &Url) -> Option<u8> {
        let host = url.host_str()?.trim_end_matches('.');
        self.sites
            .iter()
            .filter(|site| {
                host == site.host
                    || (site.subdomains
                        && host
                            .strip_suffix(&site.host)
                            .is_some_and(|prefix| prefix.ends_with('.')))
            })
            .max_by_key(|site| site.host.len())
            .map(|site| site.connections)
    }
    pub fn limit(&self, source: &str, destination: &str) -> Result<usize> {
        // A stricter rule on either end of a redirect wins. A default is used
        // only when neither host has a rule, so CDN overrides can raise it too.
        let source = Url::parse(source)?;
        let destination = Url::parse(destination)?;
        Ok([self.site_limit(&source), self.site_limit(&destination)]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(self.default_connections) as usize)
    }
}
pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS connection_settings (id INTEGER PRIMARY KEY CHECK(id=1), data TEXT NOT NULL);")?;
    conn.execute(
        "INSERT OR IGNORE INTO connection_settings(id,data) VALUES(1,?1)",
        [serde_json::to_string(&Settings::default())?],
    )?;
    Ok(())
}
impl Store {
    pub fn connection_settings(&self) -> Result<Settings> {
        let raw: String = self.connect()?.query_row(
            "SELECT data FROM connection_settings WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            raw.len() <= 64 * 1024,
            "Saved connection settings are too large"
        );
        serde_json::from_str::<Settings>(&raw)?.normalized()
    }
    pub fn save_connection_settings(&self, original: &Settings, settings: Settings) -> Result<()> {
        let settings = settings.normalized()?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: String =
            tx.query_row("SELECT data FROM connection_settings WHERE id=1", [], |r| {
                r.get(0)
            })?;
        ensure!(
            &serde_json::from_str::<Settings>(&raw)?.normalized()? == original,
            "Connection settings changed. Reopen this dialog before saving"
        );
        ensure!(
            tx.execute(
                "UPDATE connection_settings SET data=?1 WHERE id=1",
                [serde_json::to_string(&settings)?]
            )? == 1,
            "Connection settings were not saved"
        );
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_uses_dns_boundaries_specificity_and_both_redirect_hosts() {
        let settings = Settings {
            default_connections: 4,
            sites: vec![
                Site {
                    host: "EXAMPLE.com.".into(),
                    subdomains: true,
                    connections: 8,
                },
                Site {
                    host: "slow.example.com".into(),
                    subdomains: false,
                    connections: 1,
                },
                Site {
                    host: "cdn.test".into(),
                    subdomains: false,
                    connections: 16,
                },
            ],
        }
        .normalized()
        .unwrap();
        assert_eq!(
            settings
                .limit("https://example.com/a", "https://cdn.test/a")
                .unwrap(),
            8
        );
        assert_eq!(
            settings
                .limit("https://slow.example.com/a", "https://cdn.test/a")
                .unwrap(),
            1
        );
        assert_eq!(
            settings
                .limit("https://other.test/a", "https://cdn.test/a")
                .unwrap(),
            16
        );
        assert_eq!(
            settings
                .limit(
                    "https://badexample.com/a",
                    "https://example.com.attacker.test/a"
                )
                .unwrap(),
            4
        );
        assert_eq!(
            settings
                .limit(
                    "https://sub.slow.example.com/a",
                    "https://sub.slow.example.com/a"
                )
                .unwrap(),
            8
        );
        for input in [
            "",
            "*.example.com",
            "example.com/path",
            "https://example.com",
            "user@example.com",
            "example.com:80",
            "example.com?x",
        ] {
            assert!(host(input).is_err(), "{input}");
        }
    }
    #[test]
    fn settings_survive_reopen_reject_bad_rules_and_protect_concurrent_edits() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::AppPaths {
            data: dir.path().join("data"),
            downloads: dir.path().join("downloads"),
        };
        let store = Store::open(paths.clone()).unwrap();
        let original = store.connection_settings().unwrap();
        assert_eq!(original, Settings::default());
        let changed = Settings {
            default_connections: 16,
            sites: vec![Site {
                host: "Example.COM".into(),
                subdomains: false,
                connections: 1,
            }],
        };
        store
            .save_connection_settings(&original, changed.clone())
            .unwrap();
        assert!(
            store
                .save_connection_settings(&original, Settings::default())
                .is_err()
        );
        let saved = changed.normalized().unwrap();
        let mut bad = saved.clone();
        bad.default_connections = 0;
        assert!(store.save_connection_settings(&saved, bad).is_err());
        let mut bad = saved.clone();
        bad.sites.push(bad.sites[0].clone());
        assert!(store.save_connection_settings(&saved, bad).is_err());
        drop(store);
        assert_eq!(
            Store::open(paths).unwrap().connection_settings().unwrap(),
            saved
        );
    }
}

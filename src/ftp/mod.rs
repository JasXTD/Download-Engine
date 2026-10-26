//! Passive FTP and certificate-verified FTPS. No active mode or plaintext logins.
mod client;
mod transfer;

use anyhow::{Context, Result, ensure};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};

pub(crate) use transfer::transfer;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    Plain,
    Explicit,
    Implicit,
}
impl Security {
    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "FTP · anonymous, unencrypted",
            Self::Explicit => "FTPS · explicit TLS (normally port 21)",
            Self::Implicit => "FTPS · implicit TLS (normally port 990)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u8,
    pub security: Security,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<Metadata>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub size: Option<u64>,
    pub modified: Option<String>,
}
impl State {
    pub fn new(url: &reqwest::Url) -> Option<Self> {
        let security = match url.scheme() {
            "ftp" => Security::Plain,
            "ftps" if url.port() == Some(21) => Security::Explicit,
            "ftps" => Security::Implicit,
            _ => return None,
        };
        Some(Self {
            version: 1,
            security,
            metadata: None,
        })
    }
    pub fn reset(&mut self) {
        self.metadata = None;
    }
    pub(crate) fn validate(&self, url: &str) -> Result<()> {
        ensure!(self.version == 1, "Unsupported saved FTP download version");
        Endpoint::new(url, self.security)?;
        if let Some(meta) = &self.metadata {
            ensure!(
                meta.size.is_none_or(|n| n <= i64::MAX as u64)
                    && meta.modified.as_ref().is_none_or(|v| valid_timestamp(v)),
                "Invalid saved FTP metadata. Restart this download."
            );
        }
        Ok(())
    }
    pub fn origin(&self, url: &str) -> Result<String> {
        Ok(Endpoint::new(url, self.security)?.origin())
    }
}
pub fn is_url(value: &str) -> bool {
    reqwest::Url::parse(value.trim()).is_ok_and(|u| matches!(u.scheme(), "ftp" | "ftps"))
}

#[derive(Debug)]
pub(crate) struct Endpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub directories: Vec<String>,
    pub security: Security,
}
impl Endpoint {
    pub fn new(value: &str, security: Security) -> Result<Self> {
        ensure!(
            value.len() <= 16384 && !value.chars().any(char::is_control) && !value.contains('\\'),
            "Invalid FTP URL"
        );
        let url = reqwest::Url::parse(value).context("Enter a complete FTP or FTPS file URL")?;
        ensure!(
            (url.scheme() == "ftp" && security == Security::Plain)
                || (url.scheme() == "ftps" && security != Security::Plain),
            "FTP security mode does not match the URL"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "Enter login details in Authentication, never in the URL"
        );
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "FTP file URLs cannot contain a query or fragment"
        );
        let host = url
            .host_str()
            .context("FTP URL has no server")?
            .trim_matches(['[', ']'])
            .to_owned();
        ensure!(!host.is_empty(), "FTP URL has no server");
        ensure!(
            !url.path().contains(';'),
            "FTP type suffixes are unsupported. Use a binary file URL; encode a literal semicolon as %3B."
        );
        // The slash after the authority is not part of the FTP path. Decode
        // components separately: /dir/file is relative to the login directory,
        // whereas /%2Fdir/file explicitly selects the server's root directory.
        let components = url
            .path()
            .strip_prefix('/')
            .context("Missing FTP file path")?
            .split('/')
            .map(|part| {
                percent_decode_str(part)
                    .decode_utf8()
                    .map(|v| v.into_owned())
                    .context("FTP paths must be UTF-8")
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            components.len() <= 65
                && components.iter().map(String::len).sum::<usize>() <= 4096
                && components.iter().all(|part| !part.is_empty()
                    && !part.chars().any(char::is_control)
                    && !part.contains('\\')),
            "Enter an FTP file path without control characters or backslashes"
        );
        let mut directories = components;
        let path = directories.pop().context("Missing FTP filename")?;
        ensure!(!path.ends_with('/'), "Enter an FTP file, not a directory");
        let port = url.port().unwrap_or(if security == Security::Implicit {
            990
        } else {
            21
        });
        ensure!(port != 0, "Invalid FTP port");
        Ok(Self {
            host,
            port,
            path,
            directories,
            security,
        })
    }
    pub fn origin(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!(
            "{}://{host}:{}",
            if self.security == Security::Plain {
                "ftp"
            } else {
                "ftps"
            },
            self.port
        )
    }
}
fn valid_timestamp(value: &str) -> bool {
    let (seconds, fraction) = value
        .split_once('.')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    seconds.len() == 14
        && seconds.bytes().all(|b| b.is_ascii_digit())
        && fraction
            .is_none_or(|v| !v.is_empty() && v.len() <= 9 && v.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests;

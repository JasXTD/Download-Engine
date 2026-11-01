//! Per-user native-host registration, shared by the desktop and installer helper.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const HOST_NAME: &str = "com.xetro.downloadengine";
pub const FIREFOX_ID: &str = "download-engine@xetro";
// Separate manifests keep older extensions working while the desktop and
// extension are upgraded independently. Never allow both Firefox IDs in one host.
#[cfg(windows)]
const LEGACY_HOST_NAME: &str = "com.idmex.downloader";
#[cfg(windows)]
const LEGACY_FIREFOX_ID: &str = "idm-ex@local";
#[cfg(windows)]
const HOST_PROGRAM: &str = "de-host.exe";
/// The host's file name before 2.0; manifests written by those releases name it.
#[cfg(windows)]
const LEGACY_HOST_PROGRAM: &str = "idm-ex-host.exe";
const HOST_DESCRIPTION: &str = "Download Engine download queue";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Browser {
    #[default]
    Chrome,
    Edge,
    Brave,
    Firefox,
}
impl Browser {
    pub const ALL: [Self; 4] = [Self::Chrome, Self::Edge, Self::Brave, Self::Firefox];
    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Brave => "Brave",
            Self::Firefox => "Firefox",
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Brave => "brave",
            Self::Firefox => "firefox",
        }
    }
    #[cfg(windows)]
    fn registry_path(self) -> &'static str {
        match self {
            Self::Chrome => r"Google\Chrome",
            Self::Edge => r"Microsoft\Edge",
            // Brave has no key of its own on Windows. Like other non-Google Chromium
            // builds it checks the Chromium key before Chrome's, so registering here
            // keeps Brave's extension ID separate from Chrome's. Those other browsers
            // also read this key.
            Self::Brave => r"Chromium",
            Self::Firefox => r"Mozilla",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|b| b.slug() == value)
            .context("Choose chrome, edge, brave or firefox")
    }
    pub fn extension_id(self, value: &str) -> Result<String> {
        if self == Self::Firefox {
            ensure!(
                value.is_empty() || value == FIREFOX_ID,
                "Unexpected Firefox extension ID"
            );
            return Ok(FIREFOX_ID.into());
        }
        let value = value.trim();
        ensure!(
            value.len() == 32 && value.bytes().all(|b| (b'a'..=b'p').contains(&b)),
            "Paste the 32-letter extension ID shown on the browser's Extensions page"
        );
        Ok(value.into())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RegistrationStatus {
    Missing,
    Registered { extension_id: String },
    OtherCopy { manifest: PathBuf },
    NeedsRepair { message: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostManifest {
    name: String,
    description: String,
    path: PathBuf,
    #[serde(rename = "type")]
    transport: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_origins: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_extensions: Vec<String>,
}

impl HostManifest {
    fn extension_id(&self, browser: Browser, name: &str, firefox_id: &str) -> Result<String> {
        ensure!(
            self.name == name && self.transport == "stdio",
            "Unexpected native host identity"
        );
        if browser == Browser::Firefox {
            ensure!(
                self.allowed_origins.is_empty() && self.allowed_extensions == [firefox_id],
                "Unexpected extension permissions"
            );
            return Ok(firefox_id.into());
        }
        ensure!(
            self.allowed_extensions.is_empty() && self.allowed_origins.len() == 1,
            "Unexpected extension permissions"
        );
        let id = self.allowed_origins[0]
            .strip_prefix("chrome-extension://")
            .and_then(|value| value.strip_suffix('/'))
            .context("Invalid extension origin")?;
        browser.extension_id(id)
    }
}

#[cfg(windows)]
#[path = "browser_setup/windows.rs"]
mod windows;
#[cfg(windows)]
pub use windows::{BrowserSetup, unpacked_extension_id};
#[cfg(unix)]
#[path = "browser_setup/unix.rs"]
mod unix;
#[cfg(unix)]
pub use unix::{BrowserSetup, unpacked_extension_id};

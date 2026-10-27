use anyhow::{Result, bail, ensure};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub fn validate_job_id(value: &str) -> Result<()> {
    let id =
        Uuid::parse_str(value).map_err(|_| anyhow::anyhow!("Invalid saved download identifier"))?;
    ensure!(
        id.hyphenated().to_string() == value,
        "Invalid saved download identifier"
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Downloading,
    Paused,
    Complete,
    Failed,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Downloading => "Downloading",
            Self::Paused => "Paused",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub size: u64,
    pub etag: String,
    pub final_url: String,
    // The index owns segment-{index}.part for the lifetime of this plan.
    #[serde(with = "segment_layout")]
    pub segments: Vec<Segment>,
}

pub(crate) const MAX_SEGMENTS: usize = 256;

impl Plan {
    /// Validate the complete partition and return physical indices in byte order.
    pub(crate) fn byte_order(&self) -> Result<Vec<usize>> {
        ensure!(
            self.size > 0
                && self.size <= i64::MAX as u64
                && !self.segments.is_empty()
                && self.segments.len() <= MAX_SEGMENTS,
            "Invalid saved ranges. Restart this download."
        );
        let mut order: Vec<_> = (0..self.segments.len()).collect();
        order.sort_unstable_by_key(|&i| self.segments[i].start);
        let mut next = 0;
        for &index in &order {
            let segment = &self.segments[index];
            ensure!(
                segment.start == next && segment.start <= segment.end && segment.end < self.size,
                "Invalid saved ranges. Restart this download."
            );
            next = segment.end + 1;
        }
        ensure!(
            next == self.size,
            "Invalid saved ranges. Restart this download."
        );
        Ok(order)
    }
}

// Legacy builds assume an array is already in byte order and spawn all its parts.
// Give adaptive layouts a versioned representation those builds cannot misread.
mod segment_layout {
    use super::Segment;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    #[derive(Serialize, Deserialize)]
    struct Versioned {
        version: u8,
        parts: Vec<Segment>,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Legacy(Vec<Segment>),
        Versioned(Versioned),
    }

    pub fn serialize<S: Serializer>(parts: &[Segment], serializer: S) -> Result<S::Ok, S::Error> {
        if parts.len() <= 4 && parts.windows(2).all(|p| p[0].start < p[1].start) {
            parts.serialize(serializer)
        } else {
            #[derive(Serialize)]
            struct Layout<'a> {
                version: u8,
                parts: &'a [Segment],
            }
            Layout { version: 2, parts }.serialize(serializer)
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Segment>, D::Error> {
        match Stored::deserialize(deserializer)? {
            Stored::Legacy(parts) => Ok(parts),
            Stored::Versioned(Versioned { version: 2, parts }) => Ok(parts),
            Stored::Versioned(_) => Err(D::Error::custom("Unsupported saved range layout version")),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub request_id: String,
    pub url: String,
    pub name: String,
    /// Only new, unnamed jobs may accept a server name. Legacy records keep theirs.
    #[serde(default)]
    pub server_name_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_page: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_fallback: Option<crate::link_fallback::State>,
    /// Reader-derived files must keep their network boundary across restarts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub public_only: bool,
    pub directory: PathBuf,
    pub status: Status,
    /// Queued files continue after reset. Paused, non-resumable files need review.
    #[serde(default)]
    pub quota_waiting: bool,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hls: Option<crate::hls::Download>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp: Option<crate::ftp::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<crate::synchronization::RemoteFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synchronization: Option<crate::synchronization::State>,
    #[serde(default, skip_serializing_if = "crate::integrity::State::is_default")]
    pub integrity: crate::integrity::State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_file: Option<crate::capture::ExpectedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<crate::auth::Binding>,
    #[serde(default, skip_serializing_if = "crate::retry::State::is_default")]
    pub retry: crate::retry::State,
    pub error: Option<String>,
    pub saved_path: Option<PathBuf>,
    pub created: u64,
}
impl Job {
    pub fn validate_persisted(&self) -> Result<()> {
        validate_job_id(&self.id)?;
        if let Some(state) = &self.link_fallback {
            state.validate(self)?;
        }
        if crate::mega::is_url(&self.url) {
            crate::mega::validate_job(self)?;
        }
        if crate::wetransfer::is_url(&self.url) {
            crate::wetransfer::validate_job(self)?;
        }
        if let Some(page) = &self.source_page {
            validate_url(page)?;
        }
        if let Some(ftp) = &self.ftp {
            ftp.validate(&self.url)?;
            ensure!(
                self.hls.is_none()
                    && self.plan.is_none()
                    && self.synchronization.is_none()
                    && self.remote.is_none()
                    && self.expected_file.is_none(),
                "Invalid saved FTP download state"
            );
            if let Some(binding) = &self.authentication {
                ensure!(
                    ftp.security != crate::ftp::Security::Plain
                        && binding.kind == crate::auth::Kind::Basic
                        && binding.origin == ftp.origin(&self.url)?,
                    "Invalid saved FTPS authentication binding"
                );
            }
        } else {
            ensure!(
                !crate::ftp::is_url(&self.url),
                "Missing saved FTP download state"
            );
        }
        if let Some(hls) = &self.hls {
            hls.validate()?;
            ensure!(
                self.url == hls.playlist.source
                    && self.plan.is_none()
                    && self.authentication.is_none()
                    && self.synchronization.is_none()
                    && self.expected_file.is_none(),
                "Invalid saved HLS download state"
            );
        }
        Ok(())
    }

    pub fn new(
        url: &str,
        name: Option<&str>,
        directory: PathBuf,
        request_id: String,
    ) -> Result<Self> {
        let url = validate_download_url(url)?;
        if request_id.is_empty() || request_id.len() > 128 {
            bail!("Invalid request identifier");
        }
        if !directory.is_absolute() {
            bail!("Choose an absolute download folder");
        }
        let from_url = url
            .path_segments()
            .and_then(|mut p| p.next_back())
            .unwrap_or("download");
        let decoded = percent_decode_str(from_url).decode_utf8_lossy();
        let supplied_name = name.filter(|s| !s.trim().is_empty());
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            request_id,
            name: safe_filename(supplied_name.unwrap_or(&decoded)),
            server_name_pending: supplied_name.is_none(),
            source_page: None,
            link_fallback: None,
            public_only: false,
            url: url.to_string(),
            directory,
            status: Status::Queued,
            quota_waiting: false,
            downloaded: 0,
            total: None,
            plan: None,
            hls: None,
            ftp: crate::ftp::State::new(&url),
            remote: None,
            synchronization: None,
            integrity: crate::integrity::State::default(),
            expected_file: None,
            authentication: None,
            retry: crate::retry::State::default(),
            error: None,
            saved_path: None,
            created: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        })
    }
}
/// Download entry points may accept FTP. Browser, crawler and stream network
/// boundaries keep using validate_url so their HTTP-only scope never widens.
pub fn validate_download_url(value: &str) -> Result<reqwest::Url> {
    if crate::mega::is_url(value) {
        return crate::mega::canonical_url(value);
    }
    if crate::wetransfer::is_url(value) {
        return crate::wetransfer::canonical_url(value);
    }
    if !crate::ftp::is_url(value) {
        if reqwest::Url::parse(value.trim())
            .is_ok_and(|url| !matches!(url.scheme(), "http" | "https"))
        {
            bail!("Only HTTP, HTTPS, FTP and FTPS file downloads are supported");
        }
        return validate_url(value);
    }
    let url = reqwest::Url::parse(value.trim())?;
    let state = crate::ftp::State::new(&url).expect("FTP URL checked");
    state.validate(value.trim())?;
    Ok(url)
}
pub fn validate_url(value: &str) -> Result<reqwest::Url> {
    if value.len() > 16384 {
        bail!("The URL is too long");
    }
    let mut url = reqwest::Url::parse(value.trim())
        .map_err(|_| anyhow::anyhow!("Enter a complete http:// or https:// file URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Only HTTP and HTTPS downloads are supported");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URLs containing usernames or passwords are not supported");
    }
    url.set_fragment(None);
    Ok(url)
}
/// The Referer a browser would send for a request made from `page`, reduced to
/// the page's origin: many file and video servers refuse requests without it.
/// Like a browser, nothing is sent from an HTTPS page to a plain HTTP server.
pub(crate) fn page_referer(page: &str, target: &reqwest::Url) -> Option<String> {
    let page = validate_url(page).ok()?;
    (page.scheme() == "http" || target.scheme() == "https")
        .then(|| format!("{}/", page.origin().ascii_serialization()))
}
pub fn safe_filename(value: &str) -> String {
    let mut units = 0;
    let result: String = value
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 160
        })
        .map(|c| {
            if c.is_control()
                || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut result = result.trim().trim_end_matches(['.', ' ']).to_string();
    if result.is_empty() {
        result = "download".into();
    }
    let stem = result
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let device_number = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || device_number.is_some_and(|n| n.chars().count() == 1 && "123456789¹²³".contains(n))
    {
        result.insert(0, '_');
    }
    result
}

#[cfg(test)]
mod range_plan_tests {
    use super::*;

    #[test]
    fn page_referer_is_the_page_origin_and_never_downgrades() {
        let target = |url: &str| reqwest::Url::parse(url).unwrap();
        for (page, url, expected) in [
            (
                "https://www.example.test/watch/1?session=secret#t=3",
                "https://cdn.example.test/v.mp4",
                Some("https://www.example.test/"),
            ),
            (
                "https://www.example.test:8443/watch",
                "https://cdn.example.test/v.mp4",
                Some("https://www.example.test:8443/"),
            ),
            (
                "http://www.example.test/watch",
                "https://cdn.example.test/v.mp4",
                Some("http://www.example.test/"),
            ),
            (
                "http://www.example.test/watch",
                "http://cdn.example.test/v.mp4",
                Some("http://www.example.test/"),
            ),
            // Like a browser: no Referer from an HTTPS page to plain HTTP.
            (
                "https://www.example.test/watch",
                "http://cdn.example.test/v.mp4",
                None,
            ),
            (
                "file:///C:/page.html",
                "https://cdn.example.test/v.mp4",
                None,
            ),
            ("not a url", "https://cdn.example.test/v.mp4", None),
        ] {
            assert_eq!(
                page_referer(page, &target(url)).as_deref(),
                expected,
                "{page} -> {url}"
            );
        }
    }

    fn plan(segments: Vec<Segment>) -> Plan {
        Plan {
            size: 12,
            etag: "\"stable\"".into(),
            final_url: "https://example.test/file".into(),
            segments,
        }
    }

    #[test]
    fn legacy_arrays_round_trip_and_adaptive_layouts_fail_closed_for_old_readers() {
        let legacy = plan(vec![
            Segment { start: 0, end: 5 },
            Segment { start: 6, end: 11 },
        ]);
        let json = serde_json::to_value(&legacy).unwrap();
        assert!(json["segments"].is_array());
        let loaded: Plan = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.byte_order().unwrap(), [0, 1]);
        let adaptive = plan(vec![
            Segment { start: 6, end: 11 },
            Segment { start: 0, end: 5 },
        ]);
        let mut json = serde_json::to_value(&adaptive).unwrap();
        assert_eq!(json["segments"]["version"], 2);
        assert!(serde_json::from_value::<Vec<Segment>>(json["segments"].clone()).is_err());
        let loaded: Plan = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(loaded.byte_order().unwrap(), [1, 0]);
        json["segments"]["version"] = 3.into();
        assert!(serde_json::from_value::<Plan>(json).is_err());
        let many = plan((0..12).map(|n| Segment { start: n, end: n }).collect());
        assert_eq!(
            serde_json::to_value(many).unwrap()["segments"]["version"],
            2
        );
    }

    #[test]
    fn saved_ranges_must_cover_exactly_once_with_bounded_arithmetic_and_part_count() {
        for segments in [
            vec![],
            vec![Segment { start: 1, end: 11 }],
            vec![Segment { start: 0, end: 10 }],
            vec![Segment { start: 0, end: 12 }],
            vec![Segment {
                start: 0,
                end: u64::MAX,
            }],
            vec![Segment { start: 0, end: 4 }, Segment { start: 6, end: 11 }],
            vec![Segment { start: 0, end: 6 }, Segment { start: 6, end: 11 }],
            vec![Segment { start: 0, end: 6 }, Segment { start: 7, end: 5 }],
            vec![Segment { start: 0, end: 11 }; MAX_SEGMENTS + 1],
        ] {
            assert!(plan(segments).byte_order().is_err());
        }
        for size in [0, u64::MAX] {
            let mut invalid = plan(vec![Segment { start: 0, end: 11 }]);
            invalid.size = size;
            assert!(invalid.byte_order().is_err());
        }
    }
}

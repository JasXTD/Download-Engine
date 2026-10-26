use crate::{engine, model::Job};
use anyhow::{Context, Result, ensure};
use reqwest::{Response, StatusCode, header};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Browser metadata is checked without copying browser cookies or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedFile {
    pub size: u64,
    pub mime: String,
    #[serde(default)]
    pub etag: Option<String>,
}

pub fn validate_request_id(id: &str) -> Result<()> {
    ensure!(
        id.starts_with("capture:") && id.len() > 8 && id.len() <= 128,
        "Invalid capture identifier"
    );
    Ok(())
}
fn mime(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}
impl ExpectedFile {
    pub fn browser(size: u64, value: &str) -> Result<Self> {
        let mime = mime(value);
        ensure!(
            size > 0 && size <= 9_007_199_254_740_991,
            "Unknown or invalid browser file size"
        );
        ensure!(
            mime.contains('/')
                && mime.len() <= 200
                && !matches!(
                    mime.as_str(),
                    "text/html" | "application/xhtml+xml" | "application/dash+xml"
                )
                && !mime.contains("mpegurl"),
            "This file should stay in the browser"
        );
        Ok(Self {
            size,
            mime,
            etag: None,
        })
    }
    pub fn check(&self, response: &Response) -> Result<()> {
        ensure!(
            response.status() == StatusCode::OK || response.status() == StatusCode::PARTIAL_CONTENT,
            "The file is not available outside the browser (HTTP {})",
            response.status().as_u16()
        );
        ensure!(
            response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_none_or(|v| v == "identity"),
            "Encoded downloads stay in the browser"
        );
        let actual_mime = mime(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream"),
        );
        ensure!(
            actual_mime == self.mime,
            "The server returned a different file type"
        );
        let size = if response.status() == StatusCode::PARTIAL_CONTENT {
            let range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .context("Missing file range")?
                .to_str()?;
            let (start, end, size) = engine::parse_content_range(range)?;
            ensure!(
                start == 0 && end == 0,
                "The server returned an invalid probe range"
            );
            size
        } else {
            response
                .content_length()
                .context("The server did not confirm the file size")?
        };
        ensure!(
            size == self.size,
            "The server returned a different file size"
        );
        if let Some(expected) = &self.etag {
            ensure!(
                response
                    .headers()
                    .get(header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    == Some(expected.as_str()),
                "The remote file changed after the browser handoff"
            );
        }
        Ok(())
    }
}

pub async fn preflight(
    store: &crate::store::Store,
    job: &mut Job,
    mut expected: ExpectedFile,
) -> Result<()> {
    ensure!(
        !crate::hls::is_playlist_url(&job.url),
        "Video streams need a preview before queueing"
    );
    let response = tokio::time::timeout(
        Duration::from_secs(8),
        crate::auth::DownloadClient::anonymous(store)?
            .get(&job.url)?
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, "bytes=0-0")
            .send(),
    )
    .await
    .context("Browser handoff check timed out")?
    .context("The file could not be checked outside the browser")?;
    crate::auth::check_response(&response)?;
    expected.check(&response)?;
    expected.etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .filter(|s| s.starts_with('"') && s.ends_with('"'))
        .map(str::to_owned);
    job.total = Some(expected.size);
    job.expected_file = Some(expected);
    Ok(())
}

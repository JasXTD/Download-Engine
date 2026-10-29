//! Checks of this project's public GitHub releases, at app launch or on request.
use crate::{proxy::Route, store::Store};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const REPOSITORY_URL: &str = "https://github.com/JasXTD/Download-Engine";
const LATEST_API: &str = "https://api.github.com/repos/JasXTD/Download-Engine/releases/latest";
const MAX_RESPONSE: usize = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    pub newer: bool,
}

#[derive(Deserialize)]
struct Metadata {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn version(value: &str) -> Result<[u16; 3]> {
    let mut result = [0; 3];
    let mut parts = value.split('.');
    for number in &mut result {
        let part = parts.next().context("Invalid release version")?;
        ensure!(
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0')),
            "Invalid release version"
        );
        *number = part.parse().context("Invalid release version")?;
    }
    ensure!(parts.next().is_none(), "Invalid release version");
    Ok(result)
}

fn parse(body: &[u8], installed: &str) -> Result<Release> {
    let data: Metadata = serde_json::from_slice(body).context("Invalid GitHub release response")?;
    ensure!(
        !data.draft && !data.prerelease,
        "Expected a stable public release"
    );
    let name = data.tag_name.strip_prefix('v').unwrap_or(&data.tag_name);
    let latest = version(name)?;
    let current = version(installed)?;
    Ok(Release {
        version: name.into(),
        // Construct the destination from a validated numeric tag, never an API-supplied URL.
        page_url: format!("{REPOSITORY_URL}/releases/tag/{}", data.tag_name),
        newer: latest > current,
    })
}

/// Reads release metadata only; no credentials, download history, asset download or installation.
pub async fn check(store: &Store, cancel: &CancellationToken) -> Result<Release> {
    fetch(
        &Route::load(store)?,
        LATEST_API,
        env!("CARGO_PKG_VERSION"),
        cancel,
    )
    .await
}

async fn fetch(
    route: &Route,
    endpoint: &str,
    installed: &str,
    cancel: &CancellationToken,
) -> Result<Release> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("Update check cancelled"),
        result = tokio::time::timeout(Duration::from_secs(20), async {
            let client = route.configure(Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20)))?.build()?;
            let request = client.get(endpoint)
                .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                .header(reqwest::header::USER_AGENT, concat!("Download-Engine/", env!("CARGO_PKG_VERSION")))
                .build()?;
            let mut response = route.execute(&client, request, None).await
                .context("Could not connect to GitHub")?;
            match response.status() {
                StatusCode::OK => (),
                StatusCode::NOT_FOUND => bail!("No public stable release was found"),
                StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => bail!("GitHub limited this request. Try again later"),
                status => bail!("GitHub returned HTTP {}", status.as_u16()),
            }
            let media_type = response.headers().get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()).unwrap_or("")
                .split(';').next().unwrap_or("").trim();
            ensure!(matches!(media_type, "application/json" | "application/vnd.github+json"), "Expected a JSON release response");
            ensure!(response.content_length().is_none_or(|n| n <= MAX_RESPONSE as u64), "Release response is too large");
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.context("Could not read GitHub response")? {
                ensure!(chunk.len() <= MAX_RESPONSE.saturating_sub(body.len()), "Release response is too large");
                body.extend_from_slice(&chunk);
            }
            parse(&body, installed)
        }) => result.context("Update check timed out")?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn metadata(tag: &str) -> Vec<u8> {
        serde_json::to_vec(
            &serde_json::json!({"tag_name": tag, "draft": false, "prerelease": false,
            "html_url": "https://untrusted.example/installer.exe"}),
        )
        .unwrap()
    }

    #[test]
    fn numeric_versions_and_fixed_release_destination() {
        for (tag, installed, newer) in [
            ("v3.10.0", "3.9.9", true),
            ("3.2.0", "3.2.0", false),
            ("v3.1.1", "3.2.0", false),
            ("v4.0.0", "3.99.99", true),
        ] {
            let result = parse(&metadata(tag), installed).unwrap();
            assert_eq!(result.newer, newer);
            assert_eq!(
                result.page_url,
                format!("{REPOSITORY_URL}/releases/tag/{tag}")
            );
        }
    }

    #[test]
    fn rejects_unstable_malformed_and_unsafe_metadata() {
        for tag in [
            "v3.2.0-beta",
            "v3.02.0",
            "v3.2",
            "3.2.0.1",
            "3.2.-1",
            "3.2.65536",
            "v3.2.0/../../evil",
            "https://evil.example",
            "V3.2.0",
            " 3.2.0",
        ] {
            assert!(parse(&metadata(tag), "3.2.0").is_err(), "{tag}");
        }
        for body in [
            r#"{"tag_name":"v3.2.0","draft":true,"prerelease":false}"#,
            r#"{"tag_name":"v3.2.0","draft":false,"prerelease":true}"#,
            r#"{"tag_name":"v3.2.0"}"#,
            "[]",
            "not JSON",
        ] {
            assert!(parse(body.as_bytes(), "3.2.0").is_err());
        }
    }

    async fn server(response: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/release", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                let n = stream.read(&mut bytes).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&bytes[..n]);
            }
            let _ = stream.write_all(response.as_bytes()).await;
            String::from_utf8(request).unwrap()
        });
        (url, task)
    }

    #[tokio::test]
    async fn fetches_public_metadata_without_credentials_and_bounds_failures() {
        let body = String::from_utf8(metadata("v3.2.0")).unwrap();
        let (url, task) = server(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}", body.len())).await;
        let result = fetch(&Route::direct(), &url, "3.1.1", &CancellationToken::new())
            .await
            .unwrap();
        assert!(result.newer);
        let headers = task.await.unwrap().to_lowercase();
        assert!(headers.contains("user-agent: download-engine/"));
        assert!(
            !headers.contains("authorization:")
                && !headers.contains("cookie:")
                && !headers.contains("referer:")
                && !headers.contains("from:")
        );
        for response in [
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/redirect\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 200000\r\n\r\n".into(),
            format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", " ".repeat(MAX_RESPONSE + 1)),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 5\r\n\r\nwrong".into(),
        ] {
            let (url, task) = server(response).await;
            assert!(fetch(&Route::direct(), &url, "3.1.1", &CancellationToken::new()).await.is_err());
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_stalled_network_and_prevents_requests() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            fetch(&Route::direct(), "http://127.0.0.1:1", "3.2.0", &cancel)
                .await
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/release", listener.local_addr().unwrap());
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let worker =
            tokio::spawn(async move { fetch(&Route::direct(), &url, "3.2.0", &token).await });
        let (_stream, _) = listener.accept().await.unwrap();
        cancel.cancel();
        let error = tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn uses_the_configured_proxy_route() {
        let body = String::from_utf8(metadata("v3.2.0")).unwrap();
        let (proxy, task) = server(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ))
        .await;
        let route = Route {
            settings: crate::proxy::Settings {
                mode: crate::proxy::Mode::Custom,
                endpoint: proxy.trim_end_matches("/release").into(),
                ..Default::default()
            },
            login: None,
        };
        let release = fetch(
            &route,
            "http://updates.invalid/release",
            "3.1.1",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(release.newer);
        assert!(
            task.await
                .unwrap()
                .starts_with("GET http://updates.invalid/release HTTP/1.1")
        );
    }

    #[tokio::test]
    #[ignore = "Explicit live GitHub API check; no assets are downloaded"]
    async fn live_public_release_check() {
        let result = fetch(
            &Route::direct(),
            LATEST_API,
            env!("CARGO_PKG_VERSION"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(
            result
                .page_url
                .starts_with(&format!("{REPOSITORY_URL}/releases/tag/"))
        );
        println!("Latest stable version: {}", result.version);
    }
}

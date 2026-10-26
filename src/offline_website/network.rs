use super::*;
use crate::network_scope::{NetworkScope, network_scope, validate_resolution};
use reqwest::{Client, header};
use std::net::IpAddr;

pub(super) struct Resource {
    pub url: Url,
    pub bytes: Vec<u8>,
    pub extension: String,
}
pub(super) struct Network {
    clients: HashMap<String, Client>,
}
impl Network {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }
    async fn client(
        &mut self,
        url: &Url,
        start: &Url,
        route: &crate::proxy::Route,
    ) -> Result<Client> {
        let origin = url.origin().ascii_serialization();
        if let Some(client) = self.clients.get(&origin) {
            return Ok(client.clone());
        }
        let resolution = crate::website_grabber::pinned_resolution(url).await?;
        let scope = if let Some((_, addresses)) = &resolution {
            validate_resolution(addresses.iter().map(|a| a.ip()))?
        } else {
            network_scope(
                url.host_str()
                    .context("Missing host")?
                    .trim_matches(['[', ']'])
                    .parse::<IpAddr>()?,
            )?
        };
        ensure!(
            url.origin() == start.origin() || scope == NetworkScope::Public,
            "External assets cannot connect to a local or special network address"
        );
        let mut builder = route
            .configure(crate::engine::client_builder())?
            .timeout(Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none());
        if let Some((host, addresses)) = resolution {
            builder = builder.resolve_to_addrs(&host, &addresses);
        }
        let client = builder.build()?;
        self.clients.insert(origin, client.clone());
        Ok(client)
    }
    pub async fn fetch(
        &mut self,
        initial: &Url,
        kind: Kind,
        start: &Url,
        route: &crate::proxy::Route,
        total: &mut usize,
        limit: usize,
    ) -> Result<Resource> {
        let client = self.client(initial, start, route).await?;
        let mut url = initial.clone();
        let mut response = None;
        for hop in 0..=5 {
            let request = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity")
                .header(
                    header::ACCEPT,
                    match kind {
                        Kind::Page => "text/html, application/xhtml+xml",
                        Kind::Css => "text/css",
                        Kind::Asset => "*/*",
                    },
                )
                .build()?;
            let received = route.execute(&client, request, None).await?;
            if matches!(received.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                ensure!(hop < 5, "Too many resource redirects");
                let next = crate::model::validate_url(
                    url.join(
                        received
                            .headers()
                            .get(header::LOCATION)
                            .context("Redirect has no destination")?
                            .to_str()?,
                    )?
                    .as_str(),
                )?;
                ensure!(
                    next.origin() == initial.origin(),
                    "Resource redirects outside its origin are not captured; use the final URL as the starting page"
                );
                url = next;
            } else {
                crate::auth::check_response(&received)?;
                response = Some(received.error_for_status()?);
                break;
            }
        }
        let mut response = response.context("Resource returned no response")?;
        ensure!(
            response.status().as_u16() == 200,
            "Expected a complete resource (HTTP 200)"
        );
        ensure!(
            !response
                .headers()
                .get(header::CONTENT_DISPOSITION)
                .is_some_and(|v| v
                    .to_str()
                    .unwrap_or("")
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("attachment")),
            "Attachment downloads are not part of a static website copy"
        );
        ensure!(
            response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_none_or(|v| v == "identity"),
            "The server ignored the uncompressed resource request"
        );
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if kind != Kind::Asset {
            for parameter in content_type.split(';').skip(1) {
                if let Some((name, value)) = parameter.trim().split_once('=') {
                    ensure!(
                        !name.eq_ignore_ascii_case("charset")
                            || matches!(
                                value.trim().trim_matches('"').to_ascii_lowercase().as_str(),
                                "utf-8" | "utf8" | "us-ascii"
                            ),
                        "Only UTF-8 HTML and CSS are supported"
                    );
                }
            }
        }
        let extension = match kind {
            Kind::Page => {
                ensure!(
                    matches!(mime.as_str(), "text/html" | "application/xhtml+xml"),
                    "Linked resource is not an HTML page"
                );
                "html"
            }
            Kind::Css => {
                ensure!(mime == "text/css", "Stylesheet did not return CSS");
                "css"
            }
            Kind::Asset => match mime.as_str() {
                "image/png" => "png",
                "image/jpeg" => "jpg",
                "image/gif" => "gif",
                "image/webp" => "webp",
                "image/avif" => "avif",
                "image/svg+xml" => "svg",
                "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
                "image/bmp" => "bmp",
                "font/woff" | "application/font-woff" => "woff",
                "font/woff2" => "woff2",
                "font/ttf" | "application/x-font-ttf" => "ttf",
                "font/otf" | "application/x-font-opentype" => "otf",
                "application/vnd.ms-fontobject" => "eot",
                "application/octet-stream" => match url
                    .path()
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "woff" => "woff",
                    "woff2" => "woff2",
                    "ttf" => "ttf",
                    "otf" => "otf",
                    "eot" => "eot",
                    _ => anyhow::bail!("Unsupported asset type: {mime}"),
                },
                _ => anyhow::bail!("Unsupported asset type: {mime}"),
            },
        }
        .to_owned();
        let resource_limit = if kind == Kind::Asset {
            16 * 1024 * 1024
        } else {
            2 * 1024 * 1024
        };
        if let Some(size) = response.content_length() {
            ensure!(
                size <= resource_limit as u64,
                "Resource exceeds the {} MB per-file limit",
                resource_limit / 1024 / 1024
            );
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            *total = total.saturating_add(chunk.len());
            ensure!(*total <= limit, "Total capture size limit reached");
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= resource_limit,
                "Resource exceeds its per-file limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(Resource {
            url,
            bytes,
            extension,
        })
    }
}

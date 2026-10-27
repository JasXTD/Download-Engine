use super::{MAX_PLAYLIST_BYTES, Range};
use crate::{
    engine,
    model::validate_url,
    network_scope::{NetworkScope, network_scope},
    retry,
    website_grabber::pinned_resolution,
};
use anyhow::{Context, Result, ensure};
use reqwest::{Client, Response, Url, header};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct Network {
    root: Url,
    clients: Arc<Mutex<HashMap<String, Client>>>,
    route: crate::proxy::Route,
    site: Option<SiteHeaders>,
    /// The browser page the stream was found on, sent as an origin-only Referer.
    page: Option<String>,
    public_only: bool,
}

/// Headers a site's own video servers require (for example a Referer and the
/// visitor cookies of the page that listed the video). They are only sent to
/// that site's domain and its subdomains, never kept on disk.
#[derive(Clone)]
pub(crate) struct SiteHeaders {
    pub domain: &'static str,
    pub headers: header::HeaderMap,
}
impl SiteHeaders {
    pub(crate) fn applies(&self, url: &Url) -> bool {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| {
                host == self.domain
                    || host
                        .strip_suffix(self.domain)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
    }
}
impl Network {
    #[cfg(test)]
    pub async fn new(source: &str) -> Result<Self> {
        Self::via(source, crate::proxy::Route::direct()).await
    }
    pub async fn via(source: &str, route: crate::proxy::Route) -> Result<Self> {
        Ok(Self {
            root: validate_url(source)?,
            clients: Arc::new(Mutex::new(HashMap::new())),
            route,
            site: None,
            page: None,
            public_only: false,
        })
    }
    pub(crate) fn with_site(mut self, site: SiteHeaders) -> Self {
        self.site = Some(site);
        self
    }
    pub(crate) fn with_page(mut self, page: Option<&str>) -> Self {
        self.page = page.map(str::to_owned);
        self
    }
    pub(crate) fn with_public_only(mut self, public_only: bool) -> Result<Self> {
        if public_only {
            self.route.require_public_pinning()?;
        }
        self.public_only = public_only;
        self.clients = Arc::new(Mutex::new(HashMap::new()));
        Ok(self)
    }
    pub(crate) fn public_only(&self) -> bool {
        self.public_only
    }
    async fn client(&self, url: &Url) -> Result<Client> {
        let origin = url.origin().ascii_serialization();
        // Share checked/pinned clients across workers. The lock only covers
        // client creation, never response headers or bodies.
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(&origin) {
            return Ok(client.clone());
        }
        ensure!(clients.len() < 64, "Video stream uses too many servers");
        if self.public_only {
            let client = crate::network_scope::public_client(url, &self.route).await?;
            clients.insert(origin, client.clone());
            return Ok(client);
        }
        let same_origin = url.origin() == self.root.origin();
        // Only explicitly entered local origins may address local resources.
        // CDN origins must resolve entirely to public IPs, and are pinned before
        // sending a request. An explicitly configured proxy owns its DNS resolution.
        // Browser/server credentials are never inherited.
        let mut builder = self
            .route
            .configure(engine::client_builder())?
            .redirect(reqwest::redirect::Policy::none());
        let host = url
            .host_str()
            .context("Video stream URL has no host")?
            .trim_matches(['[', ']']);
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            let scope = network_scope(ip)?;
            ensure!(
                same_origin || scope == NetworkScope::Public,
                "Video stream cannot request a different local or special-use server"
            );
        } else if let Some((host, addresses)) = pinned_resolution(url).await? {
            for address in &addresses {
                ensure!(
                    same_origin || network_scope(address.ip())? == NetworkScope::Public,
                    "Video stream cannot request a different local or special-use server"
                );
            }
            builder = builder.resolve_to_addrs(&host, &addresses);
        }
        let client = builder.build()?;
        clients.insert(origin, client.clone());
        Ok(client)
    }
    pub async fn get(
        &self,
        value: &str,
        range: Option<&Range>,
        cancel: &CancellationToken,
    ) -> Result<Response> {
        let mut url = validate_url(value)?;
        for redirects in 0..=5 {
            ensure!(
                self.root.scheme() != "https" || url.scheme() == "https",
                "Video stream cannot downgrade HTTPS to HTTP"
            );
            let client = tokio::select! { _=cancel.cancelled()=>anyhow::bail!("Video stream preview or download cancelled"), result=self.client(&url)=>result? };
            let mut request = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if let Some(referer) = self
                .page
                .as_deref()
                .and_then(|page| crate::model::page_referer(page, &url))
            {
                request = request.header(header::REFERER, referer);
            }
            if let Some(site) = self.site.as_ref().filter(|site| site.applies(&url)) {
                request = request.headers(site.headers.clone());
            }
            if let Some(range) = range {
                request = request.header(
                    header::RANGE,
                    format!("bytes={}-{}", range.start, range.start + range.length - 1),
                );
            }
            let response = tokio::select! { _=cancel.cancelled()=>anyhow::bail!("Video stream preview or download cancelled"), result=self.route.execute(&client, request.build()?, None)=>result? };
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                ensure!(redirects < 5, "Too many video stream redirects");
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .context("Video stream redirect has no destination")?
                    .to_str()?;
                let next = validate_url(url.join(location)?.as_str())?;
                ensure!(
                    url.scheme() != "https" || next.scheme() == "https",
                    "Video stream cannot downgrade HTTPS to HTTP"
                );
                url = next;
                continue;
            }
            ensure!(
                !matches!(response.status().as_u16(), 401 | 403),
                "This video stream requires login or its link expired. Use a fresh public manifest URL; browser login for streams is not supported yet."
            );
            crate::auth::check_response(&response)?;
            retry::response(&response)?;
            engine::identity(&response)?;
            if let Some(range) = range {
                ensure!(
                    response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
                    "Video stream server ignored the requested byte range"
                );
                let (start, end, _) = engine::range(&response)?;
                ensure!(
                    start == range.start
                        && end == range.start + range.length - 1
                        && response.content_length().is_none_or(|n| n == range.length),
                    "Video stream server returned the wrong byte range"
                );
            } else {
                ensure!(
                    response.status() == reqwest::StatusCode::OK,
                    "Unexpected video stream HTTP status"
                );
            }
            return Ok(response);
        }
        unreachable!("redirect limit checked before following")
    }
    pub async fn playlist(
        &mut self,
        url: &str,
        cancel: &CancellationToken,
    ) -> Result<(String, String)> {
        let read = async {
            let mut response = self.get(url, None, cancel).await?;
            ensure!(
                response
                    .content_length()
                    .is_none_or(|size| size <= MAX_PLAYLIST_BYTES as u64),
                "Video stream playlist exceeds 1 MiB"
            );
            let final_url = response.url().to_string();
            let mut bytes = Vec::new();
            loop {
                let chunk = tokio::select! { _=cancel.cancelled()=>anyhow::bail!("Video stream preview cancelled"), result=engine::network_wait(response.chunk())=>result? };
                let Some(chunk) = chunk else {
                    break;
                };
                ensure!(
                    bytes.len() + chunk.len() <= MAX_PLAYLIST_BYTES,
                    "Video stream playlist exceeds 1 MiB"
                );
                bytes.extend_from_slice(&chunk);
            }
            Ok((
                final_url,
                String::from_utf8(bytes).context("Video stream playlist is not UTF-8")?,
            ))
        };
        tokio::time::timeout(Duration::from_secs(30), read)
            .await
            .context("Video stream playlist timed out")?
    }
    pub(crate) async fn index(
        &mut self,
        url: &str,
        range: &Range,
        cancel: &CancellationToken,
    ) -> Result<(Vec<u8>, Response)> {
        ensure!(
            range.length > 0
                && range.length <= MAX_PLAYLIST_BYTES as u64
                && range
                    .start
                    .checked_add(range.length)
                    .is_some_and(|end| end <= i64::MAX as u64),
            "Invalid DASH index byte range"
        );
        let read = async {
            let mut response = self.get(url, Some(range), cancel).await?;
            let mut bytes = Vec::new();
            loop {
                let chunk = tokio::select! { _=cancel.cancelled()=>anyhow::bail!("DASH index preview cancelled"), result=engine::network_wait(response.chunk())=>result? };
                let Some(chunk) = chunk else { break };
                ensure!(
                    bytes.len() + chunk.len() <= range.length as usize,
                    "DASH index response exceeds its requested range"
                );
                bytes.extend_from_slice(&chunk);
            }
            ensure!(
                bytes.len() as u64 == range.length,
                "DASH index response is incomplete"
            );
            Ok((bytes, response))
        };
        tokio::time::timeout(Duration::from_secs(30), read)
            .await
            .context("DASH index request timed out")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn reader_security_redirects_and_parts_reject_local_destinations() {
        let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let private = format!("http://{}/private", target.local_addr().unwrap());
        let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = source.local_addr().unwrap();
        let url = format!(
            "http://reader-fixture.invalid:{}/master.m3u8",
            address.port()
        );
        let network = Network::new(&url)
            .await
            .unwrap()
            .with_public_only(true)
            .unwrap();
        // Model an already checked public source using a loopback transport.
        // Only this source origin is replaced; redirect/part checks stay real.
        let client = engine::client_builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve("reader-fixture.invalid", address)
            .build()
            .unwrap();
        network.clients.lock().await.insert(
            Url::parse(&url).unwrap().origin().ascii_serialization(),
            client,
        );
        let destination = private.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = source.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(socket.read_u8().await.unwrap());
            }
            socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let cancel = CancellationToken::new();
        let error = network.get(&url, None, &cancel).await.unwrap_err();
        assert!(error.to_string().contains("public internet"), "{error:#}");
        server.await.unwrap();
        let error = network
            .get(
                &private,
                Some(&Range {
                    start: 0,
                    length: 10,
                }),
                &cancel,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("public internet"), "{error:#}");
        assert_eq!(
            target.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

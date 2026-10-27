use super::{Link, session::Session};
use crate::{engine, model::safe_filename, proxy::Route, retry, store::Store};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Response, Url, header};
use serde_json::Value;

#[derive(Clone)]
pub(super) struct Network {
    route: Route,
    client: Client,
    public_only: bool,
    // Explicit per-instance transport substitution, compiled only in unit tests.
    #[cfg(test)]
    pub(super) fixture: Option<Url>,
}
#[derive(Clone)]
pub(crate) struct Source {
    network: Network,
    url: Url,
    pub(crate) identity: String,
    pub(crate) name: String,
}
impl Network {
    pub(super) fn new(store: &Store, public_only: bool) -> Result<Self> {
        Self::via(Route::load(store)?, public_only)
    }
    pub(super) fn via(route: Route, public_only: bool) -> Result<Self> {
        let client = route
            .configure(engine::client_builder())?
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            route,
            client,
            public_only,
            #[cfg(test)]
            fixture: None,
        })
    }
    async fn client(&self, url: &Url) -> Result<Client> {
        if self.public_only {
            crate::network_scope::public_client(url, &self.route).await
        } else {
            Ok(self.client.clone())
        }
    }
    async fn send(
        &self,
        url: &Url,
        request: reqwest::RequestBuilder,
        client: &Client,
    ) -> Result<Response> {
        let request = request
            .header(header::ACCEPT_ENCODING, "identity")
            .build()
            .map_err(retry::network)?;
        #[cfg(test)]
        let request = {
            let mut request = request;
            if let Some(base) = &self.fixture {
                let mut wire = base.join(url.path())?;
                wire.set_query(url.query());
                *request.url_mut() = wire;
            }
            request
        };
        // Keep the production URL out of error chains and signed addresses out
        // of job records. Redirects are followed only by the checked callers.
        let _ = url;
        let response = self.route.execute(client, request, None).await?;
        retry::response(&response)?;
        Ok(response)
    }
    pub(super) async fn resolve(self, value: &str) -> Result<Source> {
        let mut link = Link::parse(value)?;
        let mut page = None;
        for _ in 0..6 {
            let client = self.client(&link.url).await?;
            let response = self
                .send(&link.url, client.get(link.url.clone()), &client)
                .await?;
            if response.status().is_redirection() {
                link = Link::parse(redirect(&link.url, &response)?.as_str())?;
            } else {
                page = Some(response);
                break;
            }
        }
        let response = page.context("Too many WeTransfer share-link redirects")?;
        check_api_status(&response)?;
        let transfer = link.transfer.as_deref().context("WeTransfer short link did not resolve to a transfer. Open it in your browser to check access.")?;
        let headers = response.headers().clone();
        let html = read_bounded(response, 2 * 1024 * 1024).await?;
        let mut session = Session::new(&link.url, transfer, &html)?;
        session.update(&link.url, &headers)?;
        let mut body = link.payload()?;
        let metadata = self.api(&mut session, "prepare-download", &body).await?;
        check_metadata(&metadata)?;
        let name = metadata
            .get("recommended_filename")
            .and_then(Value::as_str)
            .and_then(filename);
        body["intent"] = "entire_transfer".into();
        let download = self.api(&mut session, "download", &body).await?;
        check_error(&download)?;
        let direct = download.get("direct_link").and_then(Value::as_str)
            .context("WeTransfer did not return a file address. Open the transfer in your browser to check access.")?;
        ensure!(direct.len() <= 16384, "WeTransfer file address is too long");
        let url = Url::parse(direct).context("Invalid WeTransfer file address")?;
        validate_download_url(&url)?;
        let name = name
            .or_else(|| {
                url.path_segments()
                    .and_then(|mut p| p.next_back())
                    .and_then(|s| percent_encoding::percent_decode_str(s).decode_utf8().ok())
                    .as_deref()
                    .and_then(filename)
            })
            .unwrap_or_else(|| "WeTransfer download".into());
        Ok(Source {
            network: self,
            url,
            identity: link.identity()?,
            name,
        })
    }
    async fn api(&self, session: &mut Session, action: &str, body: &Value) -> Result<Value> {
        let url = session.api.join(action)?;
        let client = self.client(&url).await?;
        let request = session.apply(&url, client.post(url.clone()).body(body.to_string()))?;
        let response = self.send(&url, request, &client).await?;
        check_api_status(&response)?;
        session.update(&url, response.headers())?;
        let bytes = read_bounded(response, 2 * 1024 * 1024).await?;
        serde_json::from_slice(&bytes).context("WeTransfer returned an unsupported response. Open the transfer in your browser to check access.")
    }
}
impl Source {
    pub(crate) async fn get(
        &self,
        range: Option<(u64, u64)>,
        etag: Option<&str>,
    ) -> Result<Response> {
        let mut url = self.url.clone();
        for _ in 0..6 {
            validate_download_url(&url)?;
            let client = self.network.client(&url).await?;
            // No page cookies, CSRF token, browser login or share-link Referer
            // accompanies requests to file storage, including redirects.
            let mut request = client.get(url.clone());
            if let Some((start, end)) = range {
                request = request.header(header::RANGE, format!("bytes={start}-{end}"));
            }
            if let Some(etag) = etag {
                request = request.header(header::IF_RANGE, etag);
            }
            let response = self.network.send(&url, request, &client).await?;
            if response.status().is_redirection() {
                url = redirect(&url, &response)?;
                continue;
            }
            if matches!(response.status().as_u16(), 401 | 403 | 404 | 410) {
                return Err(retry::incomplete(
                    "WeTransfer file address expired or was refused. Retrying with a fresh address.",
                ));
            }
            if response.status().is_success() {
                let mime = response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                let attachment = response
                    .headers()
                    .get(header::CONTENT_DISPOSITION)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| {
                        v.split(';')
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .eq_ignore_ascii_case("attachment")
                    });
                ensure!(
                    attachment
                        || !matches!(
                            mime.as_str(),
                            "text/html"
                                | "application/xhtml+xml"
                                | "application/json"
                                | "application/xml"
                                | "text/xml"
                        ),
                    "WeTransfer returned a web page or error response instead of a file. Open the transfer in your browser to check access."
                );
            }
            return Ok(response);
        }
        bail!("Too many WeTransfer file redirects")
    }
}
fn redirect(url: &Url, response: &Response) -> Result<Url> {
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .context("Missing WeTransfer redirect address")?;
    ensure!(
        location.len() <= 16384,
        "WeTransfer redirect address is too long"
    );
    url.join(location)
        .context("Invalid WeTransfer redirect address")
}
fn check_api_status(response: &Response) -> Result<()> {
    match response.status().as_u16() {
        404 | 410 => bail!(
            "This WeTransfer transfer has expired, was deleted, or is unavailable. Ask the sender for a new link."
        ),
        401 | 403 => bail!(
            "WeTransfer requires browser access, a password, or identity verification. Protected transfers are not supported."
        ),
        407 => {
            bail!("Proxy login was required or rejected. Check Preferences → Connection → Proxy.")
        }
        200 => Ok(()),
        status => bail!(
            "WeTransfer returned HTTP {status}. Open the transfer in your browser to check access."
        ),
    }
}
fn check_error(value: &Value) -> Result<()> {
    ensure!(value.is_object(), "Invalid WeTransfer transfer response");
    if value
        .get("error")
        .is_some_and(|e| !e.is_null() && e != false)
        || value
            .get("errors")
            .is_some_and(|e| !e.is_null() && e.as_array().is_none_or(|a| !a.is_empty()))
    {
        bail!(
            "WeTransfer could not provide this transfer. It may be expired, deleted, or protected. Open it in your browser to check access."
        );
    }
    Ok(())
}
fn check_metadata(value: &Value) -> Result<()> {
    check_error(value)?;
    ensure!(
        value.get("password_protected").and_then(Value::as_bool) != Some(true),
        "Password-protected WeTransfer transfers are not supported. Open the link in your browser."
    );
    ensure!(
        value.get("state").and_then(Value::as_str) == Some("downloadable"),
        "This WeTransfer transfer is not available to download. It may be expired, deleted, not ready, or require identity verification."
    );
    Ok(())
}
async fn read_bounded(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    engine::identity(&response)?;
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "WeTransfer response is too large"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = engine::network_wait(response.chunk()).await? {
        ensure!(
            bytes.len() + chunk.len() <= limit,
            "WeTransfer response is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn filename(value: &str) -> Option<String> {
    if value.len() > 4096 {
        return None;
    }
    let leaf = value.rsplit(['/', '\\']).next()?.trim();
    (!leaf.is_empty()).then(|| safe_filename(leaf))
}
pub(super) fn validate_download_url(url: &Url) -> Result<()> {
    let host = url.host_str().unwrap_or_default();
    let buckets = ["wt-prod-euwest1-storm", "wt-prod-useast1-storm"];
    let s3 = [
        "s3.amazonaws.com",
        "s3-accelerate.amazonaws.com",
        "s3.eu-west-1.amazonaws.com",
        "s3.us-east-1.amazonaws.com",
    ];
    let storage = buckets
        .iter()
        .any(|b| s3.iter().any(|s| host == format!("{b}.{s}")))
        || (s3.contains(&host)
            && url
                .path_segments()
                .and_then(|mut p| p.next())
                .is_some_and(|b| buckets.contains(&b)));
    ensure!(
        url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && (host == "wetransfer.com"
                || host.ends_with(".wetransfer.com")
                || host == "wetransfer.net"
                || host.ends_with(".wetransfer.net")
                || storage),
        "WeTransfer returned an unsupported file server address"
    );
    Ok(())
}

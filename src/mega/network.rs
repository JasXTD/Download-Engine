use super::*;
use crate::{engine, proxy::Route, retry};
use reqwest::{Client, Response, header};

#[derive(Clone)]
pub(super) struct Network {
    route: Route,
    client: Client,
    public_only: bool,
    #[cfg(test)]
    pub(super) fixture: Option<Url>,
}
pub(super) struct Metadata {
    pub url: Url,
    pub size: u64,
    pub name: String,
}
#[derive(Clone)]
pub(crate) struct Source {
    pub(super) network: Network,
    pub(super) url: Url,
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
    async fn request(&self, url: &Url, body: Option<String>) -> Result<Response> {
        let client = if self.public_only {
            crate::network_scope::public_client(url, &self.route).await?
        } else {
            self.client.clone()
        };
        let request = if let Some(body) = body {
            client
                .post(url.clone())
                .header(header::CONTENT_TYPE, "application/json")
                .body(body)
        } else {
            client.get(url.clone())
        };
        let request = request
            .header(header::ACCEPT_ENCODING, "identity")
            .build()
            .map_err(retry::network)?;
        let response = self.route.execute(&client, request, None).await?;
        match response.status().as_u16() {
            403 => {
                return Err(retry::incomplete(
                    "MEGA download address expired. Retrying with a fresh address.",
                ));
            }
            509 => bail!("MEGA transfer quota reached. Resume when your MEGA quota resets."),
            _ => retry::response(&response)?,
        }
        ensure!(
            response.status().is_success(),
            "MEGA returned HTTP {}",
            response.status().as_u16()
        );
        engine::identity(&response)?;
        Ok(response)
    }
    pub(super) async fn resolve(&self, link: &Link) -> Result<Metadata> {
        let api = Url::parse("https://g.api.mega.co.nz/cs")?;
        #[cfg(test)]
        let api = self
            .fixture
            .as_ref()
            .map(|base| base.join("cs"))
            .transpose()?
            .unwrap_or(api);
        let mut api = api;
        api.query_pairs_mut()
            .append_pair("id", &(uuid::Uuid::new_v4().as_u128() as u64).to_string());
        // ssl=2 requests HTTPS; never borrow another application's API key.
        let body = serde_json::json!([{"a":"g", "g":1, "p":link.handle, "ssl":2}]).to_string();
        let mut response = self.request(&api, Some(body)).await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = engine::network_wait(response.chunk()).await? {
            ensure!(
                bytes.len() + chunk.len() <= 128 * 1024,
                "MEGA metadata response is too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).context("Invalid MEGA metadata response")?;
        if let Some(code) = value.as_i64() {
            return Err(api_error(code));
        }
        let results = value
            .as_array()
            .filter(|a| a.len() == 1)
            .context("Invalid MEGA metadata result")?;
        let data = &results[0];
        if let Some(code) = data
            .as_i64()
            .or_else(|| data.get("e").and_then(|v| v.as_i64()).filter(|&n| n != 0))
        {
            return Err(api_error(code));
        }
        let size = data
            .get("s")
            .and_then(|v| v.as_u64())
            .filter(|&n| n <= i64::MAX as u64)
            .context("Invalid MEGA file size")?;
        let download = data
            .get("g")
            .and_then(|v| v.as_str())
            .context("MEGA did not provide a supported single-server download address")?;
        let url = Url::parse(download).context("Invalid MEGA download address")?;
        #[cfg(test)]
        let local_fixture = self
            .fixture
            .as_ref()
            .is_some_and(|base| base.origin() == url.origin());
        #[cfg(not(test))]
        let local_fixture = false;
        if !local_fixture {
            validate_download_address(&url)?;
        }
        let attributes = data
            .get("at")
            .and_then(|v| v.as_str())
            .context("Missing MEGA file attributes")?;
        ensure!(
            attributes.len() <= 32 * 1024,
            "MEGA file attributes are too large"
        );
        let encrypted = URL_SAFE_NO_PAD
            .decode(attributes)
            .context("Invalid MEGA file attributes")?;
        let cipher = crypto::Cipher::new(&link.aes_key(), link.nonce())?;
        let decrypted = Zeroizing::new(cipher.attributes(&encrypted)?);
        let end = decrypted
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(decrypted.len());
        ensure!(
            decrypted[end..].iter().all(|&b| b == 0),
            "Invalid MEGA file attribute padding"
        );
        let json = decrypted[..end]
            .strip_prefix(b"MEGA")
            .context("Incorrect MEGA decryption key or damaged file attributes")?;
        let attributes: serde_json::Value =
            serde_json::from_slice(json).context("Invalid decrypted MEGA attributes")?;
        let name = attributes
            .get("n")
            .and_then(|v| v.as_str())
            .filter(|n| !n.trim().is_empty() && n.len() <= 4096)
            .context("Missing MEGA filename")?
            .to_owned();
        Ok(Metadata { url, size, name })
    }
}
fn validate_download_address(url: &Url) -> Result<()> {
    let host = url.host_str().unwrap_or_default();
    ensure!(
        url.as_str().len() <= 8192
            && url.scheme() == "https"
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.query().is_none()
            && (host.ends_with(".userstorage.mega.co.nz")
                || host.ends_with(".userstorage.mega.nz")),
        "MEGA returned an untrusted download address"
    );
    Ok(())
}
fn api_error(code: i64) -> anyhow::Error {
    match code {
        -3 | -4 | -18 => retry::incomplete("MEGA is temporarily unavailable. Retrying later."),
        -9 | -16 => anyhow::anyhow!("The MEGA file was removed, blocked or is no longer shared."),
        -17 => anyhow::anyhow!("MEGA transfer quota reached. Resume when your MEGA quota resets."),
        _ => anyhow::anyhow!("MEGA could not open this public file (API error {code})."),
    }
}
impl Source {
    pub(crate) async fn range(&self, start: u64, end: u64, size: u64) -> Result<Response> {
        ensure!(start <= end && end < size, "Invalid MEGA byte range");
        let mut url = self.url.clone();
        url.set_path(&format!(
            "{}/{start}-{end}",
            url.path().trim_end_matches('/')
        ));
        let response = self.network.request(&url, None).await?;
        ensure!(
            matches!(response.status().as_u16(), 200 | 206),
            "Unexpected MEGA byte-range response"
        );
        if response.headers().contains_key(header::CONTENT_RANGE) {
            ensure!(
                engine::range(&response)? == (start, end, size),
                "MEGA returned the wrong byte range"
            );
        } else {
            ensure!(
                response.status().as_u16() == 200,
                "MEGA omitted its byte range"
            );
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_addresses_must_be_official_https_and_cannot_carry_credentials() {
        for url in [
            "https://gfs1.userstorage.mega.co.nz/file",
            "https://gfs2.userstorage.mega.nz/file",
        ] {
            validate_download_address(&Url::parse(url).unwrap()).unwrap();
        }
        for url in [
            "http://gfs1.userstorage.mega.co.nz/file",
            "https://gfs1.userstorage.mega.co.nz:444/file",
            "https://gfs1.userstorage.mega.co.nz.attacker.example/file",
            "https://attacker.example/userstorage.mega.co.nz/file",
            "https://user:password@gfs1.userstorage.mega.co.nz/file",
            "https://127.0.0.1/file",
            "https://[::1]/file",
            "https://gfs1.userstorage.mega.co.nz/file#secret",
            "https://gfs1.userstorage.mega.co.nz/file?key=x",
        ] {
            assert!(
                validate_download_address(&Url::parse(url).unwrap()).is_err(),
                "{url}"
            );
        }
    }
}

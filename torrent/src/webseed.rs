//! Bounded HTTP(S) web seeds. URLs and names in metainfo are untrusted.
#[cfg(all(test, unix))]
#[path = "webseed_network_tests.rs"]
mod network_tests;
use crate::{
    discovery::{PublicNames, stays_local},
    meta::{TorrentFile, TorrentMeta},
    torrent::{Event, Live, TorrentCore},
};
use anyhow::{Context as _, Result, ensure};
use librqbit_core::network::is_service_port;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeSeed, Error, SeqAccess, Visitor},
};
use std::{
    fmt,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;
use url::Url;

pub(crate) const MAX_SEEDS: usize = 32;
const MAX_URL: usize = 2048;
const MAX_RANGE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    UrlList,
    HttpSeed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seed {
    pub kind: Kind,
    pub url: String,
}

struct Address(String);
impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct AddressVisitor;
        impl<'de> Visitor<'de> for AddressVisitor {
            type Value = Address;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a bounded web seed address")
            }
            fn visit_bytes<E: Error>(self, value: &[u8]) -> Result<Address, E> {
                if value.len() > MAX_URL {
                    return Err(E::custom("Web seed URL exceeds the size limit"));
                }
                Ok(Address(
                    std::str::from_utf8(value).map_err(E::custom)?.to_owned(),
                ))
            }
            fn visit_str<E: Error>(self, value: &str) -> Result<Address, E> {
                self.visit_bytes(value.as_bytes())
            }
        }
        de.deserialize_bytes(AddressVisitor)
    }
}
#[derive(Default)]
struct Addresses(Vec<Address>);
impl<'de> Deserialize<'de> for Addresses {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Reject;
        impl<'de> DeserializeSeed<'de> for Reject {
            type Value = ();
            fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
                Err(D::Error::custom("Too many web seeds"))
            }
        }
        struct List;
        impl<'de> Visitor<'de> for List {
            type Value = Addresses;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a web seed address or a bounded list")
            }
            fn visit_bytes<E: Error>(self, value: &[u8]) -> Result<Addresses, E> {
                if value.len() > MAX_URL {
                    return Err(E::custom("Web seed URL exceeds the size limit"));
                }
                Ok(Addresses(vec![Address(
                    std::str::from_utf8(value).map_err(E::custom)?.to_owned(),
                )]))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Addresses, A::Error> {
                let mut out = Vec::new();
                while out.len() < MAX_SEEDS {
                    match seq.next_element()? {
                        Some(value) => out.push(value),
                        None => return Ok(Addresses(out)),
                    }
                }
                seq.next_element_seed(Reject)?;
                Ok(Addresses(out))
            }
        }
        de.deserialize_any(List)
    }
}
#[derive(Deserialize)]
struct Fields {
    #[serde(default, rename = "url-list")]
    urls: Addresses,
    #[serde(default)]
    httpseeds: Addresses,
}
pub(crate) fn from_metainfo(bytes: &[u8]) -> Result<Vec<Seed>> {
    let fields: Fields = bencode::from_bytes(bytes).map_err(|e| e.into_kind())?;
    ensure!(
        fields.urls.0.len() + fields.httpseeds.0.len() <= MAX_SEEDS,
        "Too many web seeds"
    );
    let mut seeds = Vec::new();
    for (kind, values) in [
        (Kind::UrlList, fields.urls.0),
        (Kind::HttpSeed, fields.httpseeds.0),
    ] {
        for Address(value) in values {
            // Ignore unsupported schemes. Address/DNS policy is enforced again at request time.
            if let Some(url) = checked_url(&value, true) {
                let seed = Seed {
                    kind: kind.clone(),
                    url: url.to_string(),
                };
                if !seeds.contains(&seed) {
                    seeds.push(seed);
                }
            }
        }
    }
    Ok(seeds)
}
pub(crate) fn checked_url(value: &str, allow_local: bool) -> Option<Url> {
    if value.len() > MAX_URL {
        return None;
    }
    request_url(value, allow_local)
}
fn request_url(value: &str, allow_local: bool) -> Option<Url> {
    if value.len() > 32 * 1024 {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let port = url.port_or_known_default()?;
    if !allow_local && (stays_local(&url) || ((port != 80 && port != 443) && is_service_port(port)))
    {
        return None;
    }
    Some(url)
}
pub(crate) fn encoded_component(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::new();
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(byte) {
            result.push(*byte as char);
        } else {
            let _ = write!(result, "%{byte:02X}");
        }
    }
    result
}
fn file_url(base: &Url, meta: &TorrentMeta, file: &TorrentFile) -> Result<Url> {
    if !meta.multi_file && !base.path().ends_with('/') {
        return Ok(base.clone());
    }
    for part in std::iter::once(meta.web_name.as_str()).chain(file.web_path.split('/')) {
        ensure!(
            !matches!(part, "" | "." | "..")
                && !part.contains("%2F")
                && !part.contains("%5C")
                && !part.contains("%00"),
            "Unsafe web seed path component"
        );
    }
    let mut url = base.clone();
    let mut path = base.path().trim_end_matches('/').to_owned();
    path.push('/');
    path.push_str(&meta.web_name);
    if meta.multi_file {
        path.push('/');
        path.push_str(&file.web_path);
    }
    ensure!(path.len() <= 32 * 1024, "Web seed path is too long");
    url.set_path(&path);
    Ok(url)
}
fn script_url(base: &Url, meta: &TorrentMeta, piece: u32, begin: u64, len: usize) -> Result<Url> {
    ensure!(
        !base
            .query_pairs()
            .any(|(key, _)| matches!(key.as_ref(), "info_hash" | "piece" | "ranges")),
        "Conflicting HTTP seed parameters"
    );
    let mut url = base.clone();
    let prefix = base
        .query()
        .filter(|q| !q.is_empty())
        .map_or(String::new(), |q| format!("{q}&"));
    // info_hash is arbitrary bytes, not UTF-8 or form-encoded text.
    let hash = meta
        .info_hash
        .0
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    url.set_query(Some(&format!(
        "{prefix}info_hash={hash}&piece={piece}&ranges={begin}-{}",
        begin + len as u64 - 1
    )));
    Ok(url)
}

#[derive(Debug)]
struct RetryAfter(Duration);
impl std::fmt::Display for RetryAfter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Web seed asked to retry later")
    }
}
impl std::error::Error for RetryAfter {}

struct Http {
    bound: Option<crate::network::BoundHttp>,
    client: reqwest::Client,
    allow_local: bool,
    slots: Arc<Semaphore>,
}
static REQUESTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
impl Http {
    fn new(
        allow_local: bool,
        binding: Option<Arc<librqbit_dualstack_sockets::BindDevice>>,
    ) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .user_agent(crate::peer::CLIENT_NAME);
        if !allow_local {
            builder = builder.dns_resolver(Arc::new(PublicNames));
        }
        Ok(Self {
            bound: binding
                .map(|binding| crate::network::BoundHttp::new(binding, allow_local))
                .transpose()?,
            client: builder.build()?,
            allow_local,
            slots: REQUESTS.get_or_init(|| Arc::new(Semaphore::new(4))).clone(),
        })
    }
    async fn range(
        &self,
        url: Url,
        begin: u64,
        len: usize,
        total: u64,
        script: bool,
        core: Option<&TorrentCore>,
    ) -> Result<Vec<u8>> {
        ensure!(
            len > 0
                && len <= MAX_RANGE
                && begin
                    .checked_add(len as u64)
                    .is_some_and(|end| end <= total),
            "Invalid web seed range"
        );
        ensure!(
            request_url(url.as_str(), self.allow_local).is_some(),
            "Web seed endpoint is blocked"
        );
        #[cfg(all(test, windows))]
        crate::testkit::forbid_network();
        let _permit = self.slots.acquire().await?;
        let mut request = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        if !script {
            request = request.header(
                reqwest::header::RANGE,
                format!("bytes={begin}-{}", begin + len as u64 - 1),
            );
        }
        let request = request.build()?;
        let mut response = match &self.bound {
            Some(bound) => bound.send(request).await?,
            None => self
                .client
                .execute(request)
                .await
                .map_err(|e| e.without_url())?,
        };
        if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            let seconds = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
                ensure!(
                    body.len() + chunk.len() <= 256,
                    "HTTP seed error response is too large"
                );
                body.extend_from_slice(&chunk);
            }
            let seconds = seconds
                .or_else(|| std::str::from_utf8(&body).ok()?.trim().parse().ok())
                .unwrap_or(60);
            return Err(RetryAfter(Duration::from_secs(seconds.clamp(30, 3600))).into());
        }
        ensure!(
            response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_none_or(|v| v == "identity"),
            "Encoded web seed response is not allowed"
        );
        if script {
            ensure!(
                response.status() == reqwest::StatusCode::OK,
                "HTTP seed returned an error"
            );
        } else if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let expected = format!("bytes {begin}-{}/{total}", begin + len as u64 - 1);
            ensure!(
                response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|h| h.to_str().ok())
                    == Some(expected.as_str()),
                "Web seed returned the wrong range"
            );
        } else {
            ensure!(
                response.status() == reqwest::StatusCode::OK && begin == 0 && len as u64 == total,
                "Web seed did not honor the requested range"
            );
        }
        ensure!(
            response
                .content_length()
                .is_none_or(|length| length == len as u64),
            "Web seed returned the wrong length"
        );
        let mut bytes = Vec::with_capacity(len);
        while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
            ensure!(
                chunk.len() <= len - bytes.len(),
                "Web seed response exceeds the requested range"
            );
            if let Some(core) = core {
                for part in chunk.chunks(16 * 1024) {
                    core.count_downloaded(part.len() as u64);
                    core.context.down.take(part.len() as u32).await;
                    core.options.allowance.take(part.len() as u32).await;
                }
            }
            bytes.extend_from_slice(&chunk);
        }
        ensure!(bytes.len() == len, "Web seed response ended early");
        Ok(bytes)
    }
}

struct Claim {
    core: Arc<TorrentCore>,
    pieces: Vec<u32>,
}
impl Drop for Claim {
    fn drop(&mut self) {
        let mut picker = self.core.picker.lock();
        for piece in &self.pieces {
            picker.release_web(*piece);
        }
    }
}
#[derive(Debug)]
struct DiskFailure(String);
impl std::fmt::Display for DiskFailure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DiskFailure {}
#[derive(Debug)]
struct BadHash;
impl std::fmt::Display for BadHash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Web seed supplied a piece with the wrong hash")
    }
}
impl std::error::Error for BadHash {}

/// One sequential HTTP worker per torrent; at most four requests across the session/process.
/// A claim excludes peer writes until every spawned disk operation has been joined.
pub(crate) async fn run(live: Arc<Live>) {
    let core = &live.core;
    if !core.options.web_seeds || core.options.seed_only || core.meta.web_seeds.is_empty() {
        return;
    }
    let http = match Http::new(
        core.options.allow_loopback,
        core.context.peers.binding.clone(),
    ) {
        Ok(http) => http,
        Err(_) => {
            let _ = live.events.send(Event::Fatal(
                "Could not initialize web seed HTTP client".into(),
            ));
            return;
        }
    };
    struct Source {
        seed: Seed,
        ready: tokio::time::Instant,
        failures: u32,
        disabled: bool,
    }
    let mut sources = core
        .meta
        .web_seeds
        .iter()
        .cloned()
        .map(|seed| Source {
            seed,
            ready: tokio::time::Instant::now(),
            failures: 0,
            disabled: false,
        })
        .collect::<Vec<_>>();
    let mut cursor = 0;
    loop {
        if live.cancel.is_cancelled() || core.picker.lock().is_finished() {
            return;
        }
        let now = tokio::time::Instant::now();
        let next = (0..sources.len())
            .map(|n| (cursor + n) % sources.len())
            .find(|n| !sources[*n].disabled && sources[*n].ready <= now);
        let pieces = if next.is_some() {
            core.picker.lock().reserve_web(MAX_RANGE as u64)
        } else {
            Vec::new()
        };
        if pieces.is_empty() {
            if sources.iter().all(|source| source.disabled) {
                return;
            }
            // Shares the swarm's one-second scheduling cadence; cancellation wakes immediately.
            tokio::select! { biased; _=live.cancel.cancelled()=>return, _=tokio::time::sleep(Duration::from_secs(1))=>{} }
            continue;
        }
        let index = next.unwrap();
        cursor = (index + 1) % sources.len();
        let claim = Claim {
            core: core.clone(),
            pieces,
        };
        match download_claim(&live, &http, &sources[index].seed, &claim).await {
            Ok(()) => sources[index].failures = 0,
            Err(error) if error.is::<DiskFailure>() => {
                let _ = live.events.send(Event::Fatal(error.to_string()));
                return;
            }
            Err(error) if error.is::<BadHash>() => sources[index].disabled = true,
            Err(error) => {
                let source = &mut sources[index];
                source.failures = source.failures.saturating_add(1);
                let wait = error.downcast_ref::<RetryAfter>().map_or(
                    Duration::from_secs((30u64 << source.failures.min(4)).min(300)),
                    |retry| retry.0,
                );
                source.ready = tokio::time::Instant::now() + wait;
                // Never log a URL: web seeds may contain private query tokens.
                crate::debug_log!("web seed request failed; retry is delayed");
            }
        }
    }
}

async fn download_claim(live: &Live, http: &Http, seed: &Seed, claim: &Claim) -> Result<()> {
    let core = &claim.core;
    let meta = &core.meta;
    let lengths = meta.lengths;
    let base = checked_url(&seed.url, core.options.allow_loopback)
        .context("Web seed endpoint is blocked")?;
    let first = lengths.validate_piece_index(claim.pieces[0]).unwrap();
    let last = lengths
        .validate_piece_index(*claim.pieces.last().unwrap())
        .unwrap();
    let mut offset = lengths.piece_offset(first);
    let end = lengths.piece_offset(last) + u64::from(lengths.piece_length(last));
    let mut verified = 0;
    while offset < end {
        if live.cancel.is_cancelled() {
            return Ok(());
        }
        if !claim.pieces[verified..]
            .iter()
            .all(|p| core.picker.lock().web_wanted(*p))
        {
            return Ok(());
        }
        let piece = lengths
            .validate_piece_index(claim.pieces[verified])
            .unwrap();
        let piece_start = lengths.piece_offset(piece);
        let piece_end = piece_start + u64::from(lengths.piece_length(piece));
        let (url, begin, total, len, zeros) = match seed.kind {
            Kind::HttpSeed => {
                let len = (piece_end - offset).min(MAX_RANGE as u64) as usize;
                (
                    script_url(
                        &base,
                        meta,
                        claim.pieces[verified],
                        offset - piece_start,
                        len,
                    )?,
                    0,
                    len as u64,
                    len,
                    false,
                )
            }
            Kind::UrlList => {
                let file = meta
                    .files
                    .iter()
                    .find(|f| f.offset <= offset && offset < f.offset + f.len)
                    .context("Web seed file range is missing")?;
                let len = (end - offset)
                    .min(file.offset + file.len - offset)
                    .min(MAX_RANGE as u64) as usize;
                (
                    file_url(&base, meta, file)?,
                    offset - file.offset,
                    file.len,
                    len,
                    file.virtual_file,
                )
            }
        };
        let bytes = if zeros {
            vec![0; len]
        } else {
            tokio::select! { biased;
                _=live.cancel.cancelled()=>return Ok(()),
                result=http.range(url,begin,len,total,seed.kind==Kind::HttpSeed,Some(core))=>result?,
            }
        };
        if live.cancel.is_cancelled() {
            return Ok(());
        }
        if !claim.pieces[verified..]
            .iter()
            .all(|p| core.picker.lock().web_wanted(*p))
        {
            return Ok(());
        }
        let storage = core.storage.clone();
        // Do not select cancellation here: Stop all must join writes already started.
        tokio::task::spawn_blocking(move || storage.write(offset, &bytes))
            .await
            .map_err(|e| DiskFailure(format!("A web seed disk write failed: {e}")))?
            .map_err(|e| DiskFailure(format!("Could not write to the download folder: {e}")))?;
        offset += len as u64;
        while verified < claim.pieces.len() {
            let number = claim.pieces[verified];
            let piece = lengths.validate_piece_index(number).unwrap();
            if lengths.piece_offset(piece) + u64::from(lengths.piece_length(piece)) > offset {
                break;
            }
            let storage = core.storage.clone();
            let good = tokio::task::spawn_blocking(move || storage.piece_matches(number))
                .await
                .map_err(|e| DiskFailure(format!("A web seed hash check failed: {e}")))?
                .map_err(|e| DiskFailure(format!("Could not read the download files: {e}")))?;
            if !good {
                return Err(BadHash.into());
            }
            if core.picker.lock().web_verified(number) {
                let _ = live.events.send(Event::WebVerified(number));
            }
            verified += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{meta_of, multi_file_torrent, single_file_torrent};

    pub(crate) fn with_seed(mut torrent: Vec<u8>, key: &str, url: &str) -> Vec<u8> {
        torrent.pop();
        torrent.extend(format!("{}:{key}{}:{url}e", key.len(), url.len()).bytes());
        torrent
    }
    #[test]
    fn seed_metadata_is_bounded_and_deduplicated() {
        let file = single_file_torrent("a.bin", b"test", 16384);
        let meta = meta_of(&with_seed(
            file.clone(),
            "url-list",
            "https://example.org/a.bin",
        ));
        assert_eq!(meta.web_seeds.len(), 1);
        assert!(
            TorrentMeta::from_torrent_file(&with_seed(file.clone(), "url-list", &"a".repeat(2049)))
                .is_err()
        );
        let mut too_many = file.clone();
        too_many.pop();
        too_many.extend(b"8:url-listl");
        for _ in 0..33 {
            too_many.extend(b"21:https://example.org/a");
        }
        too_many.extend(b"ee");
        assert!(TorrentMeta::from_torrent_file(&too_many).is_err());
        assert!(
            meta_of(&with_seed(file, "url-list", "file:///etc/passwd"))
                .web_seeds
                .is_empty()
        );
    }
    #[test]
    fn private_urls_credentials_and_service_ports_are_blocked() {
        for url in [
            "http://127.0.0.1/a",
            "http://[::1]/a",
            "http://[::ffff:127.0.0.1]/a",
            "http://10.1.2.3/a",
            "http://169.254.169.254/a",
            "http://localhost/a",
            "https://example.org:6379/a",
            "https://u:p@example.org/a",
            "ftp://example.org/a",
            "https://example.org/a#b",
        ] {
            assert!(checked_url(url, false).is_none(), "{url}");
        }
        assert!(checked_url("https://example.org/a?token=opaque", false).is_some());
        assert!(checked_url("http://127.0.0.1:8000/a", true).is_some());
    }
    #[test]
    fn web_names_use_original_bytes_and_cannot_escape_base() {
        let meta = meta_of(&multi_file_torrent(
            "a b",
            &[(&["x?%#.bin"], b"test", false)],
            16384,
        ));
        let base = Url::parse("https://example.org/base/?token=ok").unwrap();
        let url = file_url(&base, &meta, &meta.files[0]).unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.org/base/a%20b/x%3F%25%23.bin?token=ok"
        );
        assert!(TorrentMeta::from_torrent_file(&single_file_torrent("..", b"x", 16384)).is_err());
        let mut meta =
            TorrentMeta::from_torrent_file(&single_file_torrent("normal.bin", b"x", 16384))
                .unwrap();
        meta.web_name = "..".into();
        assert!(file_url(&base, &meta, &meta.files[0]).is_err());
        meta.web_name = "normal.bin".into();
        meta.files[0].web_path = "..%2Fprivate".into();
        assert!(file_url(&base, &meta, &meta.files[0]).is_err());
        let direct = Url::parse("https://example.org/download?file=1").unwrap();
        assert_eq!(file_url(&direct, &meta, &meta.files[0]).unwrap(), direct);
    }
    #[test]
    fn bep17_binary_hash_and_ranges_are_encoded_once() {
        let meta = meta_of(&single_file_torrent("a.bin", b"test", 16384));
        let url = script_url(
            &Url::parse("https://example.org/seed?token=ok").unwrap(),
            &meta,
            3,
            42,
            10,
        )
        .unwrap();
        assert!(url.query().unwrap().starts_with("token=ok&info_hash=%"));
        assert!(url.query().unwrap().ends_with("&piece=3&ranges=42-51"));
        assert!(!url.as_str().contains("%25"));
        assert!(
            script_url(
                &Url::parse("https://example.org/seed?piece=1").unwrap(),
                &meta,
                3,
                42,
                10
            )
            .is_err()
        );
    }
}

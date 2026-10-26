//! Bilibili's public web API, called the way its own web player does: anonymous
//! visitor IDs, WBI-signed view and playurl requests, no account cookies.
//! Bangumi (PGC) episodes use the season and PGC playurl endpoints instead.
use crate::{
    engine,
    network_scope::{NetworkScope, network_scope},
    website_grabber::pinned_resolution,
};
use anyhow::{Context, Result, bail, ensure};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Client, Url, header};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const API: &str = "https://api.bilibili.com";
const MAX_BODY: usize = 2 * 1024 * 1024;
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const REFERER: &str = "https://www.bilibili.com/";
/// Bilibili's fixed WBI shuffle of the two published key halves.
const MIXIN: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];
/// Like Python's quote(safe=""): only unreserved characters stay literal.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Deserialize)]
struct Envelope<T> {
    code: i64,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<T>,
    /// Bangumi (PGC) endpoints reply with `result` instead of `data`.
    #[serde(default)]
    result: Option<T>,
}

#[derive(Deserialize)]
pub(super) struct View {
    pub bvid: String,
    pub title: String,
    pub pages: Vec<Page>,
}
#[derive(Deserialize)]
pub(super) struct Page {
    pub cid: u64,
    pub page: u32,
    pub part: String,
}

#[derive(Deserialize)]
pub(super) struct Play {
    pub timelength: Option<u64>,
    pub dash: Option<Dash>,
    /// Bangumi only: set when visitors get a short preview, not the episode.
    #[serde(default)]
    pub is_preview: Option<Value>,
    #[serde(default)]
    pub is_drm: Option<Value>,
}

/// A Bangumi season: its main episodes plus extra sections (trailers, extras).
#[derive(Deserialize)]
pub(super) struct Season {
    pub title: String,
    #[serde(default)]
    pub episodes: Option<Vec<Episode>>,
    #[serde(default)]
    pub section: Option<Vec<Section>>,
}
#[derive(Deserialize)]
pub(super) struct Section {
    #[serde(default)]
    pub episodes: Option<Vec<Episode>>,
}
#[derive(Deserialize)]
pub(super) struct Episode {
    /// The episode ID of its bangumi/play/ep… page.
    pub id: u64,
    pub cid: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub long_title: Option<String>,
    #[serde(default)]
    pub show_title: Option<String>,
}
impl Season {
    pub fn all_episodes(&self) -> impl Iterator<Item = &Episode> {
        self.episodes.iter().flatten().chain(
            self.section
                .iter()
                .flatten()
                .flat_map(|s| s.episodes.iter().flatten()),
        )
    }
}

pub(super) enum SeasonId {
    Episode(u64),
    Season(u64),
}
#[derive(Deserialize)]
pub(super) struct Dash {
    pub duration: Option<u64>,
    pub video: Vec<Stream>,
    #[serde(default)]
    pub audio: Option<Vec<Stream>>,
}
/// Bilibili sends most fields twice (camelCase and snake_case); read one of each.
#[derive(Deserialize)]
pub(super) struct Stream {
    pub id: u32,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    #[serde(rename = "backupUrl", default)]
    pub backup_url: Option<Vec<String>>,
    pub bandwidth: u64,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub codecs: String,
    #[serde(default)]
    pub codecid: u32,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(rename = "segment_base")]
    pub segment_base: Option<SegmentBase>,
}
#[derive(Deserialize)]
pub(super) struct SegmentBase {
    pub initialization: String,
    pub index_range: String,
}

pub(super) enum Id<'a> {
    Bv(&'a str),
    Av(u64),
}

pub(super) struct Api {
    client: Client,
    route: crate::proxy::Route,
    cookie: Option<String>,
    key: String,
}
impl Api {
    pub async fn open(cancel: &CancellationToken, route: &crate::proxy::Route) -> Result<Self> {
        let url = Url::parse(API)?;
        let resolved = tokio::select! { biased;
            _ = cancel.cancelled() => bail!("Bilibili lookup cancelled"),
            result = tokio::time::timeout(Duration::from_secs(10), pinned_resolution(&url)) => result.context("Bilibili DNS lookup timed out")??,
        }
        .context("Bilibili hostname did not resolve")?;
        ensure!(!resolved.1.is_empty(), "Bilibili hostname did not resolve");
        for address in &resolved.1 {
            ensure!(
                network_scope(address.ip())? == NetworkScope::Public,
                "Bilibili resolved to a non-public server"
            );
        }
        let client = route
            .configure(engine::client_builder())?
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(&resolved.0, &resolved.1)
            .build()?;
        let mut api = Self {
            client,
            route: route.clone(),
            cookie: None,
            key: String::new(),
        };
        // Anonymous visitor IDs, as the web player sends them. Signed requests also
        // work without them, so a failure here is not fatal.
        if let Ok(spi) = api
            .get::<Value>("/x/frontend/finger/spi", &[], cancel)
            .await
        {
            api.cookie = visitor_cookie(&spi);
        }
        let nav = api.nav(cancel).await?;
        api.key = mixin_key(&nav)?;
        Ok(api)
    }

    pub async fn view(&self, id: Id<'_>, cancel: &CancellationToken) -> Result<View> {
        let param = match id {
            Id::Bv(bvid) => ("bvid", bvid.to_owned()),
            Id::Av(aid) => ("aid", aid.to_string()),
        };
        let view: View = self
            .get("/x/web-interface/wbi/view", &self.sign(&[param]), cancel)
            .await?;
        ensure!(
            !view.pages.is_empty() && view.pages.len() <= 1000,
            "Bilibili returned no video parts"
        );
        Ok(view)
    }

    pub async fn play(&self, bvid: &str, cid: u64, cancel: &CancellationToken) -> Result<Play> {
        // qn 127 and fnval 4048 ask for every DASH quality, codec and audio track
        // this visitor may use; Bilibili lists only what it actually allows.
        let params = self.sign(&[
            ("bvid", bvid.to_owned()),
            ("cid", cid.to_string()),
            ("qn", "127".into()),
            ("fnval", "4048".into()),
            ("fourk", "1".into()),
        ]);
        self.get("/x/player/wbi/playurl", &params, cancel).await
    }

    pub async fn season(&self, id: SeasonId, cancel: &CancellationToken) -> Result<Season> {
        let param = match id {
            SeasonId::Episode(ep) => ("ep_id".to_owned(), ep.to_string()),
            SeasonId::Season(ss) => ("season_id".to_owned(), ss.to_string()),
        };
        let season: Season = self.get("/pgc/view/web/season", &[param], cancel).await?;
        let count = season.all_episodes().count();
        ensure!(count > 0, "This Bilibili series has no episodes yet");
        ensure!(
            count <= 5000,
            "This Bilibili series lists too many episodes"
        );
        Ok(season)
    }

    pub async fn episode_play(
        &self,
        ep: u64,
        cid: u64,
        cancel: &CancellationToken,
    ) -> Result<Play> {
        let params: Vec<(String, String)> = [
            ("ep_id", ep.to_string()),
            ("cid", cid.to_string()),
            ("qn", "127".into()),
            ("fnval", "4048".into()),
            ("fourk", "1".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let play: Play = self.get("/pgc/player/web/playurl", &params, cancel).await?;
        ensure!(
            !flag(play.is_drm.as_ref()),
            "This Bilibili episode is DRM-protected and cannot be downloaded"
        );
        ensure!(
            !flag(play.is_preview.as_ref()),
            "Bilibili shows visitors only a short preview of this episode; the full episode is for members. Sign-in is not supported yet."
        );
        Ok(play)
    }

    async fn nav(&self, cancel: &CancellationToken) -> Result<Value> {
        // Signed-out visitors get code -101 together with the WBI key images.
        let body = self.body("/x/web-interface/nav", &[], cancel).await?;
        let envelope: Envelope<Value> =
            serde_json::from_str(&body).context("Invalid Bilibili navigation reply")?;
        envelope
            .data
            .context("Bilibili did not publish its request signing keys")
    }

    fn sign(&self, params: &[(&str, String)]) -> Vec<(String, String)> {
        sign(params, &self.key, chrono::Utc::now().timestamp())
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(String, String)],
        cancel: &CancellationToken,
    ) -> Result<T> {
        payload(&self.body(path, params, cancel).await?)
    }

    async fn body(
        &self,
        path: &str,
        params: &[(String, String)],
        cancel: &CancellationToken,
    ) -> Result<String> {
        let read = async {
            let mut request = self
                .client
                .get(format!("{API}{path}?{}", query(params)))
                .header(header::USER_AGENT, WEB_UA)
                .header(header::REFERER, REFERER)
                .header(header::ACCEPT_ENCODING, "identity");
            if let Some(cookie) = &self.cookie {
                request = request.header(header::COOKIE, cookie);
            }
            let (client, request) = request.build_split();
            let mut response = self.route.execute(&client, request?, None).await?;
            let status = response.status();
            ensure!(
                status.as_u16() != 412,
                "Bilibili's anti-bot check refused this request (HTTP 412). Try again in a few minutes."
            );
            ensure!(
                status == reqwest::StatusCode::OK,
                "Bilibili returned HTTP {}",
                status.as_u16()
            );
            engine::identity(&response)?;
            ensure!(
                response
                    .content_length()
                    .is_none_or(|n| n <= MAX_BODY as u64),
                "Bilibili reply exceeds 2 MiB"
            );
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    bytes.len() + chunk.len() <= MAX_BODY,
                    "Bilibili reply exceeds 2 MiB"
                );
                bytes.extend_from_slice(&chunk);
            }
            String::from_utf8(bytes).context("Bilibili reply is not UTF-8")
        };
        tokio::select! { biased;
            _ = cancel.cancelled() => bail!("Bilibili lookup cancelled"),
            result = tokio::time::timeout(Duration::from_secs(25), read) => result.context("Bilibili request timed out")?,
        }
    }
}

/// An API reply's payload, from `data` or (Bangumi) `result`.
pub(super) fn payload<T: DeserializeOwned>(body: &str) -> Result<T> {
    // Check the code before the payload: error replies carry other data shapes.
    let envelope: Envelope<Value> =
        serde_json::from_str(body).context("Invalid Bilibili API reply")?;
    check_code(envelope.code, &envelope.message)?;
    let payload = envelope
        .data
        .or(envelope.result)
        .context("Bilibili returned an empty reply")?;
    serde_json::from_value(payload).context("Unexpected Bilibili API reply")
}

fn check_code(code: i64, message: &str) -> Result<()> {
    match code {
        0 => Ok(()),
        -404 | 62002 | 62004 | 62012 => {
            bail!("This Bilibili video was removed, is private or does not exist")
        }
        -10403 | 6002003 => bail!("This Bilibili video is not available in your region"),
        -403 | 87007 | 87008 => bail!(
            "Bilibili allows this video only for signed-in, paying or charging members. Sign-in is not supported yet."
        ),
        -352 | -412 => {
            bail!("Bilibili's anti-bot check refused this request. Try again in a few minutes.")
        }
        _ => {
            let message: String = message
                .chars()
                .filter(|c| !c.is_control())
                .take(120)
                .collect();
            bail!("Bilibili refused this request (code {code}: {message})")
        }
    }
}

/// Bilibili sends these flags as 0/1 or as booleans.
pub(super) fn flag(value: Option<&Value>) -> bool {
    value.is_some_and(|v| v.as_bool() == Some(true) || v.as_i64().is_some_and(|n| n != 0))
}

fn visitor_cookie(spi: &Value) -> Option<String> {
    let id = |key: &str| {
        spi.get(key)
            .and_then(Value::as_str)
            .filter(|v| v.len() <= 128 && v.bytes().all(|b| b.is_ascii_graphic() && b != b';'))
    };
    Some(format!(
        "buvid3={}; buvid4={}",
        id("b_3")?,
        utf8_percent_encode(id("b_4")?, QUERY)
    ))
}

/// The two halves are the file names of the published key images.
fn mixin_key(nav: &Value) -> Result<String> {
    let stem = |key: &str| -> Result<String> {
        let url = nav
            .pointer(&format!("/wbi_img/{key}"))
            .and_then(Value::as_str)
            .context("Bilibili did not publish its request signing keys")?;
        let name = url.rsplit('/').next().unwrap_or_default();
        let stem = name.split('.').next().unwrap_or_default();
        ensure!(
            stem.len() == 32 && stem.bytes().all(|b| b.is_ascii_alphanumeric()),
            "Bilibili published an unexpected request signing key"
        );
        Ok(stem.to_owned())
    };
    let raw = stem("img_url")? + &stem("sub_url")?;
    Ok(mixin(&raw))
}
pub(super) fn mixin(raw: &str) -> String {
    let raw = raw.as_bytes();
    MIXIN.iter().take(32).map(|&i| char::from(raw[i])).collect()
}

/// WBI: add the time, sort, drop `!'()*` from values, then MD5 the query with the key.
pub(super) fn sign(params: &[(&str, String)], key: &str, wts: i64) -> Vec<(String, String)> {
    let mut signed: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| {
            (
                (*k).to_owned(),
                v.chars().filter(|c| !"!'()*".contains(*c)).collect(),
            )
        })
        .chain([("wts".to_owned(), wts.to_string())])
        .collect();
    signed.sort();
    let digest = md5_hex(format!("{}{key}", query(&signed)).as_bytes());
    signed.push(("w_rid".into(), digest));
    signed
}
fn query(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, QUERY),
                utf8_percent_encode(v, QUERY)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// RFC 1321 MD5. Bilibili's request signature needs it; it protects nothing here.
pub(super) fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    // Padding made the length a whole number of 64-byte blocks.
    for block in message.as_chunks::<64>().0 {
        let bytes = block.as_chunks::<4>().0;
        let words: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes(bytes[i]));
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a
                .wrapping_add(f)
                .wrapping_add(K[i])
                .wrapping_add(words[g])
                .rotate_left(S[i]);
            (a, d, c) = (d, c, b);
            b = b.wrapping_add(rotated);
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d]) {
            *value = value.wrapping_add(add);
        }
    }
    state
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

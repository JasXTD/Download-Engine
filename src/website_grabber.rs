//! Bounded, anonymous HTML discovery. Scanning never creates download jobs.
use crate::{
    model::{safe_filename, validate_url},
    network_scope::{network_scope, validate_resolution},
};
use anyhow::{Context, Result, ensure};
use percent_encoding::percent_decode_str;
use reqwest::{Client, Url, header};
use scraper::{Html, Selector};
use std::{
    collections::{HashSet, VecDeque},
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const MAX_DEPTH: u8 = 5;
pub const MAX_PAGES: usize = 200;
pub const MAX_FILES: usize = 1000;
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SCAN_BYTES: usize = 20 * 1024 * 1024;
const MAX_PREVIEW_BYTES: usize = 700_000;
const MAX_LINKS_PER_PAGE: usize = 10_000;
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);
const SCAN_TIMEOUT: Duration = Duration::from_secs(180);
const PAGE_ACCEPT: &str = "text/html, application/xhtml+xml";

#[derive(Clone, Debug)]
pub struct FileTypes(Vec<String>);

impl FileTypes {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 512, "The file-type list is too long");
        let mut extensions = Vec::new();
        for part in text
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .filter(|s| !s.is_empty())
        {
            let part = part
                .trim_start_matches("*.")
                .trim_start_matches('.')
                .to_ascii_lowercase();
            ensure!(
                part.len() <= 24
                    && part
                        .split('.')
                        .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric())),
                "Use extensions such as pdf, zip, jpg, or tar.gz"
            );
            if !extensions.contains(&part) {
                extensions.push(part);
            }
        }
        ensure!(
            !extensions.is_empty() && extensions.len() <= 32,
            "Enter between 1 and 32 file extensions"
        );
        Ok(Self(extensions))
    }

    fn matches(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.0.iter().any(|ext| name.ends_with(&format!(".{ext}")))
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    route: crate::proxy::Route,
    start: Url,
    types: FileTypes,
    depth: u8,
    pages: usize,
}

impl Config {
    pub fn with_proxy(mut self, store: &crate::store::Store) -> Result<Self> {
        self.route = crate::proxy::Route::load(store)?;
        Ok(self)
    }
    pub fn new(url: &str, file_types: &str, depth: u8, pages: usize) -> Result<Self> {
        ensure!(
            depth <= MAX_DEPTH,
            "Scan depth must be between 0 and {MAX_DEPTH}"
        );
        ensure!(
            (1..=MAX_PAGES).contains(&pages),
            "Page limit must be between 1 and {MAX_PAGES}"
        );
        Ok(Self {
            route: crate::proxy::Route::direct(),
            start: validate_url(url)?,
            types: FileTypes::parse(file_types)?,
            depth,
            pages,
        })
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub url: String,
    /// A display hint; only an explicit HTML download name is passed to the job.
    pub name: String,
    pub filename: Option<String>,
    pub page: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StopReason {
    #[default]
    Finished,
    Cancelled,
    PageLimit,
    FileLimit,
    SizeLimit,
    TimeLimit,
}
impl StopReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Finished => "Scan finished",
            Self::Cancelled => "Scan stopped; partial results",
            Self::PageLimit => "Page limit reached; partial results",
            Self::FileLimit => "File limit reached; partial results",
            Self::SizeLimit => "Size limit reached; partial results",
            Self::TimeLimit => "Time limit reached; partial results",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub page: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub files: Vec<Candidate>,
    pub pages_read: usize,
    pub pages_failed: usize,
    pub issues: Vec<Issue>,
    pub stop: StopReason,
}
impl Report {
    fn issue(&mut self, page: &Url, message: String) {
        if self.issues.len() < 20 {
            self.issues.push(Issue {
                page: page.to_string(),
                message,
            });
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub pages_read: usize,
    pub files_found: usize,
    pub current_page: String,
}

struct Page {
    url: Url,
    html: String,
}

pub(crate) async fn pinned_resolution(start: &Url) -> Result<Option<(String, Vec<SocketAddr>)>> {
    let host = start
        .host_str()
        .context("The website URL has no host")?
        .trim_matches(['[', ']']);
    if let Ok(ip) = host.parse::<IpAddr>() {
        network_scope(ip)?;
        return Ok(None);
    }
    let port = start
        .port_or_known_default()
        .context("The website URL has no network port")?;
    let resolved = tokio::time::timeout(PAGE_TIMEOUT, tokio::net::lookup_host((host, port)))
        .await
        .context("Timed out resolving the website host")??;
    let mut addresses = Vec::new();
    for address in resolved {
        if !addresses.contains(&address.ip()) {
            addresses.push(address.ip());
        }
    }
    validate_resolution(addresses.iter().copied())?;
    let pinned = addresses
        .into_iter()
        .map(|ip| SocketAddr::new(ip, 0))
        .collect();
    Ok(Some((host.to_string(), pinned)))
}

async fn client(start: &Url, route: &crate::proxy::Route) -> Result<Client> {
    // Validate local DNS even when the selected proxy performs its own resolution.
    // That proxy is a trusted routing endpoint; direct requests retain DNS pinning.
    let mut builder = route
        .configure(crate::engine::client_builder())?
        .timeout(PAGE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if let Some((host, addresses)) = pinned_resolution(start).await? {
        builder = builder.resolve_to_addrs(&host, &addresses);
    }
    Ok(builder.build()?)
}

async fn read_page(
    client: &Client,
    mut url: Url,
    total_bytes: &mut usize,
    route: &crate::proxy::Route,
    accept: &str,
) -> Result<Page> {
    let origin = url.origin();
    let mut redirects = 0;
    let mut response = loop {
        let request = client
            .get(url.clone())
            .header(header::ACCEPT, accept)
            .build()?;
        let response = route.execute(client, request, None).await?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            ensure!(redirects < 5, "Too many page redirects");
            let location = response
                .headers()
                .get(header::LOCATION)
                .context("Page redirect has no destination")?
                .to_str()?;
            let next = validate_url(url.join(location)?.as_str())?;
            ensure!(
                next.origin() == origin,
                "Page redirects outside the starting origin are not scanned. Enter the destination page as the starting URL."
            );
            url = next;
            redirects += 1;
        } else {
            crate::auth::check_response(&response)?;
            break response
                .error_for_status()
                .map_err(reqwest::Error::without_url)?;
        }
    };
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    ensure!(
        mime.eq_ignore_ascii_case("text/html")
            || mime.eq_ignore_ascii_case("application/xhtml+xml")
            || mime.eq_ignore_ascii_case("text/fragment+html"),
        "Skipped a non-HTML response"
    );
    ensure!(
        !response.headers().contains_key(header::CONTENT_DISPOSITION),
        "Skipped an attachment response"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= MAX_PAGE_BYTES as u64),
        "Page exceeds the 2 MiB limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        *total_bytes = total_bytes.saturating_add(chunk.len());
        ensure!(
            *total_bytes <= MAX_SCAN_BYTES,
            "Scan exceeds the 20 MiB limit"
        );
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_PAGE_BYTES,
            "Page exceeds the 2 MiB limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(Page {
        url: response.url().clone(),
        html: String::from_utf8(bytes).context("This page is not UTF-8 HTML")?,
    })
}

fn url_name(url: &Url) -> String {
    percent_decode_str(
        url.path_segments()
            .and_then(|mut p| p.next_back())
            .unwrap_or(""),
    )
    .decode_utf8_lossy()
    .into_owned()
}

pub(crate) fn possible_page(url: &Url) -> bool {
    let name = url_name(url).to_ascii_lowercase();
    !name.contains('.')
        || name.ends_with('/')
        || [
            ".html", ".htm", ".xhtml", ".shtml", ".php", ".asp", ".aspx", ".jsp", ".cgi",
        ]
        .iter()
        .any(|ext| name.ends_with(ext))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Navigation {
    Page,
    Fragment(&'static str),
}

struct Link {
    url: Url,
    filename: Option<String>,
    navigation: Option<Navigation>,
}

fn download_url(url: &Url) -> Url {
    let mut download = url.clone();
    // GitHub's blob route is an HTML viewer, even when its path ends in a file
    // extension. The raw route preserves GitHub's own branch/tag resolution.
    if url.host_str() == Some("github.com") && url.port().is_none() {
        let mut segments: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
        if segments.len() >= 5 && segments[2] == "blob" && segments.iter().all(|s| !s.is_empty()) {
            segments[2] = "raw";
            download.set_path(&format!("/{}", segments.join("/")));
        }
    }
    download
}

fn page_links(page: &Page) -> (Vec<Link>, bool) {
    let document = Html::parse_document(&page.html);
    let base_selector = Selector::parse("base[href]").expect("static selector");
    // Resolve relative URLs as HTML does, then enforce origin on each crawl URL.
    let base = document
        .select(&base_selector)
        .next()
        .and_then(|e| page.url.join(e.value().attr("href")?).ok())
        .unwrap_or_else(|| page.url.clone());
    let selector = Selector::parse(
        "a[href], area[href], img[src], video[src], audio[src], source[src], include-fragment[src]",
    )
    .expect("static selector");
    let mut links = Vec::new();
    let mut truncated = false;
    for (index, element) in document.select(&selector).enumerate() {
        if index >= MAX_LINKS_PER_PAGE {
            truncated = true;
            break;
        }
        let node = element.value();
        let navigate = matches!(node.name(), "a" | "area");
        let Some(raw) = node.attr(if navigate { "href" } else { "src" }) else {
            continue;
        };
        if raw.trim().is_empty() || raw.trim().starts_with('#') || raw.len() > 16384 {
            continue;
        }
        let Ok(url) = base.join(raw.trim()) else {
            continue;
        };
        let Ok(url) = validate_url(url.as_str()) else {
            continue;
        };
        let filename = node
            .attr("download")
            .filter(|s| !s.trim().is_empty())
            .map(safe_filename);
        links.push(Link {
            url,
            filename,
            navigation: if node.name() == "include-fragment" {
                // Copy only a known HTML media type, never arbitrary request
                // headers from a page. Other fragment formats need JavaScript.
                Some(Navigation::Fragment(
                    match node
                        .attr("accept")
                        .unwrap_or("text/html")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "text/html" => "text/html",
                        "application/xhtml+xml" => "application/xhtml+xml",
                        "text/fragment+html" => "text/fragment+html",
                        _ => continue,
                    },
                ))
            } else if navigate && node.attr("download").is_none() {
                Some(Navigation::Page)
            } else {
                None
            },
        });
    }
    // Fetch static fragments (for example GitHub release assets) before ordinary
    // navigation can consume the page budget. They are parsed as HTML, never run.
    links.sort_by_key(|link| !matches!(link.navigation, Some(Navigation::Fragment(_))));
    (links, truncated)
}

/// Breadth-first discovery: depth 0 reads the starting page and its same-origin
/// HTML fragments, all within the page/byte budgets. Matches become preview
/// candidates, never jobs. External candidates are never crawled.
pub async fn scan(
    config: Config,
    cancel: CancellationToken,
    mut progress: impl FnMut(Progress),
) -> Result<Report> {
    let client = client(&config.start, &config.route).await?;
    let origin = config.start.origin();
    let mut pending = VecDeque::from([(config.start.clone(), 0u8, PAGE_ACCEPT)]);
    let mut scheduled = HashSet::from([config.start.to_string()]);
    let mut parsed = HashSet::new();
    let mut candidates = HashSet::new();
    let mut report = Report::default();
    let (mut total_bytes, mut preview_bytes, mut requests) = (0usize, 0usize, 0usize);
    let mut page_limit = false;
    let deadline = tokio::time::Instant::now() + SCAN_TIMEOUT;
    let mut pacing = tokio::time::interval(Duration::from_millis(150));
    pacing.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    while let Some((url, depth, accept)) = pending.pop_front() {
        if cancel.is_cancelled() {
            report.stop = StopReason::Cancelled;
            break;
        }
        if parsed.contains(url.as_str()) {
            continue;
        }
        if requests >= config.pages {
            report.stop = StopReason::PageLimit;
            break;
        }
        progress(Progress {
            pages_read: report.pages_read,
            files_found: report.files.len(),
            current_page: url.to_string(),
        });
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => { report.stop = StopReason::Cancelled; break; },
            _ = tokio::time::sleep_until(deadline) => { report.stop = StopReason::TimeLimit; break; },
            result = async {
                pacing.tick().await;
                requests += 1;
                tokio::time::timeout(PAGE_TIMEOUT, read_page(&client, url.clone(), &mut total_bytes, &config.route, accept)).await.context("Page timed out")?
            } => result,
        };
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                report.pages_failed += 1;
                report.issue(&url, format!("{error:#}"));
                if total_bytes > MAX_SCAN_BYTES {
                    report.stop = StopReason::SizeLimit;
                    break;
                }
                continue;
            }
        };
        if !parsed.insert(page.url.to_string()) {
            continue;
        }
        report.pages_read += 1;
        let (links, truncated) = page_links(&page);
        if truncated {
            report.issue(
                &page.url,
                "Only the first 10,000 links on this page were inspected".into(),
            );
        }
        for link in links {
            let name = link
                .filename
                .clone()
                .unwrap_or_else(|| safe_filename(&url_name(&link.url)));
            let download = download_url(&link.url);
            if !matches!(link.navigation, Some(Navigation::Fragment(_)))
                && config.types.matches(&name)
                && candidates.insert(download.to_string())
            {
                preview_bytes += download.as_str().len() + name.len() + 96;
                if preview_bytes > MAX_PREVIEW_BYTES {
                    report.stop = StopReason::SizeLimit;
                    break;
                }
                report.files.push(Candidate {
                    url: download.to_string(),
                    name,
                    filename: link.filename,
                    page: page.url.to_string(),
                });
                if report.files.len() == MAX_FILES {
                    report.stop = StopReason::FileLimit;
                    break;
                }
            }
            let (next_depth, accept) = match link.navigation {
                Some(Navigation::Fragment(accept)) => (depth, accept),
                Some(Navigation::Page) if depth < config.depth && possible_page(&link.url) => {
                    (depth + 1, PAGE_ACCEPT)
                }
                _ => continue,
            };
            if link.url.origin() == origin {
                if scheduled.contains(link.url.as_str()) {
                    continue;
                }
                if scheduled.len() >= config.pages {
                    page_limit = true;
                    continue;
                }
                scheduled.insert(link.url.to_string());
                pending.push_back((link.url, next_depth, accept));
            }
        }
        if report.stop != StopReason::Finished {
            break;
        }
    }
    if report.stop == StopReason::Finished && page_limit {
        report.stop = StopReason::PageLimit;
    }
    progress(Progress {
        pages_read: report.pages_read,
        files_found: report.files.len(),
        current_page: String::new(),
    });
    Ok(report)
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[tokio::test]
    async fn literal_addresses_are_validated_without_dns_resolution() {
        for value in [
            "http://127.0.0.1/",
            "http://[::1]/",
            "https://[2606:4700::1111]/",
        ] {
            let url = Url::parse(value).unwrap();
            assert!(pinned_resolution(&url).await.unwrap().is_none(), "{value}");
        }
        for value in ["http://0.0.0.0/", "http://[::]/", "http://[ff02::1]/"] {
            assert!(
                pinned_resolution(&Url::parse(value).unwrap())
                    .await
                    .is_err()
            );
        }
    }
}

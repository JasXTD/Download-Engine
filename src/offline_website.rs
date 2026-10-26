//! Bounded static website copies. No browser sessions or downloaded scripts run.
mod css;
mod html;
mod network;
mod svg;

use anyhow::{Context, Result, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const MAX_PAGES: usize = 200;
pub const MAX_ASSETS: usize = 2_000;
pub const MAX_DEPTH: u8 = 5;
pub const MAX_MEGABYTES: usize = 500;
const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;
const MAX_ISSUES: usize = 200;
const MAX_REFERENCES: usize = 10_000;

#[derive(Clone, Debug)]
pub struct Config {
    pub url: String,
    pub destination: PathBuf,
    pub depth: u8,
    pub pages: usize,
    pub megabytes: usize,
    pub external_assets: bool,
    route: crate::proxy::Route,
}
impl Config {
    pub fn new(url: &str, destination: PathBuf, depth: u8, pages: usize) -> Result<Self> {
        let value = Self {
            url: url.into(),
            destination,
            depth,
            pages,
            megabytes: 100,
            external_assets: true,
            route: crate::proxy::Route::direct(),
        };
        value.validate()?;
        Ok(value)
    }
    pub fn with_proxy(mut self, store: &crate::store::Store) -> Result<Self> {
        self.route = crate::proxy::Route::load(store)?;
        Ok(self)
    }
    fn validate(&self) -> Result<Url> {
        ensure!(
            self.depth <= MAX_DEPTH,
            "Depth must be between 0 and {MAX_DEPTH}"
        );
        ensure!(
            (1..=MAX_PAGES).contains(&self.pages),
            "Page limit must be between 1 and {MAX_PAGES}"
        );
        ensure!(
            (1..=MAX_MEGABYTES).contains(&self.megabytes),
            "Size limit must be between 1 and {MAX_MEGABYTES} MB"
        );
        ensure!(
            self.destination.is_absolute(),
            "Choose an absolute destination folder"
        );
        crate::model::validate_url(&self.url)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    #[default]
    Finished,
    Cancelled,
    SizeLimit,
    TimeLimit,
}
impl StopReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Finished => "Offline copy saved",
            Self::Cancelled => "Stopped — partial copy saved",
            Self::SizeLimit => "Size limit reached — partial copy saved",
            Self::TimeLimit => "Time limit reached — partial copy saved",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub url: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedFile {
    pub url: String,
    pub path: String,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub folder: PathBuf,
    pub start_url: String,
    pub pages: usize,
    pub assets: usize,
    pub bytes: usize,
    pub stop: StopReason,
    pub issues: Vec<Issue>,
    pub issue_count: usize,
    pub interactive_pages: usize,
    pub files: Vec<SavedFile>,
}
impl Report {
    fn issue(&mut self, url: &Url, message: impl Into<String>) {
        self.issue_count += 1;
        if self.issues.len() < MAX_ISSUES {
            self.issues.push(Issue {
                url: url.to_string(),
                message: message.into(),
            });
        }
    }
    pub fn entry(&self) -> PathBuf {
        self.folder.join("index.html")
    }
}
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub pages: usize,
    pub assets: usize,
    pub bytes: usize,
    pub current: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Page,
    Css,
    Asset,
}
struct Pending {
    url: Url,
    kind: Kind,
    depth: u8,
}
struct TextFile {
    url: Url,
    path: String,
    kind: Kind,
    text: String,
}

fn key(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    url.to_string()
}
fn filename(url: &Url, extension: &str) -> String {
    format!("{:x}.{extension}", Sha256::digest(key(url).as_bytes()))
}

/// Only a newly-created, unique directory is ever removed on failure. Once a
/// report exists we retain it, including intentionally stopped partial copies.
struct Output {
    path: PathBuf,
    keep: bool,
}
impl Output {
    fn create(parent: &Path, url: &Url) -> Result<Self> {
        std::fs::create_dir_all(parent).context("Cannot create the destination folder")?;
        // Keep ordinary Windows paths usable by Explorer and file associations;
        // canonicalize would add a verbatim \\?\ prefix. Remote URLs never
        // contribute path components, and this fresh child is exclusively ours.
        let parent = std::path::absolute(parent)?;
        let host = crate::model::safe_filename(url.host_str().unwrap_or("website"));
        let path = parent.join(format!(
            "{}-{}-{}",
            host.chars().take(60).collect::<String>(),
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).context("Cannot create the offline copy folder")?;
        Ok(Self { path, keep: false })
    }
    fn write(&self, name: &str, data: &[u8]) -> Result<()> {
        use std::io::Write;
        // All names are generated internally; remote paths never become paths on disk.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path.join(name))
            .context("Cannot save an offline file")?;
        file.write_all(data)?;
        Ok(())
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

struct Crawl {
    config: Config,
    origin: String,
    pending: VecDeque<Pending>,
    scheduled: HashSet<(String, Kind)>,
    omissions: Vec<(Url, String)>,
    page_count: usize,
    asset_count: usize,
    references: usize,
    report: Report,
}
impl Crawl {
    fn omit(&mut self, url: &Url, message: impl Into<String>) {
        self.omissions.push((url.clone(), message.into()));
    }

    fn schedule(&mut self, base: &Url, raw: &str, kind: Kind, depth: u8) {
        if raw.trim().is_empty() || raw.trim().starts_with("data:") {
            return;
        }
        if kind != Kind::Page && raw.trim().starts_with('#') {
            return;
        }
        let Ok(url) = base.join(raw.trim()) else {
            return;
        };
        let Ok(url) = crate::model::validate_url(url.as_str()) else {
            return;
        };
        // A clickable image can be rejected as a page but still be needed as an
        // asset. Deduplicate each use separately; successfully saved URLs remain
        // shared across all uses when the copy is finalized.
        let id = (key(&url), kind);
        if self.scheduled.contains(&id) {
            return;
        }
        if self.references >= MAX_REFERENCES {
            return;
        }
        self.references += 1;
        let same_origin = url.origin().ascii_serialization() == self.origin;
        let reason = if kind == Kind::Page {
            if !same_origin {
                Some("Link leaves the starting website")
            } else if depth > self.config.depth {
                Some("Link is beyond the selected depth")
            } else if !crate::website_grabber::possible_page(&url) {
                Some("Linked download is outside the offline page capture")
            } else if self.page_count >= self.config.pages {
                Some("Page limit reached")
            } else {
                None
            }
        } else if !same_origin && !self.config.external_assets {
            Some("Assets from other websites are disabled")
        } else if self.asset_count >= MAX_ASSETS {
            Some("Asset limit reached")
        } else {
            None
        };
        self.scheduled.insert(id);
        if let Some(reason) = reason {
            self.omit(&url, reason);
            return;
        }
        if kind == Kind::Page {
            self.page_count += 1;
        } else {
            self.asset_count += 1;
        }
        self.pending.push_back(Pending { url, kind, depth });
    }
}

/// Create an independently browsable static copy. Cancellation finalizes already
/// received pages and records missing resources; no download-queue rows are made.
pub async fn save(
    config: Config,
    cancel: CancellationToken,
    mut progress: impl FnMut(Progress),
) -> Result<Report> {
    let start = config.validate()?;
    ensure!(
        !cancel.is_cancelled(),
        "Offline capture cancelled before it started"
    );
    let mut output = Output::create(&config.destination, &start)?;
    let mut crawl = Crawl {
        origin: start.origin().ascii_serialization(),
        report: Report {
            folder: output.path.clone(),
            start_url: start.to_string(),
            pages: 0,
            assets: 0,
            bytes: 0,
            stop: StopReason::Finished,
            issues: Vec::new(),
            issue_count: 0,
            interactive_pages: 0,
            files: Vec::new(),
        },
        config,
        pending: VecDeque::from([Pending {
            url: start.clone(),
            kind: Kind::Page,
            depth: 0,
        }]),
        scheduled: HashSet::from([(key(&start), Kind::Page)]),
        omissions: Vec::new(),
        page_count: 1,
        asset_count: 0,
        references: 1,
    };
    let mut saved = HashMap::<String, String>::new();
    let mut texts = Vec::new();
    let mut text_bytes = 0usize;
    let mut network = network::Network::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    let mut pacing = tokio::time::interval(Duration::from_millis(75));
    pacing.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    while let Some(item) = crawl.pending.pop_front() {
        if saved.contains_key(&key(&item.url)) {
            continue;
        }
        progress(Progress {
            pages: crawl.report.pages,
            assets: crawl.report.assets,
            bytes: crawl.report.bytes,
            current: item.url.to_string(),
        });
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                crawl.report.stop = StopReason::Cancelled;
                crawl.omit(&item.url, "Capture stopped before this resource was saved");
                break;
            },
            _ = tokio::time::sleep_until(deadline) => {
                crawl.report.stop = StopReason::TimeLimit;
                crawl.omit(&item.url, "Capture timed out before this resource was saved");
                break;
            },
            result = async {
                pacing.tick().await;
                tokio::time::timeout(Duration::from_secs(30), network.fetch(&item.url, item.kind, &start,
                    &crawl.config.route, &mut crawl.report.bytes, crawl.config.megabytes * 1024 * 1024)).await.context("Resource timed out")?
            } => result,
        };
        let mut resource = match result {
            Ok(resource) => resource,
            Err(error) => {
                crawl.omit(&item.url, format!("{error:#}"));
                if crawl.report.bytes >= crawl.config.megabytes * 1024 * 1024 {
                    crawl.report.stop = StopReason::SizeLimit;
                    break;
                }
                continue;
            }
        };
        if resource.extension == "svg" {
            match svg::sanitize(&resource.bytes) {
                Ok((bytes, changed)) => {
                    resource.bytes = bytes;
                    if changed {
                        crawl
                            .report
                            .issue(&item.url, "Interactive or external SVG content was omitted");
                    }
                }
                Err(error) => {
                    crawl.omit(&item.url, format!("Cannot save static SVG: {error}"));
                    continue;
                }
            }
        }
        if let Some(path) = saved.get(&key(&resource.url)).cloned() {
            saved.insert(key(&item.url), path);
            continue;
        }
        // A redirect's canonical page may link to itself. Reserve that alias
        // before collecting links so it does not consume another page slot.
        crawl.scheduled.insert((key(&resource.url), item.kind));
        let path = if key(&item.url) == key(&start) {
            "index.html".into()
        } else {
            filename(&resource.url, &resource.extension)
        };
        if item.kind != Kind::Asset {
            let text = match String::from_utf8(resource.bytes) {
                Ok(text) => text,
                Err(_) => {
                    crawl.omit(&item.url, "Only UTF-8 HTML and CSS are supported");
                    continue;
                }
            };
            if text_bytes + text.len() > MAX_TEXT_BYTES {
                crawl.omit(&item.url, "The 20 MB HTML/CSS limit was reached");
                crawl.report.stop = StopReason::SizeLimit;
                break;
            }
            text_bytes += text.len();
            let references = if item.kind == Kind::Page {
                html::rewrite(&text, &resource.url, |base, raw, kind| {
                    crawl.schedule(base, raw, kind, item.depth + u8::from(kind == Kind::Page));
                    raw.to_string()
                })
                .map(|(_, interactive)| interactive)
            } else {
                css::rewrite(&text, |raw, kind| {
                    crawl.schedule(&resource.url, raw, kind, item.depth);
                    raw.to_string()
                })
                .map(|_| false)
            };
            match references {
                Ok(interactive) => {
                    crawl.report.interactive_pages += usize::from(interactive);
                }
                Err(error) => {
                    crawl.omit(&item.url, format!("Cannot rewrite this resource: {error}"));
                    continue;
                }
            }
            let bytes = text.len();
            texts.push(TextFile {
                url: resource.url.clone(),
                path: path.clone(),
                kind: item.kind,
                text,
            });
            crawl.report.files.push(SavedFile {
                url: resource.url.to_string(),
                path: path.clone(),
                bytes,
            });
        } else {
            output.write(&path, &resource.bytes)?;
            crawl.report.files.push(SavedFile {
                url: resource.url.to_string(),
                path: path.clone(),
                bytes: resource.bytes.len(),
            });
        }
        if item.kind == Kind::Page {
            crawl.report.pages += 1;
        } else {
            crawl.report.assets += 1;
        }
        saved.insert(key(&item.url), path.clone());
        saved.insert(key(&resource.url), path);
    }
    if crawl.references >= MAX_REFERENCES {
        crawl
            .report
            .issue(&start, "The link inspection limit was reached");
    }
    while let Some(pending) = crawl.pending.pop_front() {
        if !saved.contains_key(&key(&pending.url)) {
            crawl.omit(&pending.url, "Capture ended before this resource was saved");
        }
    }
    // A URL may have failed in one context and later succeeded in another (for
    // example an extensionless stylesheet linked from both <a> and <link>).
    // Report only URLs that remain unavailable in the finished copy.
    for (url, message) in crawl.omissions {
        if !saved.contains_key(&key(&url)) {
            crawl.report.issue(&url, message);
        }
    }
    ensure!(
        saved.contains_key(&key(&start)),
        "The starting page could not be saved. {}",
        crawl
            .report
            .issues
            .first()
            .map(|i| i.message.as_str())
            .unwrap_or("The operation was stopped before a page was received.")
    );
    let resolve = |base: &Url, raw: &str, kind: Kind| -> String {
        if kind != Kind::Page && raw.trim().is_empty() {
            return "data:,".into();
        }
        if kind != Kind::Page && raw.trim().starts_with('#') {
            return raw.to_string();
        }
        if kind != Kind::Page && raw.trim().starts_with("data:") {
            return raw.to_string();
        }
        let Ok(url) = base.join(raw.trim()) else {
            return missing(kind).into();
        };
        saved
            .get(&key(&url))
            .map(|path| {
                let mut path = path.clone();
                if let Some(fragment) = url.fragment() {
                    path.push('#');
                    path.push_str(fragment);
                }
                path
            })
            .unwrap_or_else(|| missing(kind).into())
    };
    for file in texts {
        let rewritten = if file.kind == Kind::Page {
            html::rewrite(&file.text, &file.url, resolve)?.0
        } else {
            css::rewrite(&file.text, |raw, kind| resolve(&file.url, raw, kind))?
        };
        output.write(&file.path, rewritten.as_bytes())?;
        if let Some(saved_file) = crawl
            .report
            .files
            .iter_mut()
            .find(|saved| saved.path == file.path)
        {
            saved_file.bytes = rewritten.len();
        }
    }
    output.write("report.html", report_html(&crawl.report).as_bytes())?;
    output.write("report.json", &serde_json::to_vec_pretty(&crawl.report)?)?;
    output.keep = true;
    progress(Progress {
        pages: crawl.report.pages,
        assets: crawl.report.assets,
        bytes: crawl.report.bytes,
        current: String::new(),
    });
    Ok(crawl.report)
}
fn missing(kind: Kind) -> &'static str {
    if kind == Kind::Page {
        "report.html"
    } else {
        "data:,"
    }
}
fn report_html(report: &Report) -> String {
    let mut text = format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Offline copy report</title><style>body{{font:16px system-ui;max-width:900px;margin:40px auto;padding:20px;line-height:1.6}}li{{overflow-wrap:anywhere;margin:12px 0}}small{{color:#555}}</style></head><body><h1>{}</h1><p><a href=index.html>Open saved website</a></p><p>{} pages · {} assets · {} reported omissions</p><p>This is a static copy. Scripts, forms, embedded pages and live services are disabled. Links beyond the chosen depth, external pages and missing resources cannot be opened offline. Their links lead here. Original-site links below require an internet connection.</p><p>Start: {}</p><h2>Saved pages and assets</h2><ul>",
        report.stop.label(),
        report.pages,
        report.assets,
        report.issue_count,
        html::escape(&report.start_url)
    );
    for file in &report.files {
        text.push_str(&format!(
            "<li><a href=\"{}\">{}</a></li>",
            html::escape(&file.path),
            html::escape(&file.url)
        ));
    }
    text.push_str("</ul><h2>Omissions and failures</h2><ul>");
    for issue in &report.issues {
        text.push_str(&format!("<li>{}<br><small>{}</small><br><a rel=\"noreferrer noopener\" href=\"{}\">Open original online</a></li>", html::escape(&issue.message), html::escape(&issue.url), html::escape(&issue.url)));
    }
    text.push_str(&format!(
        "</ul><p>Showing {} of {} notes. Raw details are in report.json.</p></body></html>",
        report.issues.len(),
        report.issue_count
    ));
    text
}

//! One JSON file holding a manifest and Lua files. Decoding saves and runs nothing.
use crate::script_automation::{Language, MAX_SCRIPT_BYTES, Script, Trigger, api, site};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const FORMAT: &str = "download-engine-extension";
pub const FORMAT_VERSION: u32 = 1;
pub const FILE_SUFFIX: &str = ".de-extension.json";
pub const MAX_PACKAGE_BYTES: usize = 1024 * 1024;
pub const MAX_CODE_BYTES: usize = 512 * 1024;
/// Windows protection seals at most 512 KiB, and the job's own fields need the rest.
const MAX_FROZEN_BYTES: usize = 496 * 1024;
pub const MAX_FILES: usize = 64;
pub const MAX_JOBS: usize = 16;
pub const MAX_SETTINGS: usize = 32;
pub const MAX_HOSTS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub format: String,
    pub format_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub api_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_api: Option<u32>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub settings: Vec<Setting>,
    pub jobs: Vec<Job>,
    pub files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub network: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub entry: String,
    pub timeout_seconds: u32,
    pub trigger: Trigger,
    /// A request. The person installing the extension decides, as with script imports.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub change_files: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Setting {
    String {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u32>,
    },
    Integer {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Boolean {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    /// Sealed when stored. Only jobs that can reach declared hosts may read it.
    Secret { id: String, label: String },
    /// Sealed when stored. Only a provider processor gets it, as an opaque handle.
    AesKey { id: String, label: String },
}

impl Setting {
    pub fn id(&self) -> &str {
        match self {
            Self::String { id, .. }
            | Self::Integer { id, .. }
            | Self::Boolean { id, .. }
            | Self::Choice { id, .. }
            | Self::Secret { id, .. }
            | Self::AesKey { id, .. } => id,
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Self::String { label, .. }
            | Self::Integer { label, .. }
            | Self::Boolean { label, .. }
            | Self::Choice { label, .. }
            | Self::Secret { label, .. }
            | Self::AesKey { label, .. } => label,
        }
    }
    pub fn protected(&self) -> bool {
        matches!(self, Self::Secret { .. } | Self::AesKey { .. })
    }
    /// The value a job sees before anyone changes this setting.
    pub fn default_value(&self) -> serde_json::Value {
        match self {
            Self::String { default, .. } | Self::Choice { default, .. } => {
                default.clone().map_or(serde_json::Value::Null, Into::into)
            }
            Self::Integer { default, .. } => (*default).map_or(serde_json::Value::Null, Into::into),
            Self::Boolean { default, .. } => (*default).map_or(serde_json::Value::Null, Into::into),
            Self::Secret { .. } | Self::AesKey { .. } => serde_json::Value::Null,
        }
    }
    /// Checks a value someone entered. Protected settings are checked where they are sealed.
    pub fn check(&self, value: &serde_json::Value) -> Result<()> {
        match self {
            Self::String { max_length, .. } => {
                let text = value.as_str().context("Enter text")?;
                check_text(text, max_length.unwrap_or(4096))
            }
            Self::Integer { min, max, .. } => {
                let number = value.as_i64().context("Enter a whole number")?;
                ensure!(
                    min.is_none_or(|min| number >= min) && max.is_none_or(|max| number <= max),
                    "Enter a number in the allowed range"
                );
                Ok(())
            }
            Self::Boolean { .. } => {
                value.as_bool().context("Choose on or off")?;
                Ok(())
            }
            Self::Choice { options, .. } => {
                let text = value.as_str().context("Choose an option")?;
                ensure!(
                    options.iter().any(|option| option == text),
                    "Choose one of the listed options"
                );
                Ok(())
            }
            Self::Secret { .. } | Self::AesKey { .. } => {
                bail!("This setting is stored through its protected field")
            }
        }
    }
    fn validate(&self) -> Result<()> {
        let id = self.id();
        ensure!(
            !id.is_empty()
                && id.len() <= 32
                && id.starts_with(|c: char| c.is_ascii_lowercase())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "Setting id {id:?} must be up to 32 lower-case letters, digits or underscores"
        );
        ensure!(
            !self.label().trim().is_empty() && self.label().chars().count() <= 128,
            "Setting {id} needs a label of up to 128 characters"
        );
        match self {
            Self::String {
                default,
                max_length,
                ..
            } => {
                ensure!(
                    max_length.is_none_or(|max| (1..=4096).contains(&max)),
                    "Setting {id}: max_length must be 1 to 4096"
                );
                if let Some(default) = default {
                    check_text(default, max_length.unwrap_or(4096))
                        .with_context(|| format!("Setting {id}: invalid default"))?;
                }
            }
            Self::Integer {
                default, min, max, ..
            } => {
                ensure!(
                    !matches!((min, max), (Some(min), Some(max)) if min > max),
                    "Setting {id}: min is above max"
                );
                if let Some(default) = default {
                    self.check(&(*default).into())
                        .with_context(|| format!("Setting {id}: invalid default"))?;
                }
            }
            Self::Choice {
                options, default, ..
            } => {
                ensure!(
                    (1..=32).contains(&options.len()),
                    "Setting {id} needs 1 to 32 options"
                );
                let mut seen = BTreeSet::new();
                for option in options {
                    ensure!(
                        !option.trim().is_empty() && option.chars().count() <= 128,
                        "Setting {id} has an empty or overlong option"
                    );
                    ensure!(seen.insert(option), "Setting {id} lists {option:?} twice");
                }
                if let Some(default) = default {
                    ensure!(
                        options.contains(default),
                        "Setting {id}: the default is not one of its options"
                    );
                }
            }
            Self::Boolean { .. } | Self::Secret { .. } | Self::AesKey { .. } => {}
        }
        Ok(())
    }
}

fn check_text(text: &str, max: u32) -> Result<()> {
    ensure!(
        text.chars().count() <= max as usize && !text.contains('\0'),
        "Enter at most {max} characters"
    );
    Ok(())
}

impl Package {
    /// Parses and fully checks a package file. Nothing is saved or executed.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_PACKAGE_BYTES,
            "An extension file can be at most 1 MiB"
        );
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let raw: serde_json::Value =
            serde_json::from_slice(bytes).context("This is not a valid extension file")?;
        let package: Self =
            serde_json::from_value(raw.clone()).context("This is not a valid extension file")?;
        // Serde ignores extra keys on tagged variants that have no fields, so a
        // trigger such as {"kind":"provider","extra":1} needs its own check.
        for (index, job) in package.jobs.iter().enumerate() {
            let keys = |value: &serde_json::Value| {
                value
                    .as_object()
                    .map(|object| object.keys().cloned().collect::<BTreeSet<_>>())
            };
            ensure!(
                keys(&raw["jobs"][index]["trigger"]) == keys(&serde_json::to_value(&job.trigger)?),
                "This is not a valid extension file: job {} has an unknown trigger field",
                job.id
            );
        }
        package.validate()?;
        Ok(package)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.format == FORMAT, "This is not an extension file");
        ensure!(
            self.format_version == FORMAT_VERSION,
            "This extension uses format {}; this app reads format {FORMAT_VERSION}",
            self.format_version
        );
        ensure!(valid_id(&self.id), "Invalid extension id {:?}", self.id);
        crate::queue::queue_name(&self.name).context("Invalid extension name")?;
        version(&self.version)?;
        ensure!(
            !self.publisher.trim().is_empty() && self.publisher.chars().count() <= 64,
            "Publisher must be 1 to 64 characters"
        );
        ensure!(
            !self.description.trim().is_empty() && self.description.chars().count() <= 200,
            "Description must be 1 to 200 characters"
        );
        if let Some(homepage) = &self.homepage {
            let url = reqwest::Url::parse(homepage).context("Invalid homepage")?;
            ensure!(
                url.scheme() == "https",
                "The homepage must be an https:// address"
            );
        }
        ensure!(
            self.license
                .as_ref()
                .is_none_or(|l| l.chars().count() <= 64),
            "License must be at most 64 characters"
        );
        ensure!(
            self.api_version == api::VERSION,
            "This extension needs Lua API {}; this app provides API {}",
            self.api_version,
            api::VERSION
        );
        let has_provider = self.jobs.iter().any(|j| j.trigger == Trigger::Provider);
        ensure!(
            self.provider_api == has_provider.then_some(crate::provider::VERSION),
            "provider_api must match this app, and only a package with a provider job sets it"
        );

        self.check_files()?;
        ensure!(
            self.permissions.network.len() <= MAX_HOSTS,
            "An extension can declare at most {MAX_HOSTS} hosts"
        );
        for pattern in &self.permissions.network {
            ensure!(
                valid_host_pattern(pattern),
                "Invalid host {pattern:?}. Use host or *.host, with no port, path or address"
            );
        }
        ensure!(
            self.settings.len() <= MAX_SETTINGS,
            "An extension can have at most {MAX_SETTINGS} settings"
        );
        let mut settings = BTreeSet::new();
        for setting in &self.settings {
            setting.validate()?;
            ensure!(
                settings.insert(setting.id()),
                "Setting {} is listed twice",
                setting.id()
            );
        }
        self.check_jobs()
    }

    fn check_files(&self) -> Result<()> {
        ensure!(
            (1..=MAX_FILES).contains(&self.files.len()),
            "An extension needs 1 to {MAX_FILES} Lua files"
        );
        let mut total = 0;
        for (path, source) in &self.files {
            ensure!(valid_path(path), "Invalid file path {path:?}");
            ensure!(source.len() <= MAX_SCRIPT_BYTES, "{path} is over 64 KiB");
            ensure!(
                !source.contains('\0') && !source.starts_with("\u{1b}Lua"),
                "{path} must be Lua text"
            );
            total += source.len();
        }
        ensure!(
            total <= MAX_CODE_BYTES,
            "An extension can hold at most 512 KiB of Lua"
        );
        Ok(())
    }

    fn check_jobs(&self) -> Result<()> {
        ensure!(
            (1..=MAX_JOBS).contains(&self.jobs.len()),
            "An extension needs 1 to {MAX_JOBS} jobs"
        );
        let mut ids = BTreeSet::new();
        let mut web = false;
        for job in &self.jobs {
            ensure!(
                !job.id.is_empty()
                    && job.id.len() <= 32
                    && job
                        .id
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "Job id {:?} must be up to 32 lower-case letters, digits or hyphens",
                job.id
            );
            ensure!(ids.insert(&job.id), "Job {} is listed twice", job.id);
            let body = self
                .files
                .get(&job.entry)
                .with_context(|| format!("Job {}: {} is not in this package", job.id, job.entry))?;
            let script = Script {
                id: uuid::Uuid::new_v4().to_string(),
                revision: 0,
                name: job.name.clone(),
                language: Language::Lua,
                body: body.clone(),
                enabled: false,
                trigger: job.trigger.clone(),
                timeout_seconds: job.timeout_seconds,
                change_files: job.change_files,
                extension: None,
            };
            script
                .validate()
                .with_context(|| format!("Job {} is not valid", job.id))?;
            match &job.trigger {
                // Queue IDs belong to one profile, so a package can never name one.
                Trigger::DownloadCompleted { queue } | Trigger::DownloadFailed { queue } => {
                    ensure!(
                        queue.is_none(),
                        "Job {}: a package can't name a queue. Use null for every queue",
                        job.id
                    );
                }
                Trigger::QueueCompleted { queue } => ensure!(
                    queue.is_empty(),
                    "Job {}: leave the queue empty. The install review asks which one to use",
                    job.id
                ),
                Trigger::SiteReader { pages } => {
                    web = true;
                    for page in pages {
                        let host = page.split('/').next().unwrap_or_default();
                        ensure!(
                            page_host_declared(&self.permissions.network, host),
                            "Job {}: {host} is not in permissions.network",
                            job.id
                        );
                    }
                }
                Trigger::Provider => {
                    web = true;
                    // A download keeps a sealed copy of its job with every file in it.
                    let copy = serde_json::to_string(&self.files)?.len()
                        + serde_json::to_string(body)?.len();
                    ensure!(
                        copy <= MAX_FROZEN_BYTES,
                        "Job {}: each download keeps a copy of this package's code, and that copy must fit in {} KiB",
                        job.id,
                        MAX_FROZEN_BYTES / 1024
                    );
                }
                Trigger::Manual | Trigger::Startup | Trigger::Interval { .. } => {}
            }
        }
        ensure!(
            !web || !self.permissions.network.is_empty(),
            "Site reader and provider jobs need permissions.network"
        );
        Ok(())
    }

    pub fn has_secret(&self) -> bool {
        self.settings
            .iter()
            .any(|s| matches!(s, Setting::Secret { .. }))
    }
    pub fn setting(&self, id: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.id() == id)
    }
    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }
    /// The file name `extensions pack` writes for this package.
    pub fn file_name(&self) -> String {
        format!("{}-{}{FILE_SUFFIX}", self.id, self.version)
    }
}

impl Job {
    /// Sees downloads and app commands rather than the web.
    pub fn is_automation(&self) -> bool {
        !matches!(self.trigger, Trigger::SiteReader { .. } | Trigger::Provider)
    }
}

fn valid_id(id: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && !part.starts_with('-')
            && !part.ends_with('-')
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    id.len() <= 64 && id.contains('.') && id.split('.').all(word)
}

/// `MAJOR.MINOR.PATCH`, compared as numbers.
pub fn version(text: &str) -> Result<(u64, u64, u64)> {
    let parse = |part: &str| -> Option<u64> {
        (!part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0')))
        .then(|| part.parse().ok())
        .flatten()
    };
    let parts: Vec<_> = text.split('.').map(parse).collect();
    match parts.as_slice() {
        [Some(major), Some(minor), Some(patch)] => Ok((*major, *minor, *patch)),
        _ => bail!("Version {text:?} must look like 1.2.0"),
    }
}

/// Files live only in memory, so these rules are about clear names rather than
/// the disk. Lower case keeps two paths from colliding on Windows if unpacked.
pub fn valid_path(path: &str) -> bool {
    path.len() <= 128
        && path.strip_suffix(".lua").is_some_and(|stem| {
            stem.split('/').all(|part| {
                !part.is_empty()
                    && part.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                    })
            })
        })
}

fn valid_host(host: &str) -> bool {
    let labels: Vec<_> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        // An all-digit last label is an address, not a name.
        && !labels[labels.len() - 1].bytes().all(|b| b.is_ascii_digit())
}

fn valid_host_pattern(pattern: &str) -> bool {
    valid_host(pattern.strip_prefix("*.").unwrap_or(pattern))
}

/// `*.example.com` covers subdomains but not example.com itself, like page patterns.
pub fn host_allowed(patterns: &[String], host: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| match pattern.strip_prefix("*.") {
            Some(domain) => host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.len() > 1 && prefix.ends_with('.')),
            None => pattern == host,
        })
}

/// The host of a page pattern. A wildcard page reaches every subdomain, so only a
/// wildcard can declare it, never one named subdomain.
fn page_host_declared(patterns: &[String], host: &str) -> bool {
    match host.strip_prefix("*.") {
        Some(domain) => {
            let wildcards: Vec<_> = patterns
                .iter()
                .filter(|pattern| pattern.starts_with("*."))
                .cloned()
                .collect();
            wildcards.iter().any(|pattern| pattern == host) || host_allowed(&wildcards, domain)
        }
        None => host_allowed(patterns, host),
    }
}

/// Whether a URL is on a declared host. Unparseable addresses never are.
pub fn url_allowed(patterns: &[String], url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host_allowed(patterns, host)))
        .unwrap_or(false)
}

/// Checks that two site reader jobs do not fight over one page. Exact for equal
/// patterns, and otherwise tries the later pattern as a page of the earlier one.
pub fn pages_overlap(earlier: &[String], later: &[String]) -> bool {
    later.iter().any(|page| {
        earlier.contains(page)
            || reqwest::Url::parse(&format!(
                "https://{}",
                page.replace("*.", "x.").replace('*', "x")
            ))
            .is_ok_and(|url| earlier.iter().any(|p| site::page_matches(p, &url)))
    })
}

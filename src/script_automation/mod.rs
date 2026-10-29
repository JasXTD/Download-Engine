//! Saved Lua/PowerShell programs, durable run receipts and scheduler-owned execution.
pub mod api;
mod fetch;
pub(crate) mod helpers;
pub(crate) mod json;
pub(crate) mod lua;
pub mod package;
pub use fetch::{FetchReply, FetchRequest, Fetcher};
pub use lua::{execute_lua, execute_reader};
mod reading;
pub use reading::{Reading, ReadingItem};
mod runner;
pub mod site;
mod store;
pub mod syntax;
pub(crate) use runner::Controller;
use serde::{Deserialize, Serialize};
pub(crate) use store::initialize;
pub use store::read_page;

pub const MAX_SCRIPT_BYTES: usize = 65_536;
pub const MAX_LOG_BYTES: usize = 131_072;
pub const MAX_ACTIVE: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Lua,
    // Definitions and run receipts saved before the language picker were PowerShell.
    #[default]
    PowerShell,
}
impl Language {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lua => "Lua",
            Self::PowerShell => "PowerShell",
        }
    }
    pub fn example(self) -> &'static str {
        match self {
            Self::Lua => LUA_EXAMPLE,
            Self::PowerShell => EXAMPLE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    Manual,
    /// Explicit downloads with frozen Lua callbacks; never an automatic trigger.
    Provider,
    Startup,
    Interval {
        minutes: u32,
    },
    DownloadCompleted {
        queue: Option<String>,
    },
    /// Downloads that reached Failed after their automatic retries.
    DownloadFailed {
        queue: Option<String>,
    },
    QueueCompleted {
        queue: String,
    },
    /// Lua only: opens matching video pages instead of a generic scan.
    SiteReader {
        pages: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    pub id: String,
    #[serde(default)]
    pub revision: u64,
    pub name: String,
    #[serde(default)]
    pub language: Language,
    pub body: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub timeout_seconds: u32,
    /// Lets the script rename, move and remove its downloads' files.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub change_files: bool,
}
impl Script {
    /// Shared structural validation for saved definitions and imported packages.
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        crate::model::validate_job_id(&self.id)?;
        crate::queue::queue_name(&self.name)?;
        ensure!(!self.body.trim().is_empty(), "Enter a script");
        ensure!(
            self.body.len() <= MAX_SCRIPT_BYTES && !self.body.contains('\0'),
            "Script must be at most 64 KiB and contain no NUL characters"
        );
        ensure!(
            (1..=3600).contains(&self.timeout_seconds),
            "Timeout must be 1 to 3600 seconds"
        );
        if let Trigger::Interval { minutes } = self.trigger {
            ensure!(
                (1..=10080).contains(&minutes),
                "Interval must be 1 to 10080 minutes"
            );
        }
        if let Trigger::SiteReader { pages } = &self.trigger {
            ensure!(
                self.language == Language::Lua,
                "Site readers must be written in Lua"
            );
            ensure!(
                !self.change_files,
                "Site readers cannot change downloaded files"
            );
            site::validate_pages(pages)?;
        }
        if self.trigger == Trigger::Provider {
            ensure!(
                self.language == Language::Lua && !self.change_files,
                "Download providers require Lua and cannot change existing downloaded files"
            );
        }
        Ok(())
    }
    /// Event scripts can be tried on downloads or a queue the person picks.
    pub fn takes_sample(&self) -> bool {
        matches!(
            self.trigger,
            Trigger::DownloadCompleted { .. }
                | Trigger::DownloadFailed { .. }
                | Trigger::QueueCompleted { .. }
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub script_id: String,
    pub name: String,
    pub revision: u64,
    #[serde(default)]
    pub language: Language,
    pub trigger: String,
    pub context: serde_json::Value,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub state: String,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
}
impl Run {
    pub fn active(&self) -> bool {
        matches!(self.state.as_str(), "queued" | "running")
    }
}

/// A download a sample run can be given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleCandidate {
    pub id: String,
    pub name: String,
    pub queue: String,
    pub error: Option<String>,
    pub attempts: u32,
    pub created: u64,
}

/// One value a script keeps between runs with xd.store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredValue {
    pub key: String,
    pub value: serde_json::Value,
    pub updated: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Log {
    pub at: i64,
    pub level: String,
    pub text: String,
}

pub(crate) struct Claim {
    pub run: Run,
    pub body: String,
    pub timeout: u32,
}

pub const EXAMPLE: &str = "# Invoke-DownloadEngine returns JSON as PowerShell objects.\n$queues = Invoke-DownloadEngine queues\nforeach ($queue in $queues) {\n    Write-Output (\"Queue: {0} | running: {1}\" -f $queue.name, $queue.running)\n}\nWrite-Output (\"Trigger: {0}\" -f $Event.trigger)\n";

pub const READER_EXAMPLE: &str = "-- A site reader: event.url is the page that was opened.\n-- Read it, or the site's own API, with xd.fetch and return the videos you find.\nlocal page = xd.fetch(event.url)\nlocal stream = page.body:match('\"(https://[^\"]-%.m3u8[^\"]*)\"')\nif not stream then\n    error(\"No video stream found on this page\")\nend\nreturn {\n    title = page.body:match(\"<title>(.-)</title>\"),\n    items = {\n        { kind = \"hls\", url = stream, label = \"All qualities\" },\n    },\n}\n";

pub const LUA_EXAMPLE: &str = "-- Write your own Lua: variables, loops and functions.\nfor _, queue in ipairs(xd.queues()) do\n    print(queue.name, \"running:\", queue.running)\nend\nprint(\"Trigger:\", event.trigger)\nfor _, download in ipairs(event.downloads) do\n    print(\"Finished:\", download.name)\nend\n";

pub const FAILED_EXAMPLE: &str = "-- Runs when downloads still fail after Download Engine's own retries.\nfor _, file in ipairs(event.downloads) do\n    local reason = file.error or \"\"\n    print((\"Failed: %s (%s)\"):format(file.name or file.id, reason))\n    if reason:find(\"timed out\") then\n        xd.retry(file.id)\n    end\nend\n";

/// Limits for what a script keeps between runs with xd.store.
pub const MAX_STATE_KEYS: usize = 200;
pub const MAX_STATE_KEY_CHARS: usize = 128;
pub const MAX_STATE_VALUE_BYTES: usize = 16 * 1024;
pub const MAX_STATE_BYTES: usize = 256 * 1024;
/// Script requests to retry or restart one download, until it completes.
pub const MAX_SCRIPT_RETRIES: i64 = 5;
/// Notifications one run can show.
pub const MAX_NOTICES: i64 = 5;

#[cfg(test)]
mod tests;

//! Shared local command inbox. Only the desktop scheduler executes mutations.
//! Receipts make a lost CLI reply distinguishable from a rejected operation.
use crate::{automation::Rule, model::Job, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Add {
        urls: Vec<String>,
        queue: String,
        folder: Option<PathBuf>,
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        script_run: Option<String>,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    PauseAll,
    CreateQueue {
        name: String,
    },
    StartQueue {
        queue: String,
    },
    PauseQueue {
        queue: String,
    },
    QueueLimit {
        queue: String,
        limit: usize,
    },
    SaveRule {
        rule: Rule,
    },
    RemoveRule {
        id: String,
    },
    SaveScript {
        script: crate::script_automation::Script,
    },
    RemoveScript {
        id: String,
    },
    RunScript {
        id: String,
    },
    /// The approval is rechecked while the exact revision is snapshotted.
    RunMcpScript {
        id: String,
        revision: u64,
    },
    /// Run an event script now with downloads you choose; changes are simulated.
    RunScriptSample {
        id: String,
        downloads: Vec<String>,
    },
    StopScript {
        run_id: String,
    },
    /// Resume a download that needs attention. `script_run` is set when a
    /// script asks; script retries and restarts are limited per download.
    Retry {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        script_run: Option<String>,
    },
    /// Start a download over, discarding what was downloaded.
    Restart {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        script_run: Option<String>,
    },
    /// Rename a finished download's file in its folder.
    Rename {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        script_run: Option<String>,
    },
    /// Move a finished download's file into a download or category folder.
    MoveFile {
        id: String,
        folder: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        script_run: Option<String>,
    },
    /// Remove a download from the list, and optionally its finished file.
    Remove {
        id: String,
        delete_file: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        script_run: Option<String>,
    },
    /// Show a Windows notification from a running script.
    Notify {
        title: String,
        text: String,
        script_run: String,
    },
    ClearScriptValues {
        id: String,
    },
}

/// Longest notification title and text, Windows' own limits for these balloons.
pub const NOTICE_TITLE_CHARS: usize = 64;
pub const NOTICE_TEXT_CHARS: usize = 256;

/// Trim `text` and keep at most `limit` characters.
pub fn clip(text: &str, limit: usize) -> String {
    text.trim()
        .chars()
        .take(limit)
        .collect::<String>()
        .trim_end()
        .into()
}

impl Action {
    /// The run a script command came from, if a script sent it.
    pub fn script_run(&self) -> Option<&str> {
        match self {
            Self::Add { script_run, .. }
            | Self::Retry { script_run, .. }
            | Self::Restart { script_run, .. }
            | Self::Rename { script_run, .. }
            | Self::MoveFile { script_run, .. }
            | Self::Remove { script_run, .. } => script_run.as_deref(),
            Self::Notify { script_run, .. } => Some(script_run),
            _ => None,
        }
    }
    /// Rename, move and remove change a script's downloads.
    pub fn changes_files(&self) -> bool {
        matches!(
            self,
            Self::Rename { .. } | Self::MoveFile { .. } | Self::Remove { .. }
        )
    }
}

/// A script may rename, move or remove downloads only with its
/// "Can change downloaded files" switch on. People using the CLI always may.
pub fn check_script_files(store: &Store, action: &Action) -> Result<()> {
    if !action.changes_files() {
        return Ok(());
    }
    let Some(run) = action.script_run() else {
        return Ok(());
    };
    let (script, _) = store.script_for_run(run)?;
    ensure!(
        script.change_files,
        "This script can't change downloaded files. Turn on “Can change downloaded files” in its settings."
    );
    Ok(())
}

/// What a sample run reports instead of changing something.
pub fn simulated(store: &Store, action: &Action) -> String {
    let name = |id: &str| {
        store
            .get(id)
            .ok()
            .flatten()
            .map_or_else(|| id.to_string(), |job| job.name)
    };
    let what = match action {
        Action::Add { urls, queue, .. } if urls.len() == 1 => {
            format!("add {} to {queue}", urls[0])
        }
        Action::Add { urls, queue, .. } => format!("add {} downloads to {queue}", urls.len()),
        Action::Pause { id } => format!("pause {}", name(id)),
        Action::Resume { id } => format!("resume {}", name(id)),
        Action::Retry { id, .. } => format!("retry {}", name(id)),
        Action::Restart { id, .. } => format!("restart {}", name(id)),
        Action::Rename { id, name: to, .. } => format!("rename {} to {to}", name(id)),
        Action::MoveFile { id, folder, .. } => {
            format!("move {} to {}", name(id), folder.display())
        }
        Action::Remove {
            id,
            delete_file: true,
            ..
        } => format!("remove {} and delete its file", name(id)),
        Action::Remove { id, .. } => format!("remove {} from the list", name(id)),
        Action::Notify { title, text, .. } => format!("show the notification {title} · {text}"),
        Action::PauseAll => "pause all downloads".into(),
        Action::CreateQueue { name } => format!("create the queue {name}"),
        Action::StartQueue { queue } => format!("start the queue {queue}"),
        Action::PauseQueue { queue } => format!("pause the queue {queue}"),
        Action::QueueLimit { queue, limit } => format!("let {queue} run {limit} at a time"),
        Action::SaveRule { rule } => format!("save the rule {}", rule.name),
        Action::RemoveRule { id } => format!("remove the rule {id}"),
        Action::SaveScript { script } => format!("save the script {}", script.name),
        Action::RemoveScript { id } => format!("remove the script {id}"),
        Action::RunScript { id } | Action::RunMcpScript { id, .. } => {
            format!("run the script {id}")
        }
        Action::RunScriptSample { id, .. } => format!("run the script {id} with a sample"),
        Action::StopScript { run_id } => format!("stop the run {run_id}"),
        Action::ClearScriptValues { id } => format!("clear the stored values of {id}"),
    };
    format!("Would {what} (sample run, nothing changed)")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub request_id: String,
    pub state: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}

pub(crate) fn initialize(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS control_requests(
        id TEXT PRIMARY KEY, action TEXT NOT NULL, expires INTEGER NOT NULL,
        state TEXT NOT NULL, result TEXT, error TEXT);
        CREATE INDEX IF NOT EXISTS control_pending ON control_requests(state);",
    )?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS control_request_ids(id TEXT PRIMARY KEY, action_hash BLOB NOT NULL CHECK(length(action_hash)=32)) WITHOUT ROWID;")?;
    // Seed every receipt still present at upgrade. Earlier, already-pruned IDs
    // cannot be recovered. Never discard a recorded ID merely to prune results.
    let existing = conn.prepare("SELECT id,action FROM control_requests WHERE id NOT IN (SELECT id FROM control_request_ids)")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, action) in existing {
        conn.execute(
            "INSERT INTO control_request_ids VALUES(?1,?2)",
            params![id, Sha256::digest(action.as_bytes()).as_slice()],
        )?;
    }
    Ok(())
}

impl Store {
    pub fn submit_control(&self, id: &str, action: &Action) -> Result<Receipt> {
        crate::model::validate_job_id(id)?;
        let data = serde_json::to_string(action)?;
        ensure!(data.len() <= 1_000_000, "Command is too large");
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = Sha256::digest(data.as_bytes());
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT action_hash FROM control_request_ids WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            ensure!(
                existing == hash.as_slice(),
                "Request ID already belongs to another command"
            );
        } else {
            let pending: i64 = tx.query_row(
                "SELECT COUNT(*) FROM control_requests WHERE state IN ('pending','running')",
                [],
                |r| r.get(0),
            )?;
            ensure!(
                pending < 100,
                "Too many pending commands; wait for the app to catch up"
            );
            tx.execute(
                "INSERT INTO control_request_ids VALUES(?1,?2)",
                params![id, hash.as_slice()],
            )?;
            tx.execute(
                "INSERT INTO control_requests(id,action,expires,state) VALUES(?1,?2,?3,'pending')",
                params![id, data, chrono::Utc::now().timestamp() + 30],
            )?;
            tx.execute("DELETE FROM control_requests WHERE state NOT IN ('pending','running') AND rowid NOT IN (SELECT rowid FROM control_requests ORDER BY rowid DESC LIMIT 1000)", [])?;
        }
        tx.commit()?;
        self.control_receipt(id)
    }

    pub fn control_receipt(&self, id: &str) -> Result<Receipt> {
        let (state, result, error): (String, Option<String>, Option<String>) = self
            .connect()?
            .query_row(
                "SELECT state,result,error FROM control_requests WHERE id=?1
                 UNION ALL SELECT 'retired',NULL,'This request was already accepted; its detailed receipt has expired. Inspect current state before using a new request ID.'
                 FROM control_request_ids WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM control_requests WHERE id=?1)",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .context("Request ID was not recorded by this profile")?;
        Ok(Receipt {
            request_id: id.into(),
            state,
            result: result.map(|s| serde_json::from_str(&s)).transpose()?,
            error,
        })
    }

    pub fn cancel_control(&self, id: &str) -> Result<Receipt> {
        self.connect()?.execute("UPDATE control_requests SET state='cancelled',error='Cancelled before execution' WHERE id=?1 AND state='pending'", [id])?;
        self.control_receipt(id)
    }

    pub(crate) fn recover_controls(&self) -> Result<()> {
        // Never blindly replay a command which may already have changed state.
        self.connect()?.execute("UPDATE control_requests SET state='interrupted',error='App stopped during this command. Inspect current state before submitting a new request.' WHERE state='running'", [])?;
        Ok(())
    }

    pub(crate) fn claim_control(&self) -> Result<Option<(String, Action)>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE control_requests SET state='expired',error='Command expired before execution' WHERE state='pending' AND expires<=?1", [chrono::Utc::now().timestamp()])?;
        let row: Option<(String,String)> = tx.query_row("SELECT id,action FROM control_requests WHERE state='pending' ORDER BY rowid LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((id, data)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        match serde_json::from_str(&data) {
            Ok(action) => {
                tx.execute(
                    "UPDATE control_requests SET state='running' WHERE id=?1",
                    [&id],
                )?;
                tx.commit()?;
                Ok(Some((id, action)))
            }
            Err(_) => {
                tx.execute("UPDATE control_requests SET state='failed',error='Unsupported saved command' WHERE id=?1", [&id])?;
                tx.commit()?;
                Ok(None)
            }
        }
    }

    pub(crate) fn finish_control(&self, id: &str, result: Result<Value>) -> Result<()> {
        let (state, value, error) = match result {
            Ok(value) => ("done", Some(serde_json::to_string(&value)?), None),
            Err(error) => ("failed", None, Some(format!("{error:#}"))),
        };
        self.connect()?.execute("UPDATE control_requests SET state=?1,result=?2,error=?3 WHERE id=?4 AND state='running'", params![state,value,error,id])?;
        Ok(())
    }
}

pub fn queue_id(store: &Store, value: &str) -> Result<String> {
    store
        .queues()?
        .into_iter()
        .find(|q| q.id == value || q.name.eq_ignore_ascii_case(value))
        .map(|q| q.id)
        .context("Queue not found; use queues to see IDs and names")
}

pub fn job_summary(store: &Store, job: &Job) -> Result<Value> {
    Ok(job_summary_in_queue(job, store.job_queue(&job.id)?))
}

pub(crate) fn job_summary_in_queue(job: &Job, queue: Option<String>) -> Value {
    json!({"id":job.id,"name":job.name,"status":job.status,"downloaded":job.downloaded,"total":job.total,"queue":queue,"directory":job.directory,"error":job.error})
}

pub fn queue_summaries(store: &Store) -> Result<Value> {
    Ok(Value::Array(store.queues()?.iter().map(|q| json!({"id":q.id,"name":q.name,"running":q.running,"max_active":q.max_active,"scheduled_at":q.scheduled_at,"has_schedule":q.has_schedule()})).collect()))
}

/// Commands without worker ownership can use the same validated Store methods.
pub(crate) fn execute_stored(store: &Store, request: &str, action: Action) -> Result<Value> {
    match action {
        Action::Add {
            urls,
            queue,
            folder,
            name,
            script_run,
        } => {
            ensure!(
                !urls.is_empty() && urls.len() <= 1000,
                "Add between 1 and 1,000 URLs"
            );
            ensure!(
                urls.len() == 1 || name.is_none(),
                "A file name applies only to one URL"
            );
            let queue = queue_id(store, &queue)?;
            let jobs = urls
                .iter()
                .enumerate()
                .map(|(i, url)| {
                    store.new_download(
                        url,
                        name.as_deref(),
                        folder.as_deref(),
                        format!("cli:{request}:{i}"),
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(run) = &script_run {
                let (_, sample) = store.script_for_run(run)?;
                ensure!(!sample, "Sample script commands cannot add real downloads");
                // Check the actual destinations, including automatic category
                // rules, before any member of this batch is queued. Permission
                // to change files never widens the allowed download roots.
                for job in &jobs {
                    store.guard_script_folder(&job.directory)?;
                }
            }
            let added = store.enqueue_batch(&jobs, &queue)?;
            Ok(Value::Array(
                added
                    .iter()
                    .map(|j| job_summary(store, j))
                    .collect::<Result<_>>()?,
            ))
        }
        Action::CreateQueue { name } => {
            let q = store.create_queue(&name)?;
            Ok(json!({"id":q.id,"name":q.name,"running":q.running}))
        }
        Action::SaveRule { rule } => {
            store.save_automation_rule(&rule)?;
            Ok(serde_json::to_value(rule)?)
        }
        Action::RemoveRule { id } => {
            store.remove_automation_rule(&id)?;
            Ok(json!({"removed":id}))
        }
        Action::SaveScript { script } => Ok(serde_json::to_value(store.save_script(&script)?)?),
        Action::RemoveScript { id } => {
            store.remove_script(&id)?;
            Ok(json!({"removed":id}))
        }
        Action::RunScript { id } => Ok(serde_json::to_value(store.run_script(&id, request)?)?),
        Action::RunMcpScript { id, revision } => Ok(serde_json::to_value(
            store.run_mcp_script(&id, revision, request)?,
        )?),
        Action::RunScriptSample { id, downloads } => Ok(serde_json::to_value(
            store.run_script_sample(&id, request, &downloads)?,
        )?),
        Action::StopScript { run_id } => {
            store.stop_script(&run_id)?;
            Ok(serde_json::to_value(store.script_run(&run_id)?)?)
        }
        Action::ClearScriptValues { id } => {
            store.script(&id)?;
            Ok(json!({"cleared":store.clear_script_values(&id)?}))
        }
        _ => anyhow::bail!("This command requires the scheduler"),
    }
}

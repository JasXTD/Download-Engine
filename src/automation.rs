//! Local, declarative rules. Routing and its receipt commit with the new job;
//! queue completion actions and their cursor commit together, including on restart.
use crate::{model::Job, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "when", rename_all = "snake_case", deny_unknown_fields)]
pub enum Workflow {
    DownloadAdded {
        /// Exact hostname; empty matches every site. No URL/query credentials.
        host: String,
        /// A single extension without a dot; empty matches every file type.
        extension: String,
        queue: String,
        bytes_per_second: Option<u64>,
    },
    QueueCompleted {
        source: String,
        start: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub workflow: Workflow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: i64,
    pub at: i64,
    pub rule_name: String,
    pub success: bool,
    pub message: String,
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS automation_rules(
            id TEXT PRIMARY KEY, data TEXT NOT NULL, generation INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS automation_runs(
            id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL,
            rule_name TEXT NOT NULL, success INTEGER NOT NULL, message TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS automation_queue_progress(
            queue_id TEXT PRIMARY KEY REFERENCES download_queues(id) ON DELETE CASCADE,
            generation INTEGER NOT NULL);
         CREATE TRIGGER IF NOT EXISTS automation_job_completed AFTER UPDATE OF data ON jobs
         WHEN json_extract(NEW.data,'$.status')='complete'
          AND json_extract(OLD.data,'$.status')<>'complete'
         BEGIN
            INSERT INTO automation_queue_progress(queue_id,generation)
            SELECT queue_id,1 FROM job_queues WHERE job_id=NEW.id
            ON CONFLICT(queue_id) DO UPDATE SET generation=generation+1;
         END;",
    )?;
    Ok(())
}

fn generation(conn: &Connection, queue: &str) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT generation FROM automation_queue_progress WHERE queue_id=?1",
            [queue],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

fn exists(conn: &Connection, queue: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_queues WHERE id=?1)",
        [queue],
        |r| r.get(0),
    )?)
}

fn read_rules(conn: &Connection) -> Result<Vec<Rule>> {
    let mut stmt = conn.prepare("SELECT data FROM automation_rules ORDER BY rowid")?;
    let values = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    values
        .into_iter()
        .map(|value| Ok(serde_json::from_str(&value)?))
        .collect()
}

fn record(conn: &Connection, rule: &Rule, success: bool, message: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO automation_runs(at,rule_name,success,message) VALUES(?1,?2,?3,?4)",
        params![chrono::Utc::now().timestamp(), rule.name, success, message],
    )?;
    conn.execute("DELETE FROM automation_runs WHERE id NOT IN (SELECT id FROM automation_runs ORDER BY id DESC LIMIT 500)", [])?;
    Ok(())
}

impl Store {
    pub fn automation_rules(&self) -> Result<Vec<Rule>> {
        read_rules(&self.connect()?)
    }

    pub fn save_automation_rule(&self, rule: &Rule) -> Result<()> {
        crate::model::validate_job_id(&rule.id)?;
        crate::queue::queue_name(&rule.name)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM automation_rules WHERE id<>?1",
            [&rule.id],
            |r| r.get(0),
        )?;
        ensure!(count < 100, "Keep at most 100 automation rules");
        let cursor = match &rule.workflow {
            Workflow::DownloadAdded {
                host,
                extension,
                queue,
                bytes_per_second,
            } => {
                if !host.is_empty() {
                    let url = reqwest::Url::parse(&format!("https://{host}/"))
                        .context("Enter a hostname such as example.com")?;
                    ensure!(
                        url.host_str() == Some(host.as_str())
                            && url.path() == "/"
                            && url.query().is_none()
                            && url.fragment().is_none()
                            && url.port().is_none()
                            && url.username().is_empty()
                            && url.password().is_none(),
                        "Use a lowercase hostname without a scheme, path or port"
                    );
                }
                ensure!(
                    extension.len() <= 20
                        && extension
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                    "Use a lowercase file extension without a dot"
                );
                crate::bandwidth::validate(*bytes_per_second)?;
                ensure!(
                    !rule.enabled || exists(&tx, queue)?,
                    "Destination queue no longer exists"
                );
                0
            }
            Workflow::QueueCompleted { source, start } => {
                ensure!(source != start, "Choose a different queue to start");
                ensure!(
                    !rule.enabled || (exists(&tx, source)? && exists(&tx, start)?),
                    "Queue no longer exists"
                );
                if rule.enabled {
                    // A loop could repeatedly restart synchronization queues.
                    let rules = read_rules(&tx)?;
                    let mut pending = vec![start.clone()];
                    let mut visited = std::collections::HashSet::new();
                    while let Some(queue) = pending.pop() {
                        ensure!(
                            &queue != source,
                            "These rules form a queue loop. Choose a different destination."
                        );
                        if !visited.insert(queue.clone()) {
                            continue;
                        }
                        for other in rules.iter().filter(|r| r.enabled && r.id != rule.id) {
                            if let Workflow::QueueCompleted { source, start } = &other.workflow
                                && source == &queue
                            {
                                pending.push(start.clone());
                            }
                        }
                    }
                }
                generation(&tx, source)?
            }
        };
        tx.execute(
            "INSERT INTO automation_rules(id,data,generation) VALUES(?1,?2,?3)
            ON CONFLICT(id) DO UPDATE SET data=excluded.data,generation=excluded.generation",
            params![rule.id, serde_json::to_string(rule)?, cursor],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_automation_rule(&self, id: &str) -> Result<()> {
        ensure!(
            self.connect()?
                .execute("DELETE FROM automation_rules WHERE id=?1", [id])?
                == 1,
            "Rule no longer exists"
        );
        Ok(())
    }

    pub fn automation_history(&self) -> Result<Vec<Run>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT id,at,rule_name,success,message FROM automation_runs ORDER BY id DESC LIMIT 500")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(Run {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    rule_name: r.get(2)?,
                    success: r.get(3)?,
                    message: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn run_queue_automations(&self) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for rule in read_rules(&tx)?.into_iter().filter(|r| r.enabled) {
            let Workflow::QueueCompleted { source, start } = &rule.workflow else {
                continue;
            };
            let cursor: i64 = tx.query_row(
                "SELECT generation FROM automation_rules WHERE id=?1",
                [&rule.id],
                |r| r.get(0),
            )?;
            let current = generation(&tx, source)?;
            if current <= cursor {
                continue;
            }
            let (total, unfinished): (i64, i64) = tx.query_row("SELECT COUNT(*),COALESCE(SUM(json_extract(j.data,'$.status')<>'complete'),0) FROM jobs j JOIN job_queues q ON q.job_id=j.id WHERE q.queue_id=?1", [source], |r| Ok((r.get(0)?,r.get(1)?)))?;
            if total == 0 || unfinished != 0 {
                continue;
            }
            if exists(&tx, start)? {
                crate::queue_store::set_running_in(&tx, start, true)?;
                record(
                    &tx,
                    &rule,
                    true,
                    "Queue completed. Started the destination queue.",
                )?;
            } else {
                record(
                    &tx,
                    &rule,
                    false,
                    "Destination queue was removed. Edit this rule.",
                )?;
            }
            tx.execute(
                "UPDATE automation_rules SET generation=?1 WHERE id=?2",
                params![current, rule.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Called only for a freshly inserted job, inside the insertion transaction.
/// First matching enabled rule wins, making overlaps predictable in the UI.
pub(crate) fn on_added(conn: &Connection, job: &Job) -> Result<()> {
    for rule in read_rules(conn)?.into_iter().filter(|r| r.enabled) {
        let Workflow::DownloadAdded {
            host,
            extension,
            queue,
            bytes_per_second,
        } = &rule.workflow
        else {
            continue;
        };
        let url = reqwest::Url::parse(&job.url)?;
        if !host.is_empty() && url.host_str() != Some(host.as_str()) {
            continue;
        }
        let suffix = std::path::Path::new(&job.name)
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        if !extension.is_empty() && !extension.eq_ignore_ascii_case(suffix) {
            continue;
        }
        if !exists(conn, queue)? {
            record(
                conn,
                &rule,
                false,
                "Destination queue was removed. Download kept in its original queue.",
            )?;
            break;
        }
        conn.execute("UPDATE job_queues SET queue_id=?1,position=COALESCE((SELECT MAX(position)+1 FROM job_queues WHERE queue_id=?1),0) WHERE job_id=?2", params![queue,job.id])?;
        if let Some(rate) = bytes_per_second {
            conn.execute(
                "INSERT INTO bandwidth_downloads(job_id,bytes_per_second) VALUES(?1,?2)",
                params![job.id, i64::try_from(*rate)?],
            )?;
        }
        record(
            conn,
            &rule,
            true,
            &format!(
                "Routed download {}{}.",
                job.id,
                if bytes_per_second.is_some() {
                    " and applied its speed limit"
                } else {
                    ""
                }
            ),
        )?;
        break;
    }
    Ok(())
}

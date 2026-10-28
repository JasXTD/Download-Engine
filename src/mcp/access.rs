//! Local permission settings and explicit approval of immutable script revisions.
use crate::{
    control::Action,
    script_automation::{Language, Script},
    store::Store,
};
use anyhow::{Context as _, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Permissions {
    pub downloads: bool,
    pub queues: bool,
    pub lua: bool,
    pub powershell: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            downloads: true,
            queues: true,
            lua: false,
            powershell: false,
        }
    }
}

impl Permissions {
    pub(super) fn allows_tool(self, name: &str) -> bool {
        match name {
            "add_downloads" | "pause_download" | "resume_download" | "pause_all" => self.downloads,
            "create_queue" | "start_queue" | "pause_queue" | "set_queue_limit" => self.queues,
            "run_script" | "stop_script" => self.lua || self.powershell,
            "cancel_request" => self.downloads || self.queues || self.lua || self.powershell,
            _ => true,
        }
    }

    pub(super) fn check_language(self, language: Language) -> Result<()> {
        ensure!(
            match language {
                Language::Lua => self.lua,
                Language::PowerShell => self.powershell,
            },
            "MCP execution of this script language is disabled"
        );
        Ok(())
    }

    pub(super) fn check_action(self, store: &Store, action: &Action) -> Result<()> {
        let allowed = match action {
            Action::Add { .. }
            | Action::Pause { .. }
            | Action::Resume { .. }
            | Action::PauseAll => self.downloads,
            Action::CreateQueue { .. }
            | Action::StartQueue { .. }
            | Action::PauseQueue { .. }
            | Action::QueueLimit { .. } => self.queues,
            Action::RunScript { id } | Action::RunMcpScript { id, .. } => {
                self.check_language(store.script(id)?.language)?;
                true
            }
            Action::StopScript { run_id } => {
                self.check_language(store.script_run(run_id)?.language)?;
                true
            }
            _ => false,
        };
        ensure!(
            allowed,
            "This command is outside the MCP server's permissions"
        );
        Ok(())
    }
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_permissions(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL);
         INSERT OR IGNORE INTO mcp_permissions VALUES(1,'{}');
         CREATE TABLE IF NOT EXISTS mcp_script_approvals(
         script_id TEXT PRIMARY KEY REFERENCES automation_scripts(id) ON DELETE CASCADE,
         revision TEXT NOT NULL, fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32));",
    )?;
    Ok(())
}

fn fingerprint(script: &Script) -> Result<Vec<u8>> {
    Ok(Sha256::digest(serde_json::to_vec(script)?).to_vec())
}

pub(crate) fn check_approval(conn: &Connection, script: &Script, revision: u64) -> Result<()> {
    ensure!(
        script.revision == revision,
        "The script changed after MCP authorization; review and approve its current revision"
    );
    let saved: Option<(String, Vec<u8>)> = conn
        .query_row(
            "SELECT revision,fingerprint FROM mcp_script_approvals WHERE script_id=?1",
            [&script.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    ensure!(
        saved == Some((revision.to_string(), fingerprint(script)?)),
        "This script revision is not approved for MCP execution; approve it locally after reviewing its source and permissions"
    );
    Ok(())
}

impl Store {
    /// Local-only approval: no MCP tool or Lua command can grant this permission.
    pub fn approve_mcp_script(&self, id: &str, revision: u64) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let data: String = tx
            .query_row(
                "SELECT data FROM automation_scripts WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .context("Script no longer exists")?;
        let script: Script = serde_json::from_str(&data)?;
        ensure!(
            script.revision == revision,
            "The script changed; review its current revision before approving it"
        );
        tx.execute("INSERT INTO mcp_script_approvals VALUES(?1,?2,?3) ON CONFLICT(script_id) DO UPDATE SET revision=excluded.revision,fingerprint=excluded.fingerprint", params![id,revision.to_string(),fingerprint(&script)?])?;
        tx.commit()?;
        Ok(())
    }

    pub fn revoke_mcp_script(&self, id: &str) -> Result<()> {
        self.connect()?
            .execute("DELETE FROM mcp_script_approvals WHERE script_id=?1", [id])?;
        Ok(())
    }

    pub(super) fn approved_mcp_script(&self, id: &str) -> Result<Script> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let data: String = tx
            .query_row(
                "SELECT data FROM automation_scripts WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .context("Script no longer exists")?;
        let script: Script = serde_json::from_str(&data)?;
        check_approval(&tx, &script, script.revision)?;
        Ok(script)
    }

    pub(super) fn cancel_mcp_control(
        &self,
        id: &str,
        permissions: Permissions,
    ) -> Result<crate::control::Receipt> {
        let data: Option<String> = self
            .connect()?
            .query_row(
                "SELECT action FROM control_requests WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(data) = data {
            permissions.check_action(self, &serde_json::from_str(&data)?)?;
            return self.cancel_control(id);
        }
        self.control_receipt(id)
    }
}

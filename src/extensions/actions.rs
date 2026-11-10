//! Account actions use the existing isolated reader worker. Only a live,
//! explicitly invoked job may replace its declared, protected text settings.
use super::{Context, Setting};
use crate::{
    script_automation::{Run, definition},
    store::Store,
};
use anyhow::{Context as _, Result, ensure};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl Store {
    /// Compare-and-set prevents a late authorization/refresh from restoring a
    /// credential the user cleared or replaced while the request was in flight.
    pub fn save_action_secret(
        &self,
        run: &str,
        context: &Context,
        key: &str,
        expected: Option<&str>,
        value: Option<&str>,
    ) -> Result<()> {
        ensure!(
            context.secret_writes.iter().any(|k| k == key),
            "This action cannot save that setting"
        );
        ensure!(
            value.is_none_or(|v| !v.is_empty() && v.len() <= 4096),
            "Protected text must contain 1 to 4096 bytes"
        );
        let scope = context
            .key_scope
            .context("Missing protected settings scope")?;
        let sealed = value
            .map(|value| {
                crate::protected_data::seal(
                    scope,
                    &mut zeroize::Zeroizing::new(value.as_bytes().to_vec()),
                )
                .map(|bytes| bytes.to_vec())
            })
            .transpose()?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let installed = super::store::read(&tx, &context.id)?.context("Plugin was removed")?;
        ensure!(
            installed.enabled
                && installed.package.version == context.version
                && installed.key_scope == scope,
            "Plugin changed or was turned off; connect again"
        );
        let job = installed
            .package
            .job(&context.job)
            .context("Action was removed")?;
        ensure!(
            job.action.is_some()
                && job.secret_writes.iter().any(|k| k == key)
                && matches!(installed.package.setting(key), Some(Setting::Secret { .. })),
            "This action cannot save that setting"
        );
        let (data, state, stopped): (String, String, bool) = tx.query_row(
            "SELECT data,state,stop_requested FROM script_runs WHERE id=?1",
            [run],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let receipt: Run = serde_json::from_str(&data)?;
        let script = definition(&tx, &receipt.script_id)?;
        ensure!(
            state == "running"
                && !stopped
                && script.enabled
                && script.revision == receipt.revision
                && script.extension.as_ref().is_some_and(|l| l.id == context.id
                    && l.job == context.job
                    && l.version == context.version),
            "Action stopped or changed; no account settings were saved"
        );
        let previous: Option<Vec<u8>> = tx.query_row(
            "SELECT value FROM extension_settings WHERE extension_id=?1 AND key=?2 AND protected=1",
            params![context.id, key], |r| r.get(0)).optional()?;
        let previous = previous
            .as_ref()
            .map(|v| crate::protected_data::open(scope, v))
            .transpose()?;
        ensure!(
            previous.as_deref().map(|v| v.as_slice()) == expected.map(str::as_bytes),
            "Account settings changed during this action; try again"
        );
        if let Some(sealed) = sealed {
            tx.execute("INSERT INTO extension_settings(extension_id,key,value,protected) VALUES(?1,?2,?3,1)
                ON CONFLICT(extension_id,key) DO UPDATE SET value=excluded.value,protected=1",
                params![context.id, key, sealed])?;
        } else {
            tx.execute(
                "DELETE FROM extension_settings WHERE extension_id=?1 AND key=?2",
                params![context.id, key],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn is_extension_action(
        &self,
        script: &crate::script_automation::Script,
    ) -> Result<bool> {
        let Some(link) = &script.extension else {
            return Ok(false);
        };
        Ok(self
            .extension(&link.id)?
            .package
            .job(&link.job)
            .is_some_and(|j| j.action.is_some()))
    }
}

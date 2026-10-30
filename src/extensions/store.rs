//! Installed extensions. Every job is saved as an ordinary script whose ID never
//! changes, so stored values, keys, approvals and run history survive updates.
use super::{Context, Package, Reach, Setting, version};
use crate::{
    script_automation::{ExtensionLink, Language, Script, Trigger, busy, definition, put_script},
    store::Store,
};
use anyhow::{Context as _, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Created outside the numbered profile schema, because an existing profile must
/// stay usable by older builds until someone installs an extension.
pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS extensions(
          id TEXT PRIMARY KEY,data TEXT NOT NULL,enabled INTEGER NOT NULL,installed_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS extension_jobs(
          extension_id TEXT NOT NULL REFERENCES extensions(id) ON DELETE CASCADE,
          job_id TEXT NOT NULL,script_id TEXT NOT NULL UNIQUE,
          PRIMARY KEY(extension_id,job_id));
         CREATE TABLE IF NOT EXISTS extension_settings(
          extension_id TEXT NOT NULL REFERENCES extensions(id) ON DELETE CASCADE,
          key TEXT NOT NULL,value BLOB NOT NULL,protected INTEGER NOT NULL,
          PRIMARY KEY(extension_id,key));",
    )?;
    Ok(())
}

/// Saved scripts now carry fields older builds refuse, so they must not write here.
fn protect_profile(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE profile_format SET data_version=MAX(data_version,4),min_writer_version=MAX(min_writer_version,4) WHERE id=1", [])?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    package: Package,
    key_scope: uuid::Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Installed {
    pub package: Package,
    pub enabled: bool,
    pub installed_at: i64,
    #[serde(skip)]
    key_scope: uuid::Uuid,
}

/// What an install or update would do. Showing it changes nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Review {
    pub package: Package,
    pub update: Option<Changes>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Changes {
    pub from_version: String,
    /// The new publisher name, when it differs. The ID proves nothing about who
    /// made a package, so this is the only sign that it changed hands.
    pub new_publisher: Option<String>,
    pub added_hosts: Vec<String>,
    pub removed_hosts: Vec<String>,
    pub added_jobs: Vec<String>,
    pub removed_jobs: Vec<String>,
    pub changed_files: Vec<String>,
    pub new_file_access: Vec<String>,
    /// Job kinds this extension did not have, which bring powers it did not have.
    pub new_kinds: Vec<String>,
    /// Jobs that now run at other times or on other pages, or start from another file.
    pub changed_jobs: Vec<String>,
    /// Labels of settings whose saved value this version can't use, so it is deleted.
    pub dropped_settings: Vec<String>,
}
impl Changes {
    pub fn needs_approval(&self) -> bool {
        self.new_publisher.is_some()
            || !self.added_hosts.is_empty()
            || !self.new_file_access.is_empty()
            || !self.new_kinds.is_empty()
            || !self.changed_jobs.is_empty()
    }
}

/// What the person decided in the review. Nothing here is taken from the package.
#[derive(Clone, Debug, Default)]
pub struct Grants {
    /// Jobs allowed to change files, out of those that ask.
    pub file_access: BTreeSet<String>,
    /// The queue for each Queue completed job.
    pub queues: BTreeMap<String, String>,
    /// The person approved the new access this update asks for.
    pub approved: bool,
}

fn kind(job: &super::Job) -> &'static str {
    match job.trigger {
        Trigger::SiteReader { .. } => "site_reader",
        Trigger::Provider => "provider",
        _ => "automation",
    }
}

fn compare(old: &Package, new: &Package) -> Changes {
    let hosts = |package: &Package| {
        package
            .permissions
            .network
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
    };
    let (old_hosts, new_hosts) = (hosts(old), hosts(new));
    let names = |left: &Package, right: &Package| {
        left.jobs
            .iter()
            .filter(|job| right.job(&job.id).is_none())
            .map(|job| job.name.clone())
            .collect::<Vec<_>>()
    };
    let old_kinds: BTreeSet<_> = old.jobs.iter().map(kind).collect();
    let new_kinds: BTreeSet<_> = new.jobs.iter().map(kind).collect();
    Changes {
        from_version: old.version.clone(),
        new_publisher: (old.publisher != new.publisher).then(|| new.publisher.clone()),
        added_hosts: new_hosts.difference(&old_hosts).cloned().collect(),
        removed_hosts: old_hosts.difference(&new_hosts).cloned().collect(),
        added_jobs: names(new, old),
        removed_jobs: names(old, new),
        changed_files: new
            .files
            .iter()
            .filter(|(path, source)| old.files.get(*path) != Some(*source))
            .map(|(path, _)| path.clone())
            .chain(
                old.files
                    .keys()
                    .filter(|path| !new.files.contains_key(*path))
                    .cloned(),
            )
            .collect(),
        new_file_access: new
            .jobs
            .iter()
            .filter(|job| job.change_files && !old.job(&job.id).is_some_and(|o| o.change_files))
            .map(|job| job.name.clone())
            .collect(),
        new_kinds: new_kinds
            .difference(&old_kinds)
            .map(|kind| (*kind).to_owned())
            .collect(),
        changed_jobs: new
            .jobs
            .iter()
            .filter(|job| {
                old.job(&job.id)
                    .is_some_and(|o| o.trigger != job.trigger || o.entry != job.entry)
            })
            .map(|job| job.name.clone())
            .collect(),
        dropped_settings: Vec::new(),
    }
}

/// Saved values a new version can't use: the setting is gone, changed type, or no
/// longer accepts what was saved.
fn stale_settings(conn: &Connection, old: &Package, new: &Package) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT key,value,protected FROM extension_settings WHERE extension_id=?1")?;
    let rows = stmt
        .query_map([&new.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(
            |(key, value, protected)| match (old.setting(key), new.setting(key)) {
                (Some(before), Some(after)) => {
                    std::mem::discriminant(before) != std::mem::discriminant(after)
                        || (!protected
                            && serde_json::from_slice(value)
                                .map_or(true, |value| after.check(&value).is_err()))
                }
                _ => true,
            },
        )
        .map(|(key, ..)| key)
        .collect())
}

fn read(conn: &Connection, id: &str) -> Result<Option<Installed>> {
    let row: Option<(String, bool, i64)> = conn
        .query_row(
            "SELECT data,enabled,installed_at FROM extensions WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    row.map(|(data, enabled, installed_at)| {
        let saved: Saved = serde_json::from_str(&data)?;
        Ok(Installed {
            package: saved.package,
            enabled,
            installed_at,
            key_scope: saved.key_scope,
        })
    })
    .transpose()
}

fn job_scripts(conn: &Connection, id: &str) -> Result<BTreeMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT job_id,script_id FROM extension_jobs WHERE extension_id=?1")?;
    let rows = stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn queue_exists(conn: &Connection, queue: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM download_queues WHERE id=?1)",
        [queue],
        |r| r.get(0),
    )?)
}

fn unfinished_downloads(conn: &Connection, id: &str) -> Result<Vec<crate::model::Job>> {
    let mut stmt = conn.prepare(
        "SELECT id,data FROM jobs WHERE json_type(data,'$.provider')='object'
         AND json_extract(data,'$.status')<>'complete'",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut jobs = Vec::new();
    for (row, data) in rows {
        let job = crate::store::decode_job(&row, &data)?;
        let state = job.provider.as_ref().context("Missing provider")?;
        if state.script.extension.as_ref().is_some_and(|l| l.id == id) {
            jobs.push(job);
        }
    }
    Ok(jobs)
}

impl Store {
    pub fn extensions(&self) -> Result<Vec<Installed>> {
        let conn = self.connect()?;
        let ids = conn
            .prepare("SELECT id FROM extensions ORDER BY rowid")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter()
            .map(|id| read(&conn, id)?.context("Extension no longer exists"))
            .collect()
    }
    pub fn extension(&self, id: &str) -> Result<Installed> {
        read(&self.connect()?, id)?.context("Extension no longer exists")
    }
    /// The saved script behind a job. Its ID stays the same across updates.
    pub fn extension_job(&self, id: &str, job: &str) -> Result<Script> {
        let conn = self.connect()?;
        let script = job_scripts(&conn, id)?
            .remove(job)
            .context("This extension has no such job")?;
        definition(&conn, &script)
    }

    /// Checks a package and says what installing it would change. Saves nothing.
    pub fn review_extension(&self, bytes: &[u8]) -> Result<Review> {
        let package = Package::decode(bytes)?;
        let conn = self.connect()?;
        let update = read(&conn, &package.id)?
            .map(|old| {
                let mut changes = compare(&old.package, &package);
                changes.dropped_settings = stale_settings(&conn, &old.package, &package)?
                    .iter()
                    .map(|key| {
                        old.package
                            .setting(key)
                            .map_or_else(|| key.clone(), |setting| setting.label().to_owned())
                    })
                    .collect();
                Ok::<_, anyhow::Error>(changes)
            })
            .transpose()?;
        Ok(Review { package, update })
    }

    /// Installs a new extension turned off, or updates an installed one.
    pub fn install_extension(&self, bytes: &[u8], grants: &Grants) -> Result<Installed> {
        let package = Package::decode(bytes)?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = read(&tx, &package.id)?;
        if let Some(old) = &old {
            ensure!(
                version(&package.version)? > version(&old.package.version)?,
                "Version {} is not newer than the installed {}. Remove the extension first to go back",
                package.version,
                old.package.version
            );
            ensure!(
                grants.approved || !compare(&old.package, &package).needs_approval(),
                "This update needs your approval. Review it and approve it to continue"
            );
        }
        let mut scripts = job_scripts(&tx, &package.id)?;
        for script in scripts.values() {
            ensure!(
                !busy(&tx, script)?,
                "Stop this extension's running jobs before updating it"
            );
        }
        let stale = old
            .as_ref()
            .map(|old| stale_settings(&tx, &old.package, &package))
            .transpose()?
            .unwrap_or_default();
        if package.jobs.iter().any(|j| j.trigger == Trigger::Provider) {
            crate::provider::protect_profile(&tx)?;
        }
        protect_profile(&tx)?;
        let enabled = old.as_ref().is_some_and(|o| o.enabled);
        let scope = old
            .as_ref()
            .map_or_else(uuid::Uuid::new_v4, |o| o.key_scope);
        let now = chrono::Utc::now().timestamp();
        tx.execute(
            "INSERT INTO extensions(id,data,enabled,installed_at) VALUES(?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            params![
                package.id,
                serde_json::to_string(&Saved {
                    package: package.clone(),
                    key_scope: scope,
                })?,
                enabled,
                now
            ],
        )?;
        for job in &package.jobs {
            let existing = scripts.remove(&job.id);
            let previous = existing
                .as_deref()
                .map(|id| definition(&tx, id))
                .transpose()?;
            let id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let mut trigger = job.trigger.clone();
            if let Trigger::QueueCompleted { queue } = &mut trigger {
                *queue = match (&grants.queues.get(&job.id), &previous) {
                    (Some(queue), _) => (*queue).clone(),
                    (
                        None,
                        Some(Script {
                            trigger: Trigger::QueueCompleted { queue },
                            ..
                        }),
                    ) => queue.clone(),
                    _ => bail!("Choose a queue for {}", job.name),
                };
                ensure!(queue_exists(&tx, queue)?, "Choose an existing queue");
            }
            // A grant covers what was asked for. It never carries over to a job
            // that stopped asking and later asks again.
            let granted = grants.file_access.contains(&job.id)
                || previous.as_ref().is_some_and(|p| p.change_files);
            let script = Script {
                revision: previous
                    .as_ref()
                    .map_or(0, |p| p.revision)
                    .checked_add(1)
                    .context("Script revision overflow")?,
                id,
                name: job.name.clone(),
                language: Language::Lua,
                body: package
                    .files
                    .get(&job.entry)
                    .context("Missing job entry")?
                    .clone(),
                enabled,
                trigger,
                timeout_seconds: job.timeout_seconds,
                change_files: job.change_files && granted,
                extension: Some(ExtensionLink {
                    id: package.id.clone(),
                    job: job.id.clone(),
                    version: package.version.clone(),
                    hosts: package.permissions.network.clone(),
                    limit_results: package.has_secret(),
                    files: BTreeMap::new(),
                }),
            };
            script.validate()?;
            put_script(&tx, &script)?;
            tx.execute(
                "INSERT INTO extension_jobs(extension_id,job_id,script_id) VALUES(?1,?2,?3)
                 ON CONFLICT(extension_id,job_id) DO NOTHING",
                params![package.id, job.id, script.id],
            )?;
        }
        // What is left belongs to jobs this version dropped.
        for (job, script) in &scripts {
            tx.execute("DELETE FROM automation_scripts WHERE id=?1", [script])?;
            tx.execute("DELETE FROM script_state WHERE script_id=?1", [script])?;
            tx.execute(
                "DELETE FROM extension_jobs WHERE extension_id=?1 AND job_id=?2",
                params![package.id, job],
            )?;
        }
        for key in stale {
            tx.execute(
                "DELETE FROM extension_settings WHERE extension_id=?1 AND key=?2",
                params![package.id, key],
            )?;
        }
        let installed = read(&tx, &package.id)?.context("Extension was not saved")?;
        tx.commit()?;
        Ok(installed)
    }

    /// Turning an extension off also stops whatever it has running.
    pub fn set_extension_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let scripts = {
            let mut conn = self.connect()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            ensure!(
                tx.execute(
                    "UPDATE extensions SET enabled=?1 WHERE id=?2",
                    params![enabled, id]
                )? == 1,
                "Extension no longer exists"
            );
            let scripts = job_scripts(&tx, id)?;
            for script in scripts.values() {
                let mut script = definition(&tx, script)?;
                script.enabled = enabled;
                put_script(&tx, &script)?;
            }
            tx.commit()?;
            scripts
        };
        if !enabled {
            for script in scripts.values() {
                for run in self.script_runs(Some(script))? {
                    if run.active() {
                        self.stop_script(&run.id)?;
                    }
                }
            }
            // Other downloads can't start: extension_context refuses them.
            for job in unfinished_downloads(&self.connect()?, id)? {
                if job.status == crate::model::Status::Downloading {
                    self.submit_control(
                        &uuid::Uuid::new_v4().to_string(),
                        &crate::control::Action::Pause { id: job.id },
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Removes an extension, its settings and stored values. Run history stays.
    pub fn remove_extension(&self, id: &str) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        read(&tx, id)?.context("Extension no longer exists")?;
        let scripts = job_scripts(&tx, id)?;
        for script in scripts.values() {
            ensure!(
                !busy(&tx, script)?,
                "Stop this extension's running jobs before removing it"
            );
        }
        let unfinished = unfinished_downloads(&tx, id)?.len();
        ensure!(
            unfinished == 0,
            "{unfinished} unfinished downloads still use this extension. Finish or remove them first"
        );
        for script in scripts.values() {
            tx.execute("DELETE FROM automation_scripts WHERE id=?1", [script])?;
            tx.execute("DELETE FROM script_state WHERE script_id=?1", [script])?;
        }
        tx.execute("DELETE FROM extensions WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    fn setting_for(&self, id: &str, key: &str) -> Result<(Installed, Setting)> {
        let installed = self.extension(id)?;
        let setting = installed
            .package
            .setting(key)
            .context("This extension has no such setting")?
            .clone();
        Ok((installed, setting))
    }

    /// Saves a plain setting after checking it against the manifest.
    pub fn set_extension_setting(
        &self,
        id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        let (_, setting) = self.setting_for(id, key)?;
        setting.check(value)?;
        self.connect()?.execute(
            "INSERT INTO extension_settings(extension_id,key,value,protected) VALUES(?1,?2,?3,0)
             ON CONFLICT(extension_id,key) DO UPDATE SET value=excluded.value,protected=0",
            params![id, key, serde_json::to_vec(value)?],
        )?;
        Ok(())
    }

    /// Seals a secret or AES key. It can be replaced or cleared but never read back here.
    pub fn set_extension_secret(&self, id: &str, key: &str, value: &str) -> Result<()> {
        let (installed, setting) = self.setting_for(id, key)?;
        let sealed = match setting {
            Setting::Secret { .. } => {
                ensure!(
                    !value.is_empty() && value.len() <= 4096,
                    "Enter 1 to 4096 characters"
                );
                let mut bytes = zeroize::Zeroizing::new(value.as_bytes().to_vec());
                crate::protected_data::seal(installed.key_scope, &mut bytes)?.to_vec()
            }
            Setting::AesKey { .. } => {
                crate::provider::seal_key(installed.key_scope, value.trim())?.into_bytes()
            }
            _ => bail!("This setting is not protected"),
        };
        self.connect()?.execute(
            "INSERT INTO extension_settings(extension_id,key,value,protected) VALUES(?1,?2,?3,1)
             ON CONFLICT(extension_id,key) DO UPDATE SET value=excluded.value,protected=1",
            params![id, key, sealed],
        )?;
        Ok(())
    }

    pub fn clear_extension_setting(&self, id: &str, key: &str) -> Result<()> {
        self.setting_for(id, key)?;
        self.connect()?.execute(
            "DELETE FROM extension_settings WHERE extension_id=?1 AND key=?2",
            params![id, key],
        )?;
        Ok(())
    }

    /// Plain values as saved, and null for protected ones that have been saved.
    pub fn extension_settings(&self, id: &str) -> Result<BTreeMap<String, serde_json::Value>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare("SELECT key,value,protected FROM extension_settings WHERE extension_id=?1")?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(key, value, protected)| {
                let value = if protected {
                    serde_json::Value::Null
                } else {
                    serde_json::from_slice(&value)?
                };
                Ok((key, value))
            })
            .collect()
    }

    /// Every saved script, with the name of the extension it belongs to, if any.
    pub fn script_origins(&self) -> Result<BTreeMap<String, Option<String>>> {
        let names: BTreeMap<String, String> = self
            .extensions()?
            .into_iter()
            .map(|e| (e.package.id, e.package.name))
            .collect();
        Ok(crate::script_automation::all_scripts(&self.connect()?)?
            .into_iter()
            .map(|script| {
                let origin = script.extension.as_ref().map(|link| {
                    names
                        .get(&link.id)
                        .cloned()
                        .unwrap_or_else(|| link.id.clone())
                });
                (script.id, origin)
            })
            .collect())
    }

    /// The first enabled reader that claims the same pages ahead of this extension.
    pub fn extension_conflict(&self, id: &str) -> Result<Option<String>> {
        let readers = self.site_readers()?;
        for (index, script) in readers.iter().enumerate() {
            let Trigger::SiteReader { pages } = &script.trigger else {
                continue;
            };
            if script.extension.as_ref().is_none_or(|l| l.id != id) {
                continue;
            }
            for earlier in &readers[..index] {
                if let Trigger::SiteReader { pages: first } = &earlier.trigger
                    && earlier.extension.as_ref().is_none_or(|l| l.id != id)
                    && super::pages_overlap(first, pages)
                {
                    return Ok(Some(earlier.name.clone()));
                }
            }
        }
        Ok(None)
    }

    /// Everything a job's worker may hold. Rust decides by the job's kind, so a
    /// job can never read a setting that belongs to a different kind of job. Every
    /// start comes through here, so nothing runs while the extension is off.
    pub fn extension_context(&self, script: &Script, reach: Reach) -> Result<Option<Context>> {
        let Some(link) = &script.extension else {
            return Ok(None);
        };
        let conn = self.connect()?;
        let installed =
            read(&conn, &link.id)?.context("The extension behind this job was removed")?;
        ensure!(
            installed.enabled,
            "Turn on {} to use it",
            installed.package.name
        );
        let mut stored = BTreeMap::new();
        {
            let mut stmt =
                conn.prepare("SELECT key,value FROM extension_settings WHERE extension_id=?1")?;
            let rows = stmt.query_map([&link.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?;
            for row in rows {
                let (key, value) = row?;
                stored.insert(key, value);
            }
        }
        let mut settings = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for setting in &installed.package.settings {
            let saved = stored.get(setting.id());
            match setting {
                Setting::Secret { .. } => {
                    if reach == Reach::Web {
                        let value = saved
                            .map(|sealed| {
                                let bytes =
                                    crate::protected_data::open(installed.key_scope, sealed)
                                        .context(
                                            "Cannot unlock this setting with this Windows account",
                                        )?;
                                Ok::<_, anyhow::Error>(String::from_utf8(bytes.to_vec())?)
                            })
                            .transpose()?;
                        settings.insert(
                            setting.id().to_owned(),
                            value.map_or(serde_json::Value::Null, Into::into),
                        );
                    }
                }
                Setting::AesKey { .. } => {
                    if reach == Reach::Processor
                        && let Some(token) = saved
                    {
                        keys.insert(setting.id().to_owned(), String::from_utf8(token.clone())?);
                    }
                }
                _ => {
                    let value = saved
                        .map(|value| serde_json::from_slice(value))
                        .transpose()?
                        .unwrap_or_else(|| setting.default_value());
                    settings.insert(setting.id().to_owned(), value);
                }
            }
        }
        Ok(Some(Context {
            id: link.id.clone(),
            version: link.version.clone(),
            job: link.job.clone(),
            hosts: link.hosts.clone(),
            // The saved settings are today's, whatever version a frozen job runs.
            limit_results: link.limit_results || installed.package.has_secret(),
            // A frozen download keeps the files it started with.
            files: if link.files.is_empty() {
                installed.package.files
            } else {
                link.files.clone()
            },
            settings,
            keys,
            key_scope: (reach == Reach::Processor).then_some(installed.key_scope),
        }))
    }

    /// Copies an extension's files into a provider job, which never changes
    /// afterwards. Refuses to start a download with an extension turned off.
    pub(crate) fn freeze_extension(&self, script: &mut Script) -> Result<()> {
        let Some(link) = &mut script.extension else {
            return Ok(());
        };
        let installed = self.extension(&link.id)?;
        ensure!(
            installed.enabled,
            "Turn on {} to use it",
            installed.package.name
        );
        link.files = installed.package.files;
        Ok(())
    }
}

/// Builds a package from a folder holding manifest.json and Lua files. Anything
/// else in the folder stops the build, so nothing is left out by accident.
pub fn pack(folder: &Path) -> Result<(String, Vec<u8>)> {
    fn walk(root: &Path, folder: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "Links cannot be packed: {}",
                    path.display()
                );
            }
            ensure!(
                !metadata.file_type().is_symlink(),
                "Links cannot be packed: {}",
                path.display()
            );
            if metadata.is_dir() {
                walk(root, &path, files)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "manifest.json" {
                continue;
            }
            ensure!(
                super::valid_path(&relative),
                "Only lower-case .lua files can be packed: {relative}"
            );
            ensure!(
                files.len() < super::MAX_FILES,
                "An extension can hold at most {} Lua files",
                super::MAX_FILES
            );
            ensure!(
                metadata.len() <= crate::script_automation::MAX_SCRIPT_BYTES as u64,
                "{relative} is over 64 KiB"
            );
            files.insert(relative, std::fs::read_to_string(&path)?);
        }
        Ok(())
    }
    let manifest = folder.join("manifest.json");
    ensure!(
        std::fs::metadata(&manifest).is_ok_and(|m| m.len() <= super::MAX_PACKAGE_BYTES as u64),
        "manifest.json is missing or over 1 MiB"
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest)?).context("Invalid manifest.json")?;
    let manifest = value
        .as_object_mut()
        .context("manifest.json must be an object")?;
    ensure!(
        !manifest.contains_key("files"),
        "manifest.json must not list files; they are read from the folder"
    );
    let mut files = BTreeMap::new();
    walk(folder, folder, &mut files)?;
    manifest.insert("files".into(), serde_json::to_value(&files)?);
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    let package = Package::decode(&bytes)?;
    Ok((package.file_name(), bytes))
}

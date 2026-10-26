//! Saved discovery settings. Scheduling and pending discoveries live separately.
use crate::{
    browser_batch::Destination, model::validate_url, store::Store, website_grabber::Config,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

pub const MAX_PROJECTS: usize = 100;
pub const MAX_NAME_CHARS: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub url: String,
    pub extensions: String,
    pub depth: u8,
    pub page_limit: usize,
    pub queue: String,
    /// A saved folder choice, resolved using current folder rules when adding files.
    pub destination: Destination,
}

impl Settings {
    pub(crate) fn validated(&self) -> Result<Self> {
        Config::new(&self.url, &self.extensions, self.depth, self.page_limit)?;
        ensure!(
            !self.queue.is_empty() && self.queue.len() <= 64,
            "Choose a download queue"
        );
        Ok(Self {
            url: validate_url(&self.url)?.to_string(),
            extensions: self.extensions.trim().to_owned(),
            ..self.clone()
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub revision: i64,
    pub settings: Settings,
}

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS grabber_projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK(length(name) BETWEEN 1 AND 80),
        revision INTEGER NOT NULL CHECK(revision>0),
        settings TEXT NOT NULL);",
    )?;
    Ok(())
}

fn project_name(name: &str) -> Result<&str> {
    let name = name.trim();
    ensure!(
        !name.is_empty()
            && name.chars().count() <= MAX_NAME_CHARS
            && !name.chars().any(char::is_control),
        "Enter a project name of 1–{MAX_NAME_CHARS} characters without control characters"
    );
    Ok(name)
}

fn require_queue(conn: &Connection, queue: &str) -> Result<()> {
    ensure!(
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM download_queues WHERE id=?1)",
            [queue],
            |r| r.get::<_, bool>(0),
        )?,
        "Queue no longer exists. Choose another queue before saving this project."
    );
    Ok(())
}

impl Store {
    pub fn grabber_projects(&self) -> Result<Vec<Project>> {
        let conn = self.connect()?;
        let mut statement = conn.prepare(
            "SELECT id,name,revision,settings FROM grabber_projects ORDER BY name COLLATE NOCASE,id",
        )?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, revision, data) = row?;
            project_name(&name).context("Invalid saved grabber project name")?;
            ensure!(revision > 0, "Invalid saved grabber project revision");
            let settings: Settings =
                serde_json::from_str(&data).context("Invalid saved grabber project settings")?;
            // A deleted queue must not erase a project or silently redirect its files.
            // The desktop asks for a replacement before saving or adding downloads.
            let settings = settings
                .validated()
                .context("Invalid saved grabber project settings")?;
            Ok(Project {
                id,
                name,
                revision,
                settings,
            })
        })
        .collect()
    }

    pub fn create_grabber_project(&self, name: &str, settings: &Settings) -> Result<Project> {
        let name = project_name(name)?;
        let settings = settings.validated()?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_queue(&tx, &settings.queue)?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM grabber_projects", [], |r| r.get(0))?;
        ensure!(
            count < MAX_PROJECTS as i64,
            "Up to {MAX_PROJECTS} projects can be saved. Delete an unused project first."
        );
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            revision: 1,
            settings,
        };
        tx.execute(
            "INSERT INTO grabber_projects(id,name,revision,settings) VALUES (?1,?2,?3,?4)",
            params![
                project.id,
                project.name,
                project.revision,
                serde_json::to_string(&project.settings)?
            ],
        )
        .context("Could not save project; choose a unique project name")?;
        tx.commit()?;
        Ok(project)
    }

    pub fn update_grabber_project(
        &self,
        original: &Project,
        name: &str,
        settings: &Settings,
    ) -> Result<Project> {
        let name = project_name(name)?;
        let settings = settings.validated()?;
        let revision = original
            .revision
            .checked_add(1)
            .context("Project revision limit reached")?;
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_queue(&tx, &settings.queue)?;
        let count = tx.execute(
            "UPDATE grabber_projects SET name=?1,revision=?2,settings=?3 WHERE id=?4 AND revision=?5",
            params![name, revision, serde_json::to_string(&settings)?, original.id, original.revision],
        ).context("Could not save project; choose a unique project name")?;
        ensure!(
            count == 1,
            "Project changed or was deleted. Reopen it before saving."
        );
        tx.commit()?;
        Ok(Project {
            id: original.id.clone(),
            name: name.into(),
            revision,
            settings,
        })
    }

    pub fn delete_grabber_project(&self, project: &Project) -> Result<()> {
        let count = self.connect()?.execute(
            "DELETE FROM grabber_projects WHERE id=?1 AND revision=?2",
            params![project.id, project.revision],
        )?;
        ensure!(
            count == 1,
            "Project changed or was deleted. Reopen it before deleting."
        );
        Ok(())
    }
}

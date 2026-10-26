use super::{Download, Playlist};
use crate::{
    model::{Job, validate_url},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub(crate) fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS hls_previews(request_id TEXT PRIMARY KEY,url TEXT,name TEXT,created INTEGER NOT NULL);")?;
    Ok(())
}
impl Store {
    pub fn enqueue_hls(
        &self,
        playlist: &Playlist,
        name: &str,
        folder: Option<&Path>,
        queue: &str,
        request_id: &str,
    ) -> Result<Job> {
        self.enqueue_found_hls(playlist, None, name, folder, queue, request_id)
    }
    /// A stream the browser found on `page`, whose origin is sent as the Referer.
    pub fn enqueue_found_hls(
        &self,
        playlist: &Playlist,
        page: Option<&str>,
        name: &str,
        folder: Option<&Path>,
        queue: &str,
        request_id: &str,
    ) -> Result<Job> {
        self.enqueue_video(
            &Download::new(playlist.clone()),
            page,
            name,
            folder,
            queue,
            request_id,
        )
    }
    pub fn enqueue_youtube(
        &self,
        preview: &crate::youtube::Preview,
        name: &str,
        folder: Option<&Path>,
        queue: &str,
        request_id: &str,
    ) -> Result<Job> {
        let mut download = Download::new(preview.playlist.clone());
        download.version = 5;
        download.youtube = Some(preview.selection.clone());
        self.enqueue_video(&download, None, name, folder, queue, request_id)
    }
    pub fn enqueue_bilibili(
        &self,
        preview: &crate::bilibili::Preview,
        name: &str,
        folder: Option<&Path>,
        queue: &str,
        request_id: &str,
    ) -> Result<Job> {
        let mut download = Download::new(preview.playlist.clone());
        download.version = 6;
        download.bilibili = Some(preview.selection.clone());
        self.enqueue_video(&download, None, name, folder, queue, request_id)
    }
    pub fn enqueue_tiktok(
        &self,
        preview: &crate::tiktok::Preview,
        name: &str,
        folder: Option<&Path>,
        queue: &str,
        request_id: &str,
    ) -> Result<Job> {
        let mut download = Download::new(preview.playlist.clone());
        download.version = 7;
        download.tiktok = Some(preview.selection.clone());
        self.enqueue_video(&download, None, name, folder, queue, request_id)
    }
    fn enqueue_video(
        &self,
        download: &Download,
        page: Option<&str>,
        name: &str,
        folder: Option<&Path>,
        queue: &str,
        request_id: &str,
    ) -> Result<Job> {
        download.validate()?;
        let playlist = &download.playlist;
        let video_folder = self
            .download_folders()?
            .category_folder(&self.paths.downloads, crate::categories::Category::Video);
        let mut job = self.new_download(
            &playlist.source,
            Some(name),
            Some(folder.unwrap_or(&video_folder)),
            request_id.into(),
        )?;
        ensure!(
            job.name
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", playlist.output_extension())),
            "Keep the video filename extension shown in the preview"
        );
        job.hls = Some(download.clone());
        job.public_only = playlist.public_only;
        job.source_page = page
            .map(|page| validate_url(page).map(|page| page.to_string()))
            .transpose()?;
        job.server_name_pending = false;
        Ok(self.enqueue_batch(&[job], queue)?.remove(0))
    }
    /// Browser handoffs only open the desktop preview. They cannot create a
    /// runnable HLS job, choose a filesystem path, or supply segment URLs.
    pub fn request_hls_preview(
        &self,
        request_id: &str,
        url: &str,
        name: Option<&str>,
    ) -> Result<()> {
        let token = request_id
            .strip_prefix("hls:")
            .context("Invalid HLS preview identifier")?;
        crate::model::validate_job_id(token)?;
        let url = validate_url(url)?.to_string();
        ensure!(
            name.is_none_or(|v| v.len() <= 1024),
            "HLS title is too long"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now = chrono::Utc::now().timestamp();
        tx.execute("DELETE FROM hls_previews WHERE created < ?1", [now - 86400])?;
        let existing: Option<Option<String>> = tx
            .query_row(
                "SELECT url FROM hls_previews WHERE request_id=?1",
                [request_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            ensure!(
                existing.is_none_or(|previous| previous == url),
                "HLS preview identifier belongs to another URL"
            );
            return Ok(());
        }
        let pending: i64 = tx.query_row(
            "SELECT count(*) FROM hls_previews WHERE url IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        ensure!(
            pending < 10,
            "Review pending HLS videos in Download Engine first"
        );
        tx.execute(
            "INSERT INTO hls_previews(request_id,url,name,created) VALUES(?1,?2,?3,?4)",
            params![request_id, url, name, now],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn next_hls_preview(&self) -> Result<Option<(String, String)>> {
        Ok(self.connect()?.query_row("SELECT request_id,url FROM hls_previews WHERE url IS NOT NULL AND created >= ?1 ORDER BY created,rowid LIMIT 1",[chrono::Utc::now().timestamp()-86400],|row| Ok((row.get(0)?,row.get(1)?))).optional()?)
    }
    pub fn acknowledge_hls_preview(&self, request_id: &str) -> Result<()> {
        self.connect()?.execute(
            "UPDATE hls_previews SET url=NULL,name=NULL WHERE request_id=?1",
            [request_id],
        )?;
        Ok(())
    }
}

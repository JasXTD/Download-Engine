//! A bounded review of direct file links found on a download landing page.
use crate::{
    model::{Job, Status, safe_filename, validate_url},
    website_grabber::Candidate,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const MAX_FILES: usize = 100;
pub(crate) const MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    /// Only one explicit, same-origin download link may be followed automatically.
    Automatic,
    File(usize),
    /// Explicitly keep the original HTML response, rather than discovering links.
    Page,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub id: Uuid,
    pub page: String,
    pub files: Vec<Candidate>,
    pub truncated: bool,
    pub selection: Option<Selection>,
}

impl State {
    pub fn needs_review(&self) -> bool {
        self.selection.is_none()
    }

    pub(crate) fn selected_file(&self) -> Option<&Candidate> {
        match self.selection {
            Some(Selection::Automatic) => self.files.first(),
            Some(Selection::File(index)) => self.files.get(index),
            _ => None,
        }
    }

    pub(crate) fn validate(&self, job: &Job) -> Result<()> {
        let page = validate_url(&self.page)?;
        ensure!(
            job.hls.is_none()
                && job.ftp.is_none()
                && job.authentication.is_none()
                && job.expected_file.is_none()
                && job.synchronization.is_none()
                && !crate::mega::is_url(&job.url)
                && !crate::wetransfer::is_url(&job.url),
            "Invalid saved webpage download state"
        );
        ensure!(
            self.files.len() <= MAX_FILES,
            "Too many saved webpage file links"
        );
        let mut bytes = 0;
        let mut urls = std::collections::HashSet::new();
        for file in &self.files {
            validate_url(&file.url)?;
            ensure!(
                file.page == self.page
                    && urls.insert(&file.url)
                    && file.name == safe_filename(&file.name)
                    && file
                        .filename
                        .as_ref()
                        .is_none_or(|name| *name == safe_filename(name)),
                "Invalid saved webpage file link"
            );
            bytes += file.url.len()
                + file.name.len()
                + file.page.len()
                + file.filename.as_ref().map_or(0, String::len);
        }
        ensure!(
            bytes <= MAX_BYTES,
            "Saved webpage file links exceed the size limit"
        );
        match self.selection {
            Some(Selection::Automatic) => ensure!(
                !self.truncated
                    && self.files.len() == 1
                    && validate_url(&self.files[0].url)?.origin() == page.origin(),
                "Invalid automatic webpage download selection"
            ),
            Some(Selection::File(index)) => {
                ensure!(index < self.files.len(), "The selected file is unavailable")
            }
            None => ensure!(
                job.plan.is_none() && job.downloaded == 0 && job.saved_path.is_none(),
                "A started download cannot have an unresolved webpage selection"
            ),
            Some(Selection::Page) => {}
        }
        Ok(())
    }
}

/// Applied by the scheduler only after the previous worker has released the job.
pub(crate) fn select(job: &mut Job, expected: Uuid, selection: Selection) -> Result<()> {
    ensure!(
        matches!(job.status, Status::Failed | Status::Paused),
        "Wait for the download to stop before choosing a file"
    );
    ensure!(
        job.plan.is_none() && job.downloaded == 0 && job.saved_path.is_none(),
        "Restart before choosing another file"
    );
    let state = job
        .link_fallback
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("This download no longer needs a file selection"))?;
    ensure!(
        state.id == expected && state.needs_review(),
        "The file links changed. Open the review again"
    );
    match selection {
        Selection::File(index) => ensure!(
            index < state.files.len(),
            "The selected file is unavailable"
        ),
        Selection::Page => {}
        Selection::Automatic => anyhow::bail!("Choose a file from the discovered links"),
    }
    state.selection = Some(selection);
    if state.selection == Some(Selection::Page)
        && job.server_name_pending
        && ![".html", ".htm", ".xhtml"]
            .iter()
            .any(|ext| job.name.to_ascii_lowercase().ends_with(ext))
    {
        job.name = safe_filename(&format!("{}.html", job.name));
    }
    job.status = Status::Queued;
    job.quota_waiting = false;
    job.retry = crate::retry::State::default();
    job.error = None;
    Ok(())
}

#[derive(Debug)]
pub(crate) struct CrossOriginRedirect;
impl std::fmt::Display for CrossOriginRedirect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The discovered link redirects to another site. Review the file links before continuing.")
    }
}
impl std::error::Error for CrossOriginRedirect {}

#[cfg(test)]
mod tests {
    use super::*;
    fn discover(html: &str) -> State {
        crate::website_grabber::fallback(
            validate_url("https://files.example/share").unwrap(),
            html.into(),
        )
    }
    fn pending(html: &str) -> Job {
        let mut job = Job::new(
            "https://files.example/share",
            None,
            std::env::temp_dir(),
            Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = Status::Failed;
        job.link_fallback = Some(discover(html));
        job
    }
    #[test]
    fn parser_uses_base_download_names_deduplication_and_ignores_page_assets() {
        let state = discover(
            "<base href='/assets/'><img src='image.zip'><script src='script.zip'></script><a href='javascript:alert(1)' download>Bad</a><a download='../../report.zip' href='file?id=1'>Get</a><a download href='file?id=1#again'>Again</a>",
        );
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].url, "https://files.example/assets/file?id=1");
        assert_eq!(
            state.files[0].filename.as_deref(),
            Some(safe_filename("../../report.zip").as_str())
        );
        assert_eq!(state.selection, Some(Selection::Automatic));
    }
    #[test]
    fn partial_results_and_cross_origin_base_never_auto_select() {
        let state =
            discover("<base href='https://other.example/'><a download href='/file'>Get</a>");
        assert!(state.needs_review());
        let mut html = "<a download href='/file'>Get</a>".to_owned();
        html.push_str(&"<a href='/navigation'>Next</a>".repeat(10_000));
        let state = discover(&html);
        assert!(state.truncated && state.needs_review());
        let html = (0..101)
            .map(|n| format!("<a href='/{n}.zip'>Get</a>"))
            .collect::<String>();
        let state = discover(&html);
        assert_eq!(state.files.len(), MAX_FILES);
        assert!(state.truncated && state.needs_review());
    }
    #[test]
    fn provider_links_and_https_downgrades_are_not_generic_candidates() {
        let state = discover(
            "<a download href='https://we.tl/t-test'>Transfer</a><a download href='http://files.example/file.zip'>Downgrade</a><a download href='/share#same'>Same</a>",
        );
        assert!(state.files.is_empty() && state.needs_review());
    }
    #[test]
    fn github_file_page_finds_raw_file_without_following_history_or_running_scripts() {
        let page = "https://github.com/mozilla/pdf.js/blob/master/test/pdfs/tracemonkey.pdf";
        let state = crate::website_grabber::fallback(
            validate_url(page).unwrap(),
            r#"<base href="https://unrelated.example/">
               <button>Download raw file</button>
               <script>{"rawBlobUrl":"https://unrelated.example/wrong.pdf"}</script>
               <a href="https://github.com/mozilla/pdf.js/commits/master/test/pdfs/tracemonkey.pdf">History</a>
               <a href="https://github.com/mozilla/pdf.js/blame/master/test/pdfs/tracemonkey.pdf">Blame</a>"#.into(),
        );
        assert_eq!(state.files.len(), 1);
        assert_eq!(
            state.files[0].url,
            "https://github.com/mozilla/pdf.js/raw/master/test/pdfs/tracemonkey.pdf"
        );
        assert_eq!(state.files[0].name, "tracemonkey.pdf");
        assert_eq!(state.files[0].page, page);
        assert!(state.needs_review());
    }
    #[test]
    fn inferred_github_file_uses_exact_host_and_deduplicates_existing_raw_links() {
        for page in [
            "https://github.com.example.test/owner/repo/blob/main/file.pdf",
            "https://github.com:444/owner/repo/blob/main/file.pdf",
            "https://github.com/owner/repo/commits/main/file.pdf",
            "https://github.com/owner/repo/blob/main",
        ] {
            let state = crate::website_grabber::fallback(
                validate_url(page).unwrap(),
                "<button>Download</button>".into(),
            );
            assert!(state.files.is_empty(), "{page}");
        }
        let state = crate::website_grabber::fallback(
            validate_url("https://github.com/owner/repo/blob/feature/topic/a%20b.pdf?download=1")
                .unwrap(),
            r#"<a href="/owner/repo/raw/feature/topic/a%20b.pdf?download=1">Raw</a>"#.into(),
        );
        assert_eq!(state.files.len(), 1);
        assert_eq!(
            state.files[0].url,
            "https://github.com/owner/repo/raw/feature/topic/a%20b.pdf?download=1"
        );
        assert_eq!(state.files[0].name, "a b.pdf");
        assert!(state.needs_review());
    }
    #[test]
    fn selection_rejects_stale_ids_invalid_indices_active_jobs_and_saved_bytes() {
        let original = pending("<a href='/one.zip'>One</a><a href='/two.zip'>Two</a>");
        let id = original.link_fallback.as_ref().unwrap().id;
        for (status, downloaded, discovery, selection) in [
            (Status::Failed, 0, Uuid::new_v4(), Selection::File(0)),
            (Status::Failed, 0, id, Selection::File(2)),
            (Status::Downloading, 0, id, Selection::File(0)),
            (Status::Paused, 1, id, Selection::File(0)),
            (Status::Failed, 0, id, Selection::Automatic),
        ] {
            let mut job = original.clone();
            job.status = status;
            job.downloaded = downloaded;
            assert!(select(&mut job, discovery, selection).is_err());
            assert!(job.link_fallback.as_ref().unwrap().needs_review());
        }
        let mut job = original.clone();
        select(&mut job, id, Selection::File(1)).unwrap();
        assert_eq!(job.status, Status::Queued);
        assert_eq!(job.url, original.url);
        assert!(select(&mut job, id, Selection::File(0)).is_err());
    }
    #[test]
    fn saved_fallback_validates_boundaries_and_legacy_jobs_still_deserialize() {
        let original = pending("<a download href='/one.zip'>One</a>");
        original.validate_persisted().unwrap();
        let mut value = serde_json::to_value(&original).unwrap();
        value.as_object_mut().unwrap().remove("link_fallback");
        let legacy: Job = serde_json::from_value(value).unwrap();
        assert!(legacy.link_fallback.is_none());
        legacy.validate_persisted().unwrap();
        let mut modified = original.clone();
        modified.link_fallback.as_mut().unwrap().files[0].url =
            "https://elsewhere.example/file.zip".into();
        assert!(modified.validate_persisted().is_err());
        let mut modified = original;
        modified.link_fallback.as_mut().unwrap().files[0].page =
            "https://elsewhere.example/share".into();
        assert!(modified.validate_persisted().is_err());
    }
}

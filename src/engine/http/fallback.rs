//! Discover once before any file bytes or resume plan are committed.
use super::*;
use crate::link_fallback::{CrossOriginRedirect, Selection};

use crate::website_grabber::MAX_PAGE_BYTES;

fn html(response: &Response) -> bool {
    !response.headers().contains_key(header::CONTENT_DISPOSITION)
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                matches!(
                    value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "text/html" | "application/xhtml+xml"
                )
            })
}

pub(super) fn selected_html(source: &Source, response: &Response) -> bool {
    matches!(source, Source::Discovered { .. }) && response.status().is_success() && html(response)
}

fn eligible(job: &Job) -> bool {
    let explicitly_html = |name: &str| {
        [".html", ".htm", ".xhtml"]
            .iter()
            .any(|extension| name.to_ascii_lowercase().ends_with(extension))
    };
    job.link_fallback.is_none()
        && job.authentication.is_none()
        && job.expected_file.is_none()
        && job.synchronization.is_none()
        && job.plan.is_none()
        && job.downloaded == 0
        && job.saved_path.is_none()
        && (job.server_name_pending || !explicitly_html(&job.name))
}

fn discovered(client: DownloadClient, state: &crate::link_fallback::State) -> Result<Source> {
    let file = state.selected_file().context("This page needs a file selection. In Download Engine, retry the download to review the links.")?;
    Ok(Source::Discovered {
        client: client.with_page(&state.page),
        url: file.url.clone(),
        name: file.name.clone(),
        automatic: state.selection == Some(Selection::Automatic),
    })
}

pub(super) async fn request(
    source: &Source,
    job: &mut Job,
    range: Option<(u64, u64)>,
) -> Result<Response> {
    let result = source.get(source.initial_url(&job.url), range, None).await;
    if result
        .as_ref()
        .is_err_and(|error| error.is::<CrossOriginRedirect>())
        && job.plan.is_none()
        && job.downloaded == 0
        && let Some(state) = &mut job.link_fallback
    {
        state.selection = None;
    }
    let response = result?;
    check_file_response(&response)?;
    if range.is_none() {
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "Server returned HTTP {}",
            response.status().as_u16()
        );
    }
    check_selected_response(source, job, &response)?;
    Ok(response)
}

pub(super) fn check_selected_response(
    source: &Source,
    job: &mut Job,
    response: &Response,
) -> Result<()> {
    if selected_html(source, response) {
        ensure!(
            job.plan.is_none() && job.downloaded == 0,
            "The selected file link now returns a webpage. Use Restart to check the original hosting page again."
        );
        if let Some(state) = &mut job.link_fallback {
            state.selection = None;
        }
        bail!(
            "The selected link returned another webpage. Review the links or open the website in your browser; a direct file link is needed."
        );
    }
    Ok(())
}

pub(super) async fn prepare(
    source: Source,
    store: &Store,
    job: &mut Job,
    cancel: &CancellationToken,
) -> Result<Option<(Source, Response)>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Ok(None),
        result = resolve(source, store, job) => result.map(Some),
    }
}

async fn resolve(mut source: Source, store: &Store, job: &mut Job) -> Result<(Source, Response)> {
    if let Source::Direct(client) = &source
        && let Some(state) = &job.link_fallback
    {
        state.validate(job)?;
        if state.selection != Some(Selection::Page) {
            source = discovered(client.clone(), state)?;
        }
    }
    let mut response = request(&source, job, Some((0, 0))).await?;
    // Without a usable validator the normal pipeline needs a full GET anyway.
    // Inspect that response too: landing sites sometimes label their tiny range
    // probe as a file but return HTML for the full request.
    if matches!(source, Source::Direct(_))
        && eligible(job)
        && response.status() == reqwest::StatusCode::PARTIAL_CONTENT
        && !html(&response)
    {
        identity(&response)?;
        if probe_size(&response)?.is_none() || strong_etag(&response).is_none() {
            drop(response);
            response = request(&source, job, None).await?;
        }
    }
    // Some servers reject a range on a landing page instead of ignoring it.
    if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && job.plan.is_none() {
        drop(response);
        response = request(&source, job, None).await?;
    }
    if !matches!(source, Source::Direct(_))
        || !eligible(job)
        || !response.status().is_success()
        || !html(&response)
    {
        return Ok((source, response));
    }
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        drop(response);
        response = request(&source, job, None).await?;
    }
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "The webpage server did not return a complete page"
    );
    if !html(&response) {
        return Ok((source, response));
    }
    identity(&response)?;
    let page = response.url().clone();
    let bytes = tokio::time::timeout(Duration::from_secs(15), async {
        ensure!(response.content_length().is_none_or(|size| size <= MAX_PAGE_BYTES as u64),
            "The webpage is too large for automatic link discovery (2 MiB limit). Use the website grabber or browser.");
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(bytes.len() + chunk.len() <= MAX_PAGE_BYTES,
                "The webpage is too large for automatic link discovery (2 MiB limit). Use the website grabber or browser.");
            bytes.extend_from_slice(&chunk);
        }
        Ok::<_, anyhow::Error>(bytes)
    }).await.context("Automatic link discovery timed out while reading the webpage")??;
    let text = String::from_utf8(bytes).context(
        "Automatic link discovery needs a UTF-8 webpage. Open this site in your browser.",
    )?;
    job.link_fallback = Some(crate::website_grabber::fallback(page, text));
    store.save(job)?;
    let state = job.link_fallback.as_ref().expect("discovery stored");
    if state.needs_review() {
        if state.files.is_empty() {
            bail!(
                "This webpage exposes no direct file links. Open it in your browser; it may require scripts, login, or a site-specific integration. Retry to review or save the page."
            );
        }
        bail!(
            "Found {} possible file links. In Download Engine, retry this download to choose a file.",
            state.files.len()
        );
    }
    let Source::Direct(client) = source else {
        unreachable!("only direct pages are discovered")
    };
    let source = discovered(client, state)?;
    let response = request(&source, job, Some((0, 0))).await?;
    Ok((source, response))
}

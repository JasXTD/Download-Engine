//! One HTTP transfer pipeline for direct links and resolved hosting links.
use super::*;
mod fallback;

#[derive(Clone)]
pub(crate) enum Source {
    Direct(DownloadClient),
    Provider {
        client: DownloadClient,
        url: String,
        identity: String,
    },
    Discovered {
        client: DownloadClient,
        url: String,
        name: String,
        automatic: bool,
    },
    WeTransfer(crate::wetransfer::Source),
}
impl Source {
    fn initial_url<'a>(&'a self, original: &'a str) -> &'a str {
        match self {
            Self::Discovered { url, .. } | Self::Provider { url, .. } => url,
            _ => original,
        }
    }
    async fn get(
        &self,
        url: &str,
        range: Option<(u64, u64)>,
        etag: Option<&str>,
    ) -> Result<Response> {
        match self {
            Self::WeTransfer(source) => source.get(range, etag).await,
            Self::Provider { client, url, .. } => {
                let mut request = client
                    .get(url)?
                    .same_origin()
                    .header(header::ACCEPT_ENCODING, "identity");
                if let Some((start, end)) = range {
                    request = request.header(header::RANGE, format!("bytes={start}-{end}"));
                }
                if let Some(etag) = etag {
                    request = request.header(header::IF_RANGE, etag);
                }
                request.send().await
            }
            Self::Direct(client) | Self::Discovered { client, .. } => {
                let mut request = client.get(url)?.header(header::ACCEPT_ENCODING, "identity");
                if matches!(
                    self,
                    Self::Discovered {
                        automatic: true,
                        ..
                    }
                ) {
                    request = request.same_origin();
                }
                if let Some((start, end)) = range {
                    request = request.header(header::RANGE, format!("bytes={start}-{end}"));
                }
                if let Some(etag) = etag {
                    request = request.header(header::IF_RANGE, etag);
                }
                request.send().await
            }
        }
    }
    fn identity<'a>(&'a self, response: &'a Response) -> &'a str {
        match self {
            Self::Direct(_) | Self::Discovered { .. } => response.url().as_str(),
            Self::WeTransfer(source) => &source.identity,
            Self::Provider { identity, .. } => identity,
        }
    }
    fn remote(&self, response: &Response) -> RemoteFile {
        let mut remote = RemoteFile::from_response(response);
        remote.final_url = self.identity(response).to_owned();
        remote
    }
    fn resolve_name(&self, job: &mut Job, headers: &header::HeaderMap) {
        let name = match self {
            Self::WeTransfer(source) => Some(&source.name),
            Self::Discovered { name, .. } => Some(name),
            Self::Direct(_) => None,
            Self::Provider { .. } => None,
        };
        if let Some(name) = name
            && job.server_name_pending
            && job.plan.is_none()
            && job.downloaded == 0
            && job.saved_path.is_none()
        {
            job.name = name.clone();
        }
        crate::server_filename::resolve(job, headers);
    }
    pub(super) async fn range(&self, plan: &Plan, start: u64, end: u64) -> Result<Response> {
        let response = self
            .get(&plan.final_url, Some((start, end)), Some(&plan.etag))
            .await?;
        check_file_response(&response)?;
        ensure!(
            !fallback::selected_html(self, &response),
            "The discovered file link returned a webpage during download. Restart to check the hosting page again."
        );
        ensure!(
            response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
            "Server changed the file or ignored a range. Restart to download it safely."
        );
        identity(&response)?;
        ensure!(
            range(&response)? == (start, end, plan.size),
            "Server returned an unexpected byte range"
        );
        ensure!(
            strong_etag(&response).as_deref() == Some(plan.etag.as_str()),
            "The remote file changed during download. Restart to get the new version."
        );
        ensure!(
            self.identity(&response) == plan.final_url,
            "The range request redirected to a different file"
        );
        Ok(response)
    }
}

pub(crate) async fn transfer(
    source: Source,
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    let connections = store.connection_settings()?;
    let Some((source, probe)) = fallback::prepare(source, store, job, cancel).await? else {
        return Ok(None);
    };
    check_file_response(&probe)?;
    synchronization::check_response(job, &probe)?;
    if let Some(expected) = &job.expected_file {
        expected.check(&probe)?;
    }
    if probe.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        identity(&probe)?;
        if let (Some(size), Some(etag)) = (probe_size(&probe)?, strong_etag(&probe)) {
            if let Some(expected) = job.provider.as_ref().and_then(|p| p.plan.as_ref()) {
                ensure!(
                    size == expected.size,
                    "Provider source size differs from its trusted plan"
                );
            }
            job.remote = Some(source.remote(&probe));
            let final_url = source.identity(&probe).to_owned();
            let connection_limit = connections.limit(&job.url, probe.url().as_str())?;
            if let Some(plan) = &job.plan {
                ensure!(
                    plan.size == size && plan.etag == etag && plan.final_url == final_url,
                    "The remote file changed. Use Restart to download the new version."
                );
            } else {
                source.resolve_name(job, probe.headers());
                let count = if size >= MIN_SEGMENT * 4 {
                    (connection_limit as u64).min(size / MIN_SEGMENT)
                } else {
                    1
                };
                let mut segments = Vec::new();
                for i in 0..count {
                    let chunk = size / count;
                    let start = chunk * i;
                    let end = if i + 1 == count {
                        size - 1
                    } else {
                        chunk * (i + 1) - 1
                    };
                    segments.push(Segment { start, end });
                }
                job.plan = Some(Plan {
                    size,
                    etag,
                    final_url,
                    segments,
                });
                job.total = Some(size);
                store.save(job)?;
            }
            drop(probe);
            if !segmented::run(
                &segmented::Source::Http(source),
                store,
                job,
                cancel,
                root,
                limiter,
                connection_limit,
            )
            .await?
            {
                return Ok(None);
            }
            return finalize(store, job, root, cancel, true).await;
        }
    } else if !probe.status().is_success()
        && probe.status() != reqwest::StatusCode::RANGE_NOT_SATISFIABLE
    {
        bail!("Server returned HTTP {}", probe.status().as_u16());
    }
    ensure!(
        job.plan.is_none(),
        "The server no longer supports safe resume. Use Restart to start over."
    );
    // Without a strong validator we never append unverified bytes from another request.
    let response = if probe.status() == reqwest::StatusCode::OK {
        probe
    } else {
        drop(probe);
        tokio::select! {
            _=cancel.cancelled()=>return Ok(None),
            result=fallback::request(&source, job, None)=>result?
        }
    };
    check_file_response(&response)?;
    fallback::check_selected_response(&source, job, &response)?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Server returned HTTP {}",
        response.status().as_u16()
    );
    identity(&response)?;
    synchronization::check_response(job, &response)?;
    if let Some(expected) = &job.expected_file {
        expected.check(&response)?;
    }
    source.resolve_name(job, response.headers());
    if let Some(expected) = job.provider.as_ref().and_then(|p| p.plan.as_ref()) {
        ensure!(
            response
                .content_length()
                .is_none_or(|size| size == expected.size),
            "Provider source size differs from its trusted plan"
        );
    }
    job.total = response.content_length().or_else(|| {
        job.provider
            .as_ref()
            .and_then(|p| p.plan.as_ref())
            .map(|p| p.size)
    });
    job.downloaded = 0;
    job.remote = Some(source.remote(&response));
    store.save(job)?;
    if !single(response, store, job, root, cancel, limiter).await? {
        return Ok(None);
    }
    finalize(store, job, root, cancel, false).await
}

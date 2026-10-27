//! Public WeTransfer share links. The share URL remains the job identity;
//! anonymous session cookies and signed file addresses live only in memory.
mod network;
mod session;

use crate::{bandwidth::Limiter, engine, model::Job, store::Store, synchronization::FinishedFile};
use anyhow::{Context, Result, ensure};
use reqwest::Url;
use std::{path::Path, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) use network::Source;

fn short_host(host: &str) -> bool {
    matches!(
        host,
        "we.tl" | "shorturls.wetransfer.com" | "go.wetransfer.com"
    )
}
fn site_host(host: &str) -> bool {
    host == "wetransfer.com"
        || host.strip_suffix(".wetransfer.com").is_some_and(|label| {
            !label.is_empty()
                && !label.contains('.')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
pub fn is_url(value: &str) -> bool {
    Url::parse(value.trim()).is_ok_and(|url| {
        let host = url.host_str().unwrap_or_default();
        short_host(host)
            || (site_host(host)
                && (url.path() == "/downloads" || url.path().starts_with("/downloads/")))
    })
}

struct Link {
    url: Url,
    transfer: Option<String>,
    recipient: Option<String>,
    hash: Option<String>,
}
impl Link {
    fn parse(value: &str) -> Result<Self> {
        ensure!(value.len() <= 16384, "The WeTransfer link is too long");
        let mut url = Url::parse(value.trim()).context("Invalid WeTransfer share link")?;
        ensure!(
            url.scheme() == "https"
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none(),
            "WeTransfer share links require HTTPS without login details or a custom port"
        );
        let host = url.host_str().unwrap_or_default();
        let path: Vec<_> = url
            .path()
            .trim_end_matches('/')
            .split('/')
            .skip(1)
            .collect();
        let (transfer, recipient, hash) = if short_host(host) {
            ensure!(
                path.len() == 1
                    && !path[0].is_empty()
                    && path[0].len() <= 128
                    && path[0]
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "Enter a complete WeTransfer short link"
            );
            (None, None, None)
        } else {
            ensure!(
                site_host(host) && matches!(path.len(), 3 | 4) && path[0] == "downloads",
                "Use a WeTransfer transfer link; boards and account pages are not supported"
            );
            let hex = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
            let transfer = path[1];
            let hash = path[path.len() - 1];
            let recipient = (path.len() == 4).then(|| path[2]);
            ensure!(
                transfer.len() == 46
                    && hex(transfer)
                    && (4..=12).contains(&hash.len())
                    && hex(hash)
                    && recipient.is_none_or(|s| s.len() == 46 && hex(s)),
                "Invalid WeTransfer transfer identifier or security hash"
            );
            (
                Some(transfer.to_owned()),
                recipient.map(str::to_owned),
                Some(hash.to_owned()),
            )
        };
        // Tracking parameters and page anchors are not part of a transfer.
        url.set_query(None);
        url.set_fragment(None);
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
        Ok(Self {
            url,
            transfer,
            recipient,
            hash,
        })
    }
    fn payload(&self) -> Result<serde_json::Value> {
        let mut body = serde_json::json!({"security_hash": self.hash.as_ref().context("Unresolved WeTransfer short link")?});
        if let Some(recipient) = &self.recipient {
            body["recipient_id"] = recipient.clone().into();
        }
        Ok(body)
    }
    fn identity(&self) -> Result<String> {
        let transfer = self
            .transfer
            .as_deref()
            .context("Unresolved WeTransfer short link")?;
        let hash = self
            .hash
            .as_deref()
            .context("Missing WeTransfer security hash")?;
        let recipient = self
            .recipient
            .as_deref()
            .map(|s| format!("{s}/"))
            .unwrap_or_default();
        Ok(format!(
            "https://wetransfer.com/downloads/{transfer}/{recipient}{hash}"
        ))
    }
}

pub fn canonical_url(value: &str) -> Result<Url> {
    Ok(Link::parse(value)?.url)
}

pub(crate) fn validate_job(job: &Job) -> Result<()> {
    let link = Link::parse(&job.url)?;
    ensure!(
        job.hls.is_none()
            && job.ftp.is_none()
            && job.authentication.is_none()
            && job.synchronization.is_none()
            && job.expected_file.is_none(),
        "WeTransfer share links do not support direct-file login, capture or synchronization state"
    );
    if let Some(plan) = &job.plan {
        let saved = Link::parse(&plan.final_url)?;
        plan.byte_order()?;
        ensure!(
            saved.transfer.is_some()
                && job.total == Some(plan.size)
                && plan.final_url == saved.identity()?
                && (link.transfer.is_none() || link.identity()? == saved.identity()?)
                && plan.etag.starts_with('"')
                && plan.etag.ends_with('"')
                && plan.etag.len() >= 2,
            "Invalid saved WeTransfer ranges. Restart this download."
        );
    }
    Ok(())
}

pub(crate) async fn transfer(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    let network = network::Network::new(store, job.public_only)?;
    transfer_with_network(store, job, root, cancel, limiter, network).await
}

async fn transfer_with_network(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
    network: network::Network,
) -> Result<Option<FinishedFile>> {
    validate_job(job)?;
    let source = tokio::select! { biased;
        _ = cancel.cancelled() => return Ok(None),
        result = tokio::time::timeout(Duration::from_secs(60), network.resolve(&job.url)) =>
            result.map_err(|_| crate::retry::incomplete("WeTransfer link lookup timed out."))??,
    };
    engine::http::transfer(
        engine::http::Source::WeTransfer(source),
        store,
        job,
        root,
        cancel,
        limiter,
    )
    .await
}

#[cfg(test)]
mod tests;

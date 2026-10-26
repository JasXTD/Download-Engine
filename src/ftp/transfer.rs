use super::{Endpoint, client};
use crate::{
    auth, bandwidth::Limiter, download_file, engine, model::Job, retry, store::Store,
    synchronization::FinishedFile,
};
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

pub(crate) async fn transfer(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
) -> Result<Option<FinishedFile>> {
    transfer_with(store, job, root, cancel, limiter, client::connector()?).await
}
pub(super) async fn transfer_with(
    store: &Store,
    job: &mut Job,
    root: &Path,
    cancel: &CancellationToken,
    limiter: &Limiter,
    tls: tokio_native_tls::TlsConnector,
) -> Result<Option<FinishedFile>> {
    let state = job.ftp.as_ref().context("Missing FTP state")?;
    state.validate(&job.url)?;
    let endpoint = Endpoint::new(&job.url, state.security)?;
    let credentials = auth::ftp_credentials(job)?;
    let mut control = client::Client::connect(endpoint, credentials.as_ref(), tls, cancel).await?;
    drop(credentials);
    let metadata = control.metadata(cancel).await?;
    if let Some(previous) = &state.metadata {
        ensure!(
            previous == &metadata,
            "The remote FTP file changed. Use Restart to download the new version."
        );
    }
    let path = root.join("ftp.part");
    let mut file = match download_file::read_write_part(&path).await {
        Ok(file) => file,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            download_file::fresh_read_write(&path).await?
        }
        Err(error) => return Err(error),
    };
    let saved = file.metadata().await?.len();
    ensure!(
        saved == 0 || state.metadata.is_some(),
        "FTP partial data has no saved identity. Use Restart."
    );
    ensure!(
        saved <= i64::MAX as u64 && metadata.size.is_none_or(|size| saved <= size),
        "FTP partial data exceeds the remote file. Use Restart."
    );
    job.downloaded = saved;
    job.total = metadata.size;
    job.server_name_pending = false;
    job.ftp.as_mut().unwrap().metadata = Some(metadata.clone());
    store.save(job)?;
    // SIZE/MDTM are not strong validators. An expected publisher checksum makes
    // REST safe because final publication requires that exact SHA-256. Otherwise
    // compare the entire saved prefix during this same RETR before appending.
    let requested = if job.integrity.expected.is_some() {
        saved
    } else {
        0
    };
    let (mut data, mut received) = control.retrieve(requested, cancel).await?;
    store.activity.set(
        &job.id,
        if received < saved {
            crate::transfer_activity::Phase::CheckingSaved
        } else {
            crate::transfer_activity::Phase::Downloading
        },
    );
    file.seek(std::io::SeekFrom::Start(received)).await?;
    let mut buffer = vec![0; 64 * 1024];
    let mut previous = vec![0; 64 * 1024];
    let mut checkpoint = Instant::now();
    let outcome: Result<bool> = async {
        loop {
            if metadata.size != Some(received) {
                limiter.check_quota().await?;
            }
            let count = client::read(&mut data, &mut buffer, cancel).await?;
            if count == 0 {
                break;
            }
            limiter.record_payload(count).await?;
            let end = received.checked_add(count as u64).context("FTP file size overflow")?;
            ensure!(
                end <= i64::MAX as u64 && metadata.size.is_none_or(|size| end <= size),
                "FTP server sent more bytes than declared"
            );
            let mut pending = &buffer[..count];
            while !pending.is_empty() {
                let Some(n) = limiter.acquire(&job.id, pending.len(), cancel).await else {
                    return Ok(false);
                };
                let verified = saved.saturating_sub(received).min(n as u64) as usize;
                if verified > 0 {
                    file.read_exact(&mut previous[..verified]).await?;
                    ensure!(
                        previous[..verified] == pending[..verified],
                        "FTP saved bytes do not match the remote file. Use Restart; nothing was appended."
                    );
                }
                if verified < n {
                    file.write_all(&pending[verified..n]).await?;
                    job.downloaded += (n - verified) as u64;
                    store.activity.set(&job.id, crate::transfer_activity::Phase::Downloading);
                }
                received += n as u64;
                pending = &pending[n..];
                if checkpoint.elapsed() >= Duration::from_millis(500) {
                    store.save(job)?;
                    checkpoint = Instant::now();
                }
            }
        }
        if metadata.size.is_some_and(|size| received != size) || received < saved {
            return Err(retry::incomplete("FTP transfer ended before all expected bytes arrived."));
        }
        drop(data);
        control.finish(cancel).await?;
        ensure!(
            control.metadata(cancel).await? == metadata,
            "FTP file changed during transfer. Use Restart."
        );
        Ok(true)
    }.await;
    file.sync_all().await?;
    drop(file);
    if !outcome? {
        return Ok(None);
    }
    job.total = Some(job.downloaded);
    engine::finalize_sources(store, job, cancel, vec![path]).await
}

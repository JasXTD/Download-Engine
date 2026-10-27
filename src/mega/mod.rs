//! Public MEGA file links. Share keys never enter network requests; temporary
//! range files contain ciphertext and publication requires the MEGA file MAC.
mod crypto;
mod network;
pub(crate) use network::Source;

use crate::{
    bandwidth::Limiter,
    download_file, engine,
    model::{Job, Plan, Segment},
    store::Store,
    synchronization::FinishedFile,
    transfer_activity::Phase,
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Url;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub fn is_url(value: &str) -> bool {
    Url::parse(value.trim()).is_ok_and(|url| {
        is_host(url.host_str().unwrap_or_default())
            && (matches!(url.path(), "/" | "/file" | "/folder")
                || url.path().starts_with("/file/")
                || url.path().starts_with("/folder/"))
    })
}
fn is_host(host: &str) -> bool {
    matches!(
        host,
        "mega.nz" | "www.mega.nz" | "mega.co.nz" | "www.mega.co.nz" | "mega.io" | "www.mega.io"
    )
}

struct Link {
    handle: String,
    key: Zeroizing<[u8; 32]>,
    canonical: Url,
}
impl Link {
    fn parse(value: &str) -> Result<Self> {
        ensure!(value.len() <= 16384, "The MEGA link is too long");
        let url = Url::parse(value.trim()).context("Invalid MEGA link")?;
        ensure!(
            is_host(url.host_str().unwrap_or_default())
                && url.scheme() == "https"
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none(),
            "Use a complete HTTPS MEGA public file link without login details or query parameters"
        );
        let fragment = url.fragment().unwrap_or_default();
        let (handle, key) = if let Some(handle) = url.path().strip_prefix("/file/") {
            ensure!(
                !handle.contains('/'),
                "MEGA folder links are not supported. Copy an individual public file link."
            );
            (handle, fragment)
        } else if url.path() == "/" && fragment.starts_with('!') {
            fragment[1..]
                .split_once('!')
                .context("MEGA file links need a decryption key")?
        } else {
            bail!(
                "Use a MEGA public file link. Folder, account and password-protected links are not supported."
            );
        };
        ensure!(
            handle.len() == 8 && URL_SAFE_NO_PAD.decode(handle).is_ok_and(|b| b.len() == 6),
            "Invalid MEGA file identifier"
        );
        let decoded = Zeroizing::new(URL_SAFE_NO_PAD.decode(key).map_err(|_| {
            anyhow::anyhow!("MEGA file links need a valid 43-character decryption key")
        })?);
        ensure!(
            decoded.len() == 32,
            "MEGA file links need a valid 43-character decryption key"
        );
        let mut secret = Zeroizing::new([0; 32]);
        secret.copy_from_slice(&decoded);
        let canonical = Url::parse(&format!("https://mega.nz/file/{handle}#{key}"))?;
        Ok(Self {
            handle: handle.into(),
            key: secret,
            canonical,
        })
    }
    fn aes_key(&self) -> Zeroizing<[u8; 16]> {
        let mut key = Zeroizing::new([0; 16]);
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = self.key[i] ^ self.key[i + 16];
        }
        key
    }
    fn nonce(&self) -> [u8; 8] {
        self.key[16..24].try_into().expect("Fixed key length")
    }
    fn mac(&self) -> [u8; 8] {
        self.key[24..32].try_into().expect("Fixed key length")
    }
    fn identity(&self) -> String {
        format!("mega-v1:{:x}", Sha256::digest(self.canonical.as_str()))
    }
}

pub fn canonical_url(value: &str) -> Result<Url> {
    Ok(Link::parse(value)?.canonical)
}

pub(crate) fn validate_job(job: &Job) -> Result<()> {
    let link = Link::parse(&job.url)?;
    ensure!(
        job.hls.is_none()
            && job.ftp.is_none()
            && job.authentication.is_none()
            && job.synchronization.is_none()
            && job.expected_file.is_none()
            && job.remote.is_none(),
        "MEGA public file downloads cannot use direct-file login, capture or synchronization state"
    );
    if let Some(plan) = &job.plan {
        plan.byte_order()?;
        ensure!(
            plan.final_url == link.canonical.as_str()
                && plan.etag == link.identity()
                && job.total == Some(plan.size),
            "Invalid saved MEGA ranges. Restart this download."
        );
    }
    // Only a MAC-verified plaintext file can have publication intent.
    ensure!(
        job.saved_path.is_none() || job.integrity.actual.is_some(),
        "Missing MEGA verification receipt. Restart this download."
    );
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
    crypto::available()?;
    let link = Link::parse(&job.url)?;
    let metadata = tokio::select! {
        _ = cancel.cancelled() => return Ok(None),
        result = tokio::time::timeout(std::time::Duration::from_secs(45), network.resolve(&link)) =>
            result.map_err(|_| crate::retry::incomplete("MEGA metadata request timed out."))??,
    };
    let limit = store
        .connection_settings()?
        .limit(&job.url, metadata.url.as_str())?;
    if let Some(plan) = &job.plan {
        ensure!(
            plan.size == metadata.size,
            "The MEGA file size changed. Restart this download."
        );
    } else {
        ensure!(
            job.downloaded == 0,
            "Missing MEGA range plan. Restart this download."
        );
        if job.server_name_pending {
            job.name = crate::model::safe_filename(&metadata.name);
        }
        job.server_name_pending = false;
        job.total = Some(metadata.size);
        if metadata.size > 0 {
            let count = metadata
                .size
                .div_ceil(2 * 1024 * 1024)
                .min(limit as u64)
                .max(1) as usize;
            let span = metadata.size / count as u64;
            job.plan = Some(Plan {
                size: metadata.size,
                etag: link.identity(),
                final_url: link.canonical.to_string(),
                segments: (0..count)
                    .map(|i| Segment {
                        start: span * i as u64,
                        end: if i + 1 == count {
                            metadata.size - 1
                        } else {
                            span * (i as u64 + 1) - 1
                        },
                    })
                    .collect(),
            });
        }
        store.save(job)?;
    }
    if metadata.size > 0 {
        let source = engine::segmented::Source::Mega(Source {
            network,
            url: metadata.url,
        });
        if !engine::segmented::run(&source, store, job, cancel, root, limiter, limit).await? {
            return Ok(None);
        }
    }
    store.activity.set(&job.id, Phase::Verifying);
    let output_path = root.join("mega-plaintext.part");
    let sources = if let Some(plan) = &job.plan {
        plan.byte_order()?
            .into_iter()
            .map(|i| {
                (
                    engine::part_path(root, i),
                    plan.segments[i].end - plan.segments[i].start + 1,
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let Some(hash) = decrypt_to_staging(link, sources, &output_path, metadata.size, cancel).await?
    else {
        return Ok(None);
    };
    let result =
        engine::finalize_verified_source(store, job, cancel, output_path.clone(), &hash).await;
    // Successful publication keeps this name as a recovery anchor until the
    // completed job is durable. Every other outcome retains only ciphertext.
    if !matches!(&result, Ok(Some(_))) {
        discard_plaintext(&output_path).await?;
    }
    result
}

async fn decrypt_to_staging(
    link: Link,
    sources: Vec<(std::path::PathBuf, u64)>,
    output_path: &Path,
    size: u64,
    cancel: &CancellationToken,
) -> Result<Option<String>> {
    let output = download_file::fresh(output_path).await?.into_std().await;
    let token = cancel.clone();
    // Always await the blocking worker, including after cancellation. Its file
    // handles and writes must end before plaintext cleanup or a later resume.
    let result =
        tokio::task::spawn_blocking(move || decrypt_file(link, sources, output, size, &token))
            .await;
    match result {
        Ok(Ok(Some(hash))) => Ok(Some(hash)),
        Ok(Ok(None)) => {
            discard_plaintext(output_path).await?;
            Ok(None)
        }
        Ok(Err(error)) => {
            discard_plaintext(output_path).await?;
            Err(error)
        }
        Err(error) => {
            discard_plaintext(output_path).await?;
            Err(error.into())
        }
    }
}

async fn discard_plaintext(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Cannot remove unverified MEGA plaintext"),
    }
}

fn decrypt_file(
    link: Link,
    sources: Vec<(std::path::PathBuf, u64)>,
    output: std::fs::File,
    size: u64,
    cancel: &CancellationToken,
) -> Result<Option<String>> {
    let mut files = Vec::new();
    let mut saved = 0;
    for (path, length) in sources {
        let file = download_file::read_part_sync(&path)?;
        ensure!(
            file.metadata()?.len() == length,
            "MEGA partial file size changed. Restart this download."
        );
        saved += length;
        files.push(file);
    }
    ensure!(saved == size, "MEGA partial data is incomplete");
    let mut input = Parts { files, index: 0 };
    let mut output = std::io::BufWriter::new(output);
    let crypto = crypto::Cipher::new(&link.aes_key(), link.nonce())?;
    let mut hash = Sha256::new();
    let mut file_mac = [0; 16];
    let mut buffer = Zeroizing::new(vec![0; 1024 * 1024]);
    let mut offset = 0;
    let mut chunk_size = 128 * 1024;
    while offset < size {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let len = (size - offset).min(chunk_size as u64) as usize;
        input.read_exact(&mut buffer[..len])?;
        crypto.ctr(&mut buffer[..len], offset)?;
        let mac = crypto.chunk_mac(&buffer[..len])?;
        crypto.fold_mac(&mut file_mac, &mac)?;
        hash.update(&buffer[..len]);
        output.write_all(&buffer[..len])?;
        offset += len as u64;
        chunk_size = (chunk_size + 128 * 1024).min(1024 * 1024);
    }
    if cancel.is_cancelled() {
        return Ok(None);
    }
    ensure!(
        crypto::condense_mac(&file_mac) == link.mac(),
        "MEGA integrity check failed. The key is incorrect or the file is damaged. Restart this download."
    );
    output.flush()?;
    output.get_ref().sync_all()?;
    Ok(Some(format!("{:x}", hash.finalize())))
}

struct Parts {
    files: Vec<std::fs::File>,
    index: usize,
}
impl Read for Parts {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        while let Some(file) = self.files.get_mut(self.index) {
            let n = file.read(buffer)?;
            if n > 0 {
                return Ok(n);
            }
            self.index += 1;
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests;

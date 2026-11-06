//! Lua providers own source resolution and byte transforms; the core owns all files.
#[cfg(any(windows, unix))]
mod crypto;
mod handoff;
mod keys;
pub mod media;
mod persistence;
pub(crate) mod primitives;
pub(crate) mod process;
mod runtime;
#[cfg(test)]
mod tests;
mod transfer;

use crate::{
    model::Job,
    script_automation::{Language, Script, Trigger},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) use keys::seal_key;
pub use persistence::FrozenScript;
pub(crate) use persistence::upgrade;
pub use runtime::worker_main;
pub(crate) use transfer::transform;
pub const VERSION: u32 = 1;
pub const CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_CALLBACK_SECONDS: u32 = 60;
pub const EXAMPLE: &str = include_str!("../../examples/download-provider.lua");

const FUNCTIONS: &[crate::script_automation::api::Function] = &[
    crate::script_automation::api::Function {
        name: "xd.binary_setting",
        arguments: "id",
        description: "Extension web jobs only: read an explicitly imported, OS-protected binary credential. No arbitrary filesystem access.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.rsa_key_setting",
        arguments: "id",
        description: "Extension web jobs only: obtain an opaque handle to an explicitly imported, OS-protected RSA private key.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.handoff_session",
        arguments: "name, session",
        description: "Resolver only: pass an exact-origin HTTP session to media segments referencing session=name. Encrypted, run-local and never given to processors.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.random_bytes",
        arguments: "count",
        description: "Return 1-4096 bytes from the operating system's cryptographic random source.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.digest",
        arguments: "algorithm, bytes",
        description: "Return a binary SHA-1 or SHA-256 digest (sha1 or sha256); input at most 1 MiB.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.hmac",
        arguments: "algorithm, key, bytes",
        description: "Binary HMAC-SHA1 or HMAC-SHA256. Message at most 1 MiB; key at most 64 KiB.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.rsa_private_key",
        arguments: "bytes",
        description: "Import an unencrypted 2048-4096-bit RSA private key (PKCS#1/PKCS#8 PEM or DER) as an opaque handle. No filesystem access.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.rsa_sign",
        arguments: "key_handle, algorithm, bytes",
        description: "Sign a message with pss-sha1, pss-sha256 or pkcs1-sha256. PSS uses a digest-length salt; input at most 1 MiB.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.rsa_decrypt",
        arguments: "key_handle, algorithm, ciphertext",
        description: "RSA OAEP decryption with oaep-sha1 or oaep-sha256, matching MGF1 digest and empty label. Returns binary plaintext.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.aes_decrypt_blocks",
        arguments: "key_or_handle, blocks",
        description: "Decrypt complete AES blocks without padding, for modes implemented in Lua. At most 256 KiB plus one block.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.media_plan",
        arguments: "plan",
        description: "Validate a finite media plan: ordered segments, up to 16 video/audio/subtitle tracks, and MKV or MP4 muxing. Sets kind=media and media_api=1; performs no downloads.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.handoff_key",
        arguments: "name, bytes",
        description: "During resolve/refresh only: seal a 16, 24 or 32-byte AES key for this run's offline processor. Never adds it to the saved plan. Up to 64 named keys.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.session_key",
        arguments: "name",
        description: "Processor only: obtain an opaque AES key handle handed off by this run's resolver. Keys are reacquired on resume and are never returned as plaintext.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.key",
        arguments: "protected_token",
        description: "Processor only: unlock a provider-bound OS-protected AES key as an opaque handle. Key bytes are never returned to Lua.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.aes_ctr",
        arguments: "key_or_handle, counter_hex, bytes, offset",
        description: "AES-128/192/256 CTR over a binary chunk, at its absolute byte offset. Counter is 16 bytes in big-endian order.",
        access: crate::script_automation::api::Access::Reader,
    },
    crate::script_automation::api::Function {
        name: "xd.aes_blocks",
        arguments: "key_or_handle, blocks",
        description: "AES encrypts complete 16-byte blocks, without padding, for custom Lua constructions. At most 256 KiB plus one block.",
        access: crate::script_automation::api::Access::Reader,
    },
];
pub fn functions() -> impl Iterator<Item = &'static crate::script_automation::api::Function> {
    crate::script_automation::api::FUNCTIONS
        .iter()
        .filter(|f| {
            matches!(
                f.name,
                "xd.supports"
                    | "xd.fetch"
                    | "xd.fetch_bytes"
                    | "xd.http_session"
                    | "xd.json"
                    | "xd.to_json"
                    | "xd.url_join"
                    | "xd.url_encode"
                    | "xd.url_decode"
                    | "xd.html_select"
            )
        })
        .chain(FUNCTIONS.iter())
}
pub fn supports(name: &str) -> bool {
    functions().any(|f| f.name == name)
}
pub(crate) fn protect_profile(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute("UPDATE profile_format SET data_version=MAX(data_version,3),min_writer_version=MAX(min_writer_version,3) WHERE id=1", [])?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub url: String,
    pub size: u64,
    pub input_sha256: String,
    pub output_sha256: String,
    pub transform_id: String,
}
impl Plan {
    pub fn validate(&self) -> Result<()> {
        crate::model::validate_url(&self.url)?;
        self.content().validate()
    }
    pub fn content(&self) -> Content {
        Content {
            size: self.size,
            input_sha256: self.input_sha256.clone(),
            output_sha256: self.output_sha256.clone(),
            transform_id: self.transform_id.clone(),
        }
    }
    pub fn same_content(&self, other: &Self) -> bool {
        self.content() == other.content()
    }
}

/// Only stable, non-secret identity crosses into storage and processor workers.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Content {
    pub size: u64,
    pub input_sha256: String,
    pub output_sha256: String,
    pub transform_id: String,
}
impl<'de> Deserialize<'de> for Content {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Saved {
            // Read old plans without retaining their signed URL, even in memory.
            #[serde(default, rename = "url")]
            _legacy_url: serde::de::IgnoredAny,
            size: u64,
            input_sha256: String,
            output_sha256: String,
            transform_id: String,
        }
        let saved = Saved::deserialize(deserializer)?;
        let content = Self {
            size: saved.size,
            input_sha256: saved.input_sha256,
            output_sha256: saved.output_sha256,
            transform_id: saved.transform_id,
        };
        content.validate().map_err(serde::de::Error::custom)?;
        Ok(content)
    }
}
impl Content {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.size > 0 && self.size <= i64::MAX as u64,
            "Invalid provider file size"
        );
        for hash in [&self.input_sha256, &self.output_sha256] {
            ensure!(
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
                "Provider hashes must be 64 lowercase hex characters"
            );
        }
        ensure!(
            !self.transform_id.is_empty()
                && self.transform_id.len() <= 128
                && !self.transform_id.chars().any(char::is_control),
            "Invalid provider transform_id"
        );
        Ok(())
    }
}

/// Frozen at explicit submission. Editing/deleting the library entry cannot alter a job.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub api_version: u32,
    pub script: FrozenScript,
    pub plan: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<media::Plan>,
}
impl State {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.api_version == VERSION,
            "Unsupported provider API version"
        );
        self.script.validate()?;
        ensure!(
            self.script.language == Language::Lua
                && self.script.trigger == Trigger::Provider
                && self.script.revision > 0,
            "Invalid saved download provider"
        );
        if let Some(plan) = &self.plan {
            plan.validate()?;
        }
        ensure!(
            self.plan.is_none() || self.media.is_none(),
            "Provider cannot contain both file and media plans"
        );
        if let Some(media) = &self.media {
            media.validate(false)?;
        }
        Ok(())
    }
    pub(crate) fn binding(&self) -> Result<String> {
        let content = self.plan.as_ref().context("Missing provider plan")?;
        // Preserve the original checkpoint binding, whose URL was always empty.
        let plan = Plan {
            url: String::new(),
            size: content.size,
            input_sha256: content.input_sha256.clone(),
            output_sha256: content.output_sha256.clone(),
            transform_id: content.transform_id.clone(),
        };
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                VERSION,
                &self.script.body,
                self.script.revision,
                plan,
                CHUNK_BYTES
            ))?)
        ))
    }
}

impl Store {
    pub fn add_provider_download(
        &self,
        script_id: &str,
        revision: u64,
        url: &str,
        name: Option<&str>,
        queue: &str,
        request: &str,
    ) -> Result<Job> {
        crate::model::validate_url(url)?;
        let mut script = self.script(script_id)?;
        ensure!(
            script.revision == revision,
            "The script changed. Reload it before adding a download."
        );
        self.freeze_extension(&mut script)?;
        let state = State {
            api_version: VERSION,
            script: FrozenScript::new(script)?,
            plan: None,
            media: None,
        };
        state.validate()?;
        let mut job = self.new_download(url, name, None, format!("provider:{request}"))?;
        ensure!(
            job.hls.is_none()
                && job.ftp.is_none()
                && !crate::mega::is_url(url)
                && !crate::wetransfer::is_url(url),
            "Use the built-in downloader for this URL"
        );
        job.server_name_pending = false;
        job.provider = Some(state);
        Ok(self.enqueue_batch(&[job], queue)?.remove(0))
    }
}

/// Starts a restricted worker and resolves/refreshes before touching remote bytes.
pub(crate) enum Source {
    File(crate::engine::http::Source),
    Media(Box<(media::Plan, handoff::Keys)>),
}
pub(crate) enum Resolution {
    File(Plan),
    Media(media::Plan),
}
impl Resolution {
    pub(crate) fn parse(value: serde_json::Value) -> Result<Self> {
        if value.get("kind").is_some() {
            let plan: media::Plan = serde_json::from_value(value)?;
            plan.validate(true)?;
            Ok(Self::Media(plan))
        } else {
            let plan: Plan = serde_json::from_value(value)?;
            plan.validate()?;
            Ok(Self::File(plan))
        }
    }
}
pub(crate) async fn resolve(
    store: &Store,
    job: &mut Job,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Source> {
    let state = job.provider.clone().context("Missing provider")?;
    let extension = store.extension_context(&state.script, crate::extensions::Reach::Web)?;
    let limit = extension
        .as_ref()
        .filter(|context| context.limit_results)
        .map(|context| context.hosts.clone());
    let paths = store.paths.clone();
    let input = job.url.clone();
    let token = cancel.clone();
    let (plan, handoff) = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut worker = process::Worker::start(
            &paths,
            &state.script,
            process::Role::Resolver,
            extension.as_ref(),
            &token,
        )?;
        let previous = if let Some(media) = &state.media {
            serde_json::to_value(media)?
        } else {
            serde_json::to_value(&state.plan)?
        };
        let plan = worker.resolve_any(&input, previous, &token)?;
        Ok((plan, worker.take_handoff()?))
    })
    .await??;
    let plan = match plan {
        Resolution::Media(plan) => {
            ensure!(
                job.provider.as_ref().unwrap().plan.is_none(),
                "Provider changed from file to media. Restart this download."
            );
            if let Some(previous) = &job.provider.as_ref().unwrap().media {
                ensure!(
                    previous == &plan.content(),
                    "Provider media content changed. Restart this download."
                );
            }
            for segment in plan.tracks.iter().flat_map(|t| &t.segments) {
                if let Some(hosts) = &limit {
                    ensure!(
                        crate::extensions::url_allowed(hosts, &segment.url),
                        "An extension with a saved secret can only return addresses on its declared hosts"
                    );
                }
                ensure!(
                    crate::model::validate_url(&job.url)?.scheme() != "https"
                        || crate::model::validate_url(&segment.url)?.scheme() == "https",
                    "Media providers cannot downgrade HTTPS to HTTP"
                );
            }
            job.provider.as_mut().unwrap().media = Some(plan.content());
            let stem = std::path::Path::new(&job.name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("media");
            job.name = crate::model::safe_filename(&format!("{stem}.{}", plan.extension()));
            store.checkpoint(job).await?;
            return Ok(Source::Media(Box::new((plan, handoff))));
        }
        Resolution::File(plan) => {
            ensure!(
                job.provider.as_ref().unwrap().media.is_none(),
                "Provider changed from media to file. Restart this download."
            );
            plan
        }
    };
    if let Some(previous) = &job.provider.as_ref().unwrap().plan {
        ensure!(
            previous == &plan.content(),
            "Provider content changed. Restart this download."
        );
    }
    if let Some(hosts) = &limit {
        ensure!(
            crate::extensions::url_allowed(hosts, &plan.url),
            "An extension with a saved secret can only return addresses on its declared hosts"
        );
    }
    // The explicitly entered origin is authorized. Other origins stay public-only.
    // Provider file requests never follow cross-origin redirects.
    job.public_only = crate::auth::origin(&job.url)? != crate::auth::origin(&plan.url)?;
    let client = crate::auth::DownloadClient::for_replacement(store, job, &plan.url)?;
    let source = crate::engine::http::Source::Provider {
        client,
        url: plan.url.clone(),
        identity: job.url.clone(),
    };
    job.provider.as_mut().unwrap().plan = Some(plan.content());
    store.checkpoint(job).await?;
    Ok(Source::File(source))
}

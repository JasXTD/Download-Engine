//! Offline experiment, intentionally separate from production download code.
mod crypto;
mod fixture;
mod runtime;
#[cfg(test)]
mod tests;
mod transfer;

use anyhow::{Result, ensure};
pub use fixture::Fixture;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};
#[cfg(test)]
use transfer::Report;
pub use transfer::{Options, State, run};
use zeroize::Zeroizing;

#[cfg(test)]
pub const SAMPLE: &str = include_str!("../lua-provider.lua");
pub const MAX_FILE: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub source: String,
    pub size: u64,
    pub input_sha256: String,
    pub output_sha256: String,
    pub key_id: String,
    pub counter: String,
    pub transform_id: String,
}

impl Plan {
    fn validate(&self) -> Result<()> {
        for value in [&self.source, &self.key_id, &self.transform_id] {
            ensure!(
                !value.is_empty()
                    && value.len() <= 80
                    && value
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
                "Invalid provider identifier"
            );
        }
        ensure!(
            self.size > 0 && self.size <= MAX_FILE,
            "Lab source must be 1 byte to 64 MiB"
        );
        for hash in [&self.input_sha256, &self.output_sha256] {
            ensure!(
                hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid SHA-256"
            );
        }
        ensure!(
            crypto::unhex(&self.counter)?.len() == 16,
            "Counter must be 16 bytes"
        );
        Ok(())
    }

    fn identity(&self) -> Self {
        let mut content = self.clone();
        content.source.clear(); // A renewable reference is not content identity.
        content
    }
}

#[derive(Clone)]
pub struct Grants {
    pub metadata: Plan,
    pub sources: BTreeMap<String, PathBuf>,
    pub keys: BTreeMap<String, Zeroizing<Vec<u8>>>,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

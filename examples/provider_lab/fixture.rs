use super::{Grants, Plan, crypto, sha256};
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, fs, path::Path};
use zeroize::Zeroizing;

pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub grants: Grants,
    pub plaintext: Vec<u8>,
}
impl Fixture {
    pub fn new(size: usize) -> Result<Self> {
        ensure!(
            size > 0 && size <= super::MAX_FILE as usize,
            "Invalid sample size"
        );
        let temp = tempfile::Builder::new()
            .prefix("download-engine-provider-lab-")
            .tempdir()?;
        // Public test material only. Never use this fixed key/counter for real data.
        let key = Zeroizing::new(crypto::unhex(
            "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
        )?);
        let counter = "000102030405060708090a0b0c0d0e0f";
        let initial: [u8; 16] = crypto::unhex(counter)?.try_into().unwrap();
        let mut plaintext = b"Download Engine offline provider sample. Harmless generated data.\n"
            .repeat(size.div_ceil(64));
        plaintext.resize(size, 0);
        for (i, byte) in plaintext.iter_mut().enumerate().skip(128) {
            *byte = (i % 256) as u8;
        }
        let mut encrypted = Vec::with_capacity(size);
        for (i, chunk) in plaintext.chunks(crypto::MAX_CHUNK).enumerate() {
            encrypted.extend(crypto::ctr(
                &key,
                &initial,
                chunk,
                (i * crypto::MAX_CHUNK) as u64,
            )?);
        }
        let path = temp.path().join("sample.encrypted.bin");
        fs::write(&path, &encrypted)?;
        fs::create_dir(temp.path().join("run"))?;
        let plan = Plan {
            source: "sample-v1".into(),
            size: size as u64,
            input_sha256: sha256(&encrypted),
            output_sha256: sha256(&plaintext),
            key_id: "sample-key".into(),
            counter: counter.into(),
            transform_id: "aes-ctr-v1".into(),
        };
        Ok(Self {
            temp,
            plaintext,
            grants: Grants {
                metadata: plan,
                sources: BTreeMap::from([
                    ("sample-v1".into(), path.clone()),
                    ("sample-v2".into(), path),
                ]),
                keys: BTreeMap::from([("sample-key".into(), key)]),
            },
        })
    }
    pub fn root(&self) -> std::path::PathBuf {
        self.temp.path().join("run")
    }
    pub fn input(&self) -> &Path {
        self.grants.sources.values().next().expect("Fixture source")
    }
}

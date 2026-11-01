//! One connection secret per profile, encrypted for the current Windows user.
use crate::paths::AppPaths;
use anyhow::{Context as _, Result, ensure};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
};
use uuid::Uuid;
use zeroize::Zeroizing;

const ENTROPY: Uuid = Uuid::from_u128(0x5eea8729_bf18_4b35_9449_10d44153c86d);

pub(super) fn token(paths: &AppPaths) -> Result<Zeroizing<String>> {
    std::fs::create_dir_all(&paths.data)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(paths.data.join("mcp-token.lock"))?;
    // Another process may be preparing the same profile. Bound the wait so
    // starting/stopping an app-owned server cannot hang on an abandoned lock.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(error) => return Err(error).context("MCP token is busy or unavailable; try again"),
        }
    }
    let path = paths.data.join("mcp-token.dpapi");
    let bytes = match std::fs::File::open(&path) {
        Ok(file) => {
            let mut encrypted = Vec::new();
            file.take(4097).read_to_end(&mut encrypted)?;
            ensure!(encrypted.len() <= 4096, "Invalid MCP token file size");
            crate::protected_data::open(ENTROPY, &encrypted).map_err(|_| anyhow::anyhow!(
                "Cannot unlock this profile's MCP token. It may be damaged or belong to another Windows account. See Setup guide for token recovery."
            ))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut bytes = Zeroizing::new(
                format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()).into_bytes(),
            );
            let encrypted = crate::protected_data::seal(ENTROPY, &mut bytes).map_err(|_| {
                anyhow::anyhow!("Cannot protect the MCP token with this Windows account")
            })?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(&encrypted)?;
            file.sync_all()?;
            bytes
        }
        Err(error) => return Err(error.into()),
    };
    ensure!(
        bytes.len() == 64 && bytes.iter().all(u8::is_ascii_hexdigit),
        "Invalid MCP token; stop the server and reset its token file"
    );
    Ok(Zeroizing::new(std::str::from_utf8(&bytes)?.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn token_lock_times_out_and_recovers_after_release() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        };
        std::fs::create_dir_all(&paths.data).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.data.join("mcp-token.lock"))
            .unwrap();
        lock.lock().unwrap();
        let started = std::time::Instant::now();
        let error = token(&paths).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("MCP token is busy or unavailable")
        );
        let elapsed = started.elapsed();
        assert!(elapsed >= std::time::Duration::from_secs(2));
        assert!(elapsed < std::time::Duration::from_secs(4));
        assert!(!paths.data.join("mcp-token.dpapi").exists());
        lock.unlock().unwrap();
        let first = token(&paths).unwrap();
        assert_eq!(*first, *token(&paths).unwrap());
    }

    #[test]
    #[cfg(windows)]
    fn concurrent_token_openers_share_one_protected_secret() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        };
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let paths = paths.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    token(&paths).unwrap()
                })
            })
            .collect();
        let secrets: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert!(secrets.iter().all(|secret| **secret == *secrets[0]));
        assert_eq!(*token(&paths).unwrap(), *secrets[0]);
    }

    #[test]
    #[cfg(windows)]
    fn token_is_stable_encrypted_and_corruption_is_not_silently_replaced() {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        };
        let first = token(&paths).unwrap();
        assert_eq!(*first, *token(&paths).unwrap());
        let path = paths.data.join("mcp-token.dpapi");
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.windows(first.len()).any(|v| v == first.as_bytes()));
        std::fs::write(&path, b"corrupt").unwrap();
        assert!(token(&paths).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"corrupt");
    }
}

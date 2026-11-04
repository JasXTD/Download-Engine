//! AES-GCM envelopes with a random master key kept in the user's OS keychain.
//! A locked/unavailable keychain is an error; secrets never fall back to disk.
use super::*;
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, OsRng, Payload, rand_core::RngCore},
};
use anyhow::Context;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

const MAGIC: &[u8; 4] = b"DEU1";

fn master_key(create: bool) -> Result<Zeroizing<Vec<u8>>> {
    let root = crate::paths::user_data_root()?.join("Download Engine");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("secret-storage.lock"))?;
    lock.lock()
        .context("Cannot lock the system keychain setup")?;
    let entry = crate::credential_vault::native_entry(
        "com.xetro.downloadengine.protected-data",
        "master-key-v1",
    )?;
    let key = match entry.get_secret() {
        Ok(bytes) => Zeroizing::new(bytes),
        Err(keyring_core::Error::NoEntry) if create => {
            let mut bytes = Zeroizing::new(vec![0; 32]);
            OsRng
                .try_fill_bytes(&mut bytes)
                .map_err(|_| anyhow::anyhow!("OS randomness is unavailable"))?;
            entry
                .set_secret(&bytes)
                .context("Unlock your system keychain to save protected data")?;
            bytes
        }
        Err(error) => {
            return Err(error).context("Unlock your system keychain to access protected data");
        }
    };
    ensure!(
        key.len() == 32,
        "The protected-data key in the system keychain is invalid"
    );
    Ok(key)
}

pub(super) fn transform(id: Uuid, bytes: &mut [u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        bytes.len() <= 512 * 1024 + 32,
        "Invalid protected data size"
    );
    // Do not create a new master key when opening an incompatible Windows blob.
    if !encrypt {
        ensure!(
            bytes.starts_with(MAGIC),
            "This saved secret belongs to another operating system. Supply it again."
        );
    }
    let key = master_key(encrypt)?;
    envelope(&key, id, bytes, encrypt)
}

fn envelope(key: &[u8], id: Uuid, bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| anyhow::anyhow!("Invalid storage key"))?;
    if encrypt {
        ensure!(bytes.len() <= 512 * 1024, "Invalid protected data size");
        let mut nonce = [0; 12];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| anyhow::anyhow!("OS randomness is unavailable"))?;
        let ciphertext = cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: bytes,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("Cannot protect saved data"))?;
        let mut output = Zeroizing::new(MAGIC.to_vec());
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    } else {
        ensure!(
            bytes.len() >= 32 && bytes.starts_with(MAGIC),
            "Invalid protected data envelope"
        );
        let plain = cipher
            .decrypt(
                (&bytes[4..16]).into(),
                Payload {
                    msg: &bytes[16..],
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| {
                anyhow::anyhow!("Saved data was modified or belongs to another account or purpose")
            })?;
        Ok(Zeroizing::new(plain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_envelopes_bind_account_and_purpose_and_reject_corruption() {
        let id = Uuid::new_v4();
        let key = [37; 32];
        let plain = b"synthetic protected payload";
        let first = envelope(&key, id, plain, true).unwrap();
        let second = envelope(&key, id, plain, true).unwrap();
        assert_ne!(*first, *second);
        assert!(!first.windows(plain.len()).any(|value| value == plain));
        assert_eq!(&*envelope(&key, id, &first, false).unwrap(), plain);
        assert!(envelope(&[42; 32], id, &first, false).is_err());
        assert!(envelope(&key, Uuid::new_v4(), &first, false).is_err());
        for index in 0..first.len() {
            let mut changed = first.to_vec();
            changed[index] ^= 1;
            assert!(envelope(&key, id, &changed, false).is_err());
        }
        for length in 0..32 {
            assert!(envelope(&key, id, &first[..length], false).is_err());
        }
    }
}

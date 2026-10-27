//! MEGA's file format uses AES-CTR and chunk CBC-MAC. AES itself is provided by
//! Windows CNG; no cryptographic primitive or third-party program is embedded.
use anyhow::{Result, ensure};

pub(super) fn available() -> Result<()> {
    ensure!(
        cfg!(windows),
        "MEGA downloads currently require Windows cryptography support"
    );
    Ok(())
}

pub(super) struct Cipher {
    ecb: backend::Aes,
    cbc: backend::Aes,
    nonce: [u8; 8],
}
impl Cipher {
    pub(super) fn new(key: &[u8; 16], nonce: [u8; 8]) -> Result<Self> {
        Ok(Self {
            ecb: backend::Aes::new(key, false)?,
            cbc: backend::Aes::new(key, true)?,
            nonce,
        })
    }
    pub(super) fn attributes(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        self.cbc.crypt(bytes, Some(&mut [0; 16]), true)
    }
    pub(super) fn ctr(&self, bytes: &mut [u8], offset: u64) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let skip = (offset % 16) as usize;
        let mut counters = vec![0; (skip + bytes.len()).div_ceil(16) * 16];
        for (i, block) in counters.as_chunks_mut::<16>().0.iter_mut().enumerate() {
            block[..8].copy_from_slice(&self.nonce);
            block[8..].copy_from_slice(&(offset / 16 + i as u64).to_be_bytes());
        }
        let stream = zeroize::Zeroizing::new(self.ecb.crypt(&counters, None, false)?);
        for (byte, mask) in bytes.iter_mut().zip(&stream[skip..]) {
            *byte ^= mask;
        }
        Ok(())
    }
    pub(super) fn chunk_mac(&self, bytes: &[u8]) -> Result<[u8; 16]> {
        ensure!(!bytes.is_empty(), "Invalid empty MEGA MAC chunk");
        let mut padded = zeroize::Zeroizing::new(vec![0; bytes.len().div_ceil(16) * 16]);
        padded[..bytes.len()].copy_from_slice(bytes);
        let mut iv = [0; 16];
        iv[..8].copy_from_slice(&self.nonce);
        iv[8..].copy_from_slice(&self.nonce);
        let encrypted = self.cbc.crypt(&padded, Some(&mut iv), false)?;
        Ok(encrypted[encrypted.len() - 16..]
            .try_into()
            .expect("AES block"))
    }
    pub(super) fn fold_mac(&self, file: &mut [u8; 16], chunk: &[u8; 16]) -> Result<()> {
        for (a, b) in file.iter_mut().zip(chunk) {
            *a ^= b;
        }
        let folded = self.ecb.crypt(file, None, false)?;
        file.copy_from_slice(&folded);
        Ok(())
    }
}
pub(super) fn condense_mac(mac: &[u8; 16]) -> [u8; 8] {
    let mut out = [0; 8];
    for i in 0..4 {
        out[i] = mac[i] ^ mac[i + 4];
        out[i + 4] = mac[i + 8] ^ mac[i + 12];
    }
    out
}

#[cfg(windows)]
mod backend {
    use super::*;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Security::Cryptography::*;
    pub(super) struct Aes {
        algorithm: BCRYPT_ALG_HANDLE,
        key: BCRYPT_KEY_HANDLE,
    }
    fn checked(status: i32) -> Result<()> {
        ensure!(
            status >= 0,
            "Windows AES operation failed (0x{:08X})",
            status as u32
        );
        Ok(())
    }
    impl Aes {
        pub(super) fn new(secret: &[u8; 16], cbc: bool) -> Result<Self> {
            let mut aes = Self {
                algorithm: null_mut(),
                key: null_mut(),
            };
            let mode: Vec<u16> = if cbc {
                "ChainingModeCBC"
            } else {
                "ChainingModeECB"
            }
            .encode_utf16()
            .chain(Some(0))
            .collect();
            // CNG copies the 16-byte secret and allocates its key object. Both
            // owning handles are closed on every success/error path by Drop.
            unsafe {
                checked(BCryptOpenAlgorithmProvider(
                    &mut aes.algorithm,
                    BCRYPT_AES_ALGORITHM,
                    null(),
                    0,
                ))?;
                checked(BCryptSetProperty(
                    aes.algorithm,
                    BCRYPT_CHAINING_MODE,
                    mode.as_ptr().cast(),
                    (mode.len() * 2) as u32,
                    0,
                ))?;
                checked(BCryptGenerateSymmetricKey(
                    aes.algorithm,
                    &mut aes.key,
                    null_mut(),
                    0,
                    secret.as_ptr(),
                    16,
                    0,
                ))?;
            }
            Ok(aes)
        }
        pub(super) fn crypt(
            &self,
            input: &[u8],
            iv: Option<&mut [u8; 16]>,
            decrypt: bool,
        ) -> Result<Vec<u8>> {
            ensure!(
                !input.is_empty()
                    && input.len().is_multiple_of(16)
                    && input.len() <= u32::MAX as usize,
                "Invalid AES input length"
            );
            let mut output = vec![0; input.len()];
            let (iv, iv_len) = iv.map_or((null_mut(), 0), |v| (v.as_mut_ptr(), 16));
            let mut written = 0;
            // Buffers remain live for the synchronous call; no padding and no
            // overlapping input/output. CBC may mutate only its supplied IV.
            let status = unsafe {
                let function = if decrypt {
                    BCryptDecrypt
                } else {
                    BCryptEncrypt
                };
                function(
                    self.key,
                    input.as_ptr(),
                    input.len() as u32,
                    null(),
                    iv,
                    iv_len,
                    output.as_mut_ptr(),
                    output.len() as u32,
                    &mut written,
                    0,
                )
            };
            checked(status)?;
            ensure!(
                written as usize == output.len(),
                "Windows AES returned an unexpected size"
            );
            Ok(output)
        }
    }
    impl Drop for Aes {
        fn drop(&mut self) {
            // A key must be destroyed before its algorithm provider.
            unsafe {
                if !self.key.is_null() {
                    BCryptDestroyKey(self.key);
                }
                if !self.algorithm.is_null() {
                    BCryptCloseAlgorithmProvider(self.algorithm, 0);
                }
            }
        }
    }
}
#[cfg(not(windows))]
mod backend {
    use super::*;
    pub(super) struct Aes;
    impl Aes {
        pub(super) fn new(_: &[u8; 16], _: bool) -> Result<Self> {
            available()?;
            unreachable!()
        }
        pub(super) fn crypt(&self, _: &[u8], _: Option<&mut [u8; 16]>, _: bool) -> Result<Vec<u8>> {
            available()?;
            unreachable!()
        }
    }
}

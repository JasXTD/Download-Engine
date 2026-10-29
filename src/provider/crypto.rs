//! Bounded host primitive. AES is provided by Windows CNG, not Lua.
use anyhow::{Context, Result, ensure};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Security::Cryptography::*;
use zeroize::Zeroizing;

pub const MAX_CHUNK: usize = 256 * 1024;

pub fn unhex(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len() <= 128 && value.len().is_multiple_of(2),
        "Invalid hex length"
    );
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |v: u8| (v as char).to_digit(16).context("Invalid hex digit");
            Ok(((digit(pair[0])? << 4) | digit(pair[1])?) as u8)
        })
        .collect()
}

struct Aes {
    algorithm: BCRYPT_ALG_HANDLE,
    key: BCRYPT_KEY_HANDLE,
}

fn checked(status: i32) -> Result<()> {
    ensure!(status >= 0, "Windows AES failed (0x{:08X})", status as u32);
    Ok(())
}

impl Aes {
    fn new(secret: &[u8]) -> Result<Self> {
        ensure!(
            matches!(secret.len(), 16 | 24 | 32),
            "AES key must be 16, 24 or 32 bytes"
        );
        let mut aes = Self {
            algorithm: null_mut(),
            key: null_mut(),
        };
        let mode: Vec<u16> = "ChainingModeECB".encode_utf16().chain(Some(0)).collect();
        // CNG owns a copy of the exact-length key. Drop releases partially
        // initialized handles too; no raw key is serialized by the host.
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
                secret.len() as u32,
                0,
            ))?;
        }
        Ok(aes)
    }

    fn blocks(&self, input: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        ensure!(
            !input.is_empty() && input.len().is_multiple_of(16),
            "Invalid AES blocks"
        );
        let mut output = Zeroizing::new(vec![0; input.len()]);
        let mut written = 0;
        // Non-overlapping, live buffers; no padding, synchronous ECB operation.
        unsafe {
            checked(BCryptEncrypt(
                self.key,
                input.as_ptr(),
                input.len() as u32,
                null(),
                null_mut(),
                0,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut written,
                0,
            ))?;
        }
        ensure!(
            written as usize == output.len(),
            "Unexpected AES result length"
        );
        Ok(output)
    }
}

impl Drop for Aes {
    fn drop(&mut self) {
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

/// Bounded AES primitive for modes implemented by the Lua script itself.
pub fn encrypt_blocks(secret: &[u8], blocks: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        blocks.len() <= MAX_CHUNK + 16,
        "AES block input exceeds limit"
    );
    Ok(Aes::new(secret)?.blocks(blocks)?.to_vec())
}

/// Full 128-bit, big-endian counter. Offset is in input bytes, not blocks.
/// CTR is symmetric and unauthenticated: the host must verify trusted output.
pub fn ctr(secret: &[u8], initial: &[u8; 16], input: &[u8], offset: u64) -> Result<Vec<u8>> {
    ensure!(input.len() <= MAX_CHUNK, "Crypto chunk exceeds 256 KiB");
    let aes = Aes::new(secret)?;
    if input.is_empty() {
        return Ok(Vec::new());
    }
    offset
        .checked_add(input.len() as u64)
        .context("Byte offset overflow")?;
    let first = u128::from_be_bytes(*initial)
        .checked_add((offset / 16) as u128)
        .context("AES counter overflow")?;
    let skip = (offset % 16) as usize;
    let count = (input.len() + skip).div_ceil(16);
    first
        .checked_add((count - 1) as u128)
        .context("AES counter overflow")?;
    let mut counters = Vec::with_capacity(count * 16);
    for i in 0..count {
        counters.extend_from_slice(&(first + i as u128).to_be_bytes());
    }
    let stream = aes.blocks(&counters)?;
    Ok(input
        .iter()
        .zip(&stream[skip..])
        .map(|(a, b)| a ^ b)
        .collect())
}

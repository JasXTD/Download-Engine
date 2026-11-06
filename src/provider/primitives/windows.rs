use super::*;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Security::Cryptography::*;

pub(super) struct Rsa {
    algorithm: BCRYPT_ALG_HANDLE,
    key: BCRYPT_KEY_HANDLE,
}
fn checked(code: i32) -> Result<()> {
    ensure!(code >= 0, "Native RSA operation failed");
    Ok(())
}
impl Drop for Rsa {
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
impl Rsa {
    pub fn new(der: &[u8]) -> Result<Self> {
        let mut size = 0;
        unsafe {
            ensure!(
                CryptDecodeObjectEx(
                    X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                    PKCS_RSA_PRIVATE_KEY,
                    der.as_ptr(),
                    der.len() as u32,
                    0,
                    null(),
                    null_mut(),
                    &mut size
                ) != 0,
                "Invalid RSA encoding"
            );
        }
        ensure!(
            size > 20 && size <= MAX_CREDENTIAL as u32,
            "Invalid RSA key size"
        );
        // An aligned buffer for the CryptoAPI blob; erase the private values after import.
        let mut blob = Zeroizing::new(vec![0_u64; (size as usize).div_ceil(8)]);
        let mut result = Self {
            algorithm: null_mut(),
            key: null_mut(),
        };
        unsafe {
            ensure!(
                CryptDecodeObjectEx(
                    X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                    PKCS_RSA_PRIVATE_KEY,
                    der.as_ptr(),
                    der.len() as u32,
                    0,
                    null(),
                    blob.as_mut_ptr().cast(),
                    &mut size
                ) != 0,
                "Invalid RSA encoding"
            );
            let bytes = std::slice::from_raw_parts(blob.as_ptr().cast::<u8>(), size as usize);
            let bits = u32::from_le_bytes(bytes[12..16].try_into()?);
            ensure!(
                (2048..=4096).contains(&bits),
                "RSA keys must be 2048-4096 bits"
            );
            checked(BCryptOpenAlgorithmProvider(
                &mut result.algorithm,
                BCRYPT_RSA_ALGORITHM,
                null(),
                0,
            ))?;
            checked(BCryptImportKeyPair(
                result.algorithm,
                null_mut(),
                LEGACY_RSAPRIVATE_BLOB,
                &mut result.key,
                blob.as_ptr().cast(),
                size,
                0,
            ))?;
        }
        Ok(result)
    }
    pub fn sign(&self, algorithm: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let (hash, name, pss) = match algorithm {
            "pss-sha1" => (super::digest("sha1", bytes)?, BCRYPT_SHA1_ALGORITHM, true),
            "pss-sha256" => (
                super::digest("sha256", bytes)?,
                BCRYPT_SHA256_ALGORITHM,
                true,
            ),
            "pkcs1-sha256" => (
                super::digest("sha256", bytes)?,
                BCRYPT_SHA256_ALGORITHM,
                false,
            ),
            _ => bail!("Unsupported RSA signature algorithm"),
        };
        let padding = BCRYPT_PSS_PADDING_INFO {
            pszAlgId: name,
            cbSalt: hash.len() as u32,
        };
        let pkcs = BCRYPT_PKCS1_PADDING_INFO { pszAlgId: name };
        let info = if pss {
            (&padding as *const BCRYPT_PSS_PADDING_INFO).cast()
        } else {
            (&pkcs as *const BCRYPT_PKCS1_PADDING_INFO).cast()
        };
        let mut output = vec![0; 512];
        let mut length = 0;
        unsafe {
            checked(BCryptSignHash(
                self.key,
                info,
                hash.as_ptr(),
                hash.len() as u32,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut length,
                if pss {
                    BCRYPT_PAD_PSS
                } else {
                    BCRYPT_PAD_PKCS1
                },
            ))?;
        }
        output.truncate(length as usize);
        Ok(output)
    }
    pub fn decrypt(&self, algorithm: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let name = match algorithm {
            "oaep-sha1" => BCRYPT_SHA1_ALGORITHM,
            "oaep-sha256" => BCRYPT_SHA256_ALGORITHM,
            _ => bail!("Unsupported RSA encryption algorithm"),
        };
        let padding = BCRYPT_OAEP_PADDING_INFO {
            pszAlgId: name,
            pbLabel: null_mut(),
            cbLabel: 0,
        };
        let mut output = Zeroizing::new(vec![0; 512]);
        let mut length = 0;
        unsafe {
            checked(BCryptDecrypt(
                self.key,
                bytes.as_ptr(),
                bytes.len() as u32,
                (&padding as *const BCRYPT_OAEP_PADDING_INFO).cast(),
                null_mut(),
                0,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut length,
                BCRYPT_PAD_OAEP,
            ))?;
        }
        Ok(output[..length as usize].to_vec())
    }
}

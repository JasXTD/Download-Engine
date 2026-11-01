//! Native Unix builds use RustCrypto; Windows retains its existing CNG backend.
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};
use anyhow::{Result, ensure};

enum Key {
    Aes128(aes::Aes128),
    Aes192(aes::Aes192),
    Aes256(aes::Aes256),
}

pub(crate) struct Aes {
    key: Key,
    cbc: bool,
}
impl Aes {
    pub(crate) fn new(secret: &[u8], cbc: bool) -> Result<Self> {
        let key = match secret.len() {
            16 => Key::Aes128(aes::Aes128::new_from_slice(secret)?),
            24 => Key::Aes192(aes::Aes192::new_from_slice(secret)?),
            32 => Key::Aes256(aes::Aes256::new_from_slice(secret)?),
            _ => anyhow::bail!("AES key must be 16, 24 or 32 bytes"),
        };
        Ok(Self { key, cbc })
    }

    pub(crate) fn crypt(
        &self,
        input: &[u8],
        iv: Option<&mut [u8; 16]>,
        decrypt: bool,
    ) -> Result<Vec<u8>> {
        ensure!(input.len().is_multiple_of(16), "Invalid AES blocks");
        ensure!(
            self.cbc == iv.is_some(),
            "Invalid AES initialization vector"
        );
        let mut chain = iv.as_deref().copied().unwrap_or([0; 16]);
        let mut output = Vec::with_capacity(input.len());
        for original in input.as_chunks::<16>().0 {
            let mut block = GenericArray::clone_from_slice(original);
            if self.cbc && !decrypt {
                for (byte, previous) in block.iter_mut().zip(chain) {
                    *byte ^= previous;
                }
            }
            macro_rules! crypt {
                ($key:expr) => {
                    if decrypt {
                        $key.decrypt_block(&mut block);
                    } else {
                        $key.encrypt_block(&mut block);
                    }
                };
            }
            match &self.key {
                Key::Aes128(key) => crypt!(key),
                Key::Aes192(key) => crypt!(key),
                Key::Aes256(key) => crypt!(key),
            }
            if self.cbc {
                if decrypt {
                    for (byte, previous) in block.iter_mut().zip(chain) {
                        *byte ^= previous;
                    }
                    chain = *original;
                } else {
                    chain.copy_from_slice(&block);
                }
            }
            output.extend_from_slice(&block);
        }
        if let Some(iv) = iv {
            *iv = chain;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn nist_aes_ecb_and_cbc_vectors_and_invalid_lengths() {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let input = hex("6bc1bee22e409f96e93d7e117393172a");
        let aes = Aes::new(&key, false).unwrap();
        let expected = hex("3ad77bb40d7a3660a89ecaf32466ef97");
        assert_eq!(aes.crypt(&input, None, false).unwrap(), expected);
        assert_eq!(aes.crypt(&expected, None, true).unwrap(), input);
        let cbc = Aes::new(&key, true).unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let expected = hex("7649abac8119b246cee98e9b12e9197d");
        assert_eq!(
            cbc.crypt(&input, Some(&mut iv.to_owned()), false).unwrap(),
            expected
        );
        assert_eq!(
            cbc.crypt(&expected, Some(&mut iv.to_owned()), true)
                .unwrap(),
            input
        );
        assert!(Aes::new(&[0; 17], false).is_err());
        assert!(aes.crypt(&[0; 15], None, false).is_err());
        assert!(cbc.crypt(&input, None, false).is_err());
    }
}

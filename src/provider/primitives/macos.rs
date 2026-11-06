use super::*;
use core_foundation::{
    base::TCFType, data::CFData, dictionary::CFDictionary, error::CFError, string::CFString,
};
use security_framework::key::{Algorithm, SecKey};
use security_framework_sys::{item::*, key::SecKeyCreateWithData};
pub(super) struct Rsa(SecKey);
impl Rsa {
    pub fn new(der: &[u8]) -> Result<Self> {
        let data = CFData::from_buffer(der);
        unsafe {
            let attributes = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::wrap_under_get_rule(kSecAttrKeyType),
                    CFString::wrap_under_get_rule(kSecAttrKeyTypeRSA),
                ),
                (
                    CFString::wrap_under_get_rule(kSecAttrKeyClass),
                    CFString::wrap_under_get_rule(kSecAttrKeyClassPrivate),
                ),
            ]);
            let mut error = std::ptr::null_mut();
            let key = SecKeyCreateWithData(
                data.as_concrete_TypeRef(),
                attributes.as_concrete_TypeRef(),
                &mut error,
            );
            if !error.is_null() {
                drop(CFError::wrap_under_create_rule(error));
            }
            ensure!(!key.is_null(), "Invalid RSA private key");
            let key = SecKey::wrap_under_create_rule(key);
            Ok(Self(key))
        }
    }
    pub fn sign(&self, algorithm: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let algorithm = match algorithm {
            "pss-sha1" => Algorithm::RSASignatureMessagePSSSHA1,
            "pss-sha256" => Algorithm::RSASignatureMessagePSSSHA256,
            "pkcs1-sha256" => Algorithm::RSASignatureMessagePKCS1v15SHA256,
            _ => bail!("Unsupported RSA signature algorithm"),
        };
        self.0
            .create_signature(algorithm, bytes)
            .map_err(|_| anyhow::anyhow!("RSA signing failed"))
    }
    pub fn decrypt(&self, algorithm: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let algorithm = match algorithm {
            "oaep-sha1" => Algorithm::RSAEncryptionOAEPSHA1,
            "oaep-sha256" => Algorithm::RSAEncryptionOAEPSHA256,
            _ => bail!("Unsupported RSA encryption algorithm"),
        };
        self.0
            .decrypt_data(algorithm, bytes)
            .map_err(|_| anyhow::anyhow!("RSA decryption failed"))
    }
}

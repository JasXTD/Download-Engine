use super::*;
use openssl::{
    hash::MessageDigest,
    pkey::{PKey, Private},
    rsa::{Padding, Rsa as Native},
};
pub(super) struct Rsa(PKey<Private>);
impl Rsa {
    pub fn new(der: &[u8]) -> Result<Self> {
        let key = Native::private_key_from_der(der)?;
        ensure!(
            (2048..=4096).contains(&(key.size() * 8)) && key.check_key()?,
            "Invalid RSA private key"
        );
        Ok(Self(PKey::from_rsa(key)?))
    }
    pub fn sign(&self, algorithm: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let (hash, pss) = match algorithm {
            "pss-sha1" => (MessageDigest::sha1(), true),
            "pss-sha256" => (MessageDigest::sha256(), true),
            "pkcs1-sha256" => (MessageDigest::sha256(), false),
            _ => bail!("Unsupported RSA signature algorithm"),
        };
        let mut signer = openssl::sign::Signer::new(hash, &self.0)?;
        if pss {
            signer.set_rsa_padding(Padding::PKCS1_PSS)?;
            signer.set_rsa_mgf1_md(hash)?;
            signer.set_rsa_pss_saltlen(openssl::sign::RsaPssSaltlen::DIGEST_LENGTH)?;
        }
        signer.update(bytes)?;
        Ok(signer.sign_to_vec()?)
    }
    pub fn decrypt(&self, algorithm: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let hash = match algorithm {
            "oaep-sha1" => MessageDigest::sha1(),
            "oaep-sha256" => MessageDigest::sha256(),
            _ => bail!("Unsupported RSA encryption algorithm"),
        };
        let mut worker = openssl::encrypt::Decrypter::new(&self.0)?;
        worker.set_rsa_padding(Padding::PKCS1_OAEP)?;
        worker.set_rsa_oaep_md(hash)?;
        worker.set_rsa_mgf1_md(hash)?;
        let mut output = Zeroizing::new(vec![0; worker.decrypt_len(bytes)?]);
        let n = worker.decrypt(bytes, &mut output)?;
        Ok(output[..n].to_vec())
    }
}

//! Generic bounded cryptography. Service protocols and container parsing stay in Lua.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use mlua::{Lua, Table};
use sha2::Digest;
use zeroize::Zeroizing;

#[cfg(windows)]
#[path = "primitives/windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "primitives/linux.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "primitives/macos.rs"]
mod native;

pub(crate) const MAX_CREDENTIAL: usize = 64 * 1024;
const MAX_MESSAGE: usize = 1024 * 1024;
pub(crate) struct Rsa(native::Rsa);
impl mlua::UserData for Rsa {}

/// Only decode the PKCS#8 wrapper; native crypto validates the enclosed RSA key.
fn field<'a>(input: &mut &'a [u8], tag: u8) -> Result<&'a [u8]> {
    ensure!(input.first() == Some(&tag), "Invalid private-key encoding");
    *input = &input[1..];
    let first = *input.first().context("Truncated private key")?;
    *input = &input[1..];
    let size = if first < 128 {
        first as usize
    } else {
        let n = (first & 127) as usize;
        ensure!(
            (1..=3).contains(&n) && input.len() >= n && input[0] != 0,
            "Invalid DER length"
        );
        let size = input[..n]
            .iter()
            .fold(0_usize, |a, b| (a << 8) | *b as usize);
        *input = &input[n..];
        ensure!(size >= 128, "Non-canonical DER length");
        size
    };
    ensure!(size <= input.len(), "Truncated private key");
    let value = &input[..size];
    *input = &input[size..];
    Ok(value)
}
fn private_der(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_CREDENTIAL,
        "Private key exceeds 64 KiB"
    );
    let decoded = if bytes.starts_with(b"-----BEGIN") {
        let text = std::str::from_utf8(bytes)?.trim();
        let (begin, end) = if text.starts_with("-----BEGIN RSA PRIVATE KEY-----") {
            (
                "-----BEGIN RSA PRIVATE KEY-----",
                "-----END RSA PRIVATE KEY-----",
            )
        } else {
            ("-----BEGIN PRIVATE KEY-----", "-----END PRIVATE KEY-----")
        };
        let body = text
            .strip_prefix(begin)
            .and_then(|s| s.strip_suffix(end))
            .context("Use an unencrypted RSA PKCS#1 or PKCS#8 private key")?;
        let body = Zeroizing::new(
            body.bytes()
                .filter(|c| !c.is_ascii_whitespace())
                .collect::<Vec<_>>(),
        );
        Zeroizing::new(STANDARD.decode(&body)?)
    } else {
        Zeroizing::new(bytes.to_vec())
    };
    let mut outer = decoded.as_slice();
    let mut inner = field(&mut outer, 0x30)?;
    ensure!(outer.is_empty(), "Trailing private-key data");
    ensure!(
        field(&mut inner, 0x02)? == [0],
        "Only two-prime RSA private keys are supported"
    );
    if inner.first() == Some(&0x02) {
        return Ok(decoded);
    }
    let mut algorithm = field(&mut inner, 0x30)?;
    ensure!(
        field(&mut algorithm, 0x06)? == [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 1, 1],
        "Expected an RSA private key"
    );
    if !algorithm.is_empty() {
        ensure!(
            field(&mut algorithm, 0x05)?.is_empty(),
            "Invalid RSA parameters"
        );
    }
    ensure!(algorithm.is_empty(), "Invalid RSA parameters");
    let key = field(&mut inner, 0x04)?;
    ensure!(inner.is_empty(), "Unsupported private-key attributes");
    Ok(Zeroizing::new(key.to_vec()))
}
impl Rsa {
    pub(crate) fn new(bytes: &[u8]) -> Result<Self> {
        let der = private_der(bytes)?;
        // Validate the common PKCS#1 envelope and exact bit length on every OS.
        // Native crypto then imports and validates the private-key material.
        let mut outer = der.as_slice();
        let mut key = field(&mut outer, 0x30)?;
        ensure!(
            outer.is_empty() && field(&mut key, 0x02)? == [0],
            "Expected a two-prime RSA private key"
        );
        for index in 0..8 {
            let integer = field(&mut key, 0x02)?;
            ensure!(
                !integer.is_empty() && integer[0] & 128 == 0,
                "Invalid RSA integer"
            );
            let unsigned = if integer.len() > 1 && integer[0] == 0 {
                ensure!(integer[1] & 128 != 0, "Non-canonical RSA integer");
                &integer[1..]
            } else {
                integer
            };
            ensure!(unsigned[0] != 0, "Invalid RSA integer");
            if index == 0 {
                let bits = unsigned.len() * 8 - unsigned[0].leading_zeros() as usize;
                ensure!(
                    (2048..=4096).contains(&bits),
                    "RSA keys must be 2048-4096 bits"
                );
            }
        }
        ensure!(key.is_empty(), "Unexpected RSA private-key data");
        Ok(Self(native::Rsa::new(&der)?))
    }
}
pub(crate) fn validate_private_key(bytes: &[u8]) -> Result<()> {
    Rsa::new(bytes).map(drop)
}

fn digest(algorithm: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(bytes.len() <= MAX_MESSAGE, "Digest input exceeds 1 MiB");
    match algorithm {
        "sha1" => Ok(sha1::Sha1::digest(bytes).to_vec()),
        "sha256" => Ok(sha2::Sha256::digest(bytes).to_vec()),
        _ => bail!("Use sha1 or sha256"),
    }
}
pub(super) fn install(lua: &Lua, xd: &Table) -> Result<()> {
    xd.set(
        "random_bytes",
        lua.create_function(|lua, count: usize| {
            if !(1..=4096).contains(&count) {
                return Err(mlua::Error::runtime("Request 1 to 4096 random bytes"));
            }
            let mut bytes = Zeroizing::new(vec![0; count]);
            getrandom::fill(&mut bytes)
                .map_err(|_| mlua::Error::runtime("System random source unavailable"))?;
            lua.create_string(&*bytes)
        })?,
    )?;
    xd.set(
        "digest",
        lua.create_function(|lua, (algorithm, bytes): (String, mlua::LuaString)| {
            lua.create_string(
                digest(&algorithm, bytes.as_bytes().as_ref()).map_err(mlua::Error::external)?,
            )
        })?,
    )?;
    xd.set(
        "hmac",
        lua.create_function(
            |lua, (algorithm, key, bytes): (String, mlua::LuaString, mlua::LuaString)| {
                use hmac::Mac;
                if bytes.as_bytes().len() > MAX_MESSAGE || key.as_bytes().len() > MAX_CREDENTIAL {
                    return Err(mlua::Error::runtime("HMAC input exceeds its limit"));
                }
                let output = match algorithm.as_str() {
                    "sha1" => {
                        let mut h =
                            hmac::Hmac::<sha1::Sha1>::new_from_slice(key.as_bytes().as_ref())
                                .map_err(mlua::Error::external)?;
                        h.update(bytes.as_bytes().as_ref());
                        h.finalize().into_bytes().to_vec()
                    }
                    "sha256" => {
                        let mut h =
                            hmac::Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes().as_ref())
                                .map_err(mlua::Error::external)?;
                        h.update(bytes.as_bytes().as_ref());
                        h.finalize().into_bytes().to_vec()
                    }
                    _ => return Err(mlua::Error::runtime("Use sha1 or sha256")),
                };
                lua.create_string(output)
            },
        )?,
    )?;
    xd.set("rsa_private_key", lua.create_function(|_, bytes: mlua::LuaString| {
        Rsa::new(bytes.as_bytes().as_ref()).map_err(|_| mlua::Error::runtime("Invalid RSA private key; use an unencrypted 2048-4096-bit PKCS#1 or PKCS#8 key"))
    })?)?;
    xd.set(
        "rsa_sign",
        lua.create_function(
            |lua, (key, algorithm, bytes): (mlua::AnyUserData, String, mlua::LuaString)| {
                if bytes.as_bytes().len() > MAX_MESSAGE {
                    return Err(mlua::Error::runtime("RSA message exceeds 1 MiB"));
                }
                let key = key.borrow::<Rsa>()?;
                let output = key
                    .0
                    .sign(&algorithm, bytes.as_bytes().as_ref())
                    .map_err(|_| {
                        mlua::Error::runtime(
                            "RSA signing failed; use pss-sha1, pss-sha256 or pkcs1-sha256",
                        )
                    })?;
                lua.create_string(output)
            },
        )?,
    )?;
    xd.set(
        "rsa_decrypt",
        lua.create_function(
            |lua, (key, algorithm, bytes): (mlua::AnyUserData, String, mlua::LuaString)| {
                if bytes.as_bytes().len() > 512 {
                    return Err(mlua::Error::runtime("RSA ciphertext exceeds 512 bytes"));
                }
                let key = key.borrow::<Rsa>()?;
                let output = Zeroizing::new(
                    key.0
                        .decrypt(&algorithm, bytes.as_bytes().as_ref())
                        .map_err(|_| {
                            mlua::Error::runtime(
                                "RSA decryption failed; use oaep-sha1 or oaep-sha256",
                            )
                        })?,
                );
                lua.create_string(&*output)
            },
        )?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_vectors_random_bounds_and_invalid_key_encodings() {
        let lua = Lua::new();
        let xd = lua.create_table().unwrap();
        install(&lua, &xd).unwrap();
        lua.globals().set("xd", xd).unwrap();
        lua.load(r#"
            local function hex(s) return (s:gsub('.',function(c)return string.format('%02x',c:byte())end)) end
            assert(hex(xd.digest('sha256','abc'))=='ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
            assert(hex(xd.digest('sha1','abc'))=='a9993e364706816aba3e25717850c26c9cd0d89d')
            assert(hex(xd.hmac('sha256',string.rep(string.char(11),20),'Hi There'))=='b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7')
            assert(hex(xd.hmac('sha1',string.rep(string.char(11),20),'Hi There'))=='b617318655057264e28bc0b6fb378c8ef146be00')
            assert(#xd.random_bytes(4096)==4096 and xd.random_bytes(32)~=xd.random_bytes(32))
            assert(not pcall(xd.random_bytes,0) and not pcall(xd.random_bytes,4097))
            assert(not pcall(xd.digest,'md5','abc'))
            assert(not pcall(xd.rsa_private_key,'not a key'))
            assert(not pcall(xd.rsa_private_key,string.rep('x',65537)))
        "#).exec().unwrap();
        let key = crate::provider::crypto::unhex("2b7e151628aed2a6abf7158809cf4f3c").unwrap();
        let cipher = crate::provider::crypto::unhex("3ad77bb40d7a3660a89ecaf32466ef97").unwrap();
        assert_eq!(
            crate::provider::crypto::decrypt_blocks(&key, &cipher).unwrap(),
            crate::provider::crypto::unhex("6bc1bee22e409f96e93d7e117393172a").unwrap()
        );
        assert!(crate::provider::crypto::decrypt_blocks(&key, &[0; 15]).is_err());
        for bad in [
            vec![],
            vec![0x30, 0x80],
            vec![0x30, 0x83, 255, 255, 255],
            vec![0x30, 0x81, 3, 2, 1, 0],
        ] {
            assert!(private_der(&bad).is_err());
        }
    }
}

//! Immutable DPAPI tokens are bound to one provider. Only processors can unlock them.
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use uuid::Uuid;
use zeroize::Zeroizing;

const PREFIX: &str = "provider-key-v1:";
const PURPOSE: &[u8] = b"XDownloader provider AES key v1\0";

impl crate::store::Store {
    /// Read key material from redirected stdin in the CLI, never arguments/receipts.
    pub fn protect_provider_key(&self, script_id: &str, hex: &str) -> Result<String> {
        let script = self.script(script_id)?;
        ensure!(
            script.language == crate::script_automation::Language::Lua
                && script.trigger == crate::script_automation::Trigger::Provider,
            "Choose a Lua download provider"
        );
        #[cfg(windows)]
        {
            let key = Zeroizing::new(super::crypto::unhex(hex)?);
            ensure!(
                matches!(key.len(), 16 | 24 | 32),
                "AES key must be 16, 24 or 32 bytes"
            );
            let mut bytes = Zeroizing::new(PURPOSE.to_vec());
            bytes.extend_from_slice(&key);
            let sealed = crate::protected_data::seal(Uuid::parse_str(script_id)?, &mut bytes)?;
            Ok(format!("{PREFIX}{}", STANDARD.encode(&sealed)))
        }
        #[cfg(not(windows))]
        {
            let _ = hex;
            anyhow::bail!("Provider keys require Windows protected storage")
        }
    }
}

pub(super) struct Key(Zeroizing<Vec<u8>>);
impl mlua::UserData for Key {}

#[cfg(windows)]
pub(super) fn unlock(script: Uuid, token: &str) -> Result<Key> {
    ensure!(token.len() <= 4096, "Invalid provider key token");
    let bytes = STANDARD.decode(
        token
            .strip_prefix(PREFIX)
            .context("Invalid provider key token")?,
    )?;
    let key = crate::protected_data::open(script, &bytes).context(
        "Provider key belongs to a different provider or Windows account, or was modified",
    )?;
    let raw = key
        .strip_prefix(PURPOSE)
        .context("Invalid provider key token")?;
    ensure!(
        matches!(raw.len(), 16 | 24 | 32),
        "Invalid provider AES key"
    );
    Ok(Key(Zeroizing::new(raw.to_vec())))
}

#[cfg(windows)]
pub(super) fn bytes(value: mlua::Value) -> mlua::Result<Zeroizing<Vec<u8>>> {
    match value {
        mlua::Value::UserData(value) => Ok(value.borrow::<Key>()?.0.clone()),
        mlua::Value::String(value) => super::crypto::unhex(&value.to_str()?)
            .map(Zeroizing::new)
            .map_err(mlua::Error::external),
        _ => Err(mlua::Error::runtime(
            "Expected an AES hex key or opaque key handle",
        )),
    }
}

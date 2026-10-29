//! Frozen code is protected once, then reused verbatim by every checkpoint.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::{Connection, TransactionBehavior, params};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sealed {
    format: String,
    id: Uuid,
    sealed: String,
}

#[derive(Clone)]
pub struct FrozenScript {
    script: Script,
    sealed: Sealed,
}
impl FrozenScript {
    pub fn new(script: Script) -> Result<Self> {
        script.validate()?;
        ensure!(
            script.language == Language::Lua
                && script.trigger == Trigger::Provider
                && script.revision > 0,
            "Invalid frozen provider"
        );
        let id = Uuid::parse_str(&script.id)?;
        let mut bytes = Zeroizing::new(serde_json::to_vec(&script)?);
        let sealed = crate::protected_data::seal(id, &mut bytes)
            .context("Cannot protect frozen provider source")?;
        Ok(Self {
            script,
            sealed: Sealed {
                format: "provider-script-v1".into(),
                id,
                sealed: STANDARD.encode(&sealed),
            },
        })
    }
}
impl std::ops::Deref for FrozenScript {
    type Target = Script;
    fn deref(&self) -> &Script {
        &self.script
    }
}
impl std::fmt::Debug for FrozenScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrozenScript")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
impl PartialEq<Script> for FrozenScript {
    fn eq(&self, other: &Script) -> bool {
        self.script == *other
    }
}
impl Serialize for FrozenScript {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.sealed.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for FrozenScript {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Protected(Sealed),
            Legacy(Script),
        }
        let result = match Saved::deserialize(deserializer)? {
            Saved::Legacy(script) => Self::new(script),
            Saved::Protected(sealed) => (|| -> Result<Self> {
                ensure!(
                    sealed.format == "provider-script-v1" && sealed.sealed.len() <= 600_000,
                    "Invalid protected provider source"
                );
                let ciphertext = STANDARD.decode(&sealed.sealed)?;
                let bytes = crate::protected_data::open(sealed.id, &ciphertext)
                    .context("Cannot unlock provider source with this Windows account")?;
                let script: Script = serde_json::from_slice(&bytes)?;
                script.validate()?;
                ensure!(
                    Uuid::parse_str(&script.id)? == sealed.id
                        && script.revision > 0
                        && script.language == Language::Lua
                        && script.trigger == Trigger::Provider,
                    "Invalid protected provider identity"
                );
                Ok(Self { script, sealed })
            })(),
        };
        result.map_err(serde::de::Error::custom)
    }
}

/// Upgrade active rows atomically. Historical backups/free pages cannot be erased here.
pub(crate) fn upgrade(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row(
        "SELECT data_version FROM profile_format WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if version >= 3 {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rows = tx
        .prepare("SELECT id,data FROM jobs WHERE json_type(data,'$.provider')='object'")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, data) in &rows {
        let job = crate::store::decode_job(id, data)?;
        tx.execute(
            "UPDATE jobs SET data=?1 WHERE id=?2",
            params![serde_json::to_string(&job)?, id],
        )?;
    }
    let has_provider: bool = !rows.is_empty() || tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM automation_scripts WHERE json_extract(data,'$.trigger.kind')='provider')",
        [], |r| r.get(0))?;
    if has_provider {
        super::protect_profile(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

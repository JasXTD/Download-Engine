//! What a job's worker receives. Rust decides what goes in here, so a job never
//! holds a setting its kind may not read, whatever its code asks for.
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which settings a worker may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Sees downloads, so it never holds a secret.
    Automation,
    /// Reaches declared hosts: site readers and provider resolvers.
    Web,
    /// Sees file bytes and has no network.
    Processor,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub id: String,
    pub version: String,
    pub job: String,
    pub hosts: Vec<String>,
    /// The package holds a secret, so its jobs may only return declared hosts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub limit_results: bool,
    pub files: BTreeMap<String, String>,
    pub settings: BTreeMap<String, serde_json::Value>,
    /// Protected key tokens by setting, for processors only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_scope: Option<uuid::Uuid>,
}

const LOADING: &str = "__loading";

fn module_path(name: &str) -> mlua::Result<String> {
    let path = format!("{}.lua", name.replace('.', "/"));
    if !super::valid_path(&path) {
        return Err(mlua::Error::runtime(format!(
            "require: invalid module name {name:?}"
        )));
    }
    Ok(path)
}

impl Context {
    /// `require`, `xd.setting` and `xd.extension`. Called before any package code
    /// runs. Modules load only from this package, as text, and never from disk.
    pub(crate) fn install(&self, lua: &mlua::Lua, xd: &mlua::Table) -> Result<()> {
        use mlua::{Value, chunk::ChunkMode};
        let files = self.files.clone();
        let loaded = lua.create_table()?;
        let cache = loaded.clone();
        lua.globals().set(
            "require",
            lua.create_function(move |lua, name: String| {
                let path = module_path(&name)?;
                let saved: Value = cache.get(path.as_str())?;
                match saved {
                    Value::Nil => {}
                    Value::String(text) if text.to_str()?.as_ref() == LOADING => {
                        return Err(mlua::Error::runtime(format!(
                            "require: {name} loads itself in a loop"
                        )));
                    }
                    value => return Ok(value),
                }
                let source = files.get(&path).ok_or_else(|| {
                    mlua::Error::runtime(format!("require: {path} is not in this extension"))
                })?;
                cache.set(path.as_str(), LOADING)?;
                let value: Value = lua
                    .load(source.as_str())
                    .set_name(path.as_str())
                    .set_mode(ChunkMode::Text)
                    .eval()?;
                let value = if value.is_nil() {
                    Value::Boolean(true)
                } else {
                    value
                };
                cache.set(path.as_str(), value.clone())?;
                Ok(value)
            })?,
        )?;
        let settings = self.settings.clone();
        xd.set(
            "setting",
            lua.create_function(move |lua, id: String| {
                let value = settings.get(&id).ok_or_else(|| {
                    mlua::Error::runtime(format!(
                        "xd.setting: {id} is not a setting this job can read"
                    ))
                })?;
                crate::script_automation::lua::to_lua(lua, value)
            })?,
        )?;
        let extension = lua.create_table()?;
        extension.set("id", self.id.as_str())?;
        extension.set("version", self.version.as_str())?;
        extension.set("job", self.job.as_str())?;
        xd.set("extension", extension)?;
        let supports: mlua::Function = xd.get("supports").context("Missing xd.supports")?;
        xd.set(
            "supports",
            lua.create_function(move |_, name: String| {
                Ok(
                    matches!(name.as_str(), "xd.setting" | "xd.extension" | "require")
                        || supports.call::<bool>(name)?,
                )
            })?,
        )?;
        Ok(())
    }
}

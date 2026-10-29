use super::{CHUNK_BYTES, Content, Plan, VERSION, process::Role};
use crate::script_automation::{Fetcher, helpers, json, lua::to_lua};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use mlua::{Lua, StdLib, Table, Value, chunk::ChunkMode};

pub(super) struct Runtime {
    lua: Lua,
    callbacks: Table,
    role: Role,
}
pub(super) enum Capabilities {
    Resolver(Fetcher),
    Processor(uuid::Uuid),
}
impl Runtime {
    pub(super) fn new(source: &str, capabilities: Capabilities) -> Result<Self> {
        ensure!(
            source.len() <= crate::script_automation::MAX_SCRIPT_BYTES,
            "Provider code exceeds 64 KiB"
        );
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            Default::default(),
        )?;
        lua.set_memory_limit(64 * 1024 * 1024)?;
        for name in ["dofile", "loadfile", "load", "print", "warn"] {
            lua.globals().set(name, Value::Nil)?;
        }
        let xd = lua.create_table()?;
        helpers::install(&lua, &xd, true)?;
        // Capabilities are installed before source evaluation. A processor can
        // never capture fetch in a local/upvalue, including at module scope.
        let role = match capabilities {
            Capabilities::Resolver(mut fetch) => {
                crate::script_automation::lua::install_fetch(&lua, &xd, move |request| {
                    fetch.fetch(request)
                })?;
                Role::Resolver
            }
            Capabilities::Processor(script_id) => {
                #[cfg(windows)]
                xd.set(
                    "key",
                    lua.create_function(move |_, token: String| {
                        super::keys::unlock(script_id, &token).map_err(mlua::Error::external)
                    })?,
                )?;
                #[cfg(not(windows))]
                let _ = script_id;
                Role::Processor
            }
        };
        xd.set("script_kind", "provider")?;
        xd.set("provider_api", VERSION)?;
        xd.set(
            "phase",
            if role == Role::Resolver {
                "resolver"
            } else {
                "processor"
            },
        )?;
        xd.set(
            "supports",
            lua.create_function(move |_, name: String| {
                Ok(super::supports(&name)
                    && match name.as_str() {
                        "xd.fetch" => role == Role::Resolver,
                        "xd.key" => cfg!(windows) && role == Role::Processor,
                        "xd.aes_ctr" | "xd.aes_blocks" => cfg!(windows),
                        _ => true,
                    })
            })?,
        )?;
        #[cfg(windows)]
        {
            xd.set(
                "aes_ctr",
                lua.create_function(
                    |lua, (key, counter, bytes, offset): (Value, String, mlua::LuaString, u64)| {
                        let key = super::keys::bytes(key)?;
                        let counter: [u8; 16] = super::crypto::unhex(&counter)
                            .map_err(mlua::Error::external)?
                            .try_into()
                            .map_err(|_| mlua::Error::runtime("Counter must be 16 bytes"))?;
                        let output =
                            super::crypto::ctr(&key, &counter, bytes.as_bytes().as_ref(), offset)
                                .map_err(mlua::Error::external)?;
                        lua.create_string(output)
                    },
                )?,
            )?;
            xd.set(
                "aes_blocks",
                lua.create_function(|lua, (key, bytes): (Value, mlua::LuaString)| {
                    let key = super::keys::bytes(key)?;
                    let output = super::crypto::encrypt_blocks(&key, bytes.as_bytes().as_ref())
                        .map_err(mlua::Error::external)?;
                    lua.create_string(output)
                })?,
            )?;
        }
        lua.globals().set("xd", xd)?;
        let callbacks: Table = lua
            .load(source)
            .set_name("provider.lua")
            .set_mode(ChunkMode::Text)
            .eval()?;
        for name in ["resolve", "refresh", "transform", "finish"] {
            callbacks.get::<mlua::Function>(name)?;
        }
        Ok(Self {
            lua,
            callbacks,
            role,
        })
    }
    pub(super) fn handle(&self, message: serde_json::Value) -> Result<serde_json::Value> {
        let op = message["op"]
            .as_str()
            .context("Missing provider operation")?;
        match op {
            "resolve" => {
                ensure!(
                    self.role == Role::Resolver,
                    "Processor workers cannot resolve or refresh"
                );
                let request = to_lua(&self.lua, &serde_json::json!({"url":message["url"]}))?;
                let previous = &message["previous"];
                let callback = self
                    .callbacks
                    .get::<mlua::Function>(if previous.is_null() {
                        "resolve"
                    } else {
                        "refresh"
                    })?;
                let value = callback.call::<Value>((request, to_lua(&self.lua, previous)?))?;
                let value = json::from_lua(&self.lua, value, 32 * 1024)?;
                let plan: Plan = serde_json::from_value(value.clone())?;
                plan.validate()?;
                Ok(value)
            }
            "transform" => {
                ensure!(
                    self.role == Role::Processor,
                    "Resolver workers cannot process file bytes"
                );
                let plan: Content = serde_json::from_value(message["plan"].clone())?;
                plan.validate()?;
                let bytes = message["bytes"]
                    .as_str()
                    .context("Missing provider chunk")?;
                ensure!(
                    bytes.len() <= CHUNK_BYTES.div_ceil(3) * 4,
                    "Provider input too large"
                );
                let bytes = STANDARD.decode(bytes)?;
                ensure!(bytes.len() <= CHUNK_BYTES, "Provider input too large");
                let offset = message["offset"].as_u64().context("Missing byte offset")?;
                ensure!(
                    offset
                        .checked_add(bytes.len() as u64)
                        .is_some_and(|n| n <= plan.size),
                    "Invalid provider offset"
                );
                let output: Value = self.callbacks.get::<mlua::Function>("transform")?.call((
                    to_lua(&self.lua, &plan)?,
                    self.lua.create_string(&bytes)?,
                    offset,
                ))?;
                let Value::String(output) = output else {
                    anyhow::bail!("Provider transform must return a binary string");
                };
                ensure!(
                    output.as_bytes().len() == bytes.len(),
                    "Provider must return exactly one output byte per input byte"
                );
                Ok(serde_json::json!(STANDARD.encode(output.as_bytes())))
            }
            "finish" => {
                ensure!(
                    self.role == Role::Processor,
                    "Resolver workers cannot finish files"
                );
                let plan: Content = serde_json::from_value(message["plan"].clone())?;
                plan.validate()?;
                let result = serde_json::json!({"size":plan.size,"sha256":plan.output_sha256});
                let finished: Value = self
                    .callbacks
                    .get::<mlua::Function>("finish")?
                    .call((to_lua(&self.lua, &plan)?, to_lua(&self.lua, &result)?))?;
                let Value::Boolean(finished) = finished else {
                    anyhow::bail!("Provider finish must return a boolean");
                };
                Ok(serde_json::json!(finished))
            }
            _ => anyhow::bail!("Unknown provider operation"),
        }
    }
}

/// Private CLI worker entry. Only framed JSON travels through its pipes; Lua has no stdio.
pub fn worker_main() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let first = super::process::read_frame(&mut input)?;
    ensure!(first["op"] == "init", "Provider must initialize first");
    // Never construct a Fetcher or load profile/network settings in a processor.
    // Reject missing/unknown roles rather than falling back to resolver access.
    let capabilities = match serde_json::from_value::<Role>(first["role"].clone())? {
        Role::Resolver => {
            let paths = serde_json::from_str(&std::env::var("IDM_EX_PROVIDER_PROFILE")?)?;
            let store = crate::store::Store::open(paths)?;
            Capabilities::Resolver(Fetcher::new(crate::proxy::Route::load(&store)?)?)
        }
        Role::Processor => Capabilities::Processor(uuid::Uuid::parse_str(
            first["script_id"]
                .as_str()
                .context("Missing provider identity")?,
        )?),
    };
    let runtime = Runtime::new(
        first["source"]
            .as_str()
            .context("Missing provider source")?,
        capabilities,
    );
    match runtime {
        Err(error) => super::process::write_frame(
            &mut output,
            &serde_json::json!({"error":format!("{error:#}").chars().take(4096).collect::<String>()}),
        ),
        Ok(runtime) => {
            super::process::write_frame(&mut output, &serde_json::json!({"value":true}))?;
            loop {
                let message = super::process::read_frame(&mut input)?;
                let reply = match runtime.handle(message) {
                    Ok(value) => serde_json::json!({"value":value}),
                    Err(error) => {
                        serde_json::json!({"error":format!("{error:#}").chars().take(4096).collect::<String>()})
                    }
                };
                super::process::write_frame(&mut output, &reply)?;
            }
        }
    }
}

use super::{CHUNK_BYTES, Content, VERSION, process::Role};
use crate::script_automation::{Fetcher, helpers, json, lua::to_lua};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use mlua::{Lua, StdLib, Table, Value, chunk::ChunkMode};
mod segments;
mod stream;

pub(super) struct Runtime {
    lua: Lua,
    callbacks: Table,
    role: Role,
    handoff: Option<super::handoff::Pending>,
    segments: std::cell::RefCell<segments::Segments>,
}
pub(super) enum Capabilities {
    Resolver(Box<Fetcher>),
    Processor(uuid::Uuid),
}
impl Runtime {
    #[cfg(test)]
    pub(super) fn new(
        source: &str,
        capabilities: Capabilities,
        extension: Option<&crate::extensions::Context>,
    ) -> Result<Self> {
        let owner = match &capabilities {
            Capabilities::Processor(id) => *id,
            Capabilities::Resolver(_) => uuid::Uuid::new_v4(),
        };
        Self::with_handoff(
            source,
            capabilities,
            extension,
            uuid::Uuid::new_v4(),
            owner,
            None,
        )
    }
    pub(super) fn with_handoff(
        source: &str,
        capabilities: Capabilities,
        extension: Option<&crate::extensions::Context>,
        scope: uuid::Uuid,
        owner: uuid::Uuid,
        keys: Option<super::handoff::Keys>,
    ) -> Result<Self> {
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
        super::primitives::install(&lua, &xd)?;
        let mut handoff = None;
        // Capabilities are installed before source evaluation. A processor can
        // never capture fetch in a local/upvalue, including at module scope.
        let role = match capabilities {
            Capabilities::Resolver(mut fetch) => {
                ensure!(keys.is_none(), "Resolver cannot receive session keys");
                crate::script_automation::lua::install_fetch(
                    &lua,
                    &xd,
                    extension.is_none_or(|e| e.http_sessions),
                    move |request| fetch.fetch(request),
                )?;
                handoff = Some(super::handoff::Pending::install(&lua, &xd, scope, owner)?);
                Role::Resolver
            }
            Capabilities::Processor(script_id) => {
                ensure!(script_id == owner, "Invalid provider key owner");
                let keys = keys.unwrap_or_else(|| super::handoff::Keys::empty(scope, owner));
                keys.validate(scope, owner)?;
                super::handoff::install_processor(&lua, &xd, keys)?;
                #[cfg(any(windows, unix))]
                xd.set(
                    "key",
                    lua.create_function(move |_, token: String| {
                        super::keys::unlock(script_id, &token).map_err(mlua::Error::external)
                    })?,
                )?;
                #[cfg(not(any(windows, unix)))]
                let _ = script_id;
                Role::Processor
            }
        };
        xd.set("script_kind", "provider")?;
        xd.set("provider_api", VERSION)?;
        xd.set("media_api", super::media::API)?;
        xd.set("segment_api", 1)?;
        xd.set(
            "media_plan",
            lua.create_function(|lua, table: Table| {
                table.set("kind", "media")?;
                table.set("media_api", super::media::API)?;
                let value = json::media_from_lua(
                    lua,
                    Value::Table(table.clone()),
                    super::media::MAX_PLAN_BYTES,
                )
                .map_err(mlua::Error::external)?;
                let plan: super::media::Plan =
                    serde_json::from_value(value).map_err(mlua::Error::external)?;
                plan.validate(true).map_err(mlua::Error::external)?;
                Ok(table)
            })?,
        )?;
        xd.set(
            "phase",
            if role == Role::Resolver {
                "resolver"
            } else {
                "processor"
            },
        )?;
        let allow_sessions = extension.is_none_or(|e| e.http_sessions);
        xd.set(
            "supports",
            lua.create_function(move |_, name: String| {
                if matches!(
                    name.as_str(),
                    "media.segment_processing"
                        | "media.stream_processing"
                        | "media.unknown_sizes"
                        | "media.ass"
                ) {
                    return Ok(true);
                }
                Ok(super::supports(&name)
                    && match name.as_str() {
                        "xd.fetch" | "xd.fetch_bytes" => role == Role::Resolver,
                        "xd.binary_setting" | "xd.rsa_key_setting" => false,
                        "xd.http_session" | "xd.handoff_session" => {
                            role == Role::Resolver && allow_sessions
                        }
                        "xd.handoff_key" => cfg!(any(windows, unix)) && role == Role::Resolver,
                        "xd.session_key" => cfg!(any(windows, unix)) && role == Role::Processor,
                        "xd.key" => cfg!(any(windows, unix)) && role == Role::Processor,
                        "xd.aes_ctr" | "xd.aes_blocks" => cfg!(any(windows, unix)),
                        _ => true,
                    })
            })?,
        )?;
        #[cfg(any(windows, unix))]
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
            xd.set(
                "aes_decrypt_blocks",
                lua.create_function(|lua, (key, bytes): (Value, mlua::LuaString)| {
                    let key = super::keys::bytes(key)?;
                    lua.create_string(
                        super::crypto::decrypt_blocks(&key, bytes.as_bytes().as_ref())
                            .map_err(mlua::Error::external)?,
                    )
                })?,
            )?;
        }
        if let Some(extension) = extension {
            extension.install(&lua, &xd)?;
            // Tokens reach only a processor, and unlock only under this extension.
            #[cfg(any(windows, unix))]
            if role == Role::Processor {
                let keys = extension.keys.clone();
                let scope = extension.key_scope;
                xd.set(
                    "key_setting",
                    lua.create_function(move |_, id: String| {
                        let token = keys.get(&id).ok_or_else(|| {
                            mlua::Error::runtime(format!("xd.key_setting: {id} has no saved key"))
                        })?;
                        let scope = scope
                            .ok_or_else(|| mlua::Error::runtime("Missing extension key scope"))?;
                        super::keys::unlock(scope, token).map_err(mlua::Error::external)
                    })?,
                )?;
            }
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
            handoff,
            segments: Default::default(),
        })
    }
    pub(super) fn handle(&self, message: serde_json::Value) -> Result<serde_json::Value> {
        let op = message["op"]
            .as_str()
            .context("Missing provider operation")?;
        match op {
            "segment_begin" | "segment_feed" | "segment_run" | "segment_read" => {
                ensure!(
                    self.role == Role::Processor,
                    "Resolver workers cannot process segments"
                );
                self.segments
                    .borrow_mut()
                    .handle(&self.lua, &self.callbacks, &message)
            }
            "resolve" => {
                ensure!(
                    self.role == Role::Resolver,
                    "Processor workers cannot resolve or refresh"
                );
                let handoff = self
                    .handoff
                    .as_ref()
                    .context("Missing resolver key scope")?;
                let _resolution = handoff.begin();
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
                let value = json::media_from_lua(&self.lua, value, super::media::MAX_PLAN_BYTES)?;
                super::Resolution::parse(value.clone())?;
                Ok(serde_json::json!({"plan":value,"handoff":handoff.take()?}))
            }
            "media_transform" => {
                ensure!(
                    self.role == Role::Processor,
                    "Resolver workers cannot process file bytes"
                );
                let context: super::media::Chunk =
                    serde_json::from_value(message["context"].clone())?;
                context.validate()?;
                let encoded = message["bytes"]
                    .as_str()
                    .context("Missing provider chunk")?;
                ensure!(
                    encoded.len() <= CHUNK_BYTES.div_ceil(3) * 4,
                    "Provider input too large"
                );
                let bytes = STANDARD.decode(encoded)?;
                let offset = message["offset"].as_u64().context("Missing byte offset")?;
                ensure!(
                    bytes.len() <= CHUNK_BYTES
                        && offset
                            .checked_add(bytes.len() as u64)
                            .is_some_and(|end| end <= context.size),
                    "Invalid media chunk bounds"
                );
                let output: mlua::LuaString =
                    self.callbacks.get::<mlua::Function>("transform")?.call((
                        to_lua(&self.lua, &context)?,
                        self.lua.create_string(&bytes)?,
                        offset,
                    ))?;
                ensure!(
                    output.as_bytes().len() == bytes.len(),
                    "Provider must return exactly one output byte per input byte"
                );
                Ok(serde_json::json!(STANDARD.encode(output.as_bytes())))
            }
            "media_finish" => {
                ensure!(
                    self.role == Role::Processor,
                    "Resolver workers cannot finish files"
                );
                let plan: super::media::Plan = serde_json::from_value(message["plan"].clone())?;
                plan.validate(false)?;
                let size = match message.get("size") {
                    None => plan.size(),
                    Some(value) => value.as_u64().context("Invalid media output size")?,
                };
                let maximum: u64 = plan
                    .tracks
                    .iter()
                    .flat_map(|t| &t.segments)
                    .map(|s| s.output_limit())
                    .sum();
                ensure!(size > 0 && size <= maximum, "Invalid media output size");
                plan.output_budget()?.include(size)?;
                ensure!(
                    plan.uses_segment_callbacks() || size == plan.size(),
                    "Fixed-size media output changed length"
                );
                let result = serde_json::json!({"size":size,"tracks":plan.tracks.len(),"stage":"before_mux"});
                let finished: Value = self
                    .callbacks
                    .get::<mlua::Function>("finish")?
                    .call((to_lua(&self.lua, &plan)?, to_lua(&self.lua, &result)?))?;
                ensure!(
                    matches!(finished, Value::Boolean(true)),
                    "Provider finish must return true"
                );
                Ok(serde_json::json!(true))
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
    let first = super::process::read_init_frame(&mut input)?;
    ensure!(first["op"] == "init", "Provider must initialize first");
    let extension: Option<crate::extensions::Context> =
        serde_json::from_value(first["extension"].clone())?;
    // Never construct a Fetcher or load profile/network settings in a processor.
    // Reject missing/unknown roles rather than falling back to resolver access.
    let capabilities = match serde_json::from_value::<Role>(first["role"].clone())? {
        Role::Resolver => {
            let paths = serde_json::from_str(&std::env::var("IDM_EX_PROVIDER_PROFILE")?)?;
            let store = crate::store::Store::open(paths)?;
            let mut fetcher = Fetcher::new(crate::proxy::Route::load(&store)?)?;
            if let Some(extension) = &extension {
                fetcher = fetcher.limit_hosts(extension.hosts.clone());
            }
            Capabilities::Resolver(Box::new(fetcher))
        }
        Role::Processor => Capabilities::Processor(uuid::Uuid::parse_str(
            first["script_id"]
                .as_str()
                .context("Missing provider identity")?,
        )?),
    };
    let runtime = Runtime::with_handoff(
        first["source"]
            .as_str()
            .context("Missing provider source")?,
        capabilities,
        extension.as_ref(),
        serde_json::from_value(first["handoff_scope"].clone())?,
        uuid::Uuid::parse_str(
            first["script_id"]
                .as_str()
                .context("Missing provider identity")?,
        )?,
        serde_json::from_value(first["handoff"].clone())?,
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

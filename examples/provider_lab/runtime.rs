//! Deliberately small lab runtime; this is not the installed app's Lua API.
use super::{Grants, Plan, crypto};
use anyhow::{Context, Result, ensure};
use mlua::{Function, HookTriggers, Lua, LuaSerdeExt, StdLib, Table, Value, VmState};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub struct Runtime {
    lua: Lua,
    provider: Table,
    deadline: Rc<Cell<Instant>>,
    stage: Rc<Cell<&'static str>>,
    budget: Duration,
    cancel: CancellationToken,
}

#[cfg(test)]
type HookObserver = Box<dyn Fn(&str)>;
#[cfg(test)]
thread_local! {
    static HOOK_OBSERVER: std::cell::RefCell<Option<HookObserver>> = const { std::cell::RefCell::new(None) };
}

/// Signal an actual executing Lua hook, instead of racing wall-clock sleeps.
#[cfg(test)]
pub fn observe_hooks<T>(observer: impl Fn(&str) + 'static, test: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            HOOK_OBSERVER.with(|value| *value.borrow_mut() = None);
        }
    }
    HOOK_OBSERVER.with(|value| *value.borrow_mut() = Some(Box::new(observer)));
    let _reset = Reset;
    test()
}

impl Runtime {
    pub fn new(
        source: &str,
        grants: &Grants,
        cancel: CancellationToken,
        budget: Duration,
    ) -> Result<Self> {
        ensure!(source.len() <= 65_536, "Provider script exceeds 64 KiB");
        ensure!(
            budget > Duration::ZERO && budget <= Duration::from_secs(5),
            "Invalid callback budget"
        );
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            Default::default(),
        )?;
        lua.set_memory_limit(16 * 1024 * 1024)?;
        // No file/network/process/module access. Protected calls and coroutines
        // are omitted in this lab so a script cannot catch a quota/cancel hook
        // and loop forever. The production worker boundary is a separate task.
        for name in [
            "dofile", "loadfile", "load", "pcall", "xpcall", "print", "warn",
        ] {
            lua.globals().set(name, Value::Nil)?;
        }
        let deadline = Rc::new(Cell::new(Instant::now() + budget));
        let stage = Rc::new(Cell::new("load"));
        #[cfg(test)]
        let hook_stage = stage.clone();
        let until = deadline.clone();
        let token = cancel.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(1000),
            move |_, _| {
                #[cfg(test)]
                HOOK_OBSERVER.with(|value| {
                    if let Some(observer) = value.borrow().as_ref() {
                        observer(hook_stage.get());
                    }
                });
                if token.is_cancelled() {
                    return Err(mlua::Error::runtime("Provider cancelled"));
                }
                if Instant::now() >= until.get() {
                    return Err(mlua::Error::runtime("Provider callback timed out"));
                }
                Ok(VmState::Continue)
            },
        )?;
        let xd = lua.create_table()?;
        xd.set("provider_api", "experimental-0")?;
        let metadata = grants.metadata.clone();
        xd.set(
            "metadata",
            lua.create_function(move |lua, asset: String| {
                if asset != "sample-episode" {
                    return Err(mlua::Error::runtime("Asset is not granted"));
                }
                lua.to_value(&metadata)
            })?,
        )?;
        let block_keys = grants.keys.clone();
        xd.set(
            "aes_blocks",
            lua.create_function(move |lua, (key_id, blocks): (String, mlua::LuaString)| {
                let key = block_keys
                    .get(&key_id)
                    .ok_or_else(|| mlua::Error::runtime("Key is not granted"))?;
                let result = crypto::encrypt_blocks(key, &blocks.as_bytes())
                    .map_err(mlua::Error::external)?;
                lua.create_string(result)
            })?,
        )?;
        let keys = grants.keys.clone();
        xd.set("aes_ctr", lua.create_function(move |lua, (key_id, counter, data, offset):
            (String, String, mlua::LuaString, u64)| {
            let result = (|| -> Result<Vec<u8>> {
                let key = keys.get(&key_id).context("Key is not granted")?;
                let counter: [u8; 16] = crypto::unhex(&counter)?.try_into()
                    .map_err(|_| anyhow::anyhow!("Counter must be 16 bytes"))?;
                crypto::ctr(key, &counter, &data.as_bytes(), offset)
            })().map_err(mlua::Error::external)?;
            lua.create_string(result)
        })?)?;
        lua.globals().set("xd", xd)?;
        let provider: Table = lua
            .load(source)
            .set_name("provider.lua")
            .set_mode(mlua::chunk::ChunkMode::Text)
            .eval()?;
        ensure!(!cancel.is_cancelled(), "Provider cancelled");
        for name in ["resolve", "refresh", "transform", "finish"] {
            let _: Function = provider
                .get(name)
                .with_context(|| format!("Missing provider.{name} function"))?;
        }
        Ok(Self {
            lua,
            provider,
            deadline,
            stage,
            budget,
            cancel,
        })
    }

    fn before(&self, stage: &'static str) -> Result<()> {
        ensure!(!self.cancel.is_cancelled(), "Provider cancelled");
        self.stage.set(stage);
        self.deadline.set(Instant::now() + self.budget);
        Ok(())
    }

    pub fn resolve(&self, previous: Option<&Plan>) -> Result<Plan> {
        let context = self.lua.create_table()?;
        context.set("asset", "sample-episode")?;
        let name = if previous.is_some() {
            "refresh"
        } else {
            "resolve"
        };
        self.before(name)?;
        let function: Function = self.provider.get(name)?;
        let prior = match previous {
            Some(p) => self.lua.to_value(p)?,
            None => Value::Nil,
        };
        let value: Value = function.call((context, prior))?;
        let plan: Plan = self.lua.from_value(value)?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn transform(&self, input: &[u8], offset: u64, plan: &Plan) -> Result<Vec<u8>> {
        ensure!(
            input.len() <= crypto::MAX_CHUNK,
            "Transform input exceeds 256 KiB"
        );
        self.before("transform")?;
        let function: Function = self.provider.get("transform")?;
        let value: Value = function.call((
            self.lua.create_string(input)?,
            offset,
            self.lua.to_value(plan)?,
        ))?;
        let Value::String(bytes) = value else {
            anyhow::bail!("Transform must return a binary string");
        };
        // This first experiment supports independently restartable, same-length
        // chunks. Variable-length/stateful codecs need another checkpoint contract.
        ensure!(
            bytes.as_bytes().len() == input.len(),
            "Transform must preserve chunk length"
        );
        ensure!(!self.cancel.is_cancelled(), "Provider cancelled");
        Ok(bytes.as_bytes().to_vec())
    }

    pub fn finish(&self, plan: &Plan, hash: &str, size: u64) -> Result<()> {
        self.before("finish")?;
        let summary = self.lua.create_table()?;
        summary.set("sha256", hash)?;
        summary.set("size", size)?;
        let function: Function = self.provider.get("finish")?;
        let result: Value = function.call((self.lua.to_value(plan)?, summary))?;
        ensure!(
            matches!(result, Value::Boolean(true)),
            "Provider finish did not accept the output"
        );
        Ok(())
    }
}

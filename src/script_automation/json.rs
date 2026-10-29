//! Bound the expanded Lua graph before serde allocates owned Rust values.
use mlua::{Lua, LuaSerdeExt, Value};
use std::collections::HashSet;

pub(super) const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 65_536;

struct Budget {
    remaining: usize,
    nodes: usize,
    parents: HashSet<usize>,
}
impl Budget {
    fn bytes(&mut self, bytes: usize) -> mlua::Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or_else(|| mlua::Error::runtime("Expanded JSON value exceeds its byte limit"))?;
        Ok(())
    }
    fn visit(&mut self, value: &Value, depth: usize) -> mlua::Result<()> {
        if depth > MAX_DEPTH || self.nodes >= MAX_NODES {
            return Err(mlua::Error::runtime(
                "JSON value exceeds its depth or item limit",
            ));
        }
        self.nodes += 1;
        match value {
            Value::String(text) => {
                // serde treats non-UTF8 Lua strings as byte arrays. Reject that
                // implicit expansion rather than budgeting it as one string.
                text.to_str()?;
                self.bytes(2)?;
                for byte in text.as_bytes().iter() {
                    self.bytes(match byte {
                        b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
                        0..=31 => 6,
                        _ => 1,
                    })?;
                }
            }
            Value::Table(table) => {
                let pointer = table.to_pointer() as usize;
                if !self.parents.insert(pointer) {
                    return Err(mlua::Error::runtime("Recursive tables cannot be JSON"));
                }
                if table.raw_len() > MAX_NODES {
                    return Err(mlua::Error::runtime("JSON array exceeds its item limit"));
                }
                self.bytes(2)?;
                // Raw pairs do not invoke script metamethods. Repeated references
                // are charged each time: serde expands them each time as well.
                for pair in table.pairs::<Value, Value>() {
                    let (key, value) = pair?;
                    self.bytes(2)?;
                    self.visit(&key, depth + 1)?;
                    self.visit(&value, depth + 1)?;
                }
                self.parents.remove(&pointer);
            }
            Value::Nil => self.bytes(4)?,
            Value::LightUserData(value) if value.0.is_null() => self.bytes(4)?,
            Value::Boolean(_) => self.bytes(5)?,
            Value::Integer(number) => self.bytes(number.to_string().len())?,
            Value::Number(number) => self.bytes(number.to_string().len().max(4))?,
            _ => return Err(mlua::Error::runtime("Unsupported JSON value")),
        }
        Ok(())
    }
}

pub(crate) fn from_lua(lua: &Lua, value: Value, limit: usize) -> mlua::Result<serde_json::Value> {
    Budget {
        remaining: limit,
        nodes: 0,
        parents: HashSet::new(),
    }
    .visit(&value, 0)?;
    lua.from_value(value)
}

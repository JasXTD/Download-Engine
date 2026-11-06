//! Whole-container callbacks still cross IPC in bounded chunks, never file paths.
use super::*;
use crate::provider::media::{Chunk, MAX_SEGMENT_INPUT, MAX_SEGMENT_OUTPUT};

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Input {
        context: Chunk,
        prepare: bool,
        output_limit: u64,
        bytes: Vec<u8>,
    },
    Output {
        bytes: Vec<u8>,
        offset: usize,
    },
    StreamInput {
        context: Chunk,
        output_limit: u64,
        spool: super::stream::Spool,
    },
    StreamOutput {
        spool: super::stream::Spool,
        offset: u64,
    },
}
#[derive(Default)]
pub(super) struct Segments {
    state: State,
}
impl Segments {
    pub fn handle(
        &mut self,
        lua: &Lua,
        callbacks: &Table,
        message: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let state = std::mem::take(&mut self.state);
        // A failed request drops every buffer and cannot resume a stale operation.
        let (next, reply) = Self::step(state, lua, callbacks, message)?;
        self.state = next;
        Ok(reply)
    }
    fn step(
        state: State,
        lua: &Lua,
        callbacks: &Table,
        message: &serde_json::Value,
    ) -> Result<(State, serde_json::Value)> {
        use serde_json::json;
        match (message["op"].as_str().unwrap_or(""), state) {
            ("segment_begin", State::Idle) => {
                let context: Chunk = serde_json::from_value(message["context"].clone())?;
                context.validate()?;
                let prepare = message["prepare"]
                    .as_bool()
                    .context("Missing segment operation")?;
                let stream = message["stream"] == true;
                let output_limit = message["output_limit"]
                    .as_u64()
                    .context("Missing segment output allowance")?;
                ensure!(
                    if prepare {
                        output_limit == 0
                    } else {
                        output_limit > 0
                            && output_limit
                                <= if stream {
                                    crate::provider::media::MAX_STREAM_OUTPUT
                                } else {
                                    MAX_SEGMENT_OUTPUT as u64
                                }
                    },
                    "Invalid segment output allowance"
                );
                if message["stream"] == true {
                    ensure!(
                        message["prepare"] == false,
                        "Initialization uses prepare_track"
                    );
                    return Ok((
                        State::StreamInput {
                            context,
                            output_limit,
                            spool: super::stream::Spool::new()?,
                        },
                        json!(true),
                    ));
                }
                ensure!(
                    context.size <= MAX_SEGMENT_INPUT as u64,
                    "Whole-segment Lua input exceeds 8 MiB"
                );
                Ok((
                    State::Input {
                        context,
                        prepare,
                        output_limit,
                        bytes: Vec::new(),
                    },
                    json!(true),
                ))
            }
            (
                "segment_feed",
                State::StreamInput {
                    context,
                    output_limit,
                    mut spool,
                },
            ) => {
                let encoded = message["bytes"].as_str().context("Missing stream bytes")?;
                ensure!(
                    encoded.len() <= CHUNK_BYTES.div_ceil(3) * 4,
                    "Stream chunk exceeds limit"
                );
                let bytes = STANDARD.decode(encoded)?;
                ensure!(
                    !bytes.is_empty() && bytes.len() <= CHUNK_BYTES,
                    "Invalid stream input"
                );
                spool.append(&bytes, context.size)?;
                Ok((
                    State::StreamInput {
                        context,
                        output_limit,
                        spool,
                    },
                    json!(true),
                ))
            }
            (
                "segment_run",
                State::StreamInput {
                    context,
                    output_limit,
                    spool,
                },
            ) => {
                ensure!(spool.len == context.size, "Incomplete stream input");
                let spool = super::stream::run(lua, callbacks, &context, spool, output_limit)?;
                let len = spool.len;
                Ok((State::StreamOutput { spool, offset: 0 }, json!(len)))
            }
            ("segment_read", State::StreamOutput { mut spool, offset }) => {
                ensure!(
                    message["offset"].as_u64() == Some(offset),
                    "Invalid stream output offset"
                );
                let bytes = spool.read(offset, CHUNK_BYTES)?;
                let end = offset + bytes.len() as u64;
                let next = if end == spool.len {
                    State::Idle
                } else {
                    State::StreamOutput { spool, offset: end }
                };
                Ok((next, json!(STANDARD.encode(bytes))))
            }
            (
                "segment_feed",
                State::Input {
                    context,
                    prepare,
                    output_limit,
                    mut bytes,
                },
            ) => {
                let encoded = message["bytes"].as_str().context("Missing segment bytes")?;
                ensure!(
                    encoded.len() <= CHUNK_BYTES.div_ceil(3) * 4,
                    "Segment chunk exceeds limit"
                );
                let chunk = STANDARD.decode(encoded)?;
                ensure!(
                    !chunk.is_empty()
                        && chunk.len() <= CHUNK_BYTES
                        && bytes.len() + chunk.len() <= context.size as usize,
                    "Invalid segment input bounds"
                );
                bytes.extend_from_slice(&chunk);
                Ok((
                    State::Input {
                        context,
                        prepare,
                        output_limit,
                        bytes,
                    },
                    json!(true),
                ))
            }
            (
                "segment_run",
                State::Input {
                    context,
                    prepare,
                    output_limit,
                    bytes,
                },
            ) => {
                ensure!(
                    bytes.len() == context.size as usize,
                    "Incomplete segment input"
                );
                let name = if prepare {
                    "prepare_track"
                } else {
                    "transform_segment"
                };
                let callback = callbacks
                    .get::<mlua::Function>(name)
                    .with_context(|| format!("Provider needs {name} for this plan"))?;
                let value: Value =
                    callback.call((to_lua(lua, &context)?, lua.create_string(&bytes)?))?;
                if prepare {
                    ensure!(
                        matches!(value, Value::Boolean(true)),
                        "Provider prepare_track must return true"
                    );
                    Ok((State::Idle, json!(0)))
                } else {
                    let Value::String(output) = value else {
                        anyhow::bail!("Provider transform_segment must return a binary string")
                    };
                    let n = output.as_bytes().len();
                    ensure!(
                        n > 0 && n as u64 <= output_limit,
                        "Whole-segment output exceeds its allowance or is empty"
                    );
                    Ok((
                        State::Output {
                            bytes: output.as_bytes().to_vec(),
                            offset: 0,
                        },
                        json!(n),
                    ))
                }
            }
            ("segment_read", State::Output { bytes, offset }) => {
                ensure!(
                    message["offset"].as_u64() == Some(offset as u64),
                    "Invalid segment output offset"
                );
                let end = (offset + CHUNK_BYTES).min(bytes.len());
                let reply = json!(STANDARD.encode(&bytes[offset..end]));
                let next = if end == bytes.len() {
                    State::Idle
                } else {
                    State::Output { bytes, offset: end }
                };
                Ok((next, reply))
            }
            _ => anyhow::bail!("Invalid whole-segment operation order"),
        }
    }
}

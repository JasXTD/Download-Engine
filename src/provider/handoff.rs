//! Ephemeral, OS-protected keys cross only from one resolver to its processor.
//! This envelope never enters a plan, checkpoint, export or extension package.
use anyhow::{Result, ensure};
use mlua::{Lua, Table};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};
use uuid::Uuid;

const MAX_KEYS: usize = 64;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    pub scope: Uuid,
    owner: Uuid,
    values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sessions: BTreeMap<String, String>,
}
impl Keys {
    pub fn empty(scope: Uuid, owner: Uuid) -> Self {
        Self {
            scope,
            owner,
            values: BTreeMap::new(),
            sessions: BTreeMap::new(),
        }
    }
    pub fn validate(&self, scope: Uuid, owner: Uuid) -> Result<()> {
        ensure!(
            !scope.is_nil() && self.scope == scope && self.owner == owner,
            "Provider key handoff belongs to another run or provider"
        );
        ensure!(
            self.values.len() <= MAX_KEYS,
            "Provider key handoff exceeds 64 keys"
        );
        for (name, token) in &self.values {
            validate_name(name)?;
            ensure!(
                token.len() <= 4096 && token.starts_with("provider-key-v1:"),
                "Invalid protected session key"
            );
        }
        ensure!(
            self.sessions.len() <= 8,
            "Provider handoff exceeds 8 HTTP sessions"
        );
        ensure!(
            self.sessions.values().map(String::len).sum::<usize>() <= 1024 * 1024,
            "HTTP session handoff exceeds 1 MiB"
        );
        for (name, token) in &self.sessions {
            validate_name(name)?;
            ensure!(token.len() <= 360_000, "Invalid protected HTTP session");
        }
        Ok(())
    }
    /// The downloader consumes these before starting the offline processor.
    pub(crate) fn take_sessions(
        &mut self,
    ) -> Result<BTreeMap<String, crate::script_automation::HttpSession>> {
        std::mem::take(&mut self.sessions)
            .into_iter()
            .map(|(name, token)| {
                let session = crate::script_automation::HttpSession::open(
                    session_scope(self.scope, self.owner, &name),
                    &token,
                )?;
                Ok((name, session))
            })
            .collect()
    }
    pub(crate) fn validate_plan(&self, plan: &super::Resolution) -> Result<()> {
        if let super::Resolution::Media(plan) = plan {
            for segment in plan.tracks.iter().flat_map(|t| &t.segments) {
                if let Some(name) = &segment.session {
                    ensure!(
                        self.sessions.contains_key(name),
                        "Media plan references a session that was not handed off"
                    );
                }
            }
        } else {
            ensure!(
                self.sessions.is_empty(),
                "HTTP session handoff requires a media plan"
            );
        }
        Ok(())
    }
}

pub(super) fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')),
        "Session key names must be 1-64 ASCII letters, digits, underscores, hyphens or dots"
    );
    Ok(())
}
fn session_scope(scope: Uuid, owner: Uuid, name: &str) -> Uuid {
    key_scope(scope, owner, &format!("http-session:{name}"))
}

fn key_scope(scope: Uuid, owner: Uuid, name: &str) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"Download Engine session AES key v1\0");
    hash.update(scope.as_bytes());
    hash.update(owner.as_bytes());
    hash.update(name.as_bytes());
    let mut binding = [0; 16];
    binding.copy_from_slice(&hash.finalize()[..16]);
    Uuid::from_bytes(binding)
}

struct State {
    scope: Uuid,
    owner: Uuid,
    active: Cell<bool>,
    values: RefCell<BTreeMap<String, String>>,
    sessions: RefCell<BTreeMap<String, crate::script_automation::HttpSession>>,
}
pub(super) struct Pending(Rc<State>);
pub(super) struct ResolutionGuard(Rc<State>);
impl Drop for ResolutionGuard {
    fn drop(&mut self) {
        self.0.active.set(false);
        self.0.values.borrow_mut().clear();
        self.0.sessions.borrow_mut().clear();
    }
}
impl Pending {
    pub fn install(lua: &Lua, xd: &Table, scope: Uuid, owner: Uuid) -> Result<Self> {
        let state = Rc::new(State {
            scope,
            owner,
            active: Cell::new(false),
            values: RefCell::new(BTreeMap::new()),
            sessions: RefCell::new(BTreeMap::new()),
        });
        let captured = state.clone();
        xd.set(
            "handoff_key",
            lua.create_function(move |_, (name, key): (String, mlua::LuaString)| {
                if !captured.active.get() {
                    return Err(mlua::Error::runtime(
                        "xd.handoff_key is only available during resolve or refresh",
                    ));
                }
                validate_name(&name).map_err(mlua::Error::external)?;
                if !matches!(key.as_bytes().len(), 16 | 24 | 32) {
                    return Err(mlua::Error::runtime("AES key must be 16, 24 or 32 bytes"));
                }
                let mut values = captured.values.borrow_mut();
                if values.len() >= MAX_KEYS || values.contains_key(&name) {
                    return Err(mlua::Error::runtime(
                        "A resolution accepts at most 64 unique session keys",
                    ));
                }
                // Sealing copies directly into a zeroizing buffer. Lua still owns the
                // input it supplied; this API cannot erase strings retained by a script.
                let token = super::keys::seal_bytes(
                    key_scope(captured.scope, captured.owner, &name),
                    key.as_bytes().as_ref(),
                )
                .map_err(|_| {
                    mlua::Error::runtime(
                        "Operating-system protection could not seal the session key",
                    )
                })?;
                values.insert(name, token);
                Ok(())
            })?,
        )?;
        let captured = state.clone();
        if xd.get::<mlua::Value>("http_session")?.is_function() {
            xd.set(
                "handoff_session",
                lua.create_function(move |_, (name, session): (String, mlua::AnyUserData)| {
                    if !captured.active.get() {
                        return Err(mlua::Error::runtime(
                            "xd.handoff_session is only available during resolve or refresh",
                        ));
                    }
                    validate_name(&name).map_err(mlua::Error::external)?;
                    let session = session
                        .borrow::<crate::script_automation::HttpSession>()?
                        .clone();
                    let mut sessions = captured.sessions.borrow_mut();
                    if sessions.len() >= 8 || sessions.contains_key(&name) {
                        return Err(mlua::Error::runtime(
                            "A resolution accepts at most 8 unique HTTP sessions",
                        ));
                    }
                    sessions.insert(name, session);
                    Ok(())
                })?,
            )?;
        }
        Ok(Self(state))
    }
    pub fn begin(&self) -> ResolutionGuard {
        self.0.values.borrow_mut().clear();
        self.0.sessions.borrow_mut().clear();
        self.0.active.set(true);
        ResolutionGuard(self.0.clone())
    }
    pub fn take(&self) -> Result<Keys> {
        let sessions = std::mem::take(&mut *self.0.sessions.borrow_mut())
            .into_iter()
            .map(|(name, session)| {
                let token = session.seal(session_scope(self.0.scope, self.0.owner, &name))?;
                Ok((name, token))
            })
            .collect::<Result<_>>()?;
        let keys = Keys {
            scope: self.0.scope,
            owner: self.0.owner,
            values: std::mem::take(&mut *self.0.values.borrow_mut()),
            sessions,
        };
        keys.validate(self.0.scope, self.0.owner)?;
        Ok(keys)
    }
}

pub(super) fn install_processor(lua: &Lua, xd: &Table, keys: Keys) -> Result<()> {
    ensure!(
        keys.sessions.is_empty(),
        "HTTP credentials must not reach the processor"
    );
    #[cfg(any(windows, unix))]
    {
        // Unlock before evaluating Lua. A tampered envelope cannot fall back to
        // another key source, and no protected token is exposed to the script.
        let mut handles = BTreeMap::new();
        for (name, token) in keys.values {
            let handle = lua.create_userdata(super::keys::unlock(
                key_scope(keys.scope, keys.owner, &name),
                &token,
            )?)?;
            handles.insert(name, handle);
        }
        xd.set(
            "session_key",
            lua.create_function(move |_, name: String| {
                validate_name(&name).map_err(mlua::Error::external)?;
                handles.get(&name).cloned().ok_or_else(|| {
                    mlua::Error::runtime(
                        "No session key with this name was handed off by the resolver",
                    )
                })
            })?,
        )?;
    }
    #[cfg(not(any(windows, unix)))]
    let _ = (lua, xd, keys);
    Ok(())
}

#[cfg(test)]
mod tests;

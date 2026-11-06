//! Anonymous scratch files owned by the worker. Lua gets bounded byte operations,
//! never names, raw handles or persistent filesystem access.
use super::*;
use crate::provider::media::budget;
use mlua::{UserData, UserDataMethods};
use std::{
    cell::RefCell,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    rc::Rc,
};

pub(super) struct Spool {
    pub file: File,
    pub len: u64,
}
impl Spool {
    pub fn new() -> Result<Self> {
        Ok(Self {
            file: tempfile::tempfile()?,
            len: 0,
        })
    }
    pub fn append(&mut self, bytes: &[u8], limit: u64) -> Result<()> {
        let end = self
            .len
            .checked_add(bytes.len() as u64)
            .context("Stream output size overflow")?;
        ensure!(end <= limit, "Stream output exceeds its limit");
        budget::check_space(&std::env::temp_dir(), bytes.len() as u64)?;
        // write_all may write a prefix before failing. Charge the full request
        // first so a Lua pcall cannot retry failed writes for uncounted disk use.
        self.len = end;
        self.file.write_all(bytes)?;
        Ok(())
    }
    pub fn read(&mut self, offset: u64, count: usize) -> Result<Vec<u8>> {
        ensure!(
            count <= CHUNK_BYTES && offset <= self.len,
            "Invalid stream read bounds"
        );
        let n = (self.len - offset).min(count as u64) as usize;
        let mut bytes = vec![0; n];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}
type Shared = Rc<RefCell<Option<Spool>>>;
struct Reader(Shared);
struct Writer(Shared, u64);
impl UserData for Reader {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("size", |_, this, ()| {
            this.0
                .borrow()
                .as_ref()
                .map(|s| s.len)
                .ok_or_else(|| mlua::Error::runtime("Segment reader expired"))
        });
        methods.add_method("read", |lua, this, (offset, count): (u64, usize)| {
            let mut held = this.0.borrow_mut();
            let spool = held
                .as_mut()
                .ok_or_else(|| mlua::Error::runtime("Segment reader expired"))?;
            lua.create_string(spool.read(offset, count).map_err(mlua::Error::external)?)
        });
    }
}
impl UserData for Writer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("write", |_, this, bytes: mlua::LuaString| {
            if bytes.as_bytes().is_empty() || bytes.as_bytes().len() > CHUNK_BYTES {
                return Err(mlua::Error::runtime("Write 1 byte to 256 KiB at a time"));
            }
            let mut held = this.0.borrow_mut();
            let spool = held
                .as_mut()
                .ok_or_else(|| mlua::Error::runtime("Segment writer expired"))?;
            spool
                .append(bytes.as_bytes().as_ref(), this.1)
                .map_err(mlua::Error::external)
        });
    }
}
struct Expire(Shared, Shared);
impl Drop for Expire {
    fn drop(&mut self) {
        self.0.borrow_mut().take();
        self.1.borrow_mut().take();
    }
}
pub(super) fn run(
    lua: &Lua,
    callbacks: &Table,
    context: &crate::provider::media::Chunk,
    input: Spool,
    output_limit: u64,
) -> Result<Spool> {
    let source = Rc::new(RefCell::new(Some(input)));
    let output = Rc::new(RefCell::new(Some(Spool::new()?)));
    let _expire = Expire(source.clone(), output.clone());
    let callback = callbacks
        .get::<mlua::Function>("transform_stream")
        .context("Provider needs transform_stream for this plan")?;
    let result: Value = callback.call((
        to_lua(lua, context)?,
        Reader(source),
        Writer(output.clone(), output_limit),
    ))?;
    ensure!(
        matches!(result, Value::Boolean(true)),
        "transform_stream must return true"
    );
    let result = output
        .borrow_mut()
        .take()
        .context("Segment writer expired")?;
    ensure!(result.len > 0, "Stream processor returned no bytes");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_disk_writes_still_consume_the_stream_allowance() {
        let file = tempfile::NamedTempFile::new().unwrap();
        // A real read-only handle produces a write failure without filling a disk.
        let mut spool = Spool {
            file: File::open(file.path()).unwrap(),
            len: 0,
        };
        assert!(spool.append(b"1234", 4).is_err());
        assert_eq!(spool.file.metadata().unwrap().len(), 0);
        assert!(
            spool
                .append(b"x", 4)
                .unwrap_err()
                .to_string()
                .contains("exceeds its limit")
        );
        assert!(spool.read(0, 4).is_err()); // Incomplete bytes cannot be returned as valid output.
    }
}

//! Direct SQL is restricted to owned test/screenshot fixtures. These version
//! capabilities match src/store/migrations.rs and leave all write guards active.
pub(super) fn open(path: impl AsRef<std::path::Path>) -> rusqlite::Result<rusqlite::Connection> {
    use rusqlite::functions::FunctionFlags;
    let conn = rusqlite::Connection::open(path)?;
    for name in [
        "xdownloader_writer_version",
        "xdownloader_schema_version",
        "xdownloader_data_version",
    ] {
        conn.create_scalar_function(
            name,
            0,
            FunctionFlags::SQLITE_UTF8
                | FunctionFlags::SQLITE_DETERMINISTIC
                | FunctionFlags::SQLITE_INNOCUOUS,
            |_| Ok(1_i64),
        )?;
    }
    Ok(conn)
}

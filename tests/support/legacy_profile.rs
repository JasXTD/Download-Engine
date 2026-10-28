/// Fixtures that restore historical tables also restore the historical,
/// unversioned contract. This helper is never compiled into shipped binaries.
pub(crate) fn unversioned_fixture(conn: &rusqlite::Connection) {
    let guards = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='trigger' AND (name LIKE 'writer_guard_%' OR name LIKE 'profile_format_%')")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for name in guards {
        conn.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))
            .unwrap();
    }
    conn.execute_batch("DROP TABLE profile_format; DROP TABLE profile_migrations;")
        .unwrap();
}

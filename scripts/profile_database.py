"""Connections for owned test profiles; leave production write guards active."""
import sqlite3


def open_profile_database(*args, **kwargs):
    connection = sqlite3.connect(*args, **kwargs)
    # Keep supported versions in sync with src/store/migrations.rs.
    for name in ("writer", "schema", "data"):
        connection.create_function(
            f"xdownloader_{name}_version", 0, lambda: 1, deterministic=True
        )
    return connection

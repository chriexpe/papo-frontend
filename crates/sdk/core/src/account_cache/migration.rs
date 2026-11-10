//! Forward-only cache migrations; version-one SQL and identity validation remain in schema.

use crate::storage_worker::WorkerError;
use rusqlite::Connection;

/// Upgrade a verified v1 cache without rewriting any existing resource rows.
pub(super) fn version_two(connection: &mut Connection) -> Result<(), WorkerError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "ALTER TABLE cache_metadata ADD COLUMN history_revision INTEGER NOT NULL DEFAULT 0;
        CREATE TABLE history_sets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
            revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
            metadata BLOB NOT NULL DEFAULT X''
        );
        CREATE TABLE history_channels (
            channel_id BLOB PRIMARY KEY CHECK(length(channel_id)=16),
            current_set INTEGER REFERENCES history_sets(id),
            verified_generation INTEGER CHECK(verified_generation>=0)
        );
        CREATE TABLE history_manifest (
            revision INTEGER NOT NULL,
            channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
            set_id INTEGER NOT NULL REFERENCES history_sets(id),
            PRIMARY KEY(revision,channel_id)
        );
        CREATE INDEX history_manifest_set ON history_manifest(set_id);
        CREATE TABLE chunks (
            set_id INTEGER NOT NULL REFERENCES history_sets(id),
            id BLOB NOT NULL CHECK(length(id)=16),
            payload BLOB NOT NULL,
            PRIMARY KEY(set_id,id)
        );
        CREATE TABLE chunk_items (
            set_id INTEGER NOT NULL REFERENCES history_sets(id),
            id BLOB NOT NULL CHECK(length(id)=16),
            payload BLOB NOT NULL,
            PRIMARY KEY(set_id,id)
        );
        CREATE TABLE messages (
            set_id INTEGER NOT NULL REFERENCES history_sets(id),
            id BLOB NOT NULL CHECK(length(id)=16),
            payload BLOB NOT NULL,
            PRIMARY KEY(set_id,id)
        );
        CREATE TABLE pins (
            set_id INTEGER NOT NULL REFERENCES history_sets(id),
            id BLOB NOT NULL CHECK(length(id)=16),
            payload BLOB NOT NULL,
            PRIMARY KEY(set_id,id)
        );
        UPDATE cache_metadata SET schema_version=2 WHERE singleton=1;",
    )?;
    transaction.pragma_update(None, "user_version", 2)?;
    transaction.commit()?;
    Ok(())
}

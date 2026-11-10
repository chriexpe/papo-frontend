//! Frozen version-one schema, forward migration dispatch and identity checks; scheduling lives elsewhere.

use crate::{AccountId, storage_worker::WorkerError};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) const TABLE_COUNT: usize = 8;
pub(super) const MEMBER_ROLES: usize = 7;

pub(super) struct Table {
    pub create: &'static str,
    pub insert: &'static str,
    pub select: &'static str,
    pub switch: &'static str,
    pub prune: &'static str,
}

macro_rules! table {
    ($name:literal, $column:literal) => {
        Table {
            create: concat!("CREATE TABLE ", $name, " (revision INTEGER NOT NULL, id BLOB NOT NULL CHECK(length(id)=16), position INTEGER NOT NULL, payload BLOB NOT NULL, PRIMARY KEY(revision,id)); CREATE INDEX ", $name, "_position ON ", $name, "(revision,position)"),
            insert: concat!("INSERT INTO ", $name, " (revision,id,position,payload) VALUES (?1,?2,?3,?4)"),
            select: concat!("SELECT position,payload FROM ", $name, " WHERE revision=?1 AND position>=?2 ORDER BY position LIMIT 500"),
            switch: concat!("UPDATE cache_metadata SET ", $column, "=?1 WHERE singleton=1"),
            prune: concat!("DELETE FROM ", $name, " WHERE rowid IN (SELECT rowid FROM ", $name, " WHERE revision < (SELECT ", $column, " FROM cache_metadata) OR revision > (SELECT ", $column, " FROM cache_metadata) LIMIT 100)"),
        }
    };
}

pub(super) const TABLES: [Table; TABLE_COUNT] = [
    table!("server_info", "server_revision"),
    table!("channels", "channels_revision"),
    table!("roles", "roles_revision"),
    table!("members", "members_revision"),
    table!("users", "users_revision"),
    table!("dms", "dms_revision"),
    table!("blocks", "blocks_revision"),
    Table {
        create: "CREATE TABLE member_roles (revision INTEGER NOT NULL, member_id BLOB NOT NULL CHECK(length(member_id)=16), role_id BLOB NOT NULL CHECK(length(role_id)=16), position INTEGER NOT NULL, PRIMARY KEY(revision,member_id,role_id))",
        insert: "INSERT INTO member_roles (revision,role_id,member_id,position) VALUES (?1,?2,?3,?4)",
        select: "SELECT role_id FROM member_roles WHERE revision=?1 AND position>=?2 ORDER BY position LIMIT 500",
        switch: "UPDATE cache_metadata SET member_roles_revision=?1 WHERE singleton=1",
        prune: "DELETE FROM member_roles WHERE rowid IN (SELECT rowid FROM member_roles WHERE revision < (SELECT member_roles_revision FROM cache_metadata) OR revision > (SELECT member_roles_revision FROM cache_metadata) LIMIT 500)",
    },
];

pub(super) struct Metadata {
    pub next_revision: i64,
    pub version: u64,
    pub revisions: [i64; TABLE_COUNT],
}

pub(super) fn initialize(
    connection: &mut Connection,
    account: AccountId,
) -> Result<(), WorkerError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match version {
        0 => create_version_one(connection, account)?,
        1 => {
            metadata_version(connection, account, 1)
                .map_err(|_| WorkerError::IntegrityCheckFailed)?;
        }
        2 => {
            metadata(connection, account).map_err(|_| WorkerError::IntegrityCheckFailed)?;
            return Ok(());
        }
        _ => return Err(WorkerError::IntegrityCheckFailed),
    }
    super::migration::version_two(connection)?;
    metadata(connection, account).map_err(|_| WorkerError::IntegrityCheckFailed)?;
    Ok(())
}

/// Frozen version-one creation path, also used by forward-migration tests.
pub(super) fn create_version_one(
    connection: &mut Connection,
    account: AccountId,
) -> Result<(), WorkerError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let tables: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    if version != 0 || tables != 0 {
        return Err(WorkerError::IntegrityCheckFailed);
    }
    let transaction = connection.transaction()?;
    transaction.execute_batch("CREATE TABLE cache_metadata (singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL, server_id BLOB NOT NULL CHECK(length(server_id)=16), user_id BLOB NOT NULL CHECK(length(user_id)=16), next_revision INTEGER NOT NULL, snapshot_version INTEGER NOT NULL, server_revision INTEGER NOT NULL DEFAULT 0, channels_revision INTEGER NOT NULL DEFAULT 0, roles_revision INTEGER NOT NULL DEFAULT 0, members_revision INTEGER NOT NULL DEFAULT 0, users_revision INTEGER NOT NULL DEFAULT 0, dms_revision INTEGER NOT NULL DEFAULT 0, blocks_revision INTEGER NOT NULL DEFAULT 0, member_roles_revision INTEGER NOT NULL DEFAULT 0)")?;
    for table in &TABLES {
        transaction.execute_batch(table.create)?;
    }
    transaction.execute("INSERT INTO cache_metadata (singleton,schema_version,server_id,user_id,next_revision,snapshot_version) VALUES (1,1,?1,?2,1,0)", params![account.server().as_bytes().as_slice(), account.user().as_bytes().as_slice()])?;
    transaction.pragma_update(None, "user_version", 1)?;
    transaction.commit()?;
    Ok(())
}

/// Reads only metadata until schema and created-for identity have matched.
pub(super) fn metadata(connection: &Connection, account: AccountId) -> rusqlite::Result<Metadata> {
    let metadata = metadata_version(connection, account, 2)?;
    let history_revision: i64 = connection.query_row(
        "SELECT history_revision FROM cache_metadata WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if history_revision < 0 || history_revision >= metadata.next_revision {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(metadata)
}

fn metadata_version(
    connection: &Connection,
    account: AccountId,
    expected_version: i64,
) -> rusqlite::Result<Metadata> {
    let identity: Option<(i64, Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT schema_version,server_id,user_id FROM cache_metadata WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if !identity.is_some_and(|(version, server, user)| {
        version == expected_version
            && server == account.server().as_bytes()
            && user == account.user().as_bytes()
    }) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    connection.query_row("SELECT next_revision,snapshot_version,server_revision,channels_revision,roles_revision,members_revision,users_revision,dms_revision,blocks_revision,member_roles_revision FROM cache_metadata WHERE singleton=1", [], |row| {
        let next_revision: i64 = row.get(0)?;
        let version: i64 = row.get(1)?;
        let revisions = [row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?];
        if next_revision < 1 || version < 0 || revisions.iter().any(|revision| *revision < 0 || *revision >= next_revision) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(Metadata { next_revision, version: version.cast_unsigned(), revisions })
    })
}

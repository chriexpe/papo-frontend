//! Forward-only durable schema; cache and media schemas belong to later capabilities.

use rusqlite::Connection;

use crate::storage_error::StorageError;

pub(crate) const VERSION: i64 = 2;

/// Refuses newer files before persistent pragmas, then migrates and recovers interrupted sends.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > VERSION {
        return Err(StorageError::NewerSchema { found: version });
    }
    if version < 0 {
        return Err(StorageError::InvalidState);
    }
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    if version == 0 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(include_str!("state_v1.sql"))?;
        transaction.pragma_update(None, "user_version", 1)?;
        transaction.commit()?;
    }
    if version < 2 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(include_str!("state_v2.sql"))?;
        transaction.pragma_update(None, "user_version", 2)?;
        transaction.commit()?;
    }
    // A committed Sending may have crossed the socket boundary, even before its first byte.
    connection.execute(
        "UPDATE operations SET state='Unknown' WHERE state='Sending'",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Migration contracts independent of registry behavior.

    use super::*;

    #[test]
    fn migration_is_repeatable_and_creates_only_session_four_tables() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        migrate(&mut connection).unwrap();
        let names: Vec<String> = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
            .query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(
            names,
            [
                "accounts",
                "drafts",
                "notification_ledger",
                "operations",
                "read_points",
                "removal_jobs",
                "servers",
                "settings"
            ]
        );
    }

    #[test]
    fn newer_schema_is_refused_without_changing_the_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("newer.sqlite");
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", VERSION + 1)
            .unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        let mut connection = Connection::open(&path).unwrap();
        assert!(matches!(
            migrate(&mut connection),
            Err(StorageError::NewerSchema { found: 3 })
        ));
        drop(connection);
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

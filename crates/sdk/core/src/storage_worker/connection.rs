//! SQLite connection policy and rebuildable-file recovery; callers supply all schemas.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use rusqlite::Connection;

use super::{WorkerError, scheduler::Specification};

pub(super) struct OpenConnection {
    pub connection: Connection,
    pub last_used: Instant,
}

pub(super) fn open(specification: &Specification) -> Result<OpenConnection, WorkerError> {
    match open_once(specification) {
        Ok(connection) => Ok(connection),
        Err(error) if !specification.durable && rebuildable(&error) => {
            remove_database(&specification.path)?;
            open_once(specification)
        }
        Err(error) => Err(error),
    }
}

fn open_once(specification: &Specification) -> Result<OpenConnection, WorkerError> {
    let existed = specification.path.exists();
    let mut connection = Connection::open(&specification.path)?;
    connection.busy_timeout(Duration::ZERO)?;
    if specification.durable {
        (specification.initializer)(&mut connection)?;
    }
    if !specification.durable && existed {
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(WorkerError::IntegrityCheckFailed);
        }
    }
    if !specification.durable {
        connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    }
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(
        None,
        "synchronous",
        if specification.durable {
            "FULL"
        } else {
            "NORMAL"
        },
    )?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    if !specification.durable {
        (specification.initializer)(&mut connection)?;
    }
    // Reassert policy after the caller-owned migration hook, before accepting writes.
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(
        None,
        "synchronous",
        if specification.durable {
            "FULL"
        } else {
            "NORMAL"
        },
    )?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.busy_timeout(Duration::ZERO)?;
    Ok(OpenConnection {
        connection,
        last_used: Instant::now(),
    })
}

fn rebuildable(error: &WorkerError) -> bool {
    matches!(
        error,
        WorkerError::IntegrityCheckFailed | WorkerError::NewerSchema { .. }
    ) || matches!(error, WorkerError::Sqlite { code: Some(code) } if code & 0xff == rusqlite::ffi::SQLITE_CORRUPT || code & 0xff == rusqlite::ffi::SQLITE_NOTADB)
}

fn remove_database(path: &Path) -> Result<(), WorkerError> {
    // Only registered, rebuildable SQLite files and their exact sidecars are removed.
    for suffix in ["-wal", "-shm", ""] {
        let mut target = path.as_os_str().to_os_string();
        target.push(suffix);
        match std::fs::remove_file(target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(WorkerError::System),
        }
    }
    Ok(())
}

pub(super) fn close(connection: OpenConnection) -> Result<(), WorkerError> {
    let (busy, _, _): (i64, i64, i64) =
        connection
            .connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
    if busy != 0 {
        return Err(WorkerError::Sqlite {
            code: Some(rusqlite::ffi::SQLITE_BUSY),
        });
    }
    connection
        .connection
        .close()
        .map_err(|(_, error)| WorkerError::from(error))
}

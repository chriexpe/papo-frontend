//! Watch snapshot publication and error-cloning helpers for saved reading positions.
//! Flush batching and revocation coordination live in `reading::positions`.

use std::sync::Arc;

use papo_api::Uuid;

use crate::{
    ClientError, Generation, HistoryError, ReadingError, catch_up::ReadingPosition,
    session::SessionInner,
};

pub(super) fn clone_error(error: &ReadingError) -> ReadingError {
    match error {
        ReadingError::InvalidPoint => ReadingError::InvalidPoint,
        ReadingError::InvalidReport => ReadingError::InvalidReport,
        ReadingError::History(err) => ReadingError::History(match err {
            HistoryError::Closed(r) => HistoryError::Closed(*r),
            HistoryError::Unverified => HistoryError::Unverified,
            HistoryError::InvalidPage => HistoryError::InvalidPage,
            HistoryError::StaleSnapshot => HistoryError::StaleSnapshot,
            HistoryError::Cache(c) => HistoryError::Cache(clone_cache_error(c)),
            HistoryError::Session(c) => HistoryError::Session(clone_client_error(c)),
        }),
        ReadingError::Storage(s) => ReadingError::Storage(clone_storage_error(s)),
        ReadingError::Session(c) => ReadingError::Session(clone_client_error(c)),
    }
}

fn clone_cache_error(c: &crate::CacheError) -> crate::CacheError {
    match c {
        crate::CacheError::Superseded => crate::CacheError::Superseded,
        crate::CacheError::InvalidData => crate::CacheError::InvalidData,
        crate::CacheError::TaskStopped => crate::CacheError::TaskStopped,
        crate::CacheError::Storage(w) => crate::CacheError::Storage(w.clone()),
        _ => crate::CacheError::Closed,
    }
}

fn clone_storage_error(s: &crate::StorageError) -> crate::StorageError {
    match s {
        crate::StorageError::NotFound => crate::StorageError::NotFound,
        crate::StorageError::InvalidOrder => crate::StorageError::InvalidOrder,
        crate::StorageError::AlreadyExists => crate::StorageError::AlreadyExists,
        crate::StorageError::ProfileInUse => crate::StorageError::ProfileInUse,
        crate::StorageError::RuntimeContext => crate::StorageError::RuntimeContext,
        crate::StorageError::NewerSchema { found } => {
            crate::StorageError::NewerSchema { found: *found }
        }
        crate::StorageError::Io(err) => {
            crate::StorageError::Io(std::io::Error::new(err.kind(), err.to_string()))
        }
        crate::StorageError::Sqlite(rusqlite::Error::SqliteFailure(ffi, msg)) => {
            crate::StorageError::Sqlite(rusqlite::Error::SqliteFailure(*ffi, msg.clone()))
        }
        crate::StorageError::Sqlite(err) => {
            crate::StorageError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some(err.to_string()),
            ))
        }
        crate::StorageError::Worker(w) => crate::StorageError::Worker(w.clone()),
        _ => crate::StorageError::InvalidState,
    }
}

fn clone_client_error(c: &ClientError) -> ClientError {
    match c {
        ClientError::ProfileInUse => ClientError::ProfileInUse,
        ClientError::RuntimeContext => ClientError::RuntimeContext,
        ClientError::Executor => ClientError::Executor,
        ClientError::Superseded => ClientError::Superseded,
        ClientError::NotFound => ClientError::NotFound,
        ClientError::Identity => ClientError::Identity,
        ClientError::ResourceData => ClientError::ResourceData,
        ClientError::ProfileLimit => ClientError::ProfileLimit,
        ClientError::ReconcileOverflow => ClientError::ReconcileOverflow,
        ClientError::Cache(cache) => ClientError::Cache(clone_cache_error(cache)),
        ClientError::InvalidAuth => ClientError::InvalidAuth,
        ClientError::SignedOut => ClientError::SignedOut,
        ClientError::Timeout => ClientError::Timeout,
        ClientError::Configuration => ClientError::Configuration,
        ClientError::Storage(storage) => ClientError::Storage(clone_storage_error(storage)),
        _ => ClientError::Closed,
    }
}

pub(crate) fn publish_positions(
    session: &SessionInner,
    generation: Generation,
    updates: Vec<(Uuid, Option<ReadingPosition>)>,
) {
    let _ = session.epochs.apply(generation, || {
        session.reading.position_image.send_if_modified(|current| {
            let mut next = (**current).clone();
            let mut changed = next.generation != generation;
            next.generation = generation;
            for (channel, position) in updates {
                let effective = if session
                    .reading
                    .positions
                    .revoked_reason(channel, generation)
                    .is_some()
                {
                    None
                } else {
                    position
                };
                let prev = next.channels.get(&channel).copied();
                if prev != effective {
                    changed = true;
                    match effective {
                        Some(pos) => {
                            next.channels.insert(channel, pos);
                        }
                        None => {
                            next.channels.remove(&channel);
                        }
                    }
                }
            }
            if !changed {
                return false;
            }
            let Some(version) = next.version.checked_add(1) else {
                return false;
            };
            next.version = version;
            *current = Arc::new(next);
            true
        });
    });
}

pub(crate) fn remove_from_image(session: &SessionInner, channels: &[Uuid], generation: Generation) {
    let updates: Vec<_> = channels.iter().map(|&id| (id, None)).collect();
    if !updates.is_empty() {
        publish_positions(session, generation, updates);
    }
}

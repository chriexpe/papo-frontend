//! Durable synchronous=FULL SQLite persistence for catch-up intervals and unified reading
//! hydration.
//! Saved reading position writes live in `catch_up_position_store`; revoked/removed channel
//! cleanup lives in `catch_up_cleanup`; coalescing and revocation coordination live in `reading`.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::Uuid;
use rusqlite::{Transaction, params};

use crate::{
    AccountId, ClientError, Generation, HistoryError, HistoryHandle, MessageKey, ReadingError,
    Registry, StorageError,
    catch_up::{CatchUpInterval, CatchUpState, ReadSpan, ReadingPosition},
    registry::{account_is_visible, visible_account},
    session::SessionInner,
};

#[cfg(test)]
#[path = "catch_up_store_tests.rs"]
mod tests;

pub(crate) struct CatchUpCommit {
    pub channel: Uuid,
    pub state: CatchUpState,
}

pub(crate) struct LoadedReadingMeta {
    pub catch_up: BTreeMap<Uuid, CatchUpState>,
    pub positions: BTreeMap<Uuid, ReadingPosition>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CatchUpWriteError {
    #[error(transparent)]
    Reading(#[from] ReadingError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Session(#[from] ClientError),
}

impl From<rusqlite::Error> for CatchUpWriteError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.into())
    }
}

impl From<crate::storage_worker::WorkerError> for CatchUpWriteError {
    fn from(error: crate::storage_worker::WorkerError) -> Self {
        Self::Storage(error.into())
    }
}

impl From<CatchUpWriteError> for ReadingError {
    fn from(error: CatchUpWriteError) -> Self {
        match error {
            CatchUpWriteError::Reading(error) => error,
            CatchUpWriteError::Storage(error) => Self::Storage(error),
            CatchUpWriteError::History(error) => Self::History(error),
            CatchUpWriteError::Session(error) => Self::Session(error),
        }
    }
}

impl From<CatchUpWriteError> for ClientError {
    fn from(error: CatchUpWriteError) -> Self {
        match error {
            CatchUpWriteError::Session(error) => error,
            CatchUpWriteError::History(error) => error.into(),
            CatchUpWriteError::Storage(error) => error.into(),
            CatchUpWriteError::Reading(_) => Self::ResourceData,
        }
    }
}

fn decode_point(bytes: Option<Vec<u8>>) -> Result<Option<MessageKey>, StorageError> {
    bytes
        .map(|raw| serde_json::from_slice(&raw).map_err(|_| StorageError::InvalidState))
        .transpose()
}

fn encode_point(point: Option<&MessageKey>) -> Result<Option<Vec<u8>>, StorageError> {
    point
        .map(|value| serde_json::to_vec(value).map_err(|_| StorageError::InvalidState))
        .transpose()
}

fn decode_spans(bytes: Option<Vec<u8>>) -> Result<Vec<ReadSpan>, StorageError> {
    let Some(raw) = bytes else {
        return Ok(Vec::new());
    };
    let spans: Vec<ReadSpan> =
        serde_json::from_slice(&raw).map_err(|_| StorageError::InvalidState)?;
    if spans.iter().any(|span| span.start > span.end)
        || spans.windows(2).any(|pair| pair[0].end >= pair[1].start)
    {
        return Err(StorageError::InvalidState);
    }
    Ok(spans)
}

fn encode_spans(spans: &[ReadSpan]) -> Result<Option<Vec<u8>>, StorageError> {
    if spans.is_empty() {
        Ok(None)
    } else {
        serde_json::to_vec(spans)
            .map(Some)
            .map_err(|_| StorageError::InvalidState)
    }
}

/// Caller must validate `visible_account(tx, account)?` once before calling inside a writer transaction.
pub(crate) fn upsert_catch_up_tx(
    tx: &Transaction<'_>,
    account: AccountId,
    channel: Uuid,
    state: &CatchUpState,
) -> Result<(), StorageError> {
    if state.seen_to.is_none() && state.interval.is_none() {
        tx.execute(
            "DELETE FROM catch_up_intervals WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",
            params![
                account.server().as_bytes(),
                account.user().as_bytes(),
                channel.as_bytes(),
            ],
        )?;
        return Ok(());
    }
    let seen_to = encode_point(state.seen_to.as_ref())?;
    let (start_after, end_before, covered_spans) = match &state.interval {
        Some(interval) => (
            encode_point(interval.start_after.as_ref())?,
            encode_point(Some(&interval.end_before))?,
            encode_spans(&interval.covered_spans)?,
        ),
        None => (None, None, None),
    };
    tx.execute(
        "INSERT INTO catch_up_intervals(server_id,user_id,channel_id,seen_to,start_after,end_before,covered_spans) \
         VALUES(?1,?2,?3,?4,?5,?6,?7) \
         ON CONFLICT(server_id,user_id,channel_id) DO UPDATE SET \
         seen_to=excluded.seen_to, start_after=excluded.start_after, end_before=excluded.end_before, covered_spans=excluded.covered_spans",
        params![
            account.server().as_bytes(),
            account.user().as_bytes(),
            channel.as_bytes(),
            seen_to,
            start_after,
            end_before,
            covered_spans,
        ],
    )?;
    Ok(())
}

impl Registry {
    pub(crate) async fn load_catch_up_and_positions(
        &self,
        account: AccountId,
    ) -> Result<LoadedReadingMeta, StorageError> {
        let lease = Arc::clone(&self.lease);
        let path = self.path.clone();
        let (catch_rows, pos_rows) = self
            .readers
            .query(path, move |tx| {
                let _lease = lease;
                if !account_is_visible(tx, account)? {
                    return Ok((Vec::new(), Vec::new()));
                }
                let catch_rows = tx
                    .prepare(
                        "SELECT channel_id,seen_to,start_after,end_before,covered_spans FROM catch_up_intervals WHERE server_id=?1 AND user_id=?2",
                    )?
                    .query_map(
                        params![account.server().as_bytes(), account.user().as_bytes()],
                        |row| {
                            Ok((
                                row.get::<_, Vec<u8>>(0)?,
                                row.get::<_, Option<Vec<u8>>>(1)?,
                                row.get::<_, Option<Vec<u8>>>(2)?,
                                row.get::<_, Option<Vec<u8>>>(3)?,
                                row.get::<_, Option<Vec<u8>>>(4)?,
                            ))
                        },
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                let pos_rows = tx
                    .prepare(
                        "SELECT channel_id,message_id,offset_units FROM saved_reading_positions WHERE server_id=?1 AND user_id=?2",
                    )?
                    .query_map(
                        params![account.server().as_bytes(), account.user().as_bytes()],
                        |row| {
                            Ok((
                                row.get::<_, Vec<u8>>(0)?,
                                row.get::<_, Vec<u8>>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((catch_rows, pos_rows))
            })
            .await?;

        let mut catch_up = BTreeMap::new();
        for (ch_bytes, seen_bytes, start_bytes, end_bytes, spans_bytes) in catch_rows {
            let channel = Uuid::from_slice(&ch_bytes).map_err(|_| StorageError::InvalidState)?;
            let seen_to = decode_point(seen_bytes)?;
            let start_after = decode_point(start_bytes)?;
            let end_before = decode_point(end_bytes)?;
            let covered_spans = decode_spans(spans_bytes)?;
            let interval = match (start_after, end_before, covered_spans) {
                (None, None, spans) if spans.is_empty() => None,
                (start_after, Some(end_before), covered_spans) => Some(CatchUpInterval {
                    start_after,
                    end_before,
                    covered_spans,
                }),
                _ => return Err(StorageError::InvalidState),
            };
            catch_up.insert(channel, CatchUpState { seen_to, interval });
        }

        let mut positions = BTreeMap::new();
        for (ch_bytes, msg_bytes, offset_units) in pos_rows {
            let channel = Uuid::from_slice(&ch_bytes).map_err(|_| StorageError::InvalidState)?;
            let message_id =
                Uuid::from_slice(&msg_bytes).map_err(|_| StorageError::InvalidState)?;
            positions.insert(
                channel,
                ReadingPosition {
                    message_id,
                    offset_units,
                },
            );
        }
        Ok(LoadedReadingMeta {
            catch_up,
            positions,
        })
    }

    pub(crate) async fn commit_visible_read_and_catch_up(
        &self,
        history: HistoryHandle,
        generation: Generation,
        version: u64,
        local_point: Option<MessageKey>,
        catch_up: Option<CatchUpState>,
    ) -> Result<(), ReadingError> {
        let account = history.account();
        let channel = history.channel();
        self.writer
            .write(move |tx| -> Result<(), CatchUpWriteError> {
                history.with_current_snapshot(generation, version, || ())?;
                history.ensure_reading_writable(generation)?;
                visible_account(tx, account)?;
                if let Some(point) = local_point {
                    crate::read_points::advance_local_in_tx(tx, account, channel, point)?;
                }
                if let Some(state) = catch_up {
                    upsert_catch_up_tx(tx, account, channel, &state)?;
                }
                Ok(())
            })
            .await
            .map_err(ReadingError::from)
    }

    pub(crate) async fn commit_clear_catch_up(
        &self,
        session: Arc<SessionInner>,
        channel: Uuid,
        generation: Generation,
        next_state: CatchUpState,
    ) -> Result<(), ReadingError> {
        let account = session.account;
        self.writer
            .write(move |tx| -> Result<(), CatchUpWriteError> {
                session.epochs.validate(generation)?;
                if let Some(reason) = session
                    .reading
                    .positions
                    .check_current_channel(channel, generation)
                {
                    return Err(HistoryError::Closed(reason).into());
                }
                visible_account(tx, account)?;
                upsert_catch_up_tx(tx, account, channel, &next_state)?;
                Ok(())
            })
            .await
            .map_err(ReadingError::from)
    }

    pub(crate) async fn commit_catch_up_batch(
        &self,
        session: Arc<SessionInner>,
        generation: Generation,
        updates: Vec<CatchUpCommit>,
    ) -> Result<(), ClientError> {
        if updates.is_empty() {
            return Ok(());
        }
        let account = session.account;
        self.writer
            .write(move |tx| -> Result<(), CatchUpWriteError> {
                session.epochs.validate(generation)?;
                visible_account(tx, account)?;
                for commit in updates {
                    if let Some(reason) = session
                        .reading
                        .positions
                        .check_current_channel(commit.channel, generation)
                    {
                        return Err(HistoryError::Closed(reason).into());
                    }
                    upsert_catch_up_tx(tx, account, commit.channel, &commit.state)?;
                }
                Ok(())
            })
            .await
            .map_err(ClientError::from)
    }
}

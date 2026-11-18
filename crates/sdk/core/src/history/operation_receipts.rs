//! Generation-bound post-ack pin refresh; ordinary history handles and window ownership stay elsewhere.

use super::{HistoryError, HistoryHandle, channel::CacheConnection};
use crate::{Generation, Session, Uuid};
use std::sync::atomic::Ordering;

pub(crate) async fn refresh_pins(
    session: &Session,
    generation: Generation,
    channel: Uuid,
) -> Result<(), HistoryError> {
    session.inner.epochs.validate(generation)?;
    let store = &session.inner.history;
    if let Some(refusal) = store.admission.read().await.refusal(channel) {
        return Err(refusal);
    }
    let connection = store
        .connection
        .read()
        .await
        .clone()
        .ok_or(HistoryError::Unverified)?;
    if connection.generation != generation {
        return Err(crate::ClientError::Superseded.into());
    }
    let channel = store.channel(channel).await;
    session.inner.epochs.validate(generation)?;
    channel.opens.fetch_add(1, Ordering::Relaxed);
    let handle = HistoryHandle {
        session: session.clone(),
        channel,
    };
    let connection = CacheConnection {
        cache: connection.cache,
        rest: connection.rest,
        generation,
    };
    super::fetch::pins(&handle, &connection, generation).await
}

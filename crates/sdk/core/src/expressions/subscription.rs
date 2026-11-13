//! Coalesced durable expression observations; clients own picker layout and encoded image consumption.

use super::{ExpressionError, ExpressionService, ExpressionSnapshot, persistence};
use std::sync::Arc;
use tokio::sync::watch;

/// Sanitized library observation failure, independent of optional catalog availability.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ExpressionFault {
    /// Durable metadata could not be loaded.
    Storage,
    /// Stored metadata was invalid.
    InvalidData,
    /// Account generation changed during the read; the next connection update refreshes it.
    Superseded,
}
/// Atomic metadata-only library image or an observable lifecycle/fault state.
#[derive(Clone)]
#[non_exhaustive]
pub enum ExpressionUpdate {
    /// Current account's durable library; no image bytes or provider key are retained.
    Ready(Arc<ExpressionSnapshot>),
    /// Durable observation failed, without hiding the failure behind an empty library.
    Fault(ExpressionFault),
    /// Account was removed/replaced or client shut down.
    Closed,
}
impl ExpressionService {
    /// Subscribes to FULL-committed favorite/confirmed-recent changes, coalescing intermediate images.
    /// # Errors
    /// Returns account revocation, initial durable read failure or executor closure.
    pub async fn subscribe(&self) -> Result<watch::Receiver<ExpressionUpdate>, ExpressionError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move {
                let mut changes = session.inner.expression_changes.subscribe();
                let mut connection = session.subscribe_connection();
                let snapshot = match persistence::snapshot(session.clone()).await {
                    Ok(value) => value,
                    Err(error) => return Ok(Err(error)),
                };
                if session.is_closed() {
                    return Err(crate::ClientError::Closed);
                }
                let (sender, receiver) =
                    watch::channel(ExpressionUpdate::Ready(Arc::new(snapshot)));
                let tasks = session.inner.tasks.clone();
                tasks.spawn(async move {
                    loop {
                        tokio::select! {
                            biased;
                            ()=session.inner.root.cancelled()=>break,
                            ()=sender.closed()=>return,
                            result=changes.changed()=>if result.is_err() {break;},
                            result=connection.changed()=>if result.is_err() {break;},
                        }
                        let result = persistence::snapshot(session.clone()).await;
                        if session.is_closed() {
                            break;
                        }
                        let update = match result {
                            Ok(snapshot) => ExpressionUpdate::Ready(Arc::new(snapshot)),
                            Err(ExpressionError::InvalidData) => {
                                ExpressionUpdate::Fault(ExpressionFault::InvalidData)
                            }
                            Err(ExpressionError::Session(_)) => {
                                ExpressionUpdate::Fault(ExpressionFault::Superseded)
                            }
                            Err(_) => ExpressionUpdate::Fault(ExpressionFault::Storage),
                        };
                        sender.send_replace(update);
                    }
                    sender.send_replace(ExpressionUpdate::Closed);
                });
                Ok(Ok(receiver))
            })
            .await?
    }
}

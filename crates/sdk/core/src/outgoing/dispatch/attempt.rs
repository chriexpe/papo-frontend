//! One claimed attempt on the wire: the request, its timeouts and a sender's cancel. What the
//! attempt's outcome means for the queue is `dispatch`'s.

use super::super::{
    Arc, Operation, UploadProgress, Uuid, persistence::Outcome, progress::ProgressReporter,
    stream::OpenedAttachment,
};
use crate::Session;
use papo_api::RestClient;
use tokio_util::sync::CancellationToken;

/// How long the answer may take once every attachment byte is handed to the transport: covers
/// draining what socket buffers still hold on a slow uplink.
const UPLOAD_ANSWER_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);

/// How one claimed attempt ended.
#[expect(
    clippy::large_enum_variant,
    reason = "one short-lived value per attempt, moved once into classify"
)]
pub(super) enum Attempt {
    /// The request finished: the server's answer, or a transport failure.
    Answered(Result<Outcome, papo_api::Error>),
    /// The sender cancelled. `body_complete` is whether every attachment byte had been consumed;
    /// only before that is it certain the server never saw the whole message.
    Cancelled { body_complete: bool },
}

/// Clears the queue's in-flight handle when the attempt ends, however it ends.
struct InFlight<'a>(&'a super::super::AccountQueue);

impl<'a> InFlight<'a> {
    fn register(
        queue: &'a super::super::AccountQueue,
        operation: Uuid,
        token: CancellationToken,
    ) -> Self {
        *queue
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((operation, token));
        Self(queue)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        *self
            .0
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

pub(super) async fn execute_send(
    session: &Session,
    epoch: &crate::generation::Epoch,
    rest: &RestClient,
    operation: &Operation,
    opened: Vec<OpenedAttachment>,
    initial_progress: Option<(u32, UploadProgress)>,
    target: Option<Uuid>,
) -> Attempt {
    let timeout = session.client.inner.config.request_timeout;
    let Some((attempt, initial)) = initial_progress else {
        let request = super::super::request::send(
            session,
            &epoch.cancel,
            rest,
            operation,
            opened,
            None,
            target,
        );
        return Attempt::Answered(tokio::select! {
            biased;
            () = session.inner.root.cancelled() => Err(papo_api::Error::Transport),
            () = epoch.cancel.cancelled() => Err(papo_api::Error::Transport),
            result = tokio::time::timeout(timeout, request) => {
                result.unwrap_or(Err(papo_api::Error::Transport))
            }
        });
    };
    let (reporter, _guard) = ProgressReporter::start(
        initial.account,
        initial.operation_id,
        attempt,
        initial.total_bytes,
        session.inner.root.clone(),
        Arc::clone(&session.inner.outgoing.image),
    );
    let mut consumed = reporter.consumed();
    let cancel = CancellationToken::new();
    let _in_flight = InFlight::register(&session.inner.outgoing, operation.id, cancel.clone());
    if session.inner.outgoing.take_cancel_request(operation.id) {
        cancel.cancel();
    }
    let request = super::super::request::send(
        session,
        &epoch.cancel,
        rest,
        operation,
        opened,
        Some(reporter),
        target,
    );
    tokio::pin!(request);
    // An upload may take far longer than one request timeout: it times out only after a whole
    // timeout with no byte consumed. Once the last byte is handed over, socket buffers may still
    // hold megabytes of it on a slow link, so the answer gets a longer, fixed window.
    let idle = tokio::time::sleep(timeout);
    tokio::pin!(idle);
    let mut watching = true;
    loop {
        tokio::select! {
            biased;
            () = session.inner.root.cancelled() => return Attempt::Answered(Err(papo_api::Error::Transport)),
            () = epoch.cancel.cancelled() => return Attempt::Answered(Err(papo_api::Error::Transport)),
            // An answer already in wins over a cancel arriving with it: it may be a receipt.
            result = &mut request => return Attempt::Answered(result),
            () = cancel.cancelled() => {
                let body_complete = *consumed.borrow() >= initial.total_bytes;
                return Attempt::Cancelled { body_complete };
            }
            () = &mut idle => return Attempt::Answered(Err(papo_api::Error::Transport)),
            changed = consumed.changed(), if watching => {
                if changed.is_err() {
                    watching = false;
                    continue;
                }
                let handed_over = *consumed.borrow() >= initial.total_bytes;
                let window = if handed_over { timeout.max(UPLOAD_ANSWER_WINDOW) } else { timeout };
                idle.as_mut().reset(tokio::time::Instant::now() + window);
                watching = !handed_over;
            }
        }
    }
}

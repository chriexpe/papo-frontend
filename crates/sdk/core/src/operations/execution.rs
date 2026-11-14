//! Runtime admission and mutation outcomes; cache refresh and domain validation live separately.

use super::{OperationError, OperationReceipt, Operations, Refresh, RefreshFault, RefreshStatus};
use crate::{ClientError, Generated, generation::Epoch};
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

impl Operations {
    pub(super) async fn read<T, F, Fut>(&self, operation: F) -> Result<Generated<T>, OperationError>
    where
        T: Send + 'static,
        F: FnOnce(papo_api::RestClient) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, papo_api::Error>> + Send,
    {
        let session = self.session.clone();
        let epoch = session.inner.epochs.current();
        if self
            .expected_generation
            .is_some_and(|expected| expected != epoch.generation)
        {
            return Err(ClientError::Superseded.into());
        }
        let token = session.inner.tasks.token();
        Ok(self
            .session
            .client
            .dispatch(async move {
                let _token = token;
                session.inner.epochs.validate(epoch.generation)?;
                let work = async {
                    let rest = crate::scoped_request::authenticated(&session, &epoch).await?;
                    let value = session.client.inner.request(operation(rest)).await?;
                    session.inner.epochs.validate(epoch.generation)?;
                    Ok(Generated {
                        generation: epoch.generation,
                        value,
                    })
                };
                tokio::select! {
                    biased;
                    () = session.inner.root.cancelled() => Err(ClientError::Closed),
                    () = epoch.cancel.cancelled() => Err(ClientError::Superseded),
                    result = work => result,
                }
            })
            .await?)
    }

    pub(super) async fn mutate<T, F, Fut>(
        &self,
        operation: F,
        refresh: Refresh,
    ) -> Result<OperationReceipt<T>, OperationError>
    where
        T: Send + 'static,
        F: FnOnce(papo_api::RestClient) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, papo_api::Error>> + Send,
    {
        let session = self.session.clone();
        let epoch = session.inner.epochs.current();
        if self
            .expected_generation
            .is_some_and(|expected| expected != epoch.generation)
        {
            return Err(ClientError::Superseded.into());
        }
        let token = session.inner.tasks.token();
        let dispatched = Arc::new(AtomicBool::new(false));
        let admitted = dispatched.clone();
        let acknowledgment = Arc::new(Mutex::new(None));
        let accepted = acknowledgment.clone();
        let result = self
            .session
            .client
            .dispatch(async move {
                let _token = token;
                let admission = async {
                    let serial = session.inner.operations_serial.lock().await;
                    session.inner.epochs.validate(epoch.generation)?;
                    let rest = crate::scoped_request::authenticated(&session, &epoch).await?;
                    Ok::<_, ClientError>((serial, rest))
                };
                let (_serial, rest) = tokio::select! {
                    biased;
                    () = session.inner.root.cancelled() => return Err(ClientError::Closed),
                    () = epoch.cancel.cancelled() => return Err(ClientError::Superseded),
                    result = admission => result?,
                };
                session.inner.epochs.validate(epoch.generation)?;
                admitted.store(true, Ordering::Release);
                let response = tokio::select! {
                    biased;
                    () = session.inner.root.cancelled() => Err(ClientError::Closed),
                    () = epoch.cancel.cancelled() => Err(ClientError::Superseded),
                    result = session.client.inner.request(operation(rest.clone())) => result,
                };
                let value = match response {
                    Ok(value) => value,
                    Err(error) => return Ok(Err(classify(error))),
                };
                // Keep known success outside the outer dispatch cancellation boundary.
                // No mutex guard is held across an await.
                *accepted
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(OperationReceipt {
                    account: session.account(),
                    generation: epoch.generation,
                    value,
                    refresh: RefreshStatus::Warning(RefreshFault::Closed),
                });
                let refreshed = refresh_after_ack(&session, &epoch, &rest, refresh).await;
                if let Some(receipt) = accepted
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_mut()
                {
                    receipt.refresh = refreshed;
                }
                Ok(Ok(()))
            })
            .await;
        if let Some(receipt) = acknowledgment
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return Ok(receipt);
        }
        match result {
            Ok(Err(error)) => Err(error),
            Ok(Ok(())) => Err(OperationError::InvalidData),
            Err(error) if dispatched.load(Ordering::Acquire) => Err(OperationError::Unknown(error)),
            Err(error) => Err(error.into()),
        }
    }
}

async fn refresh_after_ack(
    session: &crate::Session,
    epoch: &Epoch,
    rest: &papo_api::RestClient,
    refresh: Refresh,
) -> RefreshStatus {
    if matches!(refresh, Refresh::None) {
        return RefreshStatus::NotRequired;
    }
    tokio::select! {
        biased;
        () = session.inner.root.cancelled() => RefreshStatus::Warning(RefreshFault::Closed),
        () = epoch.cancel.cancelled() => RefreshStatus::Warning(RefreshFault::Superseded),
        result = super::refresh::apply(session, epoch, rest, refresh) => match result {
            Ok(()) => RefreshStatus::Updated,
            Err(fault) => RefreshStatus::Warning(fault),
        },
    }
}

fn classify(error: ClientError) -> OperationError {
    let uncertain = match &error {
        ClientError::Api(papo_api::Error::NotDispatched) => false,
        ClientError::Api(
            papo_api::Error::Problem { status, .. } | papo_api::Error::Http { status },
        ) => *status >= 500 || *status == 408,
        _ => true,
    };
    if uncertain {
        OperationError::Unknown(error)
    } else {
        OperationError::Session(error)
    }
}

#[cfg(test)]
mod tests {
    //! Dispatch failures are classified without user-controlled diagnostics or retries.
    use super::*;
    #[test]
    fn rejection_is_distinct_from_ambiguous_dispatch() {
        for status in [400, 401, 403, 404, 409, 422, 429] {
            assert!(matches!(
                classify(papo_api::Error::Http { status }.into()),
                OperationError::Session(_)
            ));
        }
        for error in [
            ClientError::Timeout,
            ClientError::Superseded,
            papo_api::Error::Transport.into(),
            papo_api::Error::Decode.into(),
            papo_api::Error::Http { status: 503 }.into(),
        ] {
            assert!(matches!(classify(error), OperationError::Unknown(_)));
        }
        assert!(matches!(
            classify(papo_api::Error::NotDispatched.into()),
            OperationError::Session(_)
        ));
    }
}

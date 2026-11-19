//! Per-account quiet baseline and exact inbox catch-up; scheduling/preemption live in the parent.
use super::BackgroundOutcome;
use crate::{ClientError, Session};
pub(super) async fn catch_up(
    session: &Session,
    deliveries: &mut Vec<crate::NotificationDelivery>,
) -> Result<BackgroundOutcome, crate::NotificationError> {
    use crate::notifications::{deliver, inbox, ledger};
    let epoch = session.inner.epochs.current();
    let work = async {
        let _poll = session.inner.notifications.poll_serial.lock().await;
        let rest = crate::scoped_request::authenticated(session, &epoch).await?;
        crate::scoped_request::refresh_policy(session, &epoch, &rest).await?;
        let resources = policy_resources(session, &rest, epoch.generation).await?;
        let mut points = session.inner.notifications.canonical_points.lock().await;
        let mut cursor = {
            let _serial = session.inner.notifications.serial.lock().await;
            ledger::load(session).await?.1
        };
        let Some(mut boundary) = cursor.take() else {
            establish_baseline(session, &rest, epoch.generation).await?;
            return Ok(BackgroundOutcome::Complete);
        };
        for _ in 0..4 {
            let cursor = (!boundary.created_at.is_empty()).then_some(&boundary);
            let page = inbox::page(
                session,
                &rest,
                epoch.generation,
                cursor,
                crate::HistoryOrder::Asc,
            )
            .await?;
            for record in page.records {
                let channel = crate::notifications::id(record.record.channel_id.as_deref())?;
                let mut candidate = deliver::record_candidate(&record.record)?;
                let point = match points
                    .resolve(session, &rest, epoch.generation, channel, candidate.message)
                    .await
                {
                    Ok(point) => point,
                    Err(crate::NotificationError::Session(ClientError::Api(
                        papo_api::Error::Http {
                            status: 403 | 404 | 410,
                        }
                        | papo_api::Error::Problem {
                            status: 403 | 404 | 410,
                            ..
                        },
                    ))) => crate::notifications::context::Resolution::Absent,
                    Err(error) => return Err(error),
                };
                match point {
                    crate::notifications::context::Resolution::Pending => {
                        // Never acknowledge an inbox row whose canonical identity is still unresolved.
                        return Ok(BackgroundOutcome::More);
                    }
                    crate::notifications::context::Resolution::Found(point) => {
                        candidate.message_point = Some(point);
                        candidate.observed_at_us = point.created_at_us;
                        match deliver::deliver_with_resources(
                            session,
                            epoch.generation,
                            candidate,
                            &resources,
                        )
                        .await?
                        {
                            deliver::DeliveryOutcome::Deferred => {
                                // Never claim or advance past a candidate whose read evidence is not ready.
                                return Ok(BackgroundOutcome::More);
                            }
                            deliver::DeliveryOutcome::Handled(Some(delivery)) => {
                                deliveries.push(delivery);
                            }
                            deliver::DeliveryOutcome::Handled(None) => {}
                        }
                    }
                    crate::notifications::context::Resolution::Absent => {}
                }
                boundary = inbox::InboxCursor::from_record(&record.record)?;
                let _serial = session.inner.notifications.serial.lock().await;
                ledger::save_cursor(session, epoch.generation, boundary.clone()).await?;
            }
            if page.next.is_none() {
                return Ok(BackgroundOutcome::Complete);
            }
        }
        Ok(BackgroundOutcome::More)
    };
    tokio::select! {
        biased;
        () = epoch.cancel.cancelled() => Err(ClientError::Superseded.into()),
        result = work => result,
    }
}

async fn establish_baseline(
    session: &Session,
    rest: &papo_api::RestClient,
    generation: crate::Generation,
) -> Result<(), crate::NotificationError> {
    use crate::notifications::{inbox, ledger};
    let page = inbox::page(session, rest, generation, None, crate::HistoryOrder::Desc).await?;
    let newest = page
        .records
        .first()
        .map(|record| inbox::InboxCursor::from_record(&record.record))
        .transpose()?
        .unwrap_or(inbox::InboxCursor {
            created_at: String::new(),
            id: String::new(),
        });
    let _serial = session.inner.notifications.serial.lock().await;
    // First observation establishes a quiet baseline, including already-read history.
    if ledger::load(session).await?.1.is_none() {
        ledger::save_cursor(session, generation, newest).await?;
    }
    Ok(())
}

async fn policy_resources(
    session: &Session,
    rest: &papo_api::RestClient,
    generation: crate::Generation,
) -> Result<crate::notifications::policy::PolicyResources, crate::NotificationError> {
    let admission = crate::resource_admission::Admission::new(&session.inner);
    // Fetch only shared channel/DM metadata through wire resource admission; never fabricate a
    // complete `ResourceSnapshot` when server, roles, members and blocks were not loaded.
    let channels = admission
        .channels(session.client.inner.request(rest.channels()).await?)
        .map_err(ClientError::from)?;
    let dms = admission
        .dms(
            session
                .client
                .inner
                .request(rest.direct_conversations())
                .await?,
        )
        .map_err(ClientError::from)?;
    session.inner.epochs.validate(generation)?;
    Ok(crate::notifications::policy::PolicyResources::channels_and_dms(generation, channels, dms))
}

//! Per-account quiet baseline and exact inbox catch-up; scheduling/preemption live in the parent.
use super::BackgroundOutcome;
use crate::{ClientError, Session};
pub(super) async fn catch_up(
    session: &Session,
    deliveries: &mut Vec<crate::NotificationDelivery>,
) -> Result<BackgroundOutcome, crate::NotificationError> {
    use crate::notifications::{inbox, ledger, live};
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
                let mut candidate = live::record_candidate(&record.record)?;
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
                if matches!(point, crate::notifications::context::Resolution::Pending) {
                    // Never acknowledge an inbox row whose canonical identity is still unresolved.
                    return Ok(BackgroundOutcome::More);
                }
                if let Some(point) = point.point() {
                    candidate.message_point = Some(point);
                    candidate.observed_at_us = point.created_at_us;
                    if let Some(delivery) = live::deliver_with_resources(
                        session,
                        epoch.generation,
                        candidate,
                        &resources,
                    )
                    .await?
                    {
                        deliveries.push(delivery);
                    }
                } else {
                    session
                        .inner
                        .notifications
                        .status
                        .send_replace(crate::NotificationStatus::Fault);
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
) -> Result<crate::ResourceSnapshot, crate::NotificationError> {
    // Fetch only shared metadata, never a roster/history or native UI projection.
    let channels = session
        .client
        .inner
        .request(rest.channels())
        .await?
        .channels
        .unwrap_or_default();
    let dms = session
        .client
        .inner
        .request(rest.direct_conversations())
        .await?
        .dms;
    let mut resources = (**session.inner.resources.borrow()).clone();
    resources.generation = generation;
    resources.channels = channels.into();
    resources.dms = dms.into();
    Ok(resources)
}

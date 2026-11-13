//! Bounded live candidate ingestion; historical cache/reconnect pages never enter this producer.

use super::{
    NotificationError, NotificationStatus, inbox, ledger,
    policy::{self, Candidate},
};
use crate::{Generated, Session};
use papo_api::gateway::{Event, KnownEvent};
use std::collections::VecDeque;

pub(crate) fn submit(
    session: &crate::session::SessionInner,
    generation: crate::Generation,
    event: &Event,
) {
    if !matches!(
        event,
        Event::Known(KnownEvent::Message(_) | KnownEvent::NewNotification(_))
    ) {
        return;
    }
    let sender = session
        .notifications
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if sender.as_ref().is_some_and(|sender| {
        sender
            .try_send(Generated {
                generation,
                value: event.clone(),
            })
            .is_err()
    }) {
        session
            .notifications
            .status
            .send_replace(NotificationStatus::Fault);
    }
}
pub(crate) fn start(session: Session) {
    let mut sender = session
        .inner
        .notifications
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if sender.is_some() {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    *sender = Some(tx);
    drop(sender);
    let token = session.inner.tasks.token();
    let runtime = session.client.inner.runtime.clone();
    runtime.spawn(async move {
        let _token = token;
        let mut recent = VecDeque::<(crate::Generation, Candidate)>::new();
        loop {
            let event = tokio::select! {
                biased;
                () = session.inner.root.cancelled() => return,
                event = rx.recv() => match event {
                    Some(event) => event,
                    None => return,
                },
            };
            let epoch = session.inner.epochs.current();
            if event.generation != epoch.generation {
                continue;
            }
            recent.retain(|(generation, _)| *generation == epoch.generation);
            let work = process(&session, event, &mut recent);
            let result = tokio::select! {
                biased;
                () = epoch.cancel.cancelled() => continue,
                result = work => result,
            };
            if result.is_err() {
                session
                    .inner
                    .notifications
                    .status
                    .send_replace(NotificationStatus::Fault);
            }
        }
    });
}
async fn process(
    session: &Session,
    event: Generated<Event>,
    recent: &mut VecDeque<(crate::Generation, Candidate)>,
) -> Result<(), NotificationError> {
    let candidate = match event.value {
        Event::Known(KnownEvent::Message(message)) => {
            let candidate = Candidate {
                channel: super::id(Some(&message.channel_id))?,
                message: super::id(Some(&message.id))?,
                author: message.author_id.parse().ok(),
                triggered: policy::mentions(&message.content, session.account().user()),
                content: message.content,
                read: false,
                observed_at_us: parse_time(&message.created_at)?,
                message_point: Some(crate::ReadPoint {
                    created_at_us: parse_time(&message.created_at)?,
                    message_id: super::id(Some(&message.id))?,
                }),
            };
            if recent.len() == 1024 {
                recent.pop_front();
            }
            recent.push_back((event.generation, candidate.clone()));
            candidate
        }
        Event::Known(KnownEvent::NewNotification(notification)) => {
            let message = super::id(Some(&notification.message_id))?;
            let recent_candidate = recent
                .iter()
                .find(|(_, candidate)| candidate.message == message);
            if let Some((_, candidate)) =
                recent_candidate.filter(|(_, candidate)| candidate.triggered)
            {
                candidate.clone()
            } else {
                let candidate = authoritative_trigger(session, event.generation, message).await?;
                let Some(mut candidate) = candidate else {
                    return Ok(());
                };
                if let Some((_, recent)) = recent_candidate {
                    candidate.message_point = recent.message_point;
                }
                if candidate.message_point.is_none() {
                    let epoch = session.inner.epochs.current();
                    let rest = crate::scoped_request::authenticated(session, &epoch).await?;
                    candidate.message_point = super::context::CanonicalPoints::default()
                        .resolve(
                            session,
                            &rest,
                            event.generation,
                            candidate.channel,
                            candidate.message,
                        )
                        .await?
                        .point();
                }
                let Some(point) = candidate.message_point else {
                    return Err(NotificationError::InvalidData);
                };
                candidate.observed_at_us = point.created_at_us;
                candidate
            }
        }
        _ => return Ok(()),
    };
    if let Some(delivery) = deliver(session, event.generation, candidate).await? {
        handoff(session, delivery)?;
    }
    Ok(())
}
pub(crate) fn record_candidate(
    record: &papo_api::wire::Notification,
) -> Result<Candidate, NotificationError> {
    Ok(Candidate {
        channel: super::id(record.channel_id.as_deref())?,
        message: super::id(record.message_id.as_deref())?,
        author: record.author_id.as_deref().and_then(|id| id.parse().ok()),
        content: record.message_content.clone().unwrap_or_default(),
        triggered: true,
        read: record.read != Some(false),
        observed_at_us: parse_time(
            record
                .created_at
                .as_deref()
                .ok_or(NotificationError::InvalidData)?,
        )?,
        message_point: None,
    })
}
pub(crate) async fn deliver(
    session: &Session,
    generation: crate::Generation,
    candidate: Candidate,
) -> Result<Option<super::NotificationDelivery>, NotificationError> {
    let resources = std::sync::Arc::clone(&session.inner.resources.borrow());
    deliver_with_resources(session, generation, candidate, &resources).await
}

pub(crate) async fn deliver_with_resources(
    session: &Session,
    generation: crate::Generation,
    candidate: Candidate,
    resources: &crate::ResourceSnapshot,
) -> Result<Option<super::NotificationDelivery>, NotificationError> {
    deliver_at(session, generation, candidate, resources, super::now_us()?).await
}

pub(super) async fn deliver_at(
    session: &Session,
    generation: crate::Generation,
    candidate: Candidate,
    resources: &crate::ResourceSnapshot,
    now_us: i64,
) -> Result<Option<super::NotificationDelivery>, NotificationError> {
    // The durable ledger has a 30-day retention window. This admission floor prevents
    // replay of an older live/inbox candidate after its identity has been collected.
    if candidate.observed_at_us < now_us.saturating_sub(30 * 24 * 60 * 60 * 1_000_000) {
        return Ok(None);
    }
    let _serial = session.inner.notifications.serial.lock().await;
    session.inner.epochs.validate(generation)?;
    let backend = session
        .inner
        .notifications
        .backend
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|policy| policy.generation == generation)
        .map(|policy| policy.config.clone())
        .ok_or(crate::ClientError::Superseded)?;
    let settings = ledger::settings(session).await?;
    let delivery = policy::decide(
        session, generation, &settings, &backend, &candidate, resources,
    );
    // Suppressed eligible candidates also consume the identity: unmuting cannot replay them later.
    // A non-triggering ordinary message must leave room for a later authoritative reply event.
    if !policy::eligible(resources, &settings, &candidate) {
        return Ok(None);
    }
    if !ledger::claim_candidate(
        session,
        generation,
        candidate.message,
        candidate.observed_at_us,
        now_us,
    )
    .await?
    {
        return Ok(None);
    }
    session.inner.epochs.validate(generation)?;
    Ok(delivery)
}
pub(crate) fn handoff(
    session: &Session,
    delivery: super::NotificationDelivery,
) -> Result<(), NotificationError> {
    session.inner.epochs.apply(delivery.target.generation, || {
        session
            .inner
            .notifications
            .deliveries
            .try_send(delivery)
            .map_err(|_| NotificationError::Overflow)
    })?
}

fn parse_time(raw: &str) -> Result<i64, NotificationError> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|date| date.timestamp_micros())
        .map_err(|_| NotificationError::InvalidData)
}

async fn authoritative_trigger(
    session: &Session,
    generation: crate::Generation,
    message: crate::Uuid,
) -> Result<Option<Candidate>, NotificationError> {
    // An ephemeral 'all' notification is not proof of a reply/everyone trigger.
    let epoch = session.inner.epochs.current();
    let rest = crate::scoped_request::authenticated(session, &epoch).await?;
    let mut cursor = None;
    for _ in 0..10 {
        let page = inbox::page(
            session,
            &rest,
            generation,
            cursor.as_ref(),
            crate::HistoryOrder::Desc,
        )
        .await?;
        if let Some(found) = page.records.into_iter().find(|row| {
            row.record
                .message_id
                .as_deref()
                .and_then(|id| id.parse().ok())
                == Some(message)
        }) {
            return record_candidate(&found.record).map(Some);
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(None)
}

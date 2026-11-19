//! Bounded live candidate ingestion; historical cache/reconnect pages never enter this producer.

use super::{
    NotificationError, NotificationStatus,
    deliver::{DeliveryOutcome, deliver, handoff},
    live_queue::{LivePoint, LiveState, enqueue_deferred, resolve_live_point},
    policy::{self, Candidate},
};
use crate::{Generated, Generation, MessageKey, Session};
use papo_api::gateway::{Event, KnownEvent};

enum LoopWork {
    Event(Generated<Event>),
    ResumeLookup,
    RetryEvidence,
}

pub(crate) fn submit(
    session: &crate::session::SessionInner,
    generation: Generation,
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
        let mut state = LiveState::new(&session);
        loop {
            // Evidence is shared only across an active event backlog, never across idle periods.
            if rx.is_empty() && state.lookup_queue.is_empty() {
                state.triggers.clear();
            }
            let wait_epoch = session.inner.epochs.current();
            state.retain_generation(wait_epoch.generation);
            let work = if state.lookup_queue.is_empty() {
                tokio::select! {
                    biased;
                    () = session.inner.root.cancelled() => return,
                    () = wait_epoch.cancel.cancelled(), if !state.evidence_queue.is_empty() => {
                        state.evidence_queue.clear();
                        continue;
                    }
                    // Drain ready deferred evidence ahead of new events so watch updates cannot be starved.
                    changed = state.resources_rx.changed(), if !state.evidence_queue.is_empty() => {
                        if changed.is_err() {
                            return;
                        }
                        LoopWork::RetryEvidence
                    }
                    changed = state.reading_rx.changed(), if !state.evidence_queue.is_empty() => {
                        if changed.is_err() {
                            return;
                        }
                        LoopWork::RetryEvidence
                    }
                    event = rx.recv() => match event {
                        Some(event) => LoopWork::Event(event),
                        None => return,
                    },
                }
            } else {
                if session.inner.root.is_cancelled() {
                    return;
                }
                match rx.try_recv() {
                    Ok(event) => LoopWork::Event(event),
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        tokio::task::yield_now().await;
                        LoopWork::ResumeLookup
                    }
                }
            };
            let epoch = session.inner.epochs.current();
            state.retain_generation(epoch.generation);
            let result = match work {
                LoopWork::Event(event) => {
                    if event.generation != epoch.generation {
                        continue;
                    }
                    tokio::select! {
                        biased;
                        () = epoch.cancel.cancelled() => continue,
                        result = process(&session, event, &mut state) => result,
                    }
                }
                LoopWork::ResumeLookup => tokio::select! {
                    biased;
                    () = epoch.cancel.cancelled() => continue,
                    result = resume_lookup(&session, epoch.generation, &mut state) => result,
                },
                LoopWork::RetryEvidence => tokio::select! {
                    biased;
                    () = epoch.cancel.cancelled() => continue,
                    result = retry_evidence(&session, epoch.generation, &mut state) => result,
                },
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
    state: &mut LiveState,
) -> Result<(), NotificationError> {
    let candidate = match event.value {
        Event::Known(KnownEvent::Message(message)) => {
            let point = MessageKey::parse(&message.created_at, &message.id)?;
            let candidate = Candidate {
                channel: super::id(Some(&message.channel_id))?,
                message: super::id(Some(&message.id))?,
                author: message.author_id.parse().ok(),
                triggered: policy::mentions(&message.content, session.account().user()),
                content: message.content,
                read: false,
                trigger_revision: None,
                observed_at_us: point.created_at_us,
                message_point: Some(point),
            };
            if state.recent.len() == 1024 {
                state.recent.pop_front();
            }
            state
                .recent
                .push_back((event.generation, candidate.clone()));
            candidate
        }
        Event::Known(KnownEvent::NewNotification(notification)) => {
            let message = super::id(Some(&notification.message_id))?;
            let recent_candidate = state
                .recent
                .iter()
                .find(|(_, candidate)| candidate.message == message);
            if let Some((_, candidate)) =
                recent_candidate.filter(|(_, candidate)| candidate.triggered)
            {
                candidate.clone()
            } else {
                let candidate = state
                    .triggers
                    .resolve(session, event.generation, message)
                    .await?;
                let Some(mut candidate) = candidate else {
                    return Ok(());
                };
                if let Some((_, recent)) = recent_candidate {
                    candidate.message_point = recent.message_point;
                }
                if candidate.message_point.is_none() {
                    if !state.lookup_queue.is_empty() {
                        // Finish the active target first so `CanonicalPoints` never ping-pongs.
                        return enqueue_deferred(
                            &mut state.lookup_queue,
                            event.generation,
                            candidate,
                            false,
                        );
                    }
                    match resolve_live_point(
                        session,
                        &mut state.points,
                        event.generation,
                        candidate.channel,
                        candidate.message,
                    )
                    .await?
                    {
                        LivePoint::Found(point) => {
                            candidate.message_point = Some(point);
                            candidate.observed_at_us = point.created_at_us;
                        }
                        LivePoint::Pending => {
                            return enqueue_deferred(
                                &mut state.lookup_queue,
                                event.generation,
                                candidate,
                                true,
                            );
                        }
                        LivePoint::Absent => return Ok(()),
                    }
                } else if let Some(point) = candidate.message_point {
                    candidate.observed_at_us = point.created_at_us;
                }
                candidate
            }
        }
        _ => return Ok(()),
    };
    deliver_or_defer(session, event.generation, candidate, state).await
}

async fn resume_lookup(
    session: &Session,
    generation: Generation,
    state: &mut LiveState,
) -> Result<(), NotificationError> {
    let Some((item_gen, mut candidate)) = state.lookup_queue.pop_front() else {
        return Ok(());
    };
    if item_gen != generation {
        return Ok(());
    }
    match resolve_live_point(
        session,
        &mut state.points,
        generation,
        candidate.channel,
        candidate.message,
    )
    .await?
    {
        LivePoint::Found(point) => {
            candidate.message_point = Some(point);
            candidate.observed_at_us = point.created_at_us;
            deliver_or_defer(session, generation, candidate, state).await
        }
        LivePoint::Pending => {
            // Keep the active target at the front so its cursor continues without thrashing.
            enqueue_deferred(&mut state.lookup_queue, generation, candidate, true)
        }
        LivePoint::Absent => Ok(()),
    }
}

async fn retry_evidence(
    session: &Session,
    generation: Generation,
    state: &mut LiveState,
) -> Result<(), NotificationError> {
    state.observe_watches();
    let count = state.evidence_queue.len();
    let mut first_error = None;
    for _ in 0..count {
        let Some((item_gen, candidate)) = state.evidence_queue.pop_front() else {
            break;
        };
        if item_gen != generation {
            continue;
        }
        let step = match deliver(session, generation, candidate.clone()).await {
            Ok(DeliveryOutcome::Handled(Some(delivery))) => handoff(session, delivery),
            Ok(DeliveryOutcome::Handled(None)) => Ok(()),
            Ok(DeliveryOutcome::Deferred) => {
                enqueue_deferred(&mut state.evidence_queue, generation, candidate, false)
            }
            Err(error) => Err(error),
        };
        if let Err(error) = step
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn deliver_or_defer(
    session: &Session,
    generation: Generation,
    candidate: Candidate,
    state: &mut LiveState,
) -> Result<(), NotificationError> {
    // Only snapshot watches when `evidence_queue` is empty so pending wakeups for already-deferred
    // candidates are never discarded by an unrelated live event.
    if state.evidence_queue.is_empty() {
        state.observe_watches();
    }
    match deliver(session, generation, candidate.clone()).await? {
        DeliveryOutcome::Handled(Some(delivery)) => handoff(session, delivery)?,
        DeliveryOutcome::Handled(None) => {}
        DeliveryOutcome::Deferred => {
            enqueue_deferred(&mut state.evidence_queue, generation, candidate, false)?;
        }
    }
    Ok(())
}

//! Server-wide mark-all-read through the existing durable local-read and debounced upload path.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use papo_api::Uuid;

use crate::{
    ClientError, Generation, HistorySnapshot, MessageKey, ReadingError, Session, UnreadCount,
    catch_up::CatchUpState,
    catch_up_store::CatchUpCommit,
    history::access::ChannelAdmission,
    reading::{ReadState, counts, upload},
};

struct MarkAllPlan {
    local_updates: Vec<(Uuid, MessageKey)>,
    catch_up_commits: Vec<CatchUpCommit>,
    catch_up_apply: Vec<(Uuid, CatchUpState)>,
    heads: BTreeMap<Uuid, MessageKey>,
    unresolved: BTreeMap<Uuid, crate::ServerReadPoint>,
    snapshots: BTreeMap<Uuid, Arc<HistorySnapshot>>,
}

pub(super) async fn mark_all_read_inner(session: &Session) -> Result<(), ReadingError> {
    let _serial = session.inner.reading.serial.lock().await;
    let generation = session.generation();
    session.inner.epochs.validate(generation)?;

    let plan = build_plan(session, generation).await?;
    if plan.local_updates.is_empty() && plan.catch_up_commits.is_empty() {
        return Ok(());
    }

    session
        .client
        .inner
        .registry
        .commit_mark_all_read(
            Arc::clone(&session.inner),
            generation,
            plan.local_updates.clone(),
            plan.catch_up_commits,
        )
        .await?;
    session.inner.epochs.validate(generation)?;

    if !plan.catch_up_apply.is_empty() {
        session.inner.reading.apply_catch_up_updates(
            &session.inner,
            generation,
            plan.catch_up_apply,
            false,
        )?;
    }

    if !plan.local_updates.is_empty() {
        publish_and_schedule(
            session,
            generation,
            plan.local_updates,
            &plan.heads,
            &plan.unresolved,
            &plan.snapshots,
        )
        .await?;
    }
    Ok(())
}

async fn build_plan(
    session: &Session,
    generation: Generation,
) -> Result<MarkAllPlan, ReadingError> {
    let resources = session.inner.resources.borrow().clone();
    if resources.generation != generation {
        return Err(ClientError::Superseded.into());
    }
    let allowed: BTreeSet<Uuid> = {
        let admissions = session.inner.history.admission.read().await;
        resources
            .channels
            .iter()
            .map(|channel| channel.id)
            .filter(|&id| {
                admissions.channel(id) == Some(ChannelAdmission::Allowed)
                    && session
                        .inner
                        .reading
                        .positions
                        .check_current_channel(id, generation)
                        .is_none()
            })
            .collect()
    };
    let heads = session.inner.reading.heads.lock().await.clone();
    let unresolved = session.inner.reading.unresolved.lock().await.clone();
    let snapshots: BTreeMap<Uuid, Arc<HistorySnapshot>> = session
        .inner
        .history
        .snapshots()
        .await
        .into_iter()
        .filter(|(_, snapshot)| snapshot.generation == generation)
        .collect();
    let current_image = session.inner.reading.image.borrow().clone();

    let mut local_updates = Vec::new();
    let mut catch_up_commits = Vec::new();
    let mut catch_up_apply = Vec::new();

    for channel in allowed {
        let cached_head = snapshots
            .get(&channel)
            .and_then(|snapshot| snapshot.messages.last())
            .map(|message| MessageKey::from_message(message))
            .transpose()?;
        let head = heads.get(&channel).copied().max(cached_head);
        let prev_state = current_image.channels.get(&channel);
        let prev_point = prev_state.and_then(ReadState::point);
        let prev_local = prev_state.and_then(|state| state.local_point);
        let new_local = head.filter(|&point| {
            Some(point) > prev_point
                || (unresolved.contains_key(&channel) && Some(point) > prev_local)
        });
        if let Some(point) = new_local {
            local_updates.push((channel, point));
        }
        if let Some(mut next_catch_up) = session.inner.reading.catch_up_state_for(channel) {
            let clear_head = new_local
                .into_iter()
                .chain(prev_point)
                .chain(next_catch_up.interval.as_ref().map(|i| i.end_before))
                .chain(next_catch_up.seen_to)
                .max();
            if next_catch_up.clear(clear_head) {
                catch_up_commits.push(CatchUpCommit {
                    channel,
                    state: next_catch_up.clone(),
                });
                catch_up_apply.push((channel, next_catch_up));
            }
        }
    }

    Ok(MarkAllPlan {
        local_updates,
        catch_up_commits,
        catch_up_apply,
        heads,
        unresolved,
        snapshots,
    })
}

async fn publish_and_schedule(
    session: &Session,
    generation: Generation,
    local_updates: Vec<(Uuid, MessageKey)>,
    heads: &BTreeMap<Uuid, MessageKey>,
    unresolved: &BTreeMap<Uuid, crate::ServerReadPoint>,
    snapshots: &BTreeMap<Uuid, Arc<HistorySnapshot>>,
) -> Result<(), ReadingError> {
    let mut image = (**session.inner.reading.image.borrow()).clone();
    image.generation = generation;
    let mut scheduled = Vec::with_capacity(local_updates.len());
    for (channel, point) in local_updates {
        if session
            .inner
            .reading
            .positions
            .check_current_channel(channel, generation)
            .is_some()
        {
            continue;
        }
        let state = image.channels.entry(channel).or_default();
        state.local_point = state.local_point.max(Some(point));
        if let Some(snapshot) = snapshots.get(&channel) {
            counts::update(state, snapshot, heads.get(&channel).copied())?;
        } else {
            state.unread_count = UnreadCount::AtLeast(u64::from(
                heads
                    .get(&channel)
                    .is_some_and(|head| Some(*head) > state.point()),
            ));
            state.first_unread = None;
        }
        if unresolved.contains_key(&channel) {
            counts::unresolved(state);
        }
        scheduled.push(channel);
    }
    session.inner.reading.publish(&session.inner, image).await?;
    for channel in scheduled {
        upload::schedule(session, channel).await;
    }
    Ok(())
}

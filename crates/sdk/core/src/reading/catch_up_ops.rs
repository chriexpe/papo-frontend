//! Account-scoped catch-up visibility reporting, clear operations and view resolution.
//! In-memory state tracking lives in `reading::catch_up_state`; interval math lives in `catch_up`.

use std::sync::Arc;

use papo_api::Uuid;

use crate::{
    CatchUpView, Generation, HistorySnapshot, MessageKey, ReadingError, Session,
    VisibilityContinuity,
    catch_up::{CatchUpHistory, CatchUpState, VisibleBounds},
    reading::{counts, upload},
};

pub(super) async fn report_current(
    session: &Session,
    observer_id: u64,
    channel: Uuid,
    generation: Generation,
    history_version: u64,
    ids: &[Uuid],
    continuity: VisibilityContinuity,
) -> Result<(), ReadingError> {
    let _serial = session.inner.reading.serial.lock().await;
    session.inner.epochs.validate(generation)?;
    let history = session.history(channel).await?;
    let (snapshot, visible_points, point) = history
        .visible_snapshot(generation, history_version, ids)
        .await?;
    if visible_points.is_empty() {
        return Ok(());
    }
    let (prev_acknowledged, new_local) = {
        let borrowed = session.inner.reading.image.borrow();
        let prev_state = borrowed.channels.get(&channel);
        let prev_acknowledged = prev_state.and_then(super::ReadState::point);
        let prev_local = prev_state.and_then(|s| s.local_point);
        (prev_acknowledged, point.filter(|&p| Some(p) > prev_local))
    };

    let index = CatchUpHistory::new(&snapshot, session.account().user())?;
    let mut next_catch_up = session
        .inner
        .reading
        .catch_up_state_for(channel)
        .unwrap_or_default();
    let last_visible = session
        .inner
        .reading
        .observer_last_visible(observer_id, channel);
    let observation = next_catch_up.observe_visible(
        &index,
        prev_acknowledged,
        last_visible,
        &visible_points,
        continuity,
    )?;

    if new_local.is_none() && !observation.changed {
        if let Some(vis) = observation.next_bounds {
            session
                .inner
                .reading
                .set_observer_last_visible(observer_id, channel, vis);
        }
        return Ok(());
    }
    session
        .client
        .inner
        .registry
        .commit_visible_read_and_catch_up(
            history,
            generation,
            history_version,
            new_local,
            observation.changed.then(|| next_catch_up.clone()),
        )
        .await?;
    session.inner.epochs.validate(generation)?;

    publish_reported(ReportedPublish {
        session,
        observer_id,
        channel,
        generation,
        snapshot: &snapshot,
        next_last_visible: observation.next_bounds,
        catch_up_changed: observation.changed,
        next_catch_up,
        new_local,
    })
    .await
}

struct ReportedPublish<'a> {
    session: &'a Session,
    observer_id: u64,
    channel: Uuid,
    generation: Generation,
    snapshot: &'a HistorySnapshot,
    next_last_visible: Option<VisibleBounds>,
    catch_up_changed: bool,
    next_catch_up: CatchUpState,
    new_local: Option<MessageKey>,
}

async fn publish_reported(p: ReportedPublish<'_>) -> Result<(), ReadingError> {
    if let Some(vis) = p.next_last_visible {
        p.session
            .inner
            .reading
            .set_observer_last_visible(p.observer_id, p.channel, vis);
    }
    if p.catch_up_changed {
        p.session.inner.reading.apply_catch_up_updates(
            &p.session.inner,
            p.generation,
            vec![(p.channel, p.next_catch_up)],
            false,
        )?;
    }
    if let Some(local) = p.new_local {
        let mut image = (**p.session.inner.reading.image.borrow()).clone();
        image.generation = p.generation;
        let state = image.channels.entry(p.channel).or_default();
        state.local_point = Some(local);
        counts::update_for(
            p.session,
            p.channel,
            state,
            p.snapshot,
            p.session
                .inner
                .reading
                .heads
                .lock()
                .await
                .get(&p.channel)
                .copied(),
        )
        .await?;
        if p.session
            .inner
            .reading
            .unresolved
            .lock()
            .await
            .contains_key(&p.channel)
        {
            counts::unresolved(state);
        }
        p.session.inner.reading.publish(&p.session.inner, image)?;
        upload::schedule(p.session, p.channel).await;
    }
    Ok(())
}

pub(super) async fn clear_catch_up_inner(
    session: &Session,
    channel: Uuid,
) -> Result<(), ReadingError> {
    let _serial = session.inner.reading.serial.lock().await;
    let generation = session.generation();
    session.inner.epochs.validate(generation)?;
    if let Some(reason) = session
        .inner
        .reading
        .positions
        .check_current_channel(channel, generation)
    {
        return Err(ReadingError::History(crate::HistoryError::Closed(reason)));
    }
    let Some(mut next) = session.inner.reading.catch_up_state_for(channel) else {
        return Ok(());
    };
    let read_head = session
        .inner
        .reading
        .image
        .borrow()
        .channels
        .get(&channel)
        .and_then(super::ReadState::point);
    let head = read_head
        .into_iter()
        .chain(next.interval.as_ref().map(|i| i.end_before))
        .chain(next.seen_to)
        .max();
    if !next.clear(head) {
        return Ok(());
    }
    session
        .client
        .inner
        .registry
        .commit_clear_catch_up(
            Arc::clone(&session.inner),
            channel,
            generation,
            next.clone(),
        )
        .await?;
    session.inner.epochs.validate(generation)?;
    session.inner.reading.apply_catch_up_updates(
        &session.inner,
        generation,
        vec![(channel, next)],
        false,
    )?;
    Ok(())
}

pub(super) fn resolve_view(
    session: &Session,
    channel: Uuid,
    snapshot: &HistorySnapshot,
) -> Result<Option<CatchUpView>, ReadingError> {
    let Some(state) = session
        .inner
        .reading
        .catch_up_state_for(channel)
        .filter(|state| state.interval.is_some())
    else {
        return Ok(None);
    };
    let index = CatchUpHistory::new(snapshot, session.account().user())?;
    state.view(&index)
}

//! Account-scoped catch-up visibility reporting, view resolution and snapshot publication.
//! Per-window observer lifecycle lives in `reading::observers`; core interval math lives in `catch_up`.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::Uuid;

use crate::{
    CatchUpSnapshot, CatchUpView, ClientError, Generation, HistorySnapshot, ReadPoint,
    ReadingError, Session, VisibilityContinuity, catch_up::SnapshotIndex, reading::counts,
    reading::upload, session::SessionInner,
};

impl super::Reading {
    pub fn publish_catch_up(
        &self,
        session: &SessionInner,
        mut image: CatchUpSnapshot,
    ) -> Result<(), ClientError> {
        session.epochs.apply(image.generation, || {
            image.version = self
                .catch_up_image
                .borrow()
                .version
                .checked_add(1)
                .ok_or(ClientError::Closed)?;
            self.catch_up_image.send_replace(Arc::new(image));
            Ok(())
        })?
    }

    pub(crate) fn record_asc_successor(
        &self,
        channel: Uuid,
        generation: Generation,
        after: ReadPoint,
        first_returned: Option<ReadPoint>,
    ) {
        let mut guard = self
            .asc_successors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (asc_version, map) = match &mut *guard {
            Some((current_gen, asc_version, map)) if *current_gen == generation => {
                (asc_version, map)
            }
            slot => {
                *slot = Some((generation, 0, BTreeMap::new()));
                let (_, asc_version, map) = slot.as_mut().expect("just assigned");
                (asc_version, map)
            }
        };
        *asc_version = asc_version.saturating_add(1);
        map.entry(channel)
            .or_default()
            .insert(after, first_returned);
    }

    pub(crate) fn asc_successors_for(
        &self,
        generation: Generation,
        channel: Uuid,
    ) -> crate::catch_up::AscSuccessors {
        self.asc_successors_with_version(generation, channel).0
    }

    pub(crate) fn asc_successors_with_version(
        &self,
        generation: Generation,
        channel: Uuid,
    ) -> (crate::catch_up::AscSuccessors, u64) {
        let guard = self
            .asc_successors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*guard {
            Some((current_gen, asc_version, map)) if *current_gen == generation => {
                (map.get(&channel).cloned().unwrap_or_default(), *asc_version)
            }
            _ => (BTreeMap::new(), 0),
        }
    }
}

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
    let mut image = (**session.inner.reading.image.borrow()).clone();
    image.generation = generation;
    let state = image.channels.entry(channel).or_default();
    let prev_acknowledged = state.point();
    let new_local = point.filter(|&p| Some(p) > state.local_point);

    let asc = session
        .inner
        .reading
        .asc_successors_for(generation, channel);
    let index = SnapshotIndex::build(&snapshot, session.account().user(), &asc)?;
    let mut catch_up_image = (**session.inner.reading.catch_up_image.borrow()).clone();
    catch_up_image.generation = generation;
    let mut next_catch_up = catch_up_image
        .channels
        .get(&channel)
        .cloned()
        .unwrap_or_default();
    let last_visible = session
        .inner
        .reading
        .observer_last_visible(observer_id, channel);
    let (catch_up_changed, next_last_visible) = next_catch_up.observe_visible(
        &index,
        prev_acknowledged,
        last_visible,
        &visible_points,
        continuity,
    );

    if new_local.is_none() && !catch_up_changed {
        if let Some(vis) = next_last_visible {
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
            catch_up_changed.then(|| next_catch_up.clone()),
        )
        .await?;
    session.inner.epochs.validate(generation)?;

    publish_reported(ReportedPublish {
        session,
        observer_id,
        channel,
        snapshot: &snapshot,
        next_last_visible,
        catch_up_changed,
        next_catch_up,
        catch_up_image,
        new_local,
        image,
    })
    .await
}

struct ReportedPublish<'a> {
    session: &'a Session,
    observer_id: u64,
    channel: Uuid,
    snapshot: &'a HistorySnapshot,
    next_last_visible: Option<(ReadPoint, ReadPoint)>,
    catch_up_changed: bool,
    next_catch_up: crate::CatchUpState,
    catch_up_image: CatchUpSnapshot,
    new_local: Option<ReadPoint>,
    image: super::ReadingSnapshot,
}

async fn publish_reported(mut p: ReportedPublish<'_>) -> Result<(), ReadingError> {
    if let Some(vis) = p.next_last_visible {
        p.session
            .inner
            .reading
            .set_observer_last_visible(p.observer_id, p.channel, vis);
    }
    if p.catch_up_changed {
        if p.next_catch_up.seen_to.is_none() && p.next_catch_up.interval.is_none() {
            p.catch_up_image.channels.remove(&p.channel);
        } else {
            p.catch_up_image.channels.insert(p.channel, p.next_catch_up);
        }
        p.session
            .inner
            .reading
            .publish_catch_up(&p.session.inner, p.catch_up_image)?;
    }
    if let Some(local) = p.new_local {
        let state = p.image.channels.entry(p.channel).or_default();
        state.local_point = Some(local);
        counts::update(
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
        )?;
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
        p.session.inner.reading.publish(&p.session.inner, p.image)?;
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
    let mut catch_up_image = (**session.inner.reading.catch_up_image.borrow()).clone();
    catch_up_image.generation = generation;
    let Some(mut next) = catch_up_image.channels.get(&channel).cloned() else {
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
    if next.seen_to.is_none() && next.interval.is_none() {
        catch_up_image.channels.remove(&channel);
    } else {
        catch_up_image.channels.insert(channel, next);
    }
    session
        .inner
        .reading
        .publish_catch_up(&session.inner, catch_up_image)?;
    Ok(())
}

pub(super) fn resolve_view(
    session: &Session,
    channel: Uuid,
    snapshot: &HistorySnapshot,
) -> Result<CatchUpView, ReadingError> {
    let Some(state) = session
        .inner
        .reading
        .catch_up_image
        .borrow()
        .channels
        .get(&channel)
        .filter(|state| state.interval.is_some())
        .cloned()
    else {
        return Ok(CatchUpView::default());
    };
    let asc = session
        .inner
        .reading
        .asc_successors_for(snapshot.generation, channel);
    let index = SnapshotIndex::build(snapshot, session.account().user(), &asc)?;
    Ok(state.view(&index))
}

//! Debounced per-channel read uploads; durable points survive cancellation and only current completions are adopted.

use super::{ReadPoint, counts, reconcile};
use crate::{
    ClientError, ConnectionState, Session,
    read_points::{ServerReadPoint, UploadedPoint},
};
use papo_api::Uuid;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

const DEBOUNCE: Duration = Duration::from_millis(300);
const RETRY: Duration = Duration::from_secs(2);

pub(super) async fn schedule(session: &Session, channel: Uuid) {
    let mut uploads = session.inner.reading.uploads.lock().await;
    if let Some(wake) = uploads.get(&channel) {
        wake.notify_one();
        return;
    }
    if session.is_closed() {
        return;
    }
    let wake = Arc::new(Notify::new());
    uploads.insert(channel, Arc::clone(&wake));
    let token = session.inner.tasks.token();
    let session = session.clone();
    let runtime = session.client.inner.runtime.clone();
    runtime.spawn(async move {
        let _token = token;
        run(&session, channel, &wake).await;
    });
}

async fn run(session: &Session, channel: Uuid, wake: &Notify) {
    let mut connection = session.subscribe_connection();
    let mut delay = DEBOUNCE;
    loop {
        {
            // Report admission uses the same order, so a report cannot lose its wake
            // between the final pending check and removal of this worker's ownership.
            let _serial = session.inner.reading.serial.lock().await;
            if session
                .reading_state(channel)
                .and_then(|state| state.pending())
                .is_none()
            {
                session.inner.reading.uploads.lock().await.remove(&channel);
                return;
            }
        }
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                biased;
                () = session.inner.root.cancelled() => return,
                () = wake.notified() => sleep.as_mut().reset(tokio::time::Instant::now() + delay),
                result = connection.changed() => { if result.is_err() { return; } },
                () = &mut sleep => break,
            }
        }
        while !matches!(connection.borrow().state, ConnectionState::Online) {
            tokio::select! {
                biased;
                () = session.inner.root.cancelled() => return,
                result = connection.changed() => if result.is_err() { return; },
                () = wake.notified() => {},
            }
        }
        let epoch = session.inner.epochs.current();
        let result = tokio::select! {
            biased;
            () = session.inner.root.cancelled() => return,
            () = epoch.cancel.cancelled() => Err(ClientError::Superseded),
            result = once(session, channel, epoch.generation) => result,
        };
        delay = match result {
            Ok(()) => DEBOUNCE,
            Err(ClientError::Closed) => return,
            Err(_) => RETRY,
        };
    }
}

async fn once(
    session: &Session,
    channel: Uuid,
    generation: crate::Generation,
) -> Result<(), ClientError> {
    session.inner.epochs.validate(generation)?;
    let Some(point) = session
        .reading_state(channel)
        .and_then(|state| state.pending())
    else {
        return Ok(());
    };
    let (connected_generation, connection) = session
        .inner
        .reading
        .connection
        .lock()
        .await
        .clone()
        .ok_or(ClientError::NotFound)?;
    if connected_generation != generation {
        return Err(ClientError::Superseded);
    }
    match session
        .client
        .inner
        .request(connection.rest.read_channel(channel, point.message_id))
        .await
    {
        Ok(()) => {}
        Err(ClientError::Api(
            papo_api::Error::Http { status: 404 } | papo_api::Error::Problem { status: 404, .. },
        )) => {
            // A deleted target cannot be uploaded again; accepting it never advances
            // another account, and the already-durable ordered point remains usable.
            return confirm(session, channel, generation, point, None, None).await;
        }
        Err(error) => return Err(error),
    }
    session.inner.epochs.validate(generation)?;
    let RemoteMark {
        message,
        recorded_at,
        head,
    } = remote_mark(session, &connection, channel).await?;
    session.inner.epochs.validate(generation)?;
    let mut accepted = point;
    let mut acknowledgement = None;
    let mut unresolved = None;
    if let Some(message) = message {
        let message_id = message.parse().map_err(|_| ClientError::ResourceData)?;
        let server = ServerReadPoint {
            message_id,
            recorded_at,
        };
        let state = session.reading_state(channel).unwrap_or_default();
        let head = head.as_ref().map(reconcile::head_point).transpose()?;
        match reconcile::resolve(
            session,
            &connection,
            generation,
            channel,
            &server,
            &state,
            head,
        )
        .await?
        {
            Some(remote) if remote >= accepted => {
                accepted = remote;
                acknowledgement = server.recorded_at;
            }
            Some(_) => {}
            None => unresolved = Some(server),
        }
        if let Some(head) = head {
            let mut heads = session.inner.reading.heads.lock().await;
            session.inner.epochs.apply(generation, || {
                heads.insert(channel, head);
            })?;
        }
    }
    confirm(
        session,
        channel,
        generation,
        accepted,
        acknowledgement,
        unresolved,
    )
    .await
}

struct RemoteMark {
    message: Option<String>,
    recorded_at: Option<String>,
    head: Option<papo_api::wire::ChannelLastMessage>,
}

async fn remote_mark(
    session: &Session,
    connection: &crate::reconcile::ResourceConnection,
    channel: Uuid,
) -> Result<RemoteMark, ClientError> {
    let dm = session
        .inner
        .resources
        .borrow()
        .dms
        .iter()
        .any(|dm| dm.id.parse::<Uuid>().ok() == Some(channel));
    if dm {
        let dm = session
            .client
            .inner
            .request(connection.rest.direct_conversation(channel))
            .await?;
        Ok(RemoteMark {
            message: dm.last_read_message,
            recorded_at: dm.last_read_at,
            head: dm.last_message,
        })
    } else {
        let channels = session
            .client
            .inner
            .request(connection.rest.channels())
            .await?;
        let mark = channels
            .channels
            .unwrap_or_default()
            .into_iter()
            .find(|item| item.id.as_deref().and_then(|id| id.parse::<Uuid>().ok()) == Some(channel))
            .ok_or(ClientError::NotFound)?;
        Ok(RemoteMark {
            message: mark.last_read_message,
            recorded_at: mark.last_read_at,
            head: mark.last_message,
        })
    }
}

async fn confirm(
    session: &Session,
    channel: Uuid,
    generation: crate::Generation,
    point: ReadPoint,
    recorded_at: Option<String>,
    unresolved: Option<ServerReadPoint>,
) -> Result<(), ClientError> {
    let _serial = session.inner.reading.serial.lock().await;
    session.inner.epochs.validate(generation)?;
    let mut image = (**session.inner.reading.image.borrow()).clone();
    let Some(state) = image.channels.get_mut(&channel) else {
        return Ok(());
    };
    let point = state
        .uploaded_point
        .max(Some(point))
        .ok_or(ClientError::ResourceData)?;
    let current_unresolved = session
        .inner
        .reading
        .unresolved
        .lock()
        .await
        .get(&channel)
        .filter(|mark| mark.message_id != point.message_id)
        .cloned();
    let unresolved = unresolved.or(current_unresolved);
    session
        .client
        .inner
        .registry
        .save_uploaded_read(
            Arc::clone(&session.inner),
            channel,
            generation,
            UploadedPoint {
                server: unresolved.clone().unwrap_or(ServerReadPoint {
                    message_id: point.message_id,
                    recorded_at,
                }),
                created_at_us: unresolved.is_none().then_some(point.created_at_us),
                known_point: Some(point),
            },
        )
        .await?;
    session.inner.epochs.validate(generation)?;
    state.uploaded_point = Some(point);
    if let Some((_, snapshot)) = session
        .inner
        .history
        .snapshots()
        .await
        .into_iter()
        .find(|(id, _)| *id == channel)
    {
        counts::update(
            state,
            &snapshot,
            session
                .inner
                .reading
                .heads
                .lock()
                .await
                .get(&channel)
                .copied(),
        )
        .map_err(|_| ClientError::ResourceData)?;
    }
    {
        let mut marks = session.inner.reading.unresolved.lock().await;
        session.inner.epochs.apply(generation, || {
            if let Some(server) = unresolved {
                marks.insert(channel, server);
            } else {
                marks.remove(&channel);
            }
        })?;
        if marks.contains_key(&channel) {
            counts::unresolved(state);
        }
    }
    image.generation = generation;
    session.inner.reading.publish(&session.inner, image)
}

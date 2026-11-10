//! Socket-side reconciliation and bounded event journaling; fetching, cache SQL and UI diffs live elsewhere.

use crate::{
    AccountCache, ClientError, ClientHandle, ConnectionState, Generated, ResourceSnapshot,
    connection_driver::Socket, generation::Epoch, session::SessionInner,
};
use papo_api::{RestClient, gateway::Event};
use std::{collections::VecDeque, future::Future, sync::Arc};

#[cfg(test)]
mod tests;

const JOURNAL_CAPACITY: usize = 1024;

#[derive(Clone)]
pub(crate) struct ResourceConnection {
    pub client: ClientHandle,
    pub rest: RestClient,
    pub cache: Arc<AccountCache>,
}

pub(crate) async fn run(
    session: &SessionInner,
    epoch: &Epoch,
    socket: &mut dyn Socket,
    connection: &ResourceConnection,
) -> Result<(), ClientError> {
    let mut journal = VecDeque::new();
    collect(session, epoch, socket, &mut journal, async {
        let _guard = session.resource_serial.lock().await;
        let snapshot = connection.cache.load(epoch.generation).await?;
        session.publish_resources(snapshot)
    })
    .await?;
    collect(session, epoch, socket, &mut journal, async {
        let _guard = session.resource_serial.lock().await;
        let snapshot = crate::reconcile_fetch::fetch(connection, session, epoch).await?;
        connection
            .client
            .inner
            .registry
            .reconcile_read_points(session.account, &snapshot.channels, &snapshot.dms)
            .await?;
        session.epochs.validate(epoch.generation)?;
        connection.cache.replace(&snapshot).await?;
        session.publish_resources(snapshot)
    })
    .await?;
    replay(session, epoch, socket, connection, &mut journal).await?;
    session.status(epoch.generation, ConnectionState::Online)?;
    loop {
        let event = next(session, epoch, socket).await?;
        journal.push_back(event);
        replay(session, epoch, socket, connection, &mut journal).await?;
    }
}

async fn replay(
    session: &SessionInner,
    epoch: &Epoch,
    socket: &mut dyn Socket,
    connection: &ResourceConnection,
    journal: &mut VecDeque<Event>,
) -> Result<(), ClientError> {
    while let Some(event) = journal.pop_front() {
        if matches!(event, Event::Closed(_)) {
            return Err(ClientError::Gateway(papo_api::gateway::Error::Transport));
        }
        if !crate::reconcile_event::relevant(&event) {
            session.epochs.apply(epoch.generation, || {
                let _ = session.events.send(Generated {
                    generation: epoch.generation,
                    value: event,
                });
            })?;
            continue;
        }
        collect(session, epoch, socket, journal, async {
            let _guard = session.resource_serial.lock().await;
            let mut snapshot = (**session.resources.borrow()).clone();
            // A new connection starts with a replacement, never merges old generation presence.
            if snapshot.generation != epoch.generation {
                snapshot = ResourceSnapshot::empty(epoch.generation);
            }
            if let Some(changed) =
                crate::reconcile_event::apply(connection, &mut snapshot, &event).await?
            {
                session.epochs.validate(epoch.generation)?;
                if changed.contains(&crate::CacheResource::Channels) {
                    connection
                        .client
                        .inner
                        .registry
                        .reconcile_read_points(session.account, &snapshot.channels, &snapshot.dms)
                        .await?;
                }
                connection.cache.update(&snapshot, &changed).await?;
                session.publish_resources(snapshot)?;
            }
            session.epochs.apply(epoch.generation, || {
                let _ = session.events.send(Generated {
                    generation: epoch.generation,
                    value: event,
                });
            })
        })
        .await?;
    }
    Ok(())
}

/// The socket keeps progressing while a REST/cache future waits. A bounded overflow
/// fails this generation, so reconnect replaces state instead of silently dropping events.
async fn collect<T>(
    session: &SessionInner,
    epoch: &Epoch,
    socket: &mut dyn Socket,
    journal: &mut VecDeque<Event>,
    work: impl Future<Output = Result<T, ClientError>>,
) -> Result<T, ClientError> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            biased;
            ()=session.root.cancelled()=>return Err(ClientError::Closed),
            ()=epoch.cancel.cancelled()=>return Err(ClientError::Superseded),
            result=&mut work=>return result,
            event=socket.next()=> {
                let event=event?.ok_or(ClientError::Gateway(papo_api::gateway::Error::Transport))?;
                if matches!(event,Event::Closed(_)) { return Err(ClientError::Gateway(papo_api::gateway::Error::Transport)); }
                if journal.len()==JOURNAL_CAPACITY { return Err(ClientError::ReconcileOverflow); }
                journal.push_back(event);
            }
        }
    }
}

async fn next(
    session: &SessionInner,
    epoch: &Epoch,
    socket: &mut dyn Socket,
) -> Result<Event, ClientError> {
    tokio::select! {
        biased;
        ()=session.root.cancelled()=>Err(ClientError::Closed),
        ()=epoch.cancel.cancelled()=>Err(ClientError::Superseded),
        event=socket.next()=>event?.ok_or(ClientError::Gateway(papo_api::gateway::Error::Transport)),
    }
}

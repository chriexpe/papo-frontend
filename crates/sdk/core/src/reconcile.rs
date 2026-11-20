//! Socket-side reconciliation and bounded event journaling; fetching, cache SQL and UI diffs live elsewhere.

use crate::{
    AccountCache, ClientError, ClientHandle, ConnectionState, Generated,
    connection_driver::Socket,
    generation::Epoch,
    history::access::Refetch,
    resource_admission::Admission,
    resource_commit::{self, FetchStamp},
    session::SessionInner,
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
    session: &Arc<SessionInner>,
    epoch: &Epoch,
    socket: &mut dyn Socket,
    connection: &ResourceConnection,
) -> Result<(), ClientError> {
    let mut journal = VecDeque::new();
    collect(session, epoch, socket, &mut journal, async {
        let _guard = session.resource_serial.lock().await;
        let held = session.resources.borrow().clone();
        // A resident coherent image preserves entity identity and rejected-evidence causes
        // across warm reconnect. A failed post-switch install must reload the visible revision.
        let warm = crate::history::access::warm(session).await;
        let mut snapshot = if !warm || session.resource_recovery.borrow().is_some() {
            let mut disk = connection.cache.load(epoch.generation).await?;
            disk.gaps = held.gaps.clone();
            disk
        } else {
            (*held).clone()
        };
        snapshot.generation = epoch.generation;
        snapshot.channel_census_complete = false;
        session.publish_resources(snapshot)
    })
    .await?;
    collect(session, epoch, socket, &mut journal, async {
        let guard = resource_commit::reserve(session).await;
        let mut stamp = FetchStamp::capture(session, epoch.generation, Refetch::Complete).await?;
        let facts = crate::reconcile_fetch::fetch(
            &mut stamp.observing(session, Some(&guard)),
            connection,
            Admission::new(session),
        )
        .await;
        resource_commit::commit(session, connection, guard, stamp, facts)
            .await?
            .require_current()
    })
    .await?;
    crate::notifications::live::start(connection.client.session(session.account)?);
    replay(session, epoch, socket, connection, &mut journal).await?;
    session.status(epoch.generation, ConnectionState::Online)?;
    loop {
        let event = next(session, epoch, socket).await?;
        journal.push_back(event);
        replay(session, epoch, socket, connection, &mut journal).await?;
    }
}

async fn replay(
    session: &Arc<SessionInner>,
    epoch: &Epoch,
    socket: &mut dyn Socket,
    connection: &ResourceConnection,
    journal: &mut VecDeque<Event>,
) -> Result<(), ClientError> {
    while let Some(event) = journal.pop_front() {
        if matches!(event, Event::Closed(_)) {
            return Err(ClientError::Gateway(papo_api::gateway::Error::Transport));
        }
        crate::notifications::live::submit(session, epoch.generation, &event);
        if crate::history::live::relevant(&event) {
            collect(session, epoch, socket, journal, async {
                crate::history::live::apply(session, connection, epoch.generation, &event).await
            })
            .await?;
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
            let guard = resource_commit::reserve(session).await;
            let mut stamp =
                FetchStamp::capture(session, epoch.generation, Refetch::Partial).await?;
            let facts = crate::reconcile_event::fetch(
                Admission::new(session),
                connection,
                &mut stamp.observing(session, Some(&guard)),
                &event,
            )
            .await;
            resource_commit::commit(session, connection, guard, stamp, facts)
                .await?
                .require_current()?;
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
                if journal.len()==JOURNAL_CAPACITY {
                    session.history.counters.reconnect_overflows.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Err(ClientError::ReconcileOverflow);
                }
                crate::activity::receive(session, epoch.generation, &event);
                journal.push_back(event);
                session.history.counters.reconnect_journal_peak.fetch_max(journal.len() as u64, std::sync::atomic::Ordering::Relaxed);
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
        event=socket.next()=> {
            let event = event?.ok_or(ClientError::Gateway(papo_api::gateway::Error::Transport))?;
            crate::activity::receive(session, epoch.generation, &event);
            Ok(event)
        },
    }
}

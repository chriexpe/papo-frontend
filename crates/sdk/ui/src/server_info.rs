//! Account-scoped live server metadata projection; persistence and settings mutations live elsewhere.

use futures_util::{Stream, StreamExt, stream};
use papo_sdk::{Generation, ResourceSnapshot, ServerRecord, Session};
use std::{pin::Pin, sync::Arc};
use tokio::sync::watch;

/// Current admitted server metadata snapshot for one account session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfoView {
    /// Connection epoch owning `server`.
    pub generation: Generation,
    /// Admitted server metadata, if synced for this generation.
    pub server: Option<Arc<ServerRecord>>,
}

/// Atomic baseline server metadata and deduplicated updates; ends on session closure.
pub struct ServerInfoSubscription {
    /// Baseline captured atomically with the resource watch.
    pub snapshot: ServerInfoView,
    /// Subsequent updates whenever generation or server metadata changes.
    pub stream: Pin<Box<dyn Stream<Item = ServerInfoView> + Send>>,
}

/// Projects live server metadata from an account's admitted resource snapshots.
#[derive(Clone)]
pub struct ServerInfoService {
    session: Session,
}

impl ServerInfoService {
    /// Binds to this account's existing SDK session.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Captures live server metadata atomically with its watch; ends when the session closes.
    #[must_use]
    pub fn subscribe(&self) -> ServerInfoSubscription {
        subscribe_resources(self.session.subscribe_resources())
    }
}

fn subscribe_resources(
    mut resources: watch::Receiver<Arc<ResourceSnapshot>>,
) -> ServerInfoSubscription {
    let initial = project(&resources.borrow_and_update());
    let changes = stream::unfold(
        (resources, initial.clone()),
        |(mut resources, previous)| async move {
            loop {
                resources.changed().await.ok()?;
                let next = project(&resources.borrow_and_update());
                if next != previous {
                    return Some((next.clone(), (resources, next)));
                }
            }
        },
    );
    ServerInfoSubscription {
        snapshot: initial,
        stream: Box::pin(changes.fuse()),
    }
}

fn project(snapshot: &ResourceSnapshot) -> ServerInfoView {
    ServerInfoView {
        generation: snapshot.generation,
        server: snapshot.server.clone(),
    }
}

#[cfg(test)]
mod tests {
    //! Verifies change-only deduplication and watch closure without network or sessions.
    use super::*;
    use futures_util::FutureExt;
    use papo_sdk::{Server, Uuid};

    fn snapshot_with_server(generation: u64, name: Option<&str>) -> Arc<ResourceSnapshot> {
        let server = name.map(|name| Server {
            id: Some(Uuid::from_u128(1).to_string()),
            name: Some(name.into()),
            owner_id: Some(Uuid::from_u128(99).to_string()),
            public: Some(true),
            ..Server::default()
        });
        Arc::new(
            ResourceSnapshot::fixture(
                Generation::for_test(generation),
                server,
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                Uuid::from_u128(2),
            )
            .unwrap(),
        )
    }

    #[test]
    fn unchanged_server_commits_stay_silent_and_updates_project_metadata() {
        let (tx, rx) = watch::channel(snapshot_with_server(1, Some("Alpha")));
        let mut sub = subscribe_resources(rx);
        assert_eq!(sub.snapshot.generation, Generation::for_test(1));
        assert_eq!(
            sub.snapshot.server.as_ref().and_then(|s| s.name.as_deref()),
            Some("Alpha")
        );

        let mut same = snapshot_with_server(1, Some("Alpha"));
        Arc::make_mut(&mut same).version = 99;
        tx.send_replace(same);
        assert!(sub.stream.next().now_or_never().is_none());

        tx.send_replace(snapshot_with_server(2, Some("Beta")));
        let update = sub.stream.next().now_or_never().unwrap().unwrap();
        assert_eq!(update.generation, Generation::for_test(2));
        assert_eq!(
            update.server.as_ref().and_then(|s| s.name.as_deref()),
            Some("Beta")
        );

        drop(tx);
        assert!(sub.stream.next().now_or_never().unwrap().is_none());
    }
}

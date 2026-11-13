//! Source coalescing, identity reuse and irrelevant-publication work budgets without sockets or wall clock.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    id: String,
    revision: usize,
}
impl Identified for Item {
    fn id(&self) -> &str {
        &self.id
    }
}
fn image() -> ResourceSnapshot {
    ResourceSnapshot {
        version: 1,
        generation: Generation::for_test(1),
        server: None,
        channels: Arc::default(),
        roles: Arc::default(),
        members: Arc::default(),
        profiles: Arc::default(),
        dms: Arc::default(),
        blocks: Arc::default(),
    }
}
#[tokio::test]
async fn unrelated_publications_never_project_or_diff_the_roster() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let baseline = Arc::new(image());
    let (sender, source) = watch::channel(baseline.clone());
    let mut subscription = subscribe(
        source,
        move |snapshot| {
            count.fetch_add(1, Ordering::SeqCst);
            (0..10_000)
                .map(|id| Item {
                    id: id.to_string(),
                    revision: snapshot.members.len(),
                })
                .collect()
        },
        |old, next| !Arc::ptr_eq(&old.members, &next.members),
    );
    let original = subscription.snapshot.clone();
    let mut current = (*baseline).clone();
    for version in 2..=10 {
        current.version = version;
        current.profiles = Arc::new(Vec::new());
        sender.send_replace(Arc::new(current.clone()));
        let update = subscription.stream.next().await.unwrap();
        assert_eq!(update.diffs.len(), 0);
        assert_eq!(update.version, version);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    current.version = 11;
    current.members = Arc::new(Vec::new());
    sender.send_replace(Arc::new(current.clone()));
    let update = subscription.stream.next().await.unwrap();
    assert_eq!(update.diffs.len(), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(Arc::ptr_eq(&original[0], &subscription.snapshot[0]));
    current.version = 15;
    sender.send_replace(Arc::new(current.clone()));
    let update = subscription.stream.next().await.unwrap();
    assert!(matches!(&update.diffs[..], [VectorDiff::Reset { values }] if values.len() == 10_000));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    current.version = 16;
    current.generation = Generation::for_test(2);
    sender.send_replace(Arc::new(current));
    assert!(matches!(
        &subscription.stream.next().await.unwrap().diffs[..],
        [VectorDiff::Reset { .. }]
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    drop(sender);
    assert!(subscription.stream.next().await.is_none());
}

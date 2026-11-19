//! Deterministic memoization regressions; reading and access remain the evidence owners.

use super::*;
use crate::{NotificationSettings, ServerReadPoint};

#[test]
fn redacted_public_image_does_not_poison_unchanged_computed_policy_zero() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            session
                .notifications()
                .set_settings(NotificationSettings {
                    enabled: false,
                    ..NotificationSettings::default()
                })
                .await
                .unwrap();
            let resources = session.inner.resources.borrow().clone();
            let reading = session.inner.reading.image.borrow().clone();
            let channel = Uuid::from_u128(2);
            let (previous, values) = build(&session, &resources, &reading, &BTreeMap::new()).await;
            let original = Arc::clone(&values[&channel]);
            assert_eq!(original.mention_count, UnreadCount::Exact(0));
            publish(&session.inner, session.generation(), true, values);
            crate::badges::restrict(&session.inner, &[channel], std::iter::empty());
            assert_eq!(
                session.inner.badges.image.borrow().channels[&channel].mention_count,
                UnreadCount::AtLeast(0)
            );
            // Quarantine and recovery can coalesce into the same final dependencies. The temporary
            // public redaction is not a replacement for the previously computed value.
            let (_, restored) = build(&session, &resources, &reading, &previous).await;
            assert!(Arc::ptr_eq(&original, &restored[&channel]));
            assert_eq!(restored[&channel].mention_count, UnreadCount::Exact(0));
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn unresolved_owner_evidence_invalidates_only_its_channel_despite_equal_read_state() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            let mut resources = (**session.inner.resources.borrow()).clone();
            let affected = Uuid::from_u128(2);
            let other = Uuid::from_u128(9);
            Arc::make_mut(&mut resources.channels).push(crate::Channel {
                id: Some(other.to_string()),
                ..crate::Channel::default()
            });
            let reading = session.inner.reading.image.borrow().clone();
            let (previous, _) = build(&session, &resources, &reading, &BTreeMap::new()).await;
            session.inner.reading.unresolved.lock().await.insert(
                affected,
                ServerReadPoint {
                    message_id: Uuid::from_u128(42),
                    recorded_at: None,
                },
            );
            let (next, _) = build(&session, &resources, &reading, &previous).await;
            assert_eq!(
                previous[&affected].input.reading,
                next[&affected].input.reading
            );
            assert!(previous[&affected].input != next[&affected].input);
            assert!(previous[&other].input == next[&other].input);
        })
        .unwrap();
    client.shutdown().unwrap();
}

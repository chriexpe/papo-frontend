//! Effective cached-history access across time and accounts: a denial survives restart, a page in
//! flight across a denial is never reused, and one account's denial leaves another untouched.
//! Fixture REST denial is scripted; this is not the Go permission engine.

use super::{
    CHANNEL, Fixture, Result, channel, online,
    permission::{DENIED, account, prepare, profile, restricted, same, warm},
    wait_event,
};
use papo_api::HistoryQuery;
use papo_sdk::{HistoryClosedReason, HistoryUpdate};
use papo_test_server::Config;

#[test]
fn denial_survives_restart_and_regrant_never_reloads_the_old_set() -> Result {
    let mut fixture = Fixture::new(Config::default())?;
    let account_id = fixture.run(async {
        let session = account(&fixture, "access-restart").await?;
        prepare(&fixture, &[(&session, "access-restart")]).await?;
        let history = session.history(channel()).await?;
        let newest = history.page(HistoryQuery::default()).await?.messages[0].clone();
        // Read up to the head, so the restarted reconcile has no unread channel to fetch
        // and any message after the regrant can only come from the old cache set.
        fixture
            .server()
            .set_read_mark(
                &session.account().user().to_string(),
                CHANNEL,
                newest.id.as_deref().ok_or("missing id")?,
                newest.created_at.as_deref().ok_or("missing time")?,
            )
            .await?;
        let mut events = session.subscribe_events();
        fixture
            .server()
            .set_channel(restricted("changed", false))
            .await?;
        wait_event(&mut events, "channel_update").await?;
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    fixture.reopen()?;
    fixture.run(async {
        // Regranted while the client was closed; the cached old set must not come back.
        fixture
            .server()
            .set_channel(restricted("regranted", true))
            .await?;
        let session = fixture.client().handle().session(account_id)?;
        online(&session).await?;
        let fresh = session.history(channel()).await?;
        assert!(fresh.subscribe().await?.snapshot.messages.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn denying_one_account_leaves_another_account_on_the_same_server_readable() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let denied = account(&fixture, "access-a").await?;
        let kept = account(&fixture, "access-b").await?;
        prepare(&fixture, &[(&denied, "access-a"), (&kept, "access-b")]).await?;
        let denied_history = warm(&denied).await?;
        let kept_history = warm(&kept).await?;
        let kept_subscription = kept_history.subscribe().await?;
        let mut events = denied.subscribe_events();
        fixture
            .server()
            .set_profile(profile(&denied, "access-a", false))
            .await?;
        wait_event(&mut events, "role_remove").await?;
        let page = denied_history.page(HistoryQuery::default()).await;
        assert!(matches!(&page, Err(error) if same(error, &DENIED)));
        assert!(!matches!(
            *kept_subscription.updates.borrow(),
            HistoryUpdate::Closed(_)
        ));
        assert_eq!(
            kept_history
                .page(HistoryQuery::default())
                .await?
                .messages
                .len(),
            12
        );
        assert_eq!(
            kept.history(channel())
                .await?
                .subscribe()
                .await?
                .snapshot
                .messages
                .len(),
            12
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn page_in_flight_across_a_denial_is_neither_published_nor_reused_after_regrant() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let session = account(&fixture, "access-inflight").await?;
        prepare(&fixture, &[(&session, "access-inflight")]).await?;
        let history = session.history(channel()).await?;
        let subscription = history.subscribe().await?;
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{CHANNEL}/messages"))
            .await?;
        let pending = {
            let history = history.clone();
            tokio::spawn(async move { history.page(HistoryQuery::default()).await })
        };
        hold.wait_started().await?;
        let mut events = session.subscribe_events();
        fixture
            .server()
            .set_channel(restricted("changed", false))
            .await?;
        wait_event(&mut events, "channel_update").await?;
        hold.release();
        let late = pending.await?;
        assert!(matches!(&late, Err(error) if same(error, &DENIED)));
        assert!(matches!(
            *subscription.updates.borrow(),
            HistoryUpdate::Closed(HistoryClosedReason::PermissionDenied)
        ));

        // The late page never reached the regranted incarnation's cache set. The regrant
        // arrives over the gateway, so no reconnect refresh refetches the channel.
        fixture
            .server()
            .set_channel(restricted("regranted", true))
            .await?;
        wait_event(&mut events, "channel_update").await?;
        let regranted = session.history(channel()).await?;
        assert!(regranted.subscribe().await?.snapshot.messages.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

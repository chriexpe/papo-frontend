//! Every defect is rejected by each producer that fetches it, naming the same group,
//! without wiping the history cache, without replacing the last-good image and without
//! a reconnect loop.

use super::{
    Account, BACKOFF_INITIAL, Result, SEEDED_MESSAGES, defects, digest, online_at, wait_for,
};
use papo_sdk::{CacheResource, ConnectionState, ConnectionStatus, RefreshFault, RefreshStatus};

/// Checks one rejection: the image keeps its last-good content, cached history survives,
/// and the diagnostics name `group`.
async fn assert_retained(
    account: &Account<'_>,
    case: &str,
    before: &[String],
    rejected_before: u64,
    group: CacheResource,
) -> Result {
    assert_eq!(
        digest(&account.snapshot()),
        before,
        "{case}: image replaced"
    );
    assert_eq!(
        account.cached_messages().await?,
        SEEDED_MESSAGES,
        "{case}: history cache lost"
    );
    let diagnostics = account.session.resource_admission();
    assert_eq!(
        diagnostics.rejected_replacements,
        rejected_before + 1,
        "{case}: rejection count"
    );
    assert_eq!(
        diagnostics.last_rejection.map(|rejection| rejection.group),
        Some(group),
        "{case}: rejected group"
    );
    Ok(())
}

/// Waits for the backoff that follows a rejected generation, checks retention inside that
/// window (before any new connection can refetch), then checks that the very next
/// generation reaches Online.
async fn assert_one_backoff_then_online(
    account: &Account<'_>,
    case: &str,
    after: ConnectionStatus,
    before: &[String],
    rejected_before: u64,
    group: CacheResource,
) -> Result {
    let mut status = account.session.subscribe_connection();
    let backoff = wait_for(&mut status, |status| {
        matches!(status.state, ConnectionState::Backoff { .. })
            && status.generation.get() > after.generation.get()
    })
    .await?;
    assert_retained(account, case, before, rejected_before, group).await?;
    let online = online_at(&account.session, |status| {
        status.generation.get() >= backoff.generation.get()
    })
    .await?;
    assert_eq!(
        online.generation, backoff.generation,
        "{case}: more than one reconnect"
    );
    assert_eq!(
        digest(&account.snapshot()),
        before,
        "{case}: recovery image"
    );
    assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
    Ok(())
}

#[test]
fn reconnect_rejects_each_defect_and_recovers_with_one_backoff() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-reconnect").await?;
        for defect in defects::all(&account) {
            let before = digest(&account.snapshot());
            let rejected = account.session.resource_admission().rejected_replacements;
            let status = account.status();
            defects::script(&account, defect.reconnect).await?;
            account.session.reconnect()?;
            assert_one_backoff_then_online(
                &account,
                defect.name,
                status,
                &before,
                rejected,
                defect.group,
            )
            .await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn gateway_rejects_each_defect_and_resyncs_with_one_backoff() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-gateway").await?;
        for defect in defects::all(&account) {
            let Some((event, replies)) = defect.gateway else {
                continue;
            };
            let before = digest(&account.snapshot());
            let rejected = account.session.resource_admission().rejected_replacements;
            let status = account.status();
            defects::script(&account, replies).await?;
            account.server().emit_event(event).await?;
            assert_one_backoff_then_online(
                &account,
                defect.name,
                status,
                &before,
                rejected,
                defect.group,
            )
            .await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn operation_refresh_rejects_each_defect_as_a_warning_without_reconnecting() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-operation").await?;
        for defect in defects::all(&account) {
            let Some((scope, replies)) = defect.operation else {
                continue;
            };
            let before = digest(&account.snapshot());
            let rejected = account.session.resource_admission().rejected_replacements;
            let status = account.status();
            defects::script(&account, replies).await?;
            let refreshed = account.session.operations().refresh_metadata(scope).await?;
            assert_eq!(
                refreshed.value,
                RefreshStatus::Warning(RefreshFault::Request),
                "{}",
                defect.name
            );
            assert_retained(&account, defect.name, &before, rejected, defect.group).await?;
            assert_eq!(
                account.status(),
                status,
                "{}: connection changed",
                defect.name
            );
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn repeated_rejections_grow_the_backoff_instead_of_retrying_at_the_initial_delay() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-backoff").await?;
        for _ in 0..3 {
            account
                .script("GET", "/channels", serde_json::json!({}))
                .await?;
        }
        let mut last = account.status().generation;
        let mut delays = Vec::new();
        let mut status = account.session.subscribe_connection();
        account.session.reconnect()?;
        while delays.len() < 3 {
            let backoff = wait_for(&mut status, |status| {
                matches!(status.state, ConnectionState::Backoff { .. })
                    && status.generation.get() > last.get()
            })
            .await?;
            let ConnectionState::Backoff { retry_at } = backoff.state else {
                unreachable!("waited for a backoff state");
            };
            delays.push(retry_at.saturating_duration_since(tokio::time::Instant::now()));
            last = backoff.generation;
        }
        // Without jitter the delays are initial/2, initial, 2 * initial; a counter reset
        // on every socket would keep all three at initial/2.
        assert!(delays[1] > BACKOFF_INITIAL * 3 / 4, "{delays:?}");
        assert!(delays[2] > BACKOFF_INITIAL * 3 / 2, "{delays:?}");
        let online = online_at(&account.session, |status| status.generation == last).await?;
        assert_eq!(online.generation, last);
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        assert_eq!(
            account.session.resource_admission().rejected_replacements,
            3
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_rejection_before_restart_leaves_a_cache_the_next_client_can_load() -> Result {
    let mut fixture = super::fixture()?;
    let (account, before) = fixture.run(async {
        let account = Account::start(&fixture, "adm-restart").await?;
        let before = digest(&account.snapshot());
        // The second reply is consumed by the restarted client's first connection.
        for _ in 0..2 {
            account
                .script("GET", "/channels", serde_json::json!({"channels": null}))
                .await?;
        }
        let status = account.status();
        let mut connection = account.session.subscribe_connection();
        account.session.reconnect()?;
        wait_for(&mut connection, |current| {
            matches!(current.state, ConnectionState::Backoff { .. })
                && current.generation.get() > status.generation.get()
        })
        .await?;
        Ok::<_, Box<dyn std::error::Error>>((account.session.account(), before))
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        let restarted = Account {
            cache: papo_sdk::DataRoot::new(fixture.profile.path()).account_cache(account),
            fixture: &fixture,
            session,
        };
        let mut connection = restarted.session.subscribe_connection();
        let backoff = wait_for(&mut connection, |current| {
            matches!(current.state, ConnectionState::Backoff { .. })
        })
        .await?;
        // The rejected fetch left the cold-start image loaded from the cache in place.
        assert_retained(
            &restarted,
            "after restart",
            &before,
            0,
            CacheResource::Channels,
        )
        .await?;
        let online = online_at(&restarted.session, |current| {
            current.generation.get() >= backoff.generation.get()
        })
        .await?;
        assert_eq!(online.generation, backoff.generation);
        assert_eq!(digest(&restarted.snapshot()), before);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

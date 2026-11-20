//! Recovery on the real pump after resource persistence switches its visible revision.

use super::{Account, CHANNEL, Result, SEEDED_MESSAGES, fixture, online_at};
use papo_api::{
    Uuid,
    gateway::{ChannelUpdateEvent, KnownEvent},
};
use papo_sdk::{HistoryError, RefreshStatus, operations::MetadataScope};
use serde_json::json;

async fn database_statement(account: &Account<'_>, sql: &'static str) -> Result {
    let path = account.cache.clone();
    tokio::task::spawn_blocking(move || rusqlite::Connection::open(path)?.execute_batch(sql))
        .await??;
    Ok(())
}

async fn name_next_channel(account: &Account<'_>, name: &str) -> Result {
    account
        .script(
            "GET",
            "/channels",
            json!({"channels":[{
                "id":CHANNEL,"type":"text","position":0,"name":name,"permissions":[]
            }]}),
        )
        .await
}

async fn confirmed_partial_write(account: &Account<'_>, id: Uuid) -> Result {
    name_next_channel(account, "recovered").await?;
    let refreshed = account
        .session
        .operations()
        .refresh_metadata(MetadataScope::Channels)
        .await?;
    assert_eq!(refreshed.value, RefreshStatus::Updated);
    assert_eq!(
        account
            .snapshot()
            .channels
            .get(id)
            .and_then(|row| row.name.as_deref()),
        Some("recovered")
    );
    assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
    Ok(())
}

async fn history_set(account: &Account<'_>, id: Uuid) -> Result<i64> {
    let path = account.cache.clone();
    Ok(tokio::task::spawn_blocking(move || {
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?
            .query_row(
                "SELECT current_set FROM history_channels WHERE channel_id=?",
                [id.as_bytes()],
                |row| row.get(0),
            )
    })
    .await??)
}

#[test]
fn failed_cleanup_preserves_the_switched_image_and_next_generation_can_write_again() -> Result {
    const FAIL_PRUNE: &str = "CREATE TRIGGER fail_cleanup BEFORE DELETE ON channels
        BEGIN SELECT RAISE(ABORT, 'fixture'); END";
    let fixture = fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "writer-recovery").await?;
        let before = account.status().generation;
        let id = Uuid::parse_str(CHANNEL)?;
        let mut reconnect = account.server().hold_next_rest("GET", "/server").await?;
        database_statement(&account, FAIL_PRUNE).await?;
        name_next_channel(&account, "switched").await?;
        let update = ChannelUpdateEvent {
            channel_id: CHANNEL.into(),
            name: "switched".into(),
            ..Default::default()
        };
        account
            .server()
            .emit_event(KnownEvent::ChannelUpdate(Box::new(update)))
            .await?;

        // The replacement has opened the same database and loaded the switched image.
        // No authoritative reconnect response can have overwritten the cache yet.
        reconnect.wait_started().await?;
        let loaded = account.snapshot();
        assert!(loaded.generation > before);
        let name = loaded.channels.get(id).and_then(|row| row.name.as_deref());
        assert_eq!(name, Some("switched"));
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        database_statement(&account, "DROP TRIGGER fail_cleanup").await?;
        reconnect.release();
        online_at(&account.session, |status| {
            status.generation == loaded.generation
        })
        .await?;
        confirmed_partial_write(&account, id).await
    })??;
    fixture.finish()
}

#[test]
fn history_install_failure_seals_the_old_handle_until_the_switched_revision_is_reconciled() -> Result
{
    let fixture = fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "install-recovery").await?;
        let id = Uuid::parse_str(CHANNEL)?;
        let history = account.session.history(id).await?;
        let retained = history_set(&account, id).await?;
        let messages = format!("/channels/{CHANNEL}/messages");
        let mut overlap = account.server().hold_next_rest("GET", &messages).await?;
        account
            .server()
            .queue_operation_response("GET", &messages, 503, json!({}))
            .await?;
        name_next_channel(&account, "committed before overlap").await?;
        account.session.reconnect()?;
        overlap.wait_started().await?;
        let failed = account.status().generation;
        let mut replacement = account.server().hold_next_rest("GET", "/server").await?;
        overlap.release();

        replacement.wait_started().await?;
        let loaded = account.snapshot();
        assert!(loaded.generation > failed);
        assert!(!loaded.channel_census_complete);
        let name = loaded.channels.get(id).and_then(|row| row.name.as_deref());
        assert_eq!(name, Some("committed before overlap"));
        assert!(matches!(
            history.subscribe().await,
            Err(HistoryError::AccessUnknown)
        ));
        assert_eq!(history_set(&account, id).await?, retained);
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        replacement.release();
        online_at(&account.session, |status| {
            status.generation == loaded.generation
        })
        .await?;
        history.subscribe().await?;
        assert_eq!(history_set(&account, id).await?, retained);
        confirmed_partial_write(&account, id).await
    })??;
    fixture.finish()
}

//! Unchanged resolved server marks must not refetch history or write the FULL read-point database.

use super::{CHANNEL, FIRST, Fixture, Result, channel, config, signed_in};

#[test]
fn unchanged_mark_on_metadata_refresh_needs_no_history_request_or_durable_update() -> Result {
    let fixture = Fixture::new(config())?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        fixture.server().set_read_mark(
            &session.account().user().to_string(), CHANNEL, FIRST, "2026-02-01T00:00:00Z",
        ).await?;
        let previous_generation = session.generation();
        let mut connection = session.subscribe_connection();
        session.reconnect()?;
        super::fixture::wait_for(&mut connection, |status| status.state == papo_sdk::ConnectionState::Online
            && status.generation > previous_generation).await?;
        let state = session.reading_state(channel()).ok_or("missing reading state")?;
        assert_eq!(state.uploaded_point.map(|point| point.message_id.to_string()).as_deref(), Some(FIRST));
        let database = rusqlite::Connection::open(fixture.profile.path().join("state.sqlite"))?;
        database.execute_batch("CREATE TRIGGER refuse_redundant_mark BEFORE UPDATE OF uploaded_point ON read_points BEGIN SELECT RAISE(ABORT, 'unexpected uploaded-point write'); END;")?;
        let generation = session.generation();
        let before = fixture.server().read_request_counts().await;
        let mut events = session.subscribe_events();
        for _ in 0..3 {
            assert!(fixture.server().rename_channel(CHANNEL, "metadata-only").await);
            loop {
                if events.next().await?.value.kind() == "channel_update" { break; }
            }
        }
        assert_eq!(session.generation(), generation);
        let after = fixture.server().read_request_counts().await;
        assert_eq!(after.history_pages, before.history_pages);
        assert_eq!(after.uploads, before.uploads);
        database.execute_batch("DROP TRIGGER refuse_redundant_mark;")?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

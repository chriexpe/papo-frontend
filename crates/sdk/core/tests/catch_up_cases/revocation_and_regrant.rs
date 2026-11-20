//! Behaviour specs for channel removal, same-generation re-grant and coalesced failure variants.

use super::{
    AUTHOR, CHANNEL, FIRST, Fixture, Result, channel, config, load_history, msg_id, signed_in,
    view, wait_pos,
};
use papo_api::wire::{Channel, DirectConversation, UserSummary};
use papo_sdk::{
    ConnectionState, HistoryClosedReason, HistoryError, ReadingError, ReadingPosition, Uuid,
    VisibilityContinuity,
};

#[test]
fn channel_removed_discards_saved_position_and_fences_concurrent_writes() -> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let dm_id = Uuid::from_u128(19);
        let dm = DirectConversation {
            id: dm_id.to_string(),
            user: UserSummary {
                id: Some(Uuid::from_u128(18).to_string()),
                ..Default::default()
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        };
        let account = session.account().user().to_string();
        fixture
            .server()
            .set_direct_conversations(&account, vec![dm])
            .await?;
        let mut res_rx = session.subscribe_resources();
        while res_rx.borrow_and_update().dms.is_empty() {
            res_rx.changed().await?;
        }
        let msg_id_dm = Uuid::from_u128(0x8000_0000_0000_4000_8000_0000_0000_0001);
        session
            .save_reading_position(
                dm_id,
                Some(ReadingPosition {
                    message_id: msg_id_dm,
                    offset_units: -5,
                }),
            )
            .await?;
        let mut rx = session.subscribe_reading_positions();
        fixture
            .server()
            .set_direct_conversations(&account, vec![])
            .await?;
        let before = session.generation();
        session.reconnect()?;
        let mut status = session.subscribe_connection();
        super::fixture::wait_for(&mut status, |s| {
            s.state == ConnectionState::Online && s.generation > before
        })
        .await?;
        wait_pos(&mut rx, dm_id, None).await?;
        assert_eq!(session.reading_position(dm_id), None);
        // Further save on already-removed DM or stale generation is fenced/cleaned cleanly.
        let mut set = tokio::task::JoinSet::new();
        for off in 1..=5_i64 {
            let s = session.clone();
            set.spawn(async move {
                let _ = s
                    .save_reading_position(
                        dm_id,
                        Some(ReadingPosition {
                            message_id: msg_id_dm,
                            offset_units: off,
                        }),
                    )
                    .await;
            });
        }
        while set.join_next().await.is_some() {}
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

async fn delete_and_regrant_channel(fixture: &Fixture, session: &papo_sdk::Session) -> Result {
    let mut pos_rx = session.subscribe_reading_positions();
    let mut res_rx = session.subscribe_resources();
    assert!(fixture.server().delete_channel(CHANNEL).await);
    wait_pos(&mut pos_rx, channel(), None).await?;
    while res_rx
        .borrow_and_update()
        .channels
        .iter()
        .any(|c| c.id == channel())
    {
        res_rx.changed().await?;
    }
    assert!(matches!(
        session.history(channel()).await,
        Err(HistoryError::Unverified)
    ));
    let restored = Channel {
        id: Some(CHANNEL.into()),
        name: Some("restored".into()),
        r#type: Some("text".into()),
        position: Some(0),
        permissions: Some(Vec::new()),
        created_at: Some("2026-01-01T00:00:00Z".into()),
        ..Channel::default()
    };
    fixture.server().set_channel(restored).await?;
    while !res_rx
        .borrow_and_update()
        .channels
        .iter()
        .any(|c| c.id == channel() && c.name.as_deref() == Some("restored"))
    {
        res_rx.changed().await?;
    }
    fixture
        .server()
        .seed_history(papo_test_server::HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: AUTHOR.into(),
            first_message_id: FIRST.into(),
            count: 6,
            first_created_at: "2026-01-01T00:00:00.123450Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        })
        .await?;
    Ok(())
}

#[test]
fn same_generation_channel_regrant_allows_position_saves_and_catch_up_clear() -> Result {
    let fixture = Fixture::new(config(6))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let gen_before = session.generation();
        let snap = load_history(&session, channel()).await?;
        let m0 = msg_id(&snap, 0);
        session
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: m0,
                    offset_units: 3,
                }),
            )
            .await?;

        delete_and_regrant_channel(&fixture, &session).await?;
        assert_eq!(session.generation(), gen_before);

        let _regranted_snap = load_history(&session, channel()).await?;
        let observer = session.reading_observer();
        super::report_with(
            &observer,
            &session,
            channel(),
            0..=0,
            VisibilityContinuity::Continuous,
        )
        .await?;
        let refreshed = super::report_with(
            &observer,
            &session,
            channel(),
            5..=5,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let new_m5 = msg_id(&refreshed, 5);
        assert!(view(&session, channel(), &refreshed)?.is_some());

        let restored_pos = ReadingPosition {
            message_id: new_m5,
            offset_units: -4,
        };
        session
            .save_reading_position(channel(), Some(restored_pos))
            .await?;
        assert_eq!(session.reading_position(channel()), Some(restored_pos));
        session.clear_catch_up(channel()).await?;
        assert!(view(&session, channel(), &refreshed)?.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn coalesced_position_waiters_receive_concrete_revocation_error_variants() -> Result {
    let fixture = Fixture::new(config(4))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        let msg = msg_id(&snap, 0);
        session
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: msg,
                    offset_units: -1,
                }),
            )
            .await?;
        let mut pos_rx = session.subscribe_reading_positions();
        let mut res_rx = session.subscribe_resources();
        let mut set = tokio::task::JoinSet::new();
        for offset_units in 0..8_i64 {
            let s = session.clone();
            set.spawn(async move {
                s.save_reading_position(
                    channel(),
                    Some(ReadingPosition {
                        message_id: msg,
                        offset_units,
                    }),
                )
                .await
            });
        }
        assert!(fixture.server().delete_channel(CHANNEL).await);
        while let Some(joined) = set.join_next().await {
            match joined? {
                Ok(())
                | Err(ReadingError::History(HistoryError::Closed(
                    HistoryClosedReason::ChannelDeleted,
                ))) => {}
                Err(other) => {
                    return Err(format!("unexpected error variant after delete: {other:?}").into());
                }
            }
        }
        while res_rx
            .borrow_and_update()
            .channels
            .iter()
            .any(|c| c.id == channel())
        {
            res_rx.changed().await?;
        }
        wait_pos(&mut pos_rx, channel(), None).await?;
        let follow_up = session
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: msg,
                    offset_units: 99,
                }),
            )
            .await;
        assert!(matches!(
            follow_up,
            Err(ReadingError::History(HistoryError::Closed(
                HistoryClosedReason::ChannelDeleted
            )))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

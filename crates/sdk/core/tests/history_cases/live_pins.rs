//! The pin list published with each history snapshot: newest pin first once fetched, and kept
//! current by live pin events. (An unread channel's list is already fetched on connect.)

use super::{AT, FIRST, Fixture, Result, channel, config, query, signed_in};
use papo_api::{HistoryOrder, Uuid};
use papo_sdk::{HistoryHandle, HistoryUpdate};

fn nth(n: u128) -> String {
    Uuid::from_u128(Uuid::parse_str(FIRST).unwrap().as_u128() + n).to_string()
}

/// Waits for a snapshot whose pins are `wanted`, newest pin first.
async fn pins(history: &HistoryHandle, wanted: &[String]) -> Result {
    let mut subscription = history.subscribe().await?;
    let matches = |snapshot: &papo_sdk::HistorySnapshot| {
        snapshot.pins.as_ref().is_some_and(|pins| {
            pins.iter()
                .map(|pin| pin.id.clone().unwrap_or_default())
                .eq(wanted.iter().cloned())
        })
    };
    if matches(&subscription.snapshot) {
        return Ok(());
    }
    loop {
        subscription.updates.changed().await?;
        let update = subscription.updates.borrow_and_update().clone();
        if let HistoryUpdate::Reset(snapshot) = update
            && matches(&snapshot)
        {
            return Ok(());
        }
    }
}

#[test]
fn pins_are_published_once_fetched_newest_first_and_follow_live_events() -> Result {
    let fixture = Fixture::new(config(12))?;
    fixture.run(async {
        let session = signed_in(&fixture, "live-pins").await?;
        let server = fixture.server();
        server.set_message_pin(&nth(5), Some(AT)).await?;
        server
            .set_message_pin(&nth(3), Some("2026-01-01T01:00:01.000000+01:00"))
            .await?;
        let history = session.history(channel()).await?;
        history.page(query(HistoryOrder::Desc)).await?;
        let fetched = history.pins().await?.value;
        assert_eq!(
            fetched
                .iter()
                .map(|pin| pin.id.clone().unwrap())
                .collect::<Vec<_>>(),
            [nth(3), nth(5)]
        );
        pins(&history, &[nth(3), nth(5)]).await?;
        server
            .set_message_pin(&nth(7), Some("2026-01-01T01:00:02.000000+01:00"))
            .await?;
        pins(&history, &[nth(7), nth(3), nth(5)]).await?;
        server.set_message_pin(&nth(3), None).await?;
        pins(&history, &[nth(7), nth(5)]).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

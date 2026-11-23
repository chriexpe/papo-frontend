//! A channel's pins stream: fetched on request, then following live pin events of held messages.

use futures_util::StreamExt;
use papo_sdk_ui::{ChannelPins, PinsEvent, Timeline};

use super::{Fixture, Result, channel, config};

fn ids(pins: Option<&imbl::Vector<std::sync::Arc<papo_sdk::Message>>>) -> Option<Vec<String>> {
    pins.map(|pins| pins.iter().filter_map(|pin| pin.id.clone()).collect())
}

#[test]
fn pins_stream_starts_once_fetched_and_follows_pin_events() -> Result {
    let fixture = Fixture::new(6, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-pins").await?;
        // Live pin events reach the messages history holds: the open timeline's.
        let _timeline = Timeline::live(session.clone(), channel(), config()).await?;
        let pins = ChannelPins::open(&session, channel()).await?;
        let mut subscription = pins.subscribe().await?;
        let first = fixture.messages[1].id.clone().ok_or("seed without ID")?;
        let second = fixture.messages[3].id.clone().ok_or("seed without ID")?;
        fixture
            .server()
            .set_message_pin(&first, Some("2026-01-01T00:00:00.000001Z"))
            .await?;
        pins.refresh().await?;
        let mut current = ids(subscription.pins.as_ref());
        while current.as_deref() != Some(std::slice::from_ref(&first)) {
            let Some(PinsEvent::Update(pins)) = subscription.stream.next().await else {
                return Err("the pins stream ended".into());
            };
            current = ids(pins.as_ref());
        }
        fixture
            .server()
            .set_message_pin(&second, Some("2026-01-01T00:00:00.000002Z"))
            .await?;
        while current.as_deref() != Some([second.clone(), first.clone()].as_slice()) {
            let Some(PinsEvent::Update(pins)) = subscription.stream.next().await else {
                return Err("the pins stream ended".into());
            };
            current = ids(pins.as_ref());
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

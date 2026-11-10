//! Pin-read cancellation at reconnect and account revocation; socket clocks are not paused.

use super::{Fixture, Result, channel, config, online, signed_in};
use papo_sdk::{ClientError, HistoryClosedReason, HistoryError};
use std::time::Duration;

#[test]
fn reconnect_cancels_held_pin_read_before_its_response_is_released() -> Result {
    let fixture = Fixture::new(config(12))?;
    fixture.run(async {
        let session = signed_in(&fixture, "pin-reconnect").await?;
        let history = session.history(channel()).await?;
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{}/pinned", channel()))
            .await?;
        let pending = tokio::spawn(async move { history.pins().await });
        hold.wait_started().await?;
        session.reconnect()?;
        let result = tokio::time::timeout(Duration::from_secs(2), pending).await??;
        assert!(matches!(
            result,
            Err(HistoryError::Session(ClientError::Superseded))
        ));
        hold.release();
        online(&session).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn logout_cancels_and_joins_held_pin_read_before_its_response_is_released() -> Result {
    let fixture = Fixture::new(config(12))?;
    fixture.run(async {
        let session = signed_in(&fixture, "pin-logout").await?;
        let history = session.history(channel()).await?;
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{}/pinned", channel()))
            .await?;
        let pending = tokio::spawn(async move { history.pins().await });
        hold.wait_started().await?;
        tokio::time::timeout(Duration::from_secs(2), session.auth().logout()).await??;
        let result = tokio::time::timeout(Duration::from_secs(2), pending).await??;
        assert!(matches!(
            result,
            Err(HistoryError::Closed(HistoryClosedReason::AccountClosed))
        ));
        hold.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

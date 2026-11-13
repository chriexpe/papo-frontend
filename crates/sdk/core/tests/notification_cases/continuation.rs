//! Bounded canonical evidence lookup must preserve catch-up progress across wakes.
use super::*;

#[test]
fn buried_mention_resumes_without_advancing_inbox_past_unresolved_message() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let client = f.client().handle();
        client.push_wake(first.account(), deadline()).await?;
        let at = timestamp();
        let record = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        let message_id = record.message_id.clone().ok_or("message")?;
        f.server()
            .set_notifications(first.account().user(), vec![record])
            .await?;
        let messages = (1..=501)
            .map(|index| papo_api::wire::Message {
                id: Some(Uuid::from_u128(20000 + index).to_string()),
                channel_id: Some(channel.to_string()),
                created_at: Some(at.clone()),
                ..Default::default()
            })
            .collect();
        f.server()
            .seed_messages(&channel.to_string(), messages)
            .await?;
        let report = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(report.accounts[0].outcome, BackgroundOutcome::More);
        assert!(report.deliveries.is_empty());
        let report = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(report.deliveries.len(), 1);
        assert_eq!(report.deliveries[0].target.message.to_string(), message_id);
        assert!(
            client
                .push_wake(first.account(), deadline())
                .await?
                .deliveries
                .is_empty()
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

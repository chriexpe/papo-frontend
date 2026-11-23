//! A reaction's outcome against the loopback backend: confirmed membership reaches history, a
//! removal that finds nothing is done, and an unresolved reaction never holds its channel.

use super::*;

fn reaction(target: Uuid, add: bool) -> OperationKind {
    OperationKind::Reaction {
        target: MessageTarget::Remote(target),
        emoji_id: None,
        unicode: Some("👍".into()),
        add,
    }
}

/// Waits until history's held `message` has (or no longer has) this account's 👍.
async fn mine(history: &papo_sdk::HistoryHandle, message: Uuid, wanted: bool) -> Result {
    let mut subscription = history.subscribe().await?;
    let holds = |snapshot: &papo_sdk::HistorySnapshot| {
        snapshot.messages.iter().any(|held| {
            held.id.as_deref() == Some(message.to_string().as_str())
                && held.user_reactions.as_ref().is_some_and(|own| {
                    own.iter()
                        .any(|reaction| reaction.unicode.as_deref() == Some("👍"))
                }) == wanted
        })
    };
    if holds(&subscription.snapshot) {
        return Ok(());
    }
    loop {
        subscription.updates.changed().await?;
        let update = subscription.updates.borrow_and_update().clone();
        if let papo_sdk::HistoryUpdate::Reset(snapshot) = update
            && holds(&snapshot)
        {
            return Ok(());
        }
    }
}

#[test]
fn a_confirmed_reaction_settles_this_accounts_membership_into_history() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let history = session.history(channel).await?;
        history.page(papo_api::HistoryQuery::default()).await?;
        let queue = session.outgoing();
        queue
            .enqueue(
                channel,
                id(1),
                OperationKind::Message(payload()),
                UnixMicros(0),
            )
            .await?;
        let message = state(&session, id(1), OperationState::Sent)
            .await?
            .message_id
            .ok_or("the receipt has no message ID")?;
        queue
            .enqueue(channel, id(2), reaction(message, true), UnixMicros(1))
            .await?;
        state(&session, id(2), OperationState::Sent).await?;
        mine(&history, message, true).await?;
        queue
            .enqueue(channel, id(3), reaction(message, false), UnixMicros(2))
            .await?;
        state(&session, id(3), OperationState::Sent).await?;
        mine(&history, message, false).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn removing_a_reaction_that_is_not_there_is_done() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        // The loopback backend answers 404 for a message it does not hold.
        queue
            .enqueue(channel, id(1), reaction(id(90), false), UnixMicros(0))
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_refused_reaction_holds_no_one_and_a_later_one_supersedes_it() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        queue
            .enqueue(channel, id(1), reaction(id(90), true), UnixMicros(0))
            .await?;
        state(&session, id(1), OperationState::Failed).await?;
        queue
            .enqueue(
                channel,
                id(2),
                OperationKind::Message(payload()),
                UnixMicros(1),
            )
            .await?;
        state(&session, id(2), OperationState::Sent).await?;
        queue
            .enqueue(channel, id(3), reaction(id(90), false), UnixMicros(2))
            .await?;
        state(&session, id(3), OperationState::Sent).await?;
        assert!(matches!(
            queue.retry(id(1), RetryDecision::DefiniteFailure).await,
            Err(OutgoingError::InvalidState)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

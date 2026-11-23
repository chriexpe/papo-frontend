//! Production pins and grouped reactor reads preserve history coverage and local read intent.

use super::{Result, is_problem, wait_sent};
use papo_sdk::operations::reactions::{ReactionEmoji, ReactionIntent};
use papo_sdk::{MessageTarget, Session, UnixMicros, Uuid};

pub async fn prove(owner: &Session, member: &Session, channel: Uuid, message: Uuid) -> Result {
    let history = owner.history(channel).await?;
    history.page(papo_api::HistoryQuery::default()).await?;
    let before = history.subscribe().await?.snapshot;
    let local_read = owner
        .reading_state(channel)
        .and_then(|state| state.local_point);
    let pin = owner
        .operations()
        .pin_message(channel, message)
        .await?
        .value;
    assert_eq!(
        pin.message_id.as_deref(),
        Some(message.to_string().as_str())
    );
    assert!(is_problem(
        &member.operations().pin_message(channel, message).await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    owner.operations().unpin_message(channel, message).await?;
    let after = history.subscribe().await?.snapshot;
    let coverage = |snapshot: &papo_sdk::HistorySnapshot| {
        snapshot
            .chunks
            .iter()
            .map(|chunk| {
                (
                    chunk.first.created_at.clone(),
                    chunk.first.id.clone(),
                    chunk.last.created_at.clone(),
                    chunk.last.id.clone(),
                    chunk.oldest_complete,
                    chunk.newest_complete,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(coverage(&before), coverage(&after));
    assert_eq!(
        owner
            .reading_state(channel)
            .and_then(|state| state.local_point),
        local_read
    );
    let local = Uuid::from_u128(13002);
    owner
        .reactions()
        .set(ReactionIntent {
            channel,
            message: MessageTarget::Remote(message),
            emoji: ReactionEmoji::Unicode("👍".into()),
            add: true,
            local,
            at: UnixMicros(0),
        })
        .await?;
    wait_sent(owner, local).await?;
    let page = owner
        .operations()
        .reaction_users(channel, message, papo_sdk::HistoryOrder::Asc, None)
        .await?
        .value;
    assert!(page.next.is_none());
    assert_eq!(page.groups.len(), 1);
    let group = &page.groups[0];
    assert_eq!(group.unicode.as_deref(), Some("👍"));
    assert_eq!(group.count, Some(1));
    assert!(group.users.as_ref().is_some_and(|users| users.len() == 1
        && users[0].user_id.as_deref() == Some(owner.account().user().to_string().as_str())));
    Ok(())
}

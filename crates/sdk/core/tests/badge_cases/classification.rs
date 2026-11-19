//! Held full-message evidence, unresolved historical triggers and coverage; never reconstructs send-time permissions.

use super::*;
use papo_api::wire::{Role, RolePermissions};

#[test]
fn direct_mention_beyond_preview_counts_while_visible_and_reading_clears_it() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-content").await?;
        let mut changes = session.subscribe_badges();
        session
            .notifications()
            .set_visible_channels([channel()].into_iter().collect())?;
        let full = format!(
            "{} @mention(<@{}>)",
            "x".repeat(600),
            session.account().user()
        );
        let direct = message(2, channel(), AUTHOR.parse()?, &full);
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![message(1, channel(), AUTHOR.parse()?, "anchor"), direct],
        )
        .await?;
        let unread = count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        assert_eq!(unread.has_unread, Some(true));
        assert!(!unread.direct_message);
        read_through(&session, channel(), message_id(2)).await?;
        let read = count(&mut changes, channel(), UnreadCount::Exact(0)).await?;
        assert_eq!(read.has_unread, Some(false));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn missing_reply_parent_arrival_and_deletion_reclassify_without_reply_revision() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-reply").await?;
        let mut changes = session.subscribe_badges();
        let parent = message(2, channel(), session.account().user(), "parent");
        let mut reply = message(3, channel(), AUTHOR.parse()?, "reply");
        reply.reply_to = Some(message_id(2).to_string());
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![
                message(1, channel(), AUTHOR.parse()?, "anchor"),
                reply.clone(),
            ],
        )
        .await?;
        count(&mut changes, channel(), UnreadCount::AtLeast(0)).await?;
        f.server().publish_message(parent).await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        assert!(
            f.server()
                .delete_message(&message_id(2).to_string())
                .await?
        );
        count(&mut changes, channel(), UnreadCount::AtLeast(0)).await?;
        let held = session
            .history(channel())
            .await?
            .subscribe()
            .await?
            .snapshot;
        let unchanged = held
            .messages
            .iter()
            .find(|value| value.id == reply.id)
            .ok_or("reply no longer held")?;
        assert_eq!(unchanged.content, reply.content);
        assert_eq!(unchanged.edited_at, reply.edited_at);
        assert_eq!(unchanged.reply_to, reply.reply_to);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn direct_positive_survives_unresolved_reply_and_everyone_branches() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-positive").await?;
        let mut changes = session.subscribe_badges();
        let mut direct = message(
            2,
            channel(),
            AUTHOR.parse()?,
            &format!("@everyone @mention(<@{}>)", session.account().user()),
        );
        direct.reply_to = Some(message_id(99).to_string());
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![message(1, channel(), AUTHOR.parse()?, "anchor"), direct],
        )
        .await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn everyone_stays_unknown_despite_current_author_grant_but_webhook_everyone_is_negative() -> Result
{
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-everyone").await?;
        let mut changes = session.subscribe_badges();
        f.server()
            .set_roles(vec![Role {
                id: Some(ROLE.into()),
                permissions: Some(RolePermissions {
                    everyone_message: Some(true),
                    ..RolePermissions::default()
                }),
                ..Role::default()
            }])
            .await;
        f.server()
            .set_profile(papo_sdk::UserProfile {
                id: Some(AUTHOR.into()),
                username: Some("author".into()),
                roles: Some(vec![papo_sdk::RoleSummary {
                    id: Some(ROLE.into()),
                    ..papo_sdk::RoleSummary::default()
                }]),
                ..papo_sdk::UserProfile::default()
            })
            .await?;
        session
            .operations()
            .refresh_metadata(papo_sdk::operations::MetadataScope::Roles)
            .await?;
        session
            .operations()
            .refresh_metadata(papo_sdk::operations::MetadataScope::Profile(
                AUTHOR.parse()?,
            ))
            .await?;
        let historical = message(2, channel(), AUTHOR.parse()?, "@everyone");
        let mut webhook = message(3, channel(), AUTHOR.parse()?, "@everyone");
        webhook.author_id = None;
        webhook.webhook_id = Some(Uuid::from_u128(600).to_string());
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![
                message(1, channel(), AUTHOR.parse()?, "anchor"),
                historical,
                webhook,
            ],
        )
        .await?;
        count(&mut changes, channel(), UnreadCount::AtLeast(0)).await?;
        f.server()
            .delete_message(&message_id(2).to_string())
            .await?;
        count(&mut changes, channel(), UnreadCount::Exact(0)).await?;
        let mut direct_webhook = message(
            4,
            channel(),
            AUTHOR.parse()?,
            &format!("@mention(<@{}>)", session.account().user()),
        );
        direct_webhook.author_id = None;
        direct_webhook.webhook_id = Some(Uuid::from_u128(600).to_string());
        f.server().publish_message(direct_webhook).await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn partial_history_is_a_lower_bound_until_read_boundary_to_head_is_covered() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-coverage").await?;
        let mut changes = session.subscribe_badges();
        let mention = format!("@mention(<@{}>)", session.account().user());
        f.server()
            .seed_messages(
                CHANNEL,
                (1..=12)
                    .map(|index| message(index, channel(), AUTHOR.parse().unwrap(), &mention))
                    .collect(),
            )
            .await?;
        let history = session.history(channel()).await?;
        let newest = history
            .page(HistoryQuery {
                limit: Some(3),
                ..HistoryQuery::default()
            })
            .await?;
        assert!(newest.has_more);
        count(&mut changes, channel(), UnreadCount::AtLeast(3)).await?;
        let mut cursor = newest.next_cursor;
        loop {
            let page = history
                .page(HistoryQuery {
                    cursor: cursor.as_ref(),
                    limit: Some(3),
                    ..HistoryQuery::default()
                })
                .await?;
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
        }
        read_through(&session, channel(), message_id(1)).await?;
        count(&mut changes, channel(), UnreadCount::Exact(11)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

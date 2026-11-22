//! Reply originals and this account's pending edits and deletions on timeline items, in live
//! and focused windows, over real loopback TCP.

use super::support::{Fixture, Result, channel, config, until};
use papo_sdk::{MessageTarget, OperationKind, OperationState, OutgoingMessage, UnixMicros, Uuid};
use papo_sdk_ui::{ReplyOriginal, Timeline, TimelineItem, TimelineSubscription};

fn local(n: u128) -> Uuid {
    Uuid::from_u128(0xd000_0000_0000_4000_8000_0000_0000_0100 + n)
}

type Image = tokio::sync::watch::Receiver<std::sync::Arc<papo_sdk::OutgoingSnapshot>>;

/// Queues text `n`, replying to `reply_to` when given.
async fn write(
    queue: &papo_sdk::OutgoingService,
    n: u128,
    reply_to: Option<MessageTarget>,
) -> Result {
    let content = if reply_to.is_some() {
        "synthetic-reply"
    } else {
        "synthetic"
    };
    queue
        .enqueue(
            channel(),
            local(n),
            OperationKind::Message(OutgoingMessage {
                content: content.into(),
                reply_to,
                ..Default::default()
            }),
            UnixMicros(n.try_into()?),
        )
        .await?;
    Ok(())
}

/// The backend ID of create `n`, once it is Sent.
async fn confirmed(image: &mut Image, n: u128) -> Result<Uuid> {
    loop {
        let sent = image
            .borrow_and_update()
            .operations
            .iter()
            .find(|op| op.id == local(n) && op.state == OperationState::Sent)
            .and_then(|op| op.message_id);
        if let Some(id) = sent {
            return Ok(id);
        }
        image.changed().await?;
    }
}

/// The message item `id`: its text and its pending operation's kind and state.
fn shown(s: &TimelineSubscription, id: Uuid) -> Option<(String, Option<(bool, OperationState)>)> {
    s.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id: current,
            message,
            pending,
            ..
        } if *current == id => Some((
            message.content.clone().unwrap_or_default(),
            pending.as_ref().map(|op| {
                (
                    matches!(op.kind.as_ref(), OperationKind::Edit { .. }),
                    op.state,
                )
            }),
        )),
        _ => None,
    })
}

fn echo_reply(s: &TimelineSubscription, id: Uuid) -> Option<ReplyOriginal> {
    s.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::LocalEcho {
            id: current, reply, ..
        } if *current == id => reply.clone(),
        _ => None,
    })
}

fn message_reply(s: &TimelineSubscription, id: Uuid) -> Option<ReplyOriginal> {
    s.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id: current, reply, ..
        } if *current == id => reply.clone(),
        _ => None,
    })
}

#[test]
fn edits_deletes_and_replies_ride_on_their_items_until_history_settles_them() -> Result {
    let f = Fixture::new(10, vec![])?;
    f.run(async {
        let session = f.signed_in("replies").await?;
        let queue = session.outgoing();
        let live = Timeline::live(session.clone(), channel(), config()).await?;
        let mut stream = live.subscribe()?;
        write(&queue, 1, None).await?;
        let mut image = queue.subscribe().await?;
        let original = confirmed(&mut image, 1).await?;
        until(&mut stream, |s| shown(s, original).is_some()).await?;
        let focused = Timeline::focused_at(session.clone(), channel(), original, config()).await?;
        let mut around = focused.subscribe()?;

        // A held reply is an echo whose original is history's; one to it is still unsent.
        let mut post = f.server().hold_next_rest("POST", "/messages").await?;
        write(&queue, 2, Some(MessageTarget::Remote(original))).await?;
        write(&queue, 3, Some(MessageTarget::Local(local(2)))).await?;
        post.wait_started().await?;
        until(&mut stream, |s| {
            matches!(echo_reply(s, local(2)), Some(ReplyOriginal::Held { id, .. }) if id == original)
                && echo_reply(s, local(3)) == Some(ReplyOriginal::Unsent { local: local(2) })
        })
        .await?;
        post.release();
        let reply = confirmed(&mut image, 2).await?;

        // The edit rides on the item in both windows; history keeps the confirmed text.
        let path = format!("/messages/{original}");
        let mut put = f.server().hold_next_rest("PUT", &path).await?;
        queue
            .enqueue(
                channel(),
                local(4),
                OperationKind::Edit {
                    target: MessageTarget::Remote(original),
                    content: "synthetic-edit".into(),
                    embeds: None,
                },
                UnixMicros(4),
            )
            .await?;
        put.wait_started().await?;
        for subscription in [&mut stream, &mut around] {
            until(subscription, |s| {
                shown(s, original) == Some(("synthetic".into(), Some((true, OperationState::Sending))))
            })
            .await?;
        }
        put.release();
        for subscription in [&mut stream, &mut around] {
            until(subscription, |s| {
                shown(s, original) == Some(("synthetic-edit".into(), None))
            })
            .await?;
        }
        until(&mut stream, |s| {
            matches!(message_reply(s, reply), Some(ReplyOriginal::Held { message, .. })
                if message.content.as_deref() == Some("synthetic-edit"))
        })
        .await?;
        assert!(matches!(live.original(original)?, ReplyOriginal::Held { id, .. } if id == original));
        assert_eq!(live.original(local(9))?, ReplyOriginal::Unknown);

        // A deletion rides the same way, then the reply's original reads deleted.
        let mut delete = f.server().hold_next_rest("DELETE", &path).await?;
        queue
            .enqueue(
                channel(),
                local(5),
                OperationKind::Delete {
                    target: MessageTarget::Remote(original),
                },
                UnixMicros(5),
            )
            .await?;
        delete.wait_started().await?;
        until(&mut stream, |s| {
            shown(s, original) == Some(("synthetic-edit".into(), Some((false, OperationState::Sending))))
        })
        .await?;
        delete.release();
        until(&mut stream, |s| {
            shown(s, original).is_none() && message_reply(s, reply) == Some(ReplyOriginal::Deleted)
        })
        .await?;
        assert_eq!(live.original(original)?, ReplyOriginal::Deleted);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

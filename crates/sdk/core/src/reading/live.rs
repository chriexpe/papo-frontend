//! Live message head tracking for admitted channels whose history is not currently open.
//! Open channels update through `history::live` and `reading::reconcile::refresh`.

use papo_api::{
    Uuid,
    gateway::{Event, KnownEvent},
};

use crate::{
    ClientError, Generation, MessageKey, UnreadCount, history::access::ChannelAdmission,
    session::SessionInner,
};

pub(crate) async fn observe_message(
    session: &SessionInner,
    generation: Generation,
    event: &Event,
) -> Result<(), ClientError> {
    let Event::Known(KnownEvent::Message(message)) = event else {
        return Ok(());
    };
    let Ok(channel) = message.channel_id.parse::<Uuid>() else {
        return Ok(());
    };
    let Ok(point) = MessageKey::parse(&message.created_at, &message.id) else {
        return Ok(());
    };
    let own = message.author_id.parse::<Uuid>().ok() == Some(session.account.user());
    let _serial = session.reading.serial.lock().await;
    session.epochs.validate(generation)?;
    if session.history.admission.read().await.channel(channel) != Some(ChannelAdmission::Allowed) {
        return Ok(());
    }
    let advanced = {
        let mut heads = session.reading.heads.lock().await;
        if heads.get(&channel).is_none_or(|&prev| point > prev) {
            session.epochs.apply(generation, || {
                heads.insert(channel, point);
            })?;
            true
        } else {
            false
        }
    };
    if !advanced || own {
        return Ok(());
    }
    let has_snapshot = session
        .history
        .snapshots()
        .await
        .into_iter()
        .any(|(id, snapshot)| id == channel && snapshot.generation == generation);
    if has_snapshot {
        return Ok(());
    }
    let unresolved = session
        .reading
        .unresolved
        .lock()
        .await
        .contains_key(&channel);
    if unresolved {
        return Ok(());
    }
    let mut image = (**session.reading.image.borrow()).clone();
    if image.generation != generation {
        return Ok(());
    }
    let Some(state) = image.channels.get_mut(&channel) else {
        return Ok(());
    };
    if Some(point) > state.point() && state.unread_count == UnreadCount::AtLeast(0) {
        state.unread_count = UnreadCount::AtLeast(1);
        session.reading.publish(session, image).await?;
    }
    Ok(())
}

//! Complete authoritative send receipts without losing newer gateway mutations; dispatch owns send outcome.

use super::{HistoryError, merge::Ticket, model::Model, ordering::Point};
use papo_api::{HistoryOrder, wire::Message};

impl Model {
    pub(super) fn receipt(
        &mut self,
        message: &Message,
        generation: u64,
    ) -> Result<(), HistoryError> {
        let point = Point::from(&message.cursor().ok_or(HistoryError::InvalidPage)?);
        let id = point.key()?.id;
        let revision = self.next_revision()?;
        if self.entries.get(&id).is_some_and(|entry| entry.deleted) {
            return Ok(());
        }
        if self.entries.get(&id).is_some_and(|entry| entry.timeline) {
            let channel = Self::id(
                message
                    .channel_id
                    .as_deref()
                    .ok_or(HistoryError::InvalidPage)?,
            )?;
            let ticket = Ticket {
                channel,
                generation,
                revision: 0,
                cursor: None,
                order: HistoryOrder::Desc,
                limit: 1,
            };
            self.merge_message(message, &point, &ticket, revision)
        } else {
            self.live_message(message, generation, revision)
        }
    }
}

#[cfg(test)]
mod tests {
    //! Full receipts supplement sparse events, preserving explicit newer field mutations and deletes.
    use super::*;
    use papo_api::{
        Uuid,
        gateway::{
            KnownEvent, MessageDeleteEvent, MessageEditEvent, MessageEmbedsUpdateEvent,
            MessageEvent,
        },
        wire::{Embed, EmbedMedia},
    };
    fn message() -> Message {
        Message {
            id: Some(Uuid::from_u128(1).to_string()),
            channel_id: Some(Uuid::from_u128(2).to_string()),
            created_at: Some("2026-01-01T00:00:00.123456Z".into()),
            content: Some("synthetic".into()),
            embeds: Some(vec![Embed {
                title: Some("synthetic".into()),
                image: Some(EmbedMedia {
                    url: Some("/media/synthetic".into()),
                    size_bytes: Some(3),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }
    }
    #[test]
    fn receipt_keeps_complete_embeds_without_events_and_supplements_an_existing_sparse_event() {
        for sparse in [false, true] {
            let mut model = Model::default();
            let message = message();
            if sparse {
                model
                    .event(
                        &KnownEvent::Message(Box::new(MessageEvent {
                            id: message.id.clone().unwrap(),
                            channel_id: message.channel_id.clone().unwrap(),
                            created_at: message.created_at.clone().unwrap(),
                            ..Default::default()
                        })),
                        1,
                    )
                    .unwrap();
            }
            model.receipt(&message, 1).unwrap();
            let actual = model
                .entries
                .get(Uuid::from_u128(1).as_bytes())
                .unwrap()
                .message
                .as_ref()
                .unwrap();
            assert_eq!(
                serde_json::to_value(&actual.embeds).unwrap(),
                serde_json::to_value(&message.embeds).unwrap()
            );
        }
    }
    #[test]
    fn newer_edits_embed_replacements_and_deletes_always_win_over_the_receipt() {
        let mut model = Model::default();
        let message = message();
        let id = message.id.clone().unwrap();
        let channel = message.channel_id.clone().unwrap();
        model.receipt(&message, 1).unwrap();
        model
            .event(
                &KnownEvent::MessageEdit(Box::new(MessageEditEvent {
                    id: id.clone(),
                    channel_id: channel.clone(),
                    content: String::new(),
                    edited_at: "2026-01-01T00:00:01Z".into(),
                })),
                1,
            )
            .unwrap();
        model
            .event(
                &KnownEvent::MessageEmbedsUpdate(Box::new(MessageEmbedsUpdateEvent {
                    channel_id: channel.clone(),
                    message_id: id.clone(),
                    embeds: vec![],
                })),
                1,
            )
            .unwrap();
        model.receipt(&message, 1).unwrap();
        let entry = &model.entries[Uuid::from_u128(1).as_bytes()];
        assert_eq!(entry.message.as_ref().unwrap().content.as_deref(), Some(""));
        assert_eq!(
            entry
                .message
                .as_ref()
                .unwrap()
                .embeds
                .as_ref()
                .unwrap()
                .len(),
            0
        );
        model
            .event(
                &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                    id,
                    channel_id: channel,
                })),
                1,
            )
            .unwrap();
        model.receipt(&message, 1).unwrap();
        assert!(model.entries[Uuid::from_u128(1).as_bytes()].deleted);
        assert_eq!(model.order.len(), 0);
    }
}

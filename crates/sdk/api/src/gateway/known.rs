//! Generated known event dispatch; unknown event preservation belongs to the gateway.
#[path = "messages.rs"]
mod messages;
pub use messages::*;
#[path = "channels.rs"]
mod channels;
pub use channels::*;
#[path = "presence.rs"]
mod presence;
pub use presence::*;
#[path = "membership.rs"]
mod membership;
pub use membership::*;
#[path = "direct_messages.rs"]
mod direct_messages;
pub use direct_messages::*;
#[path = "notifications.rs"]
mod notifications;
pub use notifications::*;
#[path = "control.rs"]
mod control;
pub use control::*;
#[path = "voice.rs"]
mod voice;
pub use voice::*;
/// Every outbound event emitted by the pinned backend, including voice offers.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum KnownEvent {
    /// Backend `message` event.
    #[serde(rename = "message")]
    Message(Box<MessageEvent>),
    /// Backend `message_edit` event.
    #[serde(rename = "message_edit")]
    MessageEdit(Box<MessageEditEvent>),
    /// Backend `message_delete` event.
    #[serde(rename = "message_delete")]
    MessageDelete(Box<MessageDeleteEvent>),
    /// Backend `attachment_moderation_update` event.
    #[serde(rename = "attachment_moderation_update")]
    AttachmentModerationUpdate(Box<AttachmentModerationUpdateEvent>),
    /// Backend `message_embeds_update` event.
    #[serde(rename = "message_embeds_update")]
    MessageEmbedsUpdate(Box<MessageEmbedsUpdateEvent>),
    /// Backend `react_update` event.
    #[serde(rename = "react_update")]
    ReactUpdate(Box<ReactUpdateEvent>),
    /// Backend `message_pin` event.
    #[serde(rename = "message_pin")]
    MessagePin(Box<MessagePinEvent>),
    /// Backend `channel_create` event.
    #[serde(rename = "channel_create")]
    ChannelCreate(Box<ChannelCreateEvent>),
    /// Backend `channel_update` event.
    #[serde(rename = "channel_update")]
    ChannelUpdate(Box<ChannelUpdateEvent>),
    /// Backend `channel_delete` event.
    #[serde(rename = "channel_delete")]
    ChannelDelete(Box<ChannelDeleteEvent>),
    /// Backend `typing` event.
    #[serde(rename = "typing")]
    Typing(Box<TypingEvent>),
    /// Backend `avatar_update` event.
    #[serde(rename = "avatar_update")]
    AvatarUpdate(Box<AvatarUpdateEvent>),
    /// Backend `presence_update` event.
    #[serde(rename = "presence_update")]
    PresenceUpdate(Box<PresenceUpdateEvent>),
    /// Backend `user_join` event.
    #[serde(rename = "user_join")]
    UserJoin(Box<UserJoinEvent>),
    /// Backend `role_add` event.
    #[serde(rename = "role_add")]
    RoleAdd(Box<RoleAddEvent>),
    /// Backend `role_remove` event.
    #[serde(rename = "role_remove")]
    RoleRemove(Box<RoleRemoveEvent>),
    /// Backend `dm_update` event.
    #[serde(rename = "dm_update")]
    DMUpdate(Box<DMUpdateEvent>),
    /// Backend `presence_sync` event.
    #[serde(rename = "presence_sync")]
    PresenceSync(Box<PresenceSyncEvent>),
    /// Backend `heartbeat_ack` event.
    #[serde(rename = "heartbeat_ack")]
    HeartbeatAck(Box<HeartbeatAckEvent>),
    /// Backend `error` event.
    #[serde(rename = "error")]
    Error(Box<ErrorEvent>),
    /// Backend `new_notification` event.
    #[serde(rename = "new_notification")]
    NewNotification(Box<NewNotificationEvent>),
    /// Backend `voice_joined` event.
    #[serde(rename = "voice_joined")]
    VoiceJoined(Box<VoiceJoinedEvent>),
    /// Backend `voice_answer` event.
    #[serde(rename = "voice_answer")]
    VoiceAnswer(Box<VoiceAnswerEvent>),
    /// Backend `voice_offer` event.
    #[serde(rename = "voice_offer")]
    VoiceOffer(Box<VoiceOfferEvent>),
    /// Backend `voice_ice_candidate` event.
    #[serde(rename = "voice_ice_candidate")]
    VoiceICECandidate(Box<VoiceICECandidateEvent>),
    /// Backend `voice_state_update` event.
    #[serde(rename = "voice_state_update")]
    VoiceStateUpdate(Box<VoiceStateUpdateEvent>),
    /// Backend `voice_leave` event.
    #[serde(rename = "voice_leave")]
    VoiceLeave(Box<VoiceLeaveEvent>),
    /// Backend `active_speaker_update` event.
    #[serde(rename = "active_speaker_update")]
    ActiveSpeakerUpdate(Box<ActiveSpeakerUpdateEvent>),
    /// Backend `voice_audio_routes` event.
    #[serde(rename = "voice_audio_routes")]
    VoiceAudioRoutes(Box<VoiceAudioRoutesEvent>),
}
impl KnownEvent {
    /// Static event kind, safe for diagnostic output.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::MessageEdit(_) => "message_edit",
            Self::MessageDelete(_) => "message_delete",
            Self::AttachmentModerationUpdate(_) => "attachment_moderation_update",
            Self::MessageEmbedsUpdate(_) => "message_embeds_update",
            Self::ReactUpdate(_) => "react_update",
            Self::MessagePin(_) => "message_pin",
            Self::ChannelCreate(_) => "channel_create",
            Self::ChannelUpdate(_) => "channel_update",
            Self::ChannelDelete(_) => "channel_delete",
            Self::Typing(_) => "typing",
            Self::AvatarUpdate(_) => "avatar_update",
            Self::PresenceUpdate(_) => "presence_update",
            Self::UserJoin(_) => "user_join",
            Self::RoleAdd(_) => "role_add",
            Self::RoleRemove(_) => "role_remove",
            Self::DMUpdate(_) => "dm_update",
            Self::PresenceSync(_) => "presence_sync",
            Self::HeartbeatAck(_) => "heartbeat_ack",
            Self::Error(_) => "error",
            Self::NewNotification(_) => "new_notification",
            Self::VoiceJoined(_) => "voice_joined",
            Self::VoiceAnswer(_) => "voice_answer",
            Self::VoiceOffer(_) => "voice_offer",
            Self::VoiceICECandidate(_) => "voice_ice_candidate",
            Self::VoiceStateUpdate(_) => "voice_state_update",
            Self::VoiceLeave(_) => "voice_leave",
            Self::ActiveSpeakerUpdate(_) => "active_speaker_update",
            Self::VoiceAudioRoutes(_) => "voice_audio_routes",
        }
    }
    /// Whether this discriminator belongs to the pinned outbound protocol.
    #[must_use]
    pub fn recognizes(kind: &str) -> bool {
        matches!(
            kind,
            "message"
                | "message_edit"
                | "message_delete"
                | "attachment_moderation_update"
                | "message_embeds_update"
                | "react_update"
                | "message_pin"
                | "channel_create"
                | "channel_update"
                | "channel_delete"
                | "typing"
                | "avatar_update"
                | "presence_update"
                | "user_join"
                | "role_add"
                | "role_remove"
                | "dm_update"
                | "presence_sync"
                | "heartbeat_ack"
                | "error"
                | "new_notification"
                | "voice_joined"
                | "voice_answer"
                | "voice_offer"
                | "voice_ice_candidate"
                | "voice_state_update"
                | "voice_leave"
                | "active_speaker_update"
                | "voice_audio_routes"
        )
    }
}
#[cfg(test)]
mod tests {
    //! Exercises all generated outbound variants without a backend checkout.
    use super::KnownEvent;
    #[test]
    fn every_known_event_round_trips_through_dispatch() {
        let events = [
            KnownEvent::Message(Box::default()),
            KnownEvent::MessageEdit(Box::default()),
            KnownEvent::MessageDelete(Box::default()),
            KnownEvent::AttachmentModerationUpdate(Box::default()),
            KnownEvent::MessageEmbedsUpdate(Box::default()),
            KnownEvent::ReactUpdate(Box::default()),
            KnownEvent::MessagePin(Box::default()),
            KnownEvent::ChannelCreate(Box::default()),
            KnownEvent::ChannelUpdate(Box::default()),
            KnownEvent::ChannelDelete(Box::default()),
            KnownEvent::Typing(Box::default()),
            KnownEvent::AvatarUpdate(Box::default()),
            KnownEvent::PresenceUpdate(Box::default()),
            KnownEvent::UserJoin(Box::default()),
            KnownEvent::RoleAdd(Box::default()),
            KnownEvent::RoleRemove(Box::default()),
            KnownEvent::DMUpdate(Box::default()),
            KnownEvent::PresenceSync(Box::default()),
            KnownEvent::HeartbeatAck(Box::default()),
            KnownEvent::Error(Box::default()),
            KnownEvent::NewNotification(Box::default()),
            KnownEvent::VoiceJoined(Box::default()),
            KnownEvent::VoiceAnswer(Box::default()),
            KnownEvent::VoiceOffer(Box::default()),
            KnownEvent::VoiceICECandidate(Box::default()),
            KnownEvent::VoiceStateUpdate(Box::default()),
            KnownEvent::VoiceLeave(Box::default()),
            KnownEvent::ActiveSpeakerUpdate(Box::default()),
            KnownEvent::VoiceAudioRoutes(Box::default()),
        ];
        for event in events {
            let kind = event.kind();
            let bytes = serde_json::to_vec(&event).unwrap();
            let decoded = crate::gateway::Event::decode(&bytes).unwrap();
            assert_eq!(decoded.kind(), kind);
            assert!(matches!(decoded, crate::gateway::Event::Known(_)));
        }
    }
}

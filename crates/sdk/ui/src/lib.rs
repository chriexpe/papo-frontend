//! Observable Papo UI services belong here; transport and durable state belong to `papo-sdk`.
//! Platform widgets, navigation and translated text belong to client applications.

pub mod message_content;
pub mod operations;
pub use operations::{OperationService, ReactionService};
mod search;
pub use search::{SearchService, SearchSubscription, SearchUpdate};
mod notifications;
pub use notifications::{AccountInbox, NotificationClient, NotificationItem};
mod activity;
mod calls;
mod expressions;
pub use activity::{ActivityService, ActivitySubscription, ActivityView, TypingItem};
pub use calls::{
    ActiveCallView, CallParticipantItem, CallParticipantKey, CallService, CallSubscription,
    CallView, ParticipantVideoBinding, RoomParticipantItem, VoiceCapability, VoiceRoomView,
};
pub use expressions::{
    ExpressionId, ExpressionItem, ExpressionLibraryService, ExpressionLibrarySubscription,
    ExpressionLibraryUpdate, ExpressionLibraryView,
};
mod capabilities;
pub use capabilities::{
    AuthorIdentity, CapabilityService, CapabilitySubscription, CapabilityView, ChannelCapabilities,
    ServerCapabilities, capabilities,
};
mod direct_messages;
pub use direct_messages::{
    DirectMessageId, DirectMessageItem, DirectMessageService, DirectMessageSubscription,
    DirectMessageUpdate,
};
mod channel_list;
mod channel_projection;
mod channel_reading;
mod people;
mod resource_list;
pub use people::{MemberItem, PeopleService, ProfileItem, RoleLabel};
pub use resource_list::{PeopleListSubscription, PeopleListUpdate};
mod content_cache;
mod list_diff;
mod list_subscription;
mod registry_projection;
mod server_badges;
mod server_list;
pub use server_badges::{
    AccountBadges, BadgeAggregate, ServerBadgesSnapshot, ServerBadgesStream,
    ServerBadgesSubscription, ServerBadgesUpdate,
};
mod timeline;
mod timeline_arrivals;
mod timeline_item;
mod timeline_reading;
mod timeline_seek;
mod timeline_window;

pub use papo_sdk::VisibilityContinuity;
pub use timeline::{
    Timeline, TimelineConfig, TimelineError, TimelineEvent, TimelinePage, TimelineStream,
    TimelineSubscription, TimelineUpdate,
};
pub use timeline_arrivals::TimelineArrivals;
pub use timeline_item::{
    ReplyOriginal, TimelineDirection, TimelineItem, UtcOffset, UtcOffsetProvider,
};
pub use timeline_reading::{
    TimelineCatchUp, TimelineReadingEvent, TimelineReadingState, TimelineReadingStream,
    TimelineReadingSubscription,
};
pub use timeline_seek::CatchUpSeek;

pub use channel_list::{
    ChannelListItem, ChannelListService, ChannelListStream, ChannelListSubscription,
    ChannelListUpdate,
};
pub use server_list::{
    AccountListStream, AccountListSubscription, AccountListUpdate, ServerListService,
    ServerListSnapshot, ServerListStream, ServerListSubscription, ServerListUpdate,
};

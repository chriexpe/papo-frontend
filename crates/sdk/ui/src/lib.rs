//! Observable Papo UI services belong here; transport and durable state belong to `papo-sdk`.
//! Platform widgets, navigation and translated text belong to client applications.

mod message_actions;
pub mod message_content;
pub use message_actions::{
    DeleteAs, EditTarget, MessageActions, PinChange, message_actions, newest_editable, pinned,
};
mod pins;
pub use pins::{ChannelPins, PinsEvent, PinsStream, PinsSubscription};
mod reactions;
pub use reactions::{
    DEFAULT_QUICK_REACTIONS, ReactionChip, Reactor, Reactors, quick_reactions, reaction_chips,
    unresolved_reactions,
};
mod message_attention;
pub use message_attention::{Attention, attention};
mod mention_suggestions;
mod message_snippet;
pub use mention_suggestions::{
    MAX_MENTION_SUGGESTIONS, MentionQuery, MentionSuggestion, display_name, mention_query,
    mention_suggestions,
};
pub use message_snippet::{Snippet, SnippetPart, snippet};
pub mod operations;
pub use operations::{OperationService, ReactionService};
mod search;
pub use search::{
    DateSuggestion, MAX_SEARCH_SUGGESTIONS, SearchCount, SearchResultItem, SearchResults,
    SearchService, SearchSubscription, SearchSuggestion, SearchUpdate, SearchWindow,
    SuggestedValue, localize_search, search_candidates, search_date_suggestions,
    search_suggestions,
};
mod notifications;
pub use notifications::{AccountInbox, NotificationClient, NotificationItem};
mod activity;
mod calls;
mod expressions;
pub use activity::{
    AccountActivity, ActivityService, ActivitySubscription, PresenceMark, TypingItem,
};
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
pub use people::{MemberItem, PeopleService, ProfileItem, RoleLabel, person, requested_profile};
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

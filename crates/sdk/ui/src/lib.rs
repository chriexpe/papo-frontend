//! Observable Papo UI services belong here; transport and durable state belong to `papo-sdk`.
//! Platform widgets, navigation and translated text belong to client applications.

mod search;
pub use search::{SearchService, SearchSubscription, SearchUpdate};
mod notifications;
pub use notifications::{AccountInbox, NotificationClient, NotificationItem};
mod activity;
mod expressions;
pub use activity::{ActivityService, ActivitySubscription, ActivityView, TypingItem};
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
mod people_list;
pub use people::{MemberItem, PeopleService, ProfileItem, RoleLabel};
pub use people_list::{PeopleListSubscription, PeopleListUpdate};
mod list_diff;
mod list_subscription;
mod registry_projection;
mod server_list;
mod timeline;
mod timeline_item;
mod timeline_window;

pub use timeline::{
    Timeline, TimelineConfig, TimelineError, TimelineEvent, TimelinePage, TimelineStream,
    TimelineSubscription, TimelineUpdate,
};
pub use timeline_item::{TimelineDirection, TimelineItem, UtcOffset, UtcOffsetProvider};

pub use channel_list::{
    ChannelListItem, ChannelListService, ChannelListStream, ChannelListSubscription,
    ChannelListUpdate,
};
pub use server_list::{
    AccountListStream, AccountListSubscription, AccountListUpdate, ServerListService,
    ServerListSnapshot, ServerListStream, ServerListSubscription, ServerListUpdate,
};

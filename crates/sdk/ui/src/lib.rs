//! Observable Papo UI services belong here; transport and durable state belong to `papo-sdk`.
//! Platform widgets, navigation and translated text belong to client applications.

mod channel_list;
mod channel_projection;
mod list_diff;
mod registry_projection;
mod server_list;

pub use channel_list::{
    ChannelListItem, ChannelListService, ChannelListStream, ChannelListSubscription,
    ChannelListUpdate,
};
pub use server_list::{
    AccountListStream, AccountListSubscription, AccountListUpdate, ServerListService,
    ServerListSnapshot, ServerListStream, ServerListSubscription, ServerListUpdate,
};

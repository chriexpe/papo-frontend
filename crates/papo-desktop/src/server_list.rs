//! The live server list: the rail's servers in their saved order, each with
//! its name, icon and channels and the account signed in to it.
//!
//! Owns the projection from the SDK's registry and each account session's
//! resources ([`project`]) and the watch that posts it whenever it changes.
//! The SDK owns the registry, sessions and resources; this keeps only what
//! the rail and the channel list draw, badges included: each channel's from
//! its account's badge image, each server's from the SDK's aggregate for the
//! same account.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use futures_util::StreamExt;
use papo_sdk::{
    AccountId, BadgesSnapshot, ChannelKind, ChannelRecord, ClientHandle, MAX_ICON_BYTES,
    MediaSource, RegistrySnapshot, ResourceSnapshot, SavedAccount, ServerId, Session,
};
use papo_sdk_ui::{AccountBadges, ServerListService};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::badges::Badge;
use crate::sdk_bridge::Updates;
use crate::sign_in::shown_address;

/// The rail's servers, in the order the person saved them.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ServerList {
    pub(crate) servers: Vec<ServerEntry>,
}

impl ServerList {
    pub(crate) fn get(&self, id: ServerId) -> Option<&ServerEntry> {
        self.servers.iter().find(|server| server.id == id)
    }

    pub(crate) fn order(&self) -> Vec<ServerId> {
        self.servers.iter().map(|server| server.id).collect()
    }
}

impl std::fmt::Debug for ServerList {
    /// Names no server: the count only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerList")
            .field("servers", &self.servers.len())
            .finish()
    }
}

/// One server in the rail.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ServerEntry {
    pub(crate) id: ServerId,
    /// The name its resource gives once an account is signed in, else its
    /// address as shown.
    pub(crate) name: String,
    /// Its owner's username, when the resource names one.
    pub(crate) owner: Option<String>,
    /// Its icon's encoded bytes from the account's media store, at most
    /// `MAX_ICON_BYTES`; not decoded here.
    pub(crate) icon: Option<Arc<[u8]>>,
    /// The account the channel list shows: the active one, else the first.
    pub(crate) account: Option<AccountEntry>,
    /// Text channels in the server's order; channels of unknown kinds too.
    pub(crate) text_channels: Vec<ChannelEntry>,
    /// Voice channels in the server's order.
    pub(crate) voice_channels: Vec<ChannelEntry>,
    /// The server's badge: its shown account's unread and mentions.
    pub(crate) badge: Badge,
}

/// The account a server's channel list belongs to.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AccountEntry {
    pub(crate) id: AccountId,
    /// Its display name: the nickname, or the username without one.
    pub(crate) name: String,
    pub(crate) username: String,
}

/// A channel row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChannelEntry {
    /// The server's identity for it: the row's key.
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) badge: Badge,
}

/// What one account session's resources say about its server.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ServerResources {
    pub(crate) name: Option<String>,
    pub(crate) owner: Option<String>,
    pub(crate) icon: Option<Arc<[u8]>>,
    pub(crate) text_channels: Vec<ChannelEntry>,
    pub(crate) voice_channels: Vec<ChannelEntry>,
}

/// Builds the rail from the registry and the resources of each server's
/// account.
pub(crate) fn project(
    registry: &RegistrySnapshot,
    resources: &HashMap<AccountId, ServerResources>,
    account_badges: &BTreeMap<AccountId, Arc<AccountBadges>>,
) -> ServerList {
    let servers = registry
        .servers
        .iter()
        .map(|saved| {
            let id = saved.address.id();
            let account = shown_account(&registry.accounts, id);
            let found = account.and_then(|account| resources.get(&account.id));
            ServerEntry {
                id,
                name: found
                    .and_then(|found| found.name.clone())
                    .unwrap_or_else(|| shown_address(saved.address.as_str()).to_owned()),
                owner: found.and_then(|found| found.owner.clone()),
                icon: found.and_then(|found| found.icon.clone()),
                account: account.map(|account| AccountEntry {
                    id: account.id,
                    name: account
                        .display_name
                        .clone()
                        .unwrap_or_else(|| account.username.clone()),
                    username: account.username.clone(),
                }),
                text_channels: found
                    .map(|found| found.text_channels.clone())
                    .unwrap_or_default(),
                voice_channels: found
                    .map(|found| found.voice_channels.clone())
                    .unwrap_or_default(),
                // The shown account's aggregate, so the tile always agrees
                // with the rows below it, active account or not.
                badge: account
                    .and_then(|account| account_badges.get(&account.id))
                    .map(|badges| Badge::of_aggregate(&badges.all))
                    .unwrap_or_default(),
            }
        })
        .collect();
    ServerList { servers }
}

/// The account the channel list shows for server `id`: the active one, else
/// the first saved.
fn shown_account(accounts: &[SavedAccount], id: ServerId) -> Option<&SavedAccount> {
    let mut on_server = accounts.iter().filter(|account| account.id.server() == id);
    let first = on_server.clone().next();
    on_server.find(|account| account.active).or(first)
}

/// Groups named text and voice channels, preserving backend order within each
/// group, each with its badge from `badges` (none without an image).
pub(crate) fn channels<'a>(
    all: impl IntoIterator<Item = &'a ChannelRecord>,
    badges: Option<&BadgesSnapshot>,
) -> (Vec<ChannelEntry>, Vec<ChannelEntry>) {
    let mut text = Vec::new();
    let mut voice = Vec::new();
    for channel in all {
        let Some(name) = &channel.name else {
            continue;
        };
        let rows = match channel.kind {
            ChannelKind::Text => &mut text,
            ChannelKind::Voice => &mut voice,
            ChannelKind::Category => continue,
        };
        let badge = badges
            .and_then(|badges| badges.channels.get(&channel.id))
            .map(|badges| Badge::of_channel(badges))
            .unwrap_or_default();
        rows.push(ChannelEntry {
            id: channel.id.to_string(),
            name: name.clone(),
            badge,
        });
    }
    (text, voice)
}

/// Runs until the client closes: posts the server list now, then again each
/// time the registry, the set of sessions or a shown account's resources
/// change it.
pub(crate) async fn run<S: 'static>(
    client: ClientHandle,
    updates: Updates<S>,
    apply: fn(&mut S, ServerList) -> bool,
) {
    let mut registry = client.subscribe_registry();
    let mut sessions = client.subscribe_session_changes();
    let badges = ServerListService::new(client.clone()).subscribe_badges();
    let mut account_badges = badges.snapshot.accounts;
    let mut badge_changes = badges.stream;
    let mut icons = Icons::default();
    let mut posted = None;
    loop {
        let snapshot = Arc::clone(&registry.borrow_and_update());
        sessions.borrow_and_update();
        // Rebuilt on every pass; dropping the old set stops its waits.
        let mut resource_changes = JoinSet::new();
        let resources = read_resources(&client, &snapshot, &mut icons, &mut resource_changes);
        let list = project(&snapshot, &resources, &account_badges);
        if posted.as_ref() != Some(&list) {
            posted = Some(list.clone());
            updates.send(move |state| apply(state, list));
        }
        tokio::select! {
            changed = registry.changed() => if changed.is_err() { return },
            changed = sessions.changed() => if changed.is_err() { return },
            // One badge change wakes both this stream and the shown
            // account's badge watch: two passes, the second one complete. The
            // stream keeps its pending read across select! rounds.
            update = badge_changes.next() => match update {
                Some(update) => account_badges = update.snapshot.accounts,
                // The client closed: the subscription ends with it.
                None => return,
            },
            Some(_) = resource_changes.join_next(), if !resource_changes.is_empty() => {}
            Some(fetched) = icons.fetches.join_next(), if !icons.fetches.is_empty() => {
                if let Ok(fetch) = fetched {
                    match fetch.bytes {
                        Some(bytes) => {
                            icons.fetched.insert(fetch.account, bytes);
                        }
                        // No answer (signed out, offline): ask again next time.
                        None if !fetch.answered => {
                            icons.asked.remove(&fetch.account);
                        }
                        None => {}
                    }
                }
            }
        }
    }
}

/// Each shown account's server icon, fetched through its media store once
/// per run, refreshing its cached copy so a changed icon shows on the next
/// start. A fetch that gets no answer is asked again on the next change.
#[derive(Default)]
struct Icons {
    /// Accounts whose icon was asked for, fetched or not.
    asked: HashSet<AccountId>,
    fetched: HashMap<AccountId, Arc<[u8]>>,
    /// Fetches still running; kept across passes.
    fetches: JoinSet<IconFetch>,
}

/// One fetch's outcome.
struct IconFetch {
    account: AccountId,
    bytes: Option<Arc<[u8]>>,
    /// Whether the media store answered at all; without an answer the icon
    /// is asked for again.
    answered: bool,
}

impl Icons {
    /// The icon fetched for `account`, asking for it the first time.
    fn for_account(&mut self, account: AccountId, session: &Session) -> Option<Arc<[u8]>> {
        if self.asked.insert(account) {
            let media = session.media();
            self.fetches.spawn(async move {
                let fetched = media
                    .fetch(MediaSource::ServerIcon, true)
                    .await
                    .inspect_err(|error| log::debug!("no server icon: {error}"));
                let answered = fetched.is_ok();
                let bytes = fetched
                    .ok()
                    .map(|data| data.value.bytes)
                    .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_ICON_BYTES);
                if let Some(bytes) = &bytes {
                    log::debug!("fetched a server icon of {} bytes", bytes.len());
                }
                IconFetch {
                    account,
                    bytes,
                    answered,
                }
            });
        }
        self.fetched.get(&account).cloned()
    }
}

/// Reads the resources of each server's shown account, and queues a wait for
/// their next change in `changes`.
fn read_resources(
    client: &ClientHandle,
    registry: &RegistrySnapshot,
    icons: &mut Icons,
    changes: &mut JoinSet<()>,
) -> HashMap<AccountId, ServerResources> {
    let mut found = HashMap::new();
    for saved in &registry.servers {
        let Some(account) = shown_account(&registry.accounts, saved.address.id()) else {
            continue;
        };
        // A session not restored yet arrives with a session change.
        let Ok(session) = client.session(account.id) else {
            continue;
        };
        let snapshot = Arc::clone(&wait_for_change(session.subscribe_resources(), changes));
        let badges = Arc::clone(&wait_for_change(session.subscribe_badges(), changes));
        // An image from an earlier connection is stale until the SDK
        // recounts for this one, as the aggregate already treats it.
        let current = (badges.version != 0 && badges.generation == session.generation())
            .then_some(badges.as_ref());
        let icon = icons.for_account(account.id, &session);
        found.insert(account.id, resources(&snapshot, current, icon));
    }
    found
}

/// The watch's current value, with a wait for its next change queued in
/// `changes`.
fn wait_for_change<T: Clone + Send + Sync + 'static>(
    mut watch: watch::Receiver<T>,
    changes: &mut JoinSet<()>,
) -> T {
    let current = watch.borrow_and_update().clone();
    changes.spawn(async move {
        if watch.changed().await.is_err() {
            // A closed session never changes again; the session change that
            // closed it wakes the loop instead.
            std::future::pending::<()>().await;
        }
    });
    current
}

fn resources(
    snapshot: &ResourceSnapshot,
    badges: Option<&BadgesSnapshot>,
    icon: Option<Arc<[u8]>>,
) -> ServerResources {
    let server = snapshot.server.as_deref();
    let (text_channels, voice_channels) = channels(snapshot.channels.iter(), badges);
    ServerResources {
        name: server
            .and_then(|server| server.name.clone())
            .filter(|name| !name.trim().is_empty()),
        owner: server.and_then(|server| server.owner_username.clone()),
        icon,
        text_channels,
        voice_channels,
    }
}

#[cfg(test)]
mod tests;

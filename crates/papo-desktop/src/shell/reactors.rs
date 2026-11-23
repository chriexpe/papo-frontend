//! Who reacted, from the shell's side: reading the SDK's `reaction_users`
//! pages for a message (oldest reaction first) into its timeline's lists,
//! one page at a time while more are needed, and asking for the profiles
//! of people the account does not know yet, so their names show (the SDK
//! publishes them with the account's resources).

use papo_sdk::{HistoryOrder, Uuid};
use papo_sdk_ui::operations::ReactionEmoji;
use papo_sdk_ui::{TimelineItem, reaction_chips};

use super::Shell;
use crate::timeline::TimelineKey;

impl Shell {
    /// Reads the next page of who reacted to message `id` of timeline
    /// `key`, when one is needed to show `filter`'s people (everyone's
    /// when none) and none is on its way.
    pub(crate) fn load_reactors(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        filter: Option<&ReactionEmoji>,
    ) {
        let outgoing = self.outgoing(key.account);
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        let Some(message) = open.items.iter().find_map(|item| match item.as_ref() {
            TimelineItem::Message {
                id: current,
                message,
                ..
            } if *current == id => Some(std::sync::Arc::clone(message)),
            _ => None,
        }) else {
            return;
        };
        let chips = reaction_chips(&message, &outgoing);
        if chips.is_empty() {
            return;
        }
        let Some(request) = open.reactors.wanted(id, key.account.user(), &chips, filter) else {
            return;
        };
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let page = match client.session(key.account) {
                Ok(session) => session
                    .operations()
                    .reaction_users(key.channel, request.id, HistoryOrder::Asc, request.cursor)
                    .await
                    .map(|page| page.value)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let page = page
                .inspect_err(|error| log::warn!("who reacted was not read: {error}"))
                .ok();
            updates
                .send(move |shell| shell.reactors_arrived(key, request.id, request.ticket, page));
        });
    }

    /// A page of who reacted to message `id` arrived (none when it could
    /// not be read). Returns whether the view changed.
    fn reactors_arrived(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        ticket: u64,
        page: Option<papo_sdk::operations::interactions::ReactorPage>,
    ) -> bool {
        let Some(open) = self.shown_timeline_mut(key) else {
            return false;
        };
        let people: Vec<Uuid> = page
            .iter()
            .flat_map(|page| &page.groups)
            .flat_map(|group| group.users.iter().flatten())
            .filter_map(|user| user.user_id.as_deref())
            .filter_map(|user| Uuid::parse_str(user).ok())
            .filter(|user| {
                open.resources
                    .as_ref()
                    .is_none_or(|resources| resources.users.get(*user).is_none())
            })
            .collect();
        if !open.reactors.arrived(id, ticket, page) {
            return false;
        }
        let unknown = open.reactors.unasked(people);
        if !unknown.is_empty()
            && let Some(sdk) = &self.sdk
        {
            sdk.run(move |client| async move {
                let Ok(session) = client.session(key.account) else {
                    return;
                };
                if let Err(error) = session.profiles(&unknown).await {
                    log::debug!("reactors' profiles were not read: {error}");
                }
            });
        }
        true
    }
}

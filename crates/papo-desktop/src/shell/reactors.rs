//! Who reacted, from the shell's side: reading the SDK's `reaction_users`
//! pages for a message (oldest reaction first) into its timeline's lists,
//! one page at a time (the first for a chip's hint; for the reactions list,
//! until its tab holds its count), and asking for the profiles of people
//! the account does not know yet, so their names show (the SDK publishes
//! them with the account's resources).

use papo_sdk::operations::interactions::ReactorPage;
use papo_sdk::{HistoryOrder, Uuid};
use papo_sdk_ui::reaction_chips;

use super::Shell;
use crate::timeline::TimelineKey;
use crate::timeline::reactors::Want;

impl Shell {
    /// Reads the next page of who reacted to message `id` of the shown
    /// timeline `key`, when `want` needs one and none is on its way.
    /// Returns whether a page was asked for.
    pub(crate) fn load_reactors(&mut self, key: TimelineKey, id: Uuid, want: Want<'_>) -> bool {
        let outgoing = self.outgoing(key.account);
        let Some(open) = self.shown_timeline_mut(key) else {
            return false;
        };
        let Some(message) = open
            .message(id)
            .map(|(message, _)| std::sync::Arc::clone(message))
        else {
            return false;
        };
        let chips = reaction_chips(&message, &outgoing);
        if chips.is_empty() {
            return false;
        }
        let Some(request) = open.reactors.wanted(id, key.account.user(), &chips, want) else {
            return false;
        };
        let Some(sdk) = &self.sdk else {
            return false;
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
        true
    }

    /// A page of who reacted to message `id` arrived (none when it could
    /// not be read). Taken by its timeline, shown or not, so no list waits
    /// for an answer it dropped. Returns whether the view changed.
    fn reactors_arrived(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        ticket: u64,
        page: Option<ReactorPage>,
    ) -> bool {
        let Some(open) = self.tabs.open_mut(key) else {
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
        // A tab still short of its count reads on: its rows may not have
        // changed, so nothing else would ask.
        let view = self
            .nav
            .menu
            .as_ref()
            .and_then(|menu| menu.reactions.clone())
            .filter(|view| view.key == key && view.id == id);
        if let Some(view) = view {
            self.load_reactors(key, id, Want::People(view.filter.as_ref()));
        }
        true
    }
}

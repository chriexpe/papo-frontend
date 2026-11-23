//! The people a New message picker offers (`docs/design.md` §10, Direct
//! messages): everyone on the servers of the given accounts, one row per
//! server (a DM belongs to one account), never the account's own person, a
//! person it blocks or a banned member. With nothing typed, everyone A–Z;
//! otherwise names starting with what is typed first, then names
//! containing it, each A–Z, ignoring case. Reads the rosters the SDK
//! already holds; nothing is fetched. The picker's look and keys are the
//! client's; opening the conversation is `Operations::open_direct_conversation`.

use papo_sdk::{AccountId, ResourceSnapshot, Uuid};

use crate::display_name;

/// One person the picker offers, on one account's server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectCandidate {
    /// The account the conversation would be from.
    pub account: AccountId,
    /// The person.
    pub user: Uuid,
    /// The nickname, else the username.
    pub name: String,
    /// The username, shown beside the name.
    pub username: Option<String>,
}

/// The people `query` finds across `accounts` (each with its resource
/// snapshot, in the order a client lists its servers), ranked as the
/// module says; a person on two servers is two rows, in that order.
#[must_use]
pub fn direct_candidates(
    accounts: &[(AccountId, &ResourceSnapshot)],
    query: &str,
) -> Vec<DirectCandidate> {
    let query = query.trim().trim_start_matches('@').to_lowercase();
    let mut starting = Vec::new();
    let mut containing = Vec::new();
    for (account_order, (account, snapshot)) in accounts.iter().enumerate() {
        for id in snapshot.members.iter() {
            if id == account.user() || snapshot.blocks.contains(id) {
                continue;
            }
            let Some(user) = snapshot.users.get(id) else {
                continue;
            };
            if user.banned == Some(true) {
                continue;
            }
            let Some(name) = display_name(user.nickname.as_deref(), user.username.as_deref())
            else {
                continue;
            };
            let names: Vec<String> = [Some(name), user.username.as_deref()]
                .into_iter()
                .flatten()
                .map(str::to_lowercase)
                .collect();
            let candidate = || Ranked {
                sort_name: name.to_lowercase(),
                account_order,
                candidate: DirectCandidate {
                    account: *account,
                    user: id,
                    name: name.to_owned(),
                    username: user.username.clone(),
                },
            };
            if names.iter().any(|name| name.starts_with(&query)) {
                starting.push(candidate());
            } else if names.iter().any(|name| name.contains(&query)) {
                containing.push(candidate());
            }
        }
    }
    starting.sort_by(Ranked::order);
    containing.sort_by(Ranked::order);
    starting
        .into_iter()
        .chain(containing)
        .map(|ranked| ranked.candidate)
        .collect()
}

/// A candidate with what it sorts by: its name, then its server's place.
struct Ranked {
    sort_name: String,
    account_order: usize,
    candidate: DirectCandidate,
}

impl Ranked {
    fn order(a: &Ranked, b: &Ranked) -> std::cmp::Ordering {
        a.sort_name
            .cmp(&b.sort_name)
            .then(a.account_order.cmp(&b.account_order))
    }
}

#[cfg(test)]
mod tests;

//! Pure account/server/DM badge aggregation; SDK history and reading own coverage and eligibility.

use std::{collections::BTreeMap, sync::Arc};

use papo_sdk::{AccountId, ReadingSnapshot, RegistrySnapshot, ResourceSnapshot, UnreadCount, Uuid};

use crate::badges::{BadgeSnapshot, BadgeTotals};

pub(crate) struct AccountImage {
    pub resources: Arc<ResourceSnapshot>,
    pub reading: Arc<ReadingSnapshot>,
}

pub(crate) fn project(
    registry: &RegistrySnapshot,
    images: &BTreeMap<AccountId, AccountImage>,
) -> BadgeSnapshot {
    let mut accounts = BTreeMap::new();
    let mut direct_messages = Some(BadgeTotals::empty());
    for account in &registry.accounts {
        let totals = images.get(&account.id).and_then(account_totals);
        accounts.insert(account.id, totals.map(|(channels, _)| channels));
        direct_messages = match (direct_messages, totals) {
            (Some(mut total), Some((_, dm))) => {
                // Preserve unread uncertainty without converting it into an exact count.
                let unread = match dm.has_unread {
                    Some(true) => UnreadCount::AtLeast(1),
                    Some(false) => UnreadCount::Exact(0),
                    None => UnreadCount::AtLeast(0),
                };
                total.include(unread, dm.mentions);
                Some(total)
            }
            _ => None,
        };
    }
    let servers = registry
        .servers
        .iter()
        .map(|server| {
            let id = server.address.id();
            let badge = registry
                .accounts
                .iter()
                .find(|account| account.id.server() == id && account.active)
                .and_then(|account| accounts.get(&account.id).copied().flatten());
            (id, badge)
        })
        .collect();
    BadgeSnapshot {
        accounts,
        servers,
        direct_messages,
    }
}

fn account_totals(image: &AccountImage) -> Option<(BadgeTotals, BadgeTotals)> {
    let resources = &image.resources;
    if resources.server.is_none() || resources.generation != image.reading.generation {
        return None;
    }
    let channels = total_ids(
        resources
            .channels
            .iter()
            .map(|channel| channel.id.as_deref()),
        &image.reading,
    )?;
    let dms = total_ids(
        resources.dms.iter().map(|dm| Some(dm.id.as_str())),
        &image.reading,
    )?;
    Some((channels, dms))
}

fn total_ids<'a>(
    ids: impl Iterator<Item = Option<&'a str>>,
    reading: &ReadingSnapshot,
) -> Option<BadgeTotals> {
    let mut totals = BadgeTotals::empty();
    let mut seen = std::collections::BTreeSet::new();
    for raw in ids {
        let id: Uuid = raw?.parse().ok()?;
        if !seen.insert(id) {
            continue;
        }
        let state = reading.channels.get(&id)?;
        totals.include(state.unread_count, state.mention_count);
    }
    Some(totals)
}

#[cfg(test)]
mod tests;

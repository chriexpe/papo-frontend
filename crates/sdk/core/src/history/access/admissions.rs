//! The per-session admission state machine: which channels are allowed, quarantined or
//! denied, and how one accepted set of decisions moves them. Pure and synchronous; durable
//! revocation, reading fences and resource producers live in `history::access`.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::Uuid;

use crate::{
    history::{HistoryClosedReason, HistoryError, channel::Channel},
    permissions::read_access::{ReadAccess, ReadDecisions},
};

/// What this session currently admits for one channel. A channel with no entry has never
/// been admitted in this session and is `Unverified`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::history) enum Admission {
    Allowed,
    /// Access is unknown: sealed, but cache and reading state are kept for revalidation.
    Quarantined,
    /// Terminally sealed. Until `durable`, the old cache set may still be current on disk,
    /// so a regrant must write the revoke marker before it admits anything.
    Denied {
        reason: HistoryClosedReason,
        durable: bool,
    },
}

#[derive(Default)]
pub(crate) struct Admissions {
    pub(super) channels: BTreeMap<Uuid, Admission>,
}

impl Admissions {
    pub(in crate::history) fn get(&self, channel: Uuid) -> Option<Admission> {
        self.channels.get(&channel).copied()
    }

    pub(crate) fn allowed(&self) -> BTreeSet<Uuid> {
        self.channels
            .iter()
            .filter(|(_, admission)| **admission == Admission::Allowed)
            .map(|(id, _)| *id)
            .collect()
    }

    pub(crate) fn quarantined(&self) -> BTreeSet<Uuid> {
        self.channels
            .iter()
            .filter(|(_, admission)| **admission == Admission::Quarantined)
            .map(|(id, _)| *id)
            .collect()
    }

    /// The admission error for a channel that is not allowed.
    pub(in crate::history) fn refusal(&self, channel: Uuid) -> Option<HistoryError> {
        match self.get(channel) {
            Some(Admission::Allowed) => None,
            Some(Admission::Quarantined) => Some(HistoryError::AccessUnknown),
            Some(Admission::Denied { reason, .. }) => Some(HistoryError::Closed(reason)),
            None => Some(HistoryError::Unverified),
        }
    }

    /// Seal channels that must stop now and return those needing a durable revoke.
    /// Runs synchronously under the channel-map lock, so no open can interleave.
    pub(super) fn seal(
        &mut self,
        decisions: &ReadDecisions,
        incarnations: &BTreeMap<Uuid, std::sync::Arc<Channel>>,
    ) -> Sealed {
        let mut sealed = Sealed::default();
        let known: BTreeSet<Uuid> = self
            .channels
            .keys()
            .chain(incarnations.keys())
            .copied()
            .chain(decisions.with(ReadAccess::Denied))
            .chain(decisions.with(ReadAccess::Unknown))
            .collect();
        for id in known {
            let current = self.get(id);
            let next = match (decisions.get(id), current) {
                // A grant is applied after the resource write, by `grant`.
                (Some(ReadAccess::Allowed), _) => continue,
                // The manifest stops carrying an unlisted channel's set; nothing else to keep.
                (None, Some(Admission::Denied { durable: true, .. })) => {
                    self.channels.remove(&id);
                    continue;
                }
                // A denial stays sealed (retrying an unconfirmed marker) until a later
                // Allowed; Unknown evidence never reopens it.
                (_, Some(denied @ Admission::Denied { .. })) => denied,
                (Some(ReadAccess::Unknown), _) => {
                    if current != Some(Admission::Quarantined) {
                        sealed.quarantined.push(id);
                    }
                    Admission::Quarantined
                }
                (Some(ReadAccess::Denied), _) => Admission::Denied {
                    reason: HistoryClosedReason::PermissionDenied,
                    durable: false,
                },
                (None, _) => Admission::Denied {
                    reason: HistoryClosedReason::ChannelRemoved,
                    durable: false,
                },
            };
            if let Admission::Denied { reason, durable } = next {
                if !durable {
                    sealed.denied.push((id, reason));
                }
                if let Some(channel) = incarnations.get(&id) {
                    channel.close(reason);
                }
            } else if let Some(channel) = incarnations.get(&id) {
                channel.quarantine();
            }
            self.channels.insert(id, next);
        }
        sealed
    }

    /// Mark one channel denied before its durable revoke, for a backend 403/404 or deletion.
    pub(super) fn deny(&mut self, channel: Uuid, reason: HistoryClosedReason) {
        let reason = match self.get(channel) {
            Some(Admission::Denied { reason, .. }) => reason,
            _ => reason,
        };
        self.channels.insert(
            channel,
            Admission::Denied {
                reason,
                durable: false,
            },
        );
    }

    /// Record a confirmed revoke marker. A channel that has left the lists is forgotten.
    pub(super) fn revoked(&mut self, channel: Uuid, listed: bool) {
        if let Some(Admission::Denied { reason, .. }) = self.get(channel) {
            if listed {
                self.channels.insert(
                    channel,
                    Admission::Denied {
                        reason,
                        durable: true,
                    },
                );
            } else {
                self.channels.remove(&channel);
            }
        }
    }
}

/// Channels a seal stopped: newly quarantined, and denials whose marker is not yet durable.
#[derive(Default)]
pub(super) struct Sealed {
    pub quarantined: Vec<Uuid>,
    pub denied: Vec<(Uuid, HistoryClosedReason)>,
}

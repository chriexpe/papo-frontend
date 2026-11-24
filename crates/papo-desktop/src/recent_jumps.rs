//! The last places Ctrl+K jumped to (Christian, 2026-11-23: an empty jump
//! field lists the last three first). Kept in this device's settings file
//! through `rune::prefs`, read at launch and saved on each jump, newest
//! first, each with what it needs to show again (its name) and to go there
//! (its account and target, as plain text). A jump whose account is no
//! longer saved is skipped when read.
//!
//! Owns only the list; what a jump does is `shell::jump`'s.

use papo_sdk::{AccountId, Uuid};
use papo_sdk_ui::{JumpKind, JumpTarget};
use rune::prefs::Prefs;

use crate::device_settings::{Remembered, SettingsFile, Write};

/// The settings key.
const RECENT_JUMPS_KEY: &str = "recent_jumps";
/// How many are kept.
pub(crate) const RECENT_JUMPS: usize = 3;

/// One jump, as remembered. Saved as a list of text fields in this order
/// (an absent one empty), so the settings file needs no schema of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Saved {
    /// The account, as `{server}/{user}`.
    account: String,
    /// `channel`, `direct`, `person` or `server`.
    kind: String,
    /// The channel, conversation or person; none for a server.
    id: Option<Uuid>,
    /// A conversation's other person.
    peer: Option<Uuid>,
    name: String,
    username: Option<String>,
}

/// The last jumps, and where they are saved.
pub(crate) struct RecentJumps {
    saved: Vec<Saved>,
    remembered: Remembered,
}

impl Default for RecentJumps {
    /// None, kept nowhere: before the settings file is read, and in tests.
    fn default() -> RecentJumps {
        RecentJumps::read(&Prefs::memory())
    }
}

impl Saved {
    /// account, kind, id, peer, name, username.
    fn fields(&self) -> Vec<String> {
        let uuid = |id: Option<Uuid>| id.map(|id| id.to_string()).unwrap_or_default();
        vec![
            self.account.clone(),
            self.kind.clone(),
            uuid(self.id),
            uuid(self.peer),
            self.name.clone(),
            self.username.clone().unwrap_or_default(),
        ]
    }

    /// The jump `fields` hold, as [`Saved::fields`] writes them; none when
    /// malformed.
    fn from_fields(fields: &[String]) -> Option<Saved> {
        let [account, kind, id, peer, name, username] = fields else {
            return None;
        };
        let uuid = |text: &str| (!text.is_empty()).then(|| text.parse().ok()).flatten();
        Some(Saved {
            account: account.clone(),
            kind: kind.clone(),
            id: uuid(id),
            peer: uuid(peer),
            name: name.clone(),
            username: (!username.is_empty()).then(|| username.clone()),
        })
    }
}

fn account_text(account: AccountId) -> String {
    format!("{}/{}", account.server(), account.user())
}

impl RecentJumps {
    /// The jumps in `prefs`, this device's settings file; an unreadable
    /// list is empty.
    pub(crate) fn read(prefs: &Prefs) -> RecentJumps {
        let fields: Vec<Vec<String>> = prefs.get(RECENT_JUMPS_KEY).unwrap_or_default();
        RecentJumps {
            saved: fields
                .iter()
                .filter_map(|fields| Saved::from_fields(fields))
                .collect(),
            remembered: Remembered::new(RECENT_JUMPS_KEY, SettingsFile::of(prefs)),
        }
    }

    /// The last jumps, newest first, to accounts among `accounts`.
    pub(crate) fn targets(&self, accounts: &[AccountId]) -> Vec<JumpTarget> {
        self.saved
            .iter()
            .filter_map(|saved| {
                let account = *accounts
                    .iter()
                    .find(|account| account_text(**account) == saved.account)?;
                let kind = match (saved.kind.as_str(), saved.id, saved.peer) {
                    ("channel", Some(channel), _) => JumpKind::Channel { channel },
                    ("direct", Some(channel), Some(peer)) => JumpKind::Direct { channel, peer },
                    ("person", Some(user), _) => JumpKind::Person { user },
                    ("server", _, _) => JumpKind::Server,
                    _ => return None,
                };
                Some(JumpTarget {
                    account,
                    kind,
                    name: saved.name.clone(),
                    username: saved.username.clone(),
                })
            })
            .collect()
    }

    /// `target` was jumped to: it goes first, once, and the list keeps
    /// [`RECENT_JUMPS`]; the write that saves the list, unless one is
    /// running (it then follows). A failed save keeps it for this run.
    pub(crate) fn jumped(&mut self, target: &JumpTarget) -> Option<Write> {
        let (kind, id, peer) = match target.kind {
            JumpKind::Channel { channel } => ("channel", Some(channel), None),
            JumpKind::Direct { channel, peer } => ("direct", Some(channel), Some(peer)),
            JumpKind::Person { user } => ("person", Some(user), None),
            JumpKind::Server => ("server", None, None),
        };
        let saved = Saved {
            account: account_text(target.account),
            kind: kind.to_owned(),
            id,
            peer,
            name: target.name.clone(),
            username: target.username.clone(),
        };
        self.saved.retain(|known| {
            (&known.account, &known.kind, known.id) != (&saved.account, &saved.kind, saved.id)
        });
        self.saved.insert(0, saved);
        self.saved.truncate(RECENT_JUMPS);
        let fields: Vec<Vec<String>> = self.saved.iter().map(Saved::fields).collect();
        self.remembered.remember(serde_json::json!(fields))
    }

    /// The running write landed, or did not; the one to run next.
    pub(crate) fn written(&mut self, ok: bool) -> Option<Write> {
        self.remembered.written(ok)
    }
}

#[cfg(test)]
mod tests {
    use papo_sdk::{AccountId, ServerAddress, Uuid};
    use papo_sdk_ui::{JumpKind, JumpTarget};
    use rune::prefs::Prefs;

    use super::{RECENT_JUMPS, RecentJumps};

    fn account() -> AccountId {
        AccountId::new(
            ServerAddress::parse("https://lumen.example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(1),
        )
    }

    fn channel(n: u128) -> JumpTarget {
        JumpTarget {
            account: account(),
            kind: JumpKind::Channel {
                channel: Uuid::from_u128(n),
            },
            name: format!("c{n}"),
            username: None,
        }
    }

    #[test]
    fn the_last_three_jumps_newest_first_each_once_and_read_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let mut recent = RecentJumps::read(&Prefs::at(&path));
        for n in [1, 2, 3, 2, 4] {
            if let Some(write) = recent.jumped(&channel(n)) {
                write.run().unwrap();
            }
            while let Some(next) = recent.written(true) {
                next.run().unwrap();
            }
        }
        let names: Vec<_> = recent
            .targets(&[account()])
            .into_iter()
            .map(|target| target.name)
            .collect();
        assert_eq!(names, ["c4", "c2", "c3"]);
        assert_eq!(names.len(), RECENT_JUMPS);
        let again = RecentJumps::read(&Prefs::at(&path));
        assert_eq!(
            again.targets(&[account()]).len(),
            3,
            "read back next launch"
        );
        assert!(
            again.targets(&[]).is_empty(),
            "a removed account's are skipped"
        );
    }

    #[test]
    fn a_malformed_row_or_a_list_of_another_shape_reads_as_none() {
        let mut prefs = Prefs::memory();
        prefs
            .set(
                super::RECENT_JUMPS_KEY,
                &vec![vec!["only", "three", "fields"]],
            )
            .unwrap();
        assert_eq!(RecentJumps::read(&prefs).targets(&[account()]).len(), 0);
        let mut prefs = Prefs::memory();
        prefs.set(super::RECENT_JUMPS_KEY, &42).unwrap();
        assert_eq!(RecentJumps::read(&prefs).targets(&[account()]).len(), 0);
    }
}

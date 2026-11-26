//! The account's custom emoji catalog as message content resolves against it: the names to
//! match and the revision that tells a stale parse from a current one. The catalog itself is
//! `papo_sdk::EmojiCatalogService`'s.

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{EmojiCatalog, EmojiCatalogUpdate, Uuid};

use super::{CustomEmojiRef, MessageContent, parser};

/// What `:name:` resolves to for one account. The default resolves nothing (no catalog yet).
#[derive(Clone, Debug, Default)]
pub struct CustomEmojis {
    revision: u64,
    refs: Arc<[CustomEmojiRef]>,
    /// Where each emoji is in `refs`, built once per catalog.
    index: Arc<Index>,
}

/// The ways into a catalog's emoji. One `Arc` for both keeps `CustomEmojis` small: clients hold
/// one in every timeline window and search.
#[derive(Debug, Default)]
struct Index {
    /// Each name's first emoji: a timeline resolves every `:candidate:` of thousands of messages
    /// against up to `MAX_CUSTOM_EMOJI` names.
    names: HashMap<String, usize>,
    /// Each ID's emoji: a reaction names its emoji by ID, and a row of chips asks for each.
    ids: HashMap<Uuid, usize>,
}

impl PartialEq for CustomEmojis {
    /// The same emoji: the index follows from them.
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision && self.refs == other.refs
    }
}

impl Eq for CustomEmojis {}

/// How the parser finds a `:name:`: by a scan of a caller's list, or through a catalog's index.
#[derive(Clone, Copy)]
pub(crate) enum EmojiNames<'a> {
    List(&'a [CustomEmojiRef]),
    Index(&'a CustomEmojis),
}

impl<'a> EmojiNames<'a> {
    pub(crate) fn is_empty(self) -> bool {
        match self {
            EmojiNames::List(list) => list.is_empty(),
            EmojiNames::Index(emojis) => emojis.refs.is_empty(),
        }
    }

    /// The emoji named `name` (without colons); the first, should a name repeat.
    pub(crate) fn find(self, name: &str) -> Option<&'a CustomEmojiRef> {
        match self {
            EmojiNames::List(list) => list.iter().find(|entry| entry.name == name),
            EmojiNames::Index(emojis) => emojis.index.names.get(name).map(|at| &emojis.refs[*at]),
        }
    }
}

impl CustomEmojis {
    /// The emoji of `catalog`. An empty catalog resolves nothing, as no catalog does, so both
    /// share their parses.
    #[must_use]
    pub fn new(catalog: &EmojiCatalog) -> Self {
        if catalog.emojis.is_empty() {
            return Self::default();
        }
        let refs: Arc<[CustomEmojiRef]> = catalog
            .emojis
            .iter()
            .map(|emoji| CustomEmojiRef {
                id: emoji.id,
                name: emoji.name.clone(),
            })
            .collect();
        let mut index = Index {
            names: HashMap::with_capacity(refs.len()),
            ids: HashMap::with_capacity(refs.len()),
        };
        for (at, entry) in refs.iter().enumerate() {
            index.names.entry(entry.name.clone()).or_insert(at);
            index.ids.entry(entry.id).or_insert(at);
        }
        Self {
            revision: catalog.revision,
            refs,
            index: Arc::new(index),
        }
    }

    /// The emoji of a catalog update; none for an unreadable or closed catalog. A client that
    /// already resolves against a catalog keeps it through those instead.
    #[must_use]
    pub fn of_update(update: &EmojiCatalogUpdate) -> Self {
        match update {
            EmojiCatalogUpdate::Ready(catalog) => Self::new(catalog),
            _ => Self::default(),
        }
    }

    /// The revision [`CustomEmojis::new`] would give `catalog`, without building it.
    #[must_use]
    pub fn revision_of(catalog: &EmojiCatalog) -> u64 {
        if catalog.emojis.is_empty() {
            0
        } else {
            catalog.revision
        }
    }

    /// The catalog revision these emoji came from: content parsed against another revision is
    /// parsed again.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The emoji named `name` (without colons), through the catalog's index; the first,
    /// should a name repeat.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&CustomEmojiRef> {
        EmojiNames::Index(self).find(name)
    }

    /// The emoji with backend ID `id`, through the catalog's index; none when the server no
    /// longer has it.
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&CustomEmojiRef> {
        self.index.ids.get(&id).map(|at| &self.refs[*at])
    }

    /// The emoji, in the catalog's order.
    #[must_use]
    pub fn refs(&self) -> &[CustomEmojiRef] {
        &self.refs
    }

    /// Parses `source`, resolving `:name:` against these emoji.
    #[must_use]
    pub fn parse(&self, source: &str) -> MessageContent {
        parser::parse(source, EmojiNames::Index(self))
    }
}

#[cfg(test)]
mod tests {
    use papo_sdk::{CustomEmoji, EmojiCatalog, Uuid};

    use super::CustomEmojis;

    fn emoji(id: u128, name: &str) -> CustomEmoji {
        CustomEmoji {
            id: Uuid::from_u128(id),
            name: name.to_owned(),
            format: "png".to_owned(),
            created_at: String::new(),
        }
    }

    #[test]
    fn an_emoji_is_found_by_its_name_and_by_its_id() {
        let catalog = EmojiCatalog {
            revision: 4,
            emojis: vec![emoji(1, "gato"), emoji(2, "cafe")],
        };
        let emojis = CustomEmojis::new(&catalog);
        assert_eq!(
            emojis.find("cafe").map(|found| found.id),
            Some(Uuid::from_u128(2))
        );
        assert_eq!(
            emojis
                .get(Uuid::from_u128(1))
                .map(|found| found.name.as_str()),
            Some("gato")
        );
        assert_eq!(emojis.get(Uuid::from_u128(3)), None);
        assert_eq!(CustomEmojis::default().get(Uuid::from_u128(1)), None);
    }
}

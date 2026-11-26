//! The emoji picker's Emoji tab (`docs/design.md` §16.1, §16.2, §16.5): what it is open for (the
//! field, or a reaction to one message), its search, the cell the keyboard is on, the emoji under
//! the pointer, and the rows it lists: Frequently used, this server's emoji and the Unicode
//! categories, or a search's matches, nine to a row under their headings.
//!
//! Pure: the rows are built from what the shell hands in (the account's catalog, its emoji count
//! as it stood when the picker opened, the SDK's search), and the keys move a cursor over them.
//! Opening, closing, picking and drawing are the shell's and `view`'s.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use papo_sdk::{CustomEmoji, EmojiCatalog, Uuid};
use papo_sdk_ui::operations::ReactionEmoji;
use papo_sdk_ui::{
    EmojiCategory, EmojiSuggestion, UnicodeEmoji, emoji_search, frequent_emoji, unicode_emoji,
    unicode_emoji_of,
};
use rune::TextEdit;

use crate::timeline::TimelineKey;

mod keys;
pub(crate) mod view;

pub(crate) use keys::Step;

/// Cells to a row (§16.2).
pub(crate) const COLUMNS: usize = 9;
/// The space between cells, in points (§16.2).
pub(crate) const CELL_GAP_PT: f32 = 2.0;
/// Frequently used's cells: two rows.
pub(crate) const FREQUENT: usize = 18;
/// The most a search lists.
const MAX_RESULTS: usize = 450;

/// One emoji a cell offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EmojiChoice {
    /// This server's.
    Custom(CustomEmoji),
    /// A Unicode emoji.
    Unicode(&'static UnicodeEmoji),
}

impl EmojiChoice {
    /// Its cell's element key in `section`, by the emoji (so a cell keeps its identity as rows
    /// change; Frequently used holds emoji another section holds too).
    pub(crate) fn key(&self, section: Section) -> String {
        let section = section_key(section);
        match self {
            EmojiChoice::Custom(emoji) => format!("emoji-cell-{section}-{}", emoji.id),
            EmojiChoice::Unicode(emoji) => format!("emoji-cell-{section}-{}", emoji.code),
        }
    }

    /// Its code with colons, as the preview bar shows it: `:gato:`, `:face_with_tears_of_joy:`.
    pub(crate) fn code(&self) -> String {
        match self {
            EmojiChoice::Custom(emoji) => format!(":{}:", emoji.name),
            EmojiChoice::Unicode(emoji) => format!(":{}:", emoji.code),
        }
    }

    /// The reaction it makes.
    pub(crate) fn reaction(&self) -> ReactionEmoji {
        match self {
            EmojiChoice::Custom(emoji) => ReactionEmoji::Custom(emoji.id),
            EmojiChoice::Unicode(emoji) => ReactionEmoji::Unicode(emoji.glyph.to_owned()),
        }
    }

    /// The choice an emoji the account used is, when it still exists.
    fn of_reaction(emoji: &ReactionEmoji, catalog: Option<&EmojiCatalog>) -> Option<EmojiChoice> {
        match emoji {
            ReactionEmoji::Custom(id) => catalog?
                .get(*id)
                .map(|emoji| EmojiChoice::Custom(emoji.clone())),
            ReactionEmoji::Unicode(glyph) => unicode_emoji_of(glyph).map(EmojiChoice::Unicode),
        }
    }
}

/// What a pick does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickerFor {
    /// Inserts at the field's caret in this conversation.
    Field(TimelineKey),
    /// Reacts to `message` in this conversation (More emoji, §10): the tab opens alone.
    Reaction { key: TimelineKey, message: Uuid },
}

impl PickerFor {
    /// The conversation it is open in.
    pub(crate) fn key(self) -> TimelineKey {
        match self {
            PickerFor::Field(key) | PickerFor::Reaction { key, .. } => key,
        }
    }
}

/// A run of cells' heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Section {
    /// Your most used.
    Frequent,
    /// This server's custom emoji (in a search, its matches).
    Server,
    /// A Unicode category.
    Category(EmojiCategory),
    /// A search's Unicode matches.
    Unicode,
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    /// A section's heading, and how many it holds (shown for this server's and a search's).
    Heading { section: Section, count: usize },
    /// Up to [`COLUMNS`] cells: the indices of their emoji in [`EmojiPicker::cells`].
    Cells { section: Section, cells: Vec<usize> },
}

/// A section's short key, for element keys and jumps.
pub(crate) fn section_key(section: Section) -> String {
    match section {
        Section::Frequent => "frequent".to_owned(),
        Section::Server => "server".to_owned(),
        Section::Unicode => "unicode".to_owned(),
        Section::Category(category) => format!("{category:?}").to_lowercase(),
    }
}

/// The element key of section `section`'s row in the list: what the strip jumps to.
pub(crate) fn block_key(section: Section) -> String {
    format!("emoji-section-{}", section_key(section))
}

/// The element key of section `section`'s list of rows of cells.
pub(crate) fn block_rows_key(section: Section) -> String {
    format!("{}-rows", block_key(section))
}

/// The element key of row `row` of `section`'s cells (counted from its first row of cells).
pub(crate) fn cell_row_key(section: Section, row: usize) -> String {
    format!("emoji-row-{}-{row}", section_key(section))
}

/// One section as the list draws it: its heading and its rows of cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) section: Section,
    /// How many emoji it holds.
    pub(crate) count: usize,
    /// Its rows in [`EmojiPicker::rows`], heading included.
    pub(crate) rows: Range<usize>,
}

impl Block {
    /// Its rows of cells in [`EmojiPicker::rows`]: all but the heading.
    pub(crate) fn cell_rows(&self) -> Range<usize> {
        self.rows.start + 1..self.rows.end
    }
}

/// The open picker.
pub(crate) struct EmojiPicker {
    pub(crate) target: PickerFor,
    /// Its search field.
    pub(crate) search: TextEdit,
    /// The keyboard's cell (an index into `cells`); none while the keyboard is in the search.
    pub(crate) cursor: Option<usize>,
    /// The emoji under the pointer, for the preview bar.
    pub(crate) hovered: Option<usize>,
    /// The sections the list shows, by index in `blocks`.
    shown: Range<usize>,
    /// Each section's rows of cells in view, as its own list last said (counted from its first
    /// row of cells). Only the custom emoji of shown sections' rows are fetched, so a server's
    /// thousands are not. Kept across a rebuild: Rune says again only when what it shows changes.
    rows_shown: HashMap<Section, Range<usize>>,
    /// The account's emoji count when the picker opened: Frequently used holds still while it
    /// is open, so a pick never moves a cell under the pointer.
    usage: Arc<[(String, u64)]>,
    cells: Vec<EmojiChoice>,
    rows: Vec<Row>,
    blocks: Vec<Block>,
    /// Bumped when the rows are rebuilt: the virtual list's version.
    pub(crate) version: u64,
}

impl EmojiPicker {
    /// A picker for `target`, listing the account's `catalog` and its emoji count `usage`.
    pub(crate) fn new(
        target: PickerFor,
        catalog: Option<&EmojiCatalog>,
        usage: Arc<[(String, u64)]>,
    ) -> EmojiPicker {
        let mut picker = EmojiPicker {
            target,
            search: TextEdit::new(""),
            cursor: None,
            hovered: None,
            shown: 0..0,
            rows_shown: HashMap::new(),
            usage,
            cells: Vec::new(),
            rows: Vec::new(),
            blocks: Vec::new(),
            version: 0,
        };
        picker.rebuild(catalog);
        picker
    }

    /// The emoji the list holds, in order.
    pub(crate) fn cells(&self) -> &[EmojiChoice] {
        &self.cells
    }

    /// The rows the list shows.
    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The sections the list shows, in order.
    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The list shows sections `range`. Whether that changed.
    pub(crate) fn sections_shown(&mut self, range: Range<usize>) -> bool {
        if self.shown == range {
            return false;
        }
        self.shown = range;
        true
    }

    /// Section `section`'s list shows its rows of cells `range`. Whether that changed.
    pub(crate) fn rows_shown(&mut self, section: Section, range: Range<usize>) -> bool {
        if self.rows_shown.get(&section) == Some(&range) {
            return false;
        }
        self.rows_shown.insert(section, range);
        true
    }

    /// The custom emoji on screen: what is fetched for the picker.
    pub(crate) fn shown_custom(&self) -> Vec<Uuid> {
        let mut ids = Vec::new();
        for block in self.blocks.get(self.shown.clone()).unwrap_or_default() {
            let Some(range) = self.rows_shown.get(&block.section) else {
                continue;
            };
            let first = block.cell_rows().start;
            let rows = first + range.start..(first + range.end).min(block.rows.end);
            for row in self.rows.get(rows).unwrap_or_default() {
                if let Row::Cells { cells, .. } = row {
                    ids.extend(cells.iter().filter_map(|at| match self.cells.get(*at) {
                        Some(EmojiChoice::Custom(emoji)) => Some(emoji.id),
                        _ => None,
                    }));
                }
            }
        }
        ids
    }

    /// The search as typed, trimmed; empty without a search. The SDK folds it as it searches.
    pub(crate) fn query(&self) -> &str {
        self.search.text.trim()
    }

    /// The emoji the keyboard is on.
    pub(crate) fn chosen(&self) -> Option<&EmojiChoice> {
        self.cursor.and_then(|at| self.cells.get(at))
    }

    /// The emoji the preview bar names: the keyboard's, else the pointer's.
    pub(crate) fn previewed(&self) -> Option<&EmojiChoice> {
        self.chosen()
            .or_else(|| self.hovered.and_then(|at| self.cells.get(at)))
    }

    /// Lists again for the search as it stands and `catalog`; the keyboard goes back to the
    /// search. What is on screen stays as the lists last said, as they say again only when
    /// their rows change.
    pub(crate) fn rebuild(&mut self, catalog: Option<&EmojiCatalog>) {
        self.cells.clear();
        self.rows.clear();
        self.blocks.clear();
        self.cursor = None;
        self.hovered = None;
        self.version = self.version.wrapping_add(1);
        let custom = catalog.map_or(&[][..], |catalog| catalog.emojis.as_slice());
        let query = self.search.text.trim().to_owned();
        if query.is_empty() {
            let frequent: Vec<EmojiChoice> = frequent_emoji(&self.usage, catalog, FREQUENT)
                .iter()
                .filter_map(|emoji| EmojiChoice::of_reaction(emoji, catalog))
                .collect();
            self.section(Section::Frequent, frequent);
            self.section(
                Section::Server,
                custom.iter().cloned().map(EmojiChoice::Custom).collect(),
            );
            for category in EmojiCategory::ALL {
                let emoji = unicode_emoji()
                    .iter()
                    .filter(|emoji| emoji.category == category)
                    .map(EmojiChoice::Unicode)
                    .collect();
                self.section(Section::Category(category), emoji);
            }
        } else {
            let (server, unicode): (Vec<EmojiChoice>, Vec<EmojiChoice>) =
                emoji_search(&query, custom, MAX_RESULTS)
                    .into_iter()
                    .map(|found| match found {
                        EmojiSuggestion::Custom(emoji) => EmojiChoice::Custom(emoji),
                        EmojiSuggestion::Unicode(emoji) => EmojiChoice::Unicode(emoji),
                    })
                    .partition(|choice| matches!(choice, EmojiChoice::Custom(_)));
            self.section(Section::Server, server);
            self.section(Section::Unicode, unicode);
        }
    }

    /// Appends `section`'s heading and rows; an empty section shows nothing.
    fn section(&mut self, section: Section, emoji: Vec<EmojiChoice>) {
        if emoji.is_empty() {
            return;
        }
        let first_row = self.rows.len();
        let count = emoji.len();
        self.rows.push(Row::Heading { section, count });
        let first = self.cells.len();
        self.cells.extend(emoji);
        let indices: Vec<usize> = (first..self.cells.len()).collect();
        for chunk in indices.chunks(COLUMNS) {
            self.rows.push(Row::Cells {
                section,
                cells: chunk.to_vec(),
            });
        }
        self.blocks.push(Block {
            section,
            count,
            rows: first_row..self.rows.len(),
        });
    }

    /// The sections listed, in order: what the category strip offers.
    pub(crate) fn sections(&self) -> Vec<Section> {
        self.blocks.iter().map(|block| block.section).collect()
    }
}

#[cfg(test)]
mod tests;

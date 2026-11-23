//! A message's body (`docs/design.md` §9): the SDK's parsed content drawn
//! as blocks of selectable rich text, with spoilers, highlighted code, mentions,
//! links, tables, jumbo emoji and the "(edited)" marker.
//!
//! `doc` lays the content out, purely; `blocks`, `inline`, `code` and `table`
//! draw it; `copy` turns a selection back into text from the same layout.
//! What a press asks for (open a link, reveal a spoiler, copy code) is the
//! host's: the view only names it.

mod blocks;
mod code;
pub(crate) mod copy;
mod doc;
mod emoji;
mod inline;
pub(crate) mod kept;
mod table;
#[cfg(test)]
mod tests;

use papo_sdk::Uuid;
use rune::{El, col, text};

pub(crate) use self::doc::{Context, Doc, mention_names};
use self::inline::Trailer;
use super::{TimelineHost, TimelineKey};
use crate::design::{MESSAGE_BLOCK_GAP_PT, Palette, TYPE_CAPTION};
use crate::strings;

/// What a press inside a message's content asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Press {
    /// A safe link (`http`, `https` or `mailto`).
    Url(String),
    /// Spoiler `index` of the message, to reveal.
    Spoiler(u32),
    /// The "(edited)" marker, which only explains itself.
    Edited,
    /// A person's mention: their card.
    Person(Uuid),
}

impl Press {
    /// The press as a Rune link id. A link's id is its address, which always
    /// has a scheme; the others are marked apart from any.
    fn id(&self) -> String {
        match self {
            Press::Url(url) => url.clone(),
            Press::Spoiler(index) => format!("#spoiler/{index}"),
            Press::Edited => "#edited".to_owned(),
            Press::Person(user) => format!("#person/{user}"),
        }
    }

    fn parse(id: &str) -> Option<Press> {
        if id == "#edited" {
            return Some(Press::Edited);
        }
        if let Some(index) = id.strip_prefix("#spoiler/") {
            return index.parse().ok().map(Press::Spoiler);
        }
        if let Some(user) = id.strip_prefix("#person/") {
            return Uuid::parse_str(user).ok().map(Press::Person);
        }
        Some(Press::Url(id.to_owned()))
    }
}

/// What drawing one message's content reads besides the content.
pub(crate) struct Draw<'a> {
    pub(crate) palette: &'a Palette,
    pub(crate) key: TimelineKey,
    pub(crate) message: Uuid,
    /// The code block, by its text's index, whose Copy reads "Copied".
    pub(crate) copied: Option<usize>,
    /// Set when the message was edited.
    pub(crate) edited: Option<Edited>,
}

/// A message was edited.
pub(crate) struct Edited {
    /// When, as the marker's hint says it; none when the time does not read.
    pub(crate) hint: Option<String>,
}

/// The body of a message laid out as `doc`.
pub(crate) fn body<S: TimelineHost>(doc: &Doc, draw: &Draw<'_>) -> El {
    let palette = draw.palette;
    let marker = draw.edited.as_ref().map(|edited| Trailer {
        words: strings::edited().to_owned(),
        font: TYPE_CAPTION,
        ink: palette.text_muted,
        help: edited.hint.clone(),
    });
    let inline_marker = marker.as_ref().filter(|_| doc.ends_in_text());
    let mut body = col()
        .key("body")
        .w_full()
        .gap(MESSAGE_BLOCK_GAP_PT)
        .children(blocks::nodes::<S>(
            &doc.nodes,
            draw,
            palette.text_primary,
            inline_marker,
        ));
    // After a code block, a table, a list or a quote, the marker takes a
    // line of its own.
    if let Some(marker) = marker.filter(|_| !doc.ends_in_text()) {
        let mut edited = text(marker.words)
            .key("edited")
            .font(marker.font)
            .color(marker.ink)
            .role("note");
        if let Some(help) = marker.help {
            edited = edited.label(help.clone()).focusable(true).help(help);
        }
        body = body.child(edited);
    }
    body
}

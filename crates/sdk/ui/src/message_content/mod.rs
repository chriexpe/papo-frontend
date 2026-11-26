//! Framework-neutral message content parser, mention Layer 2 placement and pure composer helpers.
//!
//! Parsing `CommonMark` (+ tables, strikethrough, task lists, spoilers) and composer utilities
//! live here, keeping display names, resolved colours, media widgets and `UniFFI` shapes in clients.
//! Canonical mention token syntax lives exclusively in [`papo_sdk::mentions`].

mod composer_mentions;
mod custom_emojis;
mod fences;
mod model;
mod pair_closing;
mod pair_markers;
mod parser;
mod preprocess;
mod preview;
mod selection_format;
mod spoiler;
mod tokens;

#[cfg(test)]
mod composer_tests;
#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod token_tests;

pub use composer_mentions::{
    ComposerMentionBinding, DecodedComposerMentions, decode_composer_mentions,
    encode_composer_mentions, utf8_offset_to_utf16, utf8_range_to_utf16, utf16_offset_to_utf8,
    utf16_range_to_utf8,
};
pub use custom_emojis::CustomEmojis;
pub use fences::normalize_fenced_code_blocks;
pub use model::{
    Block, CustomEmojiRef, Inline, ListItem, MAX_NESTING_DEPTH, MessageContent, TableAlignment,
    TextStyle,
};
pub use pair_closing::{
    PairClosing, PairEdit, TextChangeSource, on_pair_change, skip_pair_closer, track_pair_change,
};
pub use preprocess::{is_plain, is_safe_href};
pub use preview::{PreviewSpan, PreviewSpanKind, preview_spans, preview_spans_with_bindings};
pub use selection_format::{
    InvalidSelection, SelectionEdit, SelectionFormat, apply_selection_format,
};

//! Pure UTF-16 composer formatting, auto-pair closing, and spoiler filename helpers.
//! Mention binding and syntax preview span helpers live in `composer_tokens.rs`.

use std::sync::{Arc, Mutex};

use papo_sdk_ui::message_content::{
    PairClosing, PairEdit, SelectionFormat, TextChangeSource, apply_selection_format,
    is_spoiler_filename, normalize_fenced_code_blocks, on_pair_change, skip_pair_closer,
    track_pair_change, utf16_offset_to_utf8, utf16_range_to_utf8, with_spoiler_prefix,
    without_spoiler_prefix,
};

use crate::errors::FfiClientError;

/// Normalizes unmatched/inline fenced code block boundaries per shared composer rules.
#[uniffi::export]
#[must_use]
pub fn normalize_composer_fenced_code_blocks(content: String) -> String {
    normalize_fenced_code_blocks(&content)
}

/// Checks whether a filename carries the shared `SPOILER_` visibility prefix.
#[uniffi::export]
#[must_use]
pub fn is_spoiler_attachment_filename(filename: String) -> bool {
    is_spoiler_filename(&filename)
}

/// Applies the `SPOILER_` prefix idempotently without losing basename characters.
#[uniffi::export]
#[must_use]
pub fn with_spoiler_attachment_prefix(filename: String) -> String {
    with_spoiler_prefix(&filename)
}

/// Strips one `SPOILER_` prefix when present.
#[uniffi::export]
#[must_use]
pub fn without_spoiler_attachment_prefix(filename: String) -> String {
    without_spoiler_prefix(&filename).to_owned()
}

/// UTF-16 code-unit composer selection edit request format kind.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiSelectionFormat {
    /// Wrap/unwrap bold markers (`**...**`).
    Bold,
    /// Wrap/unwrap italic markers (`*...*`).
    Italic,
    /// Wrap/unwrap strikethrough markers (`~~...~~`).
    Strikethrough,
    /// Wrap/unwrap inline or multiline fenced code markers.
    Code,
    /// Wrap/unwrap spoiler markers (`||...||`).
    Spoiler,
    /// Wrap/unwrap Markdown link (`[label](url)`), placing caret inside `()` when `url` is `None`.
    Link {
        /// Optional destination URL to insert inside `(...)`.
        url: Option<String>,
    },
}

impl From<FfiSelectionFormat> for SelectionFormat {
    fn from(value: FfiSelectionFormat) -> Self {
        match value {
            FfiSelectionFormat::Bold => Self::Bold,
            FfiSelectionFormat::Italic => Self::Italic,
            FfiSelectionFormat::Strikethrough => Self::Strikethrough,
            FfiSelectionFormat::Code => Self::Code,
            FfiSelectionFormat::Spoiler => Self::Spoiler,
            FfiSelectionFormat::Link { url } => Self::Link { url },
        }
    }
}

/// Result of a UTF-16-indexed composer text edit (`text` + replacement cursor/selection range).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTextSelectionEdit {
    /// Replacement full draft string.
    pub text: String,
    /// New UTF-16 selection start.
    pub selection_start_utf16: u32,
    /// New UTF-16 selection end.
    pub selection_end_utf16: u32,
}

/// Applies a Markdown formatting action over UTF-16 selection offsets safely.
///
/// # Errors
/// Returns [`FfiClientError::InvalidArgument`] if UTF-16 offsets split surrogate pairs or exceed text length.
#[uniffi::export]
pub fn apply_composer_selection_format(
    text: String,
    selection_start_utf16: u32,
    selection_end_utf16: u32,
    format: FfiSelectionFormat,
) -> Result<FfiTextSelectionEdit, FfiClientError> {
    let range = utf16_range_to_utf8(
        &text,
        &(selection_start_utf16 as usize..selection_end_utf16 as usize),
    )
    .ok_or(FfiClientError::InvalidArgument)?;
    let sdk_format: SelectionFormat = format.into();
    let edit = apply_selection_format(&text, range, &sdk_format)
        .map_err(|_| FfiClientError::InvalidArgument)?;
    Ok(FfiTextSelectionEdit {
        text: edit.text,
        selection_start_utf16: u32::try_from(edit.utf16_selection.start)
            .map_err(|_| FfiClientError::IndexOutOfRange)?,
        selection_end_utf16: u32::try_from(edit.utf16_selection.end)
            .map_err(|_| FfiClientError::IndexOutOfRange)?,
    })
}

/// Opaque auto-paired delimiter tracker held by Kotlin composer adapters without duplicating stack state.
#[derive(Default, uniffi::Object)]
pub struct FfiPairClosingState {
    inner: Mutex<Option<PairClosing>>,
}

#[uniffi::export]
impl FfiPairClosingState {
    /// Creates a fresh empty pair-closing stack for one composer instance.
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Clears all pending auto-inserted closing delimiters.
    pub fn reset(&self) {
        *self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// True while an auto-inserted closing delimiter frame remains active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }
}

/// Origin of a composer text edit matching `papo_sdk_ui::message_content::TextChangeSource`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTextChangeSource {
    /// Direct single-character user keystroke or backspace.
    Typing,
    /// Clipboard paste.
    Paste,
    /// IME composition change or commit.
    ImeCommit,
    /// Programmatic replacement (draft restore, mention insertion, undo).
    Programmatic,
}

impl From<FfiTextChangeSource> for TextChangeSource {
    fn from(value: FfiTextChangeSource) -> Self {
        match value {
            FfiTextChangeSource::Typing => Self::Typing,
            FfiTextChangeSource::Paste => Self::Paste,
            FfiTextChangeSource::ImeCommit => Self::ImeCommit,
            FfiTextChangeSource::Programmatic => Self::Programmatic,
        }
    }
}

/// Resulting buffer and UTF-16 caret emitted by pair-closing edits.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPairEdit {
    /// Updated composer text.
    pub text: String,
    /// Caret offset in UTF-16 code units.
    pub caret_utf16: u32,
}

/// Maps `PairEdit` into UTF-16 FFI record, returning `None` if index overflows `u32`.
fn map_pair_edit(edit: PairEdit) -> Option<FfiPairEdit> {
    Some(FfiPairEdit {
        text: edit.text,
        caret_utf16: u32::try_from(edit.utf16_caret).ok()?,
    })
}

/// Applies shared auto-pair closing logic over UTF-16 text/caret inputs, updating `state`.
#[uniffi::export]
#[must_use]
pub fn composer_on_pair_change(
    previous_text: String,
    next_text: String,
    caret_utf16: u32,
    source: FfiTextChangeSource,
    state: Arc<FfiPairClosingState>,
) -> Option<FfiPairEdit> {
    let caret_utf8 = utf16_offset_to_utf8(&next_text, caret_utf16 as usize)?;
    let mut guard = state
        .inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    on_pair_change(
        &previous_text,
        &next_text,
        caret_utf8,
        source.into(),
        &mut guard,
    )
    .and_then(map_pair_edit)
}

/// Tracks non-typing text mutations (paste, IME) in `state` without auto-closing delimiters.
#[uniffi::export]
pub fn composer_track_pair_change(
    previous_text: String,
    next_text: String,
    state: Arc<FfiPairClosingState>,
) {
    let mut guard = state
        .inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    track_pair_change(&previous_text, &next_text, &mut guard);
}

/// Jumps the caret past the innermost active pending closer in `state` (typically bound to Tab).
#[uniffi::export]
#[must_use]
pub fn composer_skip_pair_closer(
    text: String,
    caret_utf16: u32,
    state: Arc<FfiPairClosingState>,
) -> Option<FfiPairEdit> {
    let caret_utf8 = utf16_offset_to_utf8(&text, caret_utf16 as usize)?;
    let mut guard = state
        .inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    skip_pair_closer(&text, caret_utf8, &mut guard).and_then(map_pair_edit)
}

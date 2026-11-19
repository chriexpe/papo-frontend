//! The root view: the conversation surface with the composer pill.
//!
//! Owns only presentation state (the composer's edit buffer and focus). There
//! is no server, account or message state yet; when there is, it comes from
//! the SDK and is never copied here. Sending is not wired: the field keeps
//! what is typed.

use rune::{Cx, El, TextEdit, col, row, text_input};

use crate::design::{
    COMPOSER_HEIGHT_PT, PILL_OUTLINE_PT, Palette, RADIUS_COMPOSER_PT, SPACE_LG_PT, TYPE_BODY, Theme,
};
use crate::strings;

/// The element key of the composer's text field.
pub(crate) const COMPOSER_KEY: &str = "composer";

/// The root view's state.
pub(crate) struct Shell {
    composer: TextEdit,
}

impl Default for Shell {
    /// The composer starts focused, so typing goes to it without a click.
    fn default() -> Self {
        let mut composer = TextEdit::new("");
        composer.focused = true;
        Shell { composer }
    }
}

/// Draws the shell. Pure: it reads state and the system's light or dark
/// preference, and changes nothing.
pub(crate) fn view(shell: &Shell, cx: &mut Cx<Shell>) -> El {
    let palette = Palette::new(Theme::following_system(cx.system_dark()));
    let field = text_input(&shell.composer, TYPE_BODY)
        .key(COMPOSER_KEY)
        .label(strings::composer_label())
        .single_line()
        .grow(1.0)
        .color(palette.text_primary)
        .placeholder(strings::composer_placeholder(), palette.text_muted)
        .caret(palette.accent, palette.text_selection)
        .on_edit(|shell: &mut Shell| &mut shell.composer);
    let pill = row()
        .key("composer-pill")
        .w_full()
        .h(COMPOSER_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .center_y()
        .radius(RADIUS_COMPOSER_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .child(field);
    col()
        .w_full()
        .h_full()
        .justify_end()
        .padding(SPACE_LG_PT)
        .fill(palette.surface_content)
        .child(pill)
}

#[cfg(test)]
mod tests;

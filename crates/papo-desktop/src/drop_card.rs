//! Files dragged over the window (`docs/design.md` §14.1, way A): the
//! whole window dims as behind a popup, and one card sits in the middle of
//! the conversation, naming where a drop goes and the SDK's limits. A drop
//! anywhere, rail and channel list included, attaches to that conversation.
//! While a message is edited the card says the edit must end first, and a
//! drop takes nothing.
//!
//! Pure: what is drawn while a drag is over; taking the drop is the
//! shell's (`shell/attaching.rs`).

use rune::{Color, El, col, el, icon, text};

use crate::design::{
    DROP_CARD_PAD_PT, DROP_DIM_ALPHA, DROP_EDGE_PT, DROP_ICON_PT, Look, RADIUS_PILL, SPACE_SM,
    TYPE_BODY_STRONG, TYPE_SMALL,
};
use crate::icons::PAPERCLIP;
use crate::strings;

/// What the card says.
pub(crate) enum DropTarget {
    /// Files go to `name` ("#general", or the person in a DM).
    Conversation(String),
    /// A message is being edited: nothing is taken.
    Editing,
}

/// The dimmed window, over everything.
pub(crate) fn dim() -> El {
    el().key("drop-dim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(DROP_DIM_ALPHA))
        .a11y_hidden()
}

/// The card, centred in `area` (the conversation's rect: left, top,
/// width, height, in window points).
pub(crate) fn card(target: &DropTarget, area: [f32; 4], look: &Look) -> El {
    let [left, top, width, height] = area;
    let mut card = col()
        .key("drop-card")
        .items_center()
        .gap(look.space(SPACE_SM))
        .px(DROP_CARD_PAD_PT.0)
        .py(DROP_CARD_PAD_PT.1)
        .radius(look.radius(RADIUS_PILL))
        // Solid, not glass: a glass surface draws its own outline over the
        // dashed edge, and the dimmed window behind leaves nothing to see
        // through.
        .fill(look.palette.surface_pill)
        .border(DROP_EDGE_PT.0, look.palette.line_strong)
        .border_dash(DROP_EDGE_PT.1, DROP_EDGE_PT.2)
        .role("status")
        .child(
            icon(&PAPERCLIP, DROP_ICON_PT)
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        );
    match target {
        DropTarget::Conversation(name) => {
            card = card
                .label(strings::drop_title())
                .child(
                    text(strings::drop_title())
                        .font(look.font(TYPE_BODY_STRONG))
                        .color(look.palette.text_primary),
                )
                .child(
                    text(strings::drop_target_limits(name))
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_muted),
                );
        }
        DropTarget::Editing => {
            card = card.label(strings::drop_editing()).child(
                text(strings::drop_editing())
                    .font(look.font(TYPE_BODY_STRONG))
                    .color(look.palette.text_primary),
            );
        }
    }
    el().key("drop-area")
        .absolute(left, top)
        .size(width, height)
        .center()
        .child(card)
}

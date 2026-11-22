//! The reply line over a message that replies (`docs/design.md` §11): one
//! line on the content column in `type.small`, `text.muted`: a 15 pt
//! avatar, the original's author at 600 in `text.secondary` ("@Name" when
//! the reply pinged them) and its snippet. A 2 pt `line.strong` elbow runs
//! from the message's avatar up to it; Extra compact, which has no avatar,
//! shows the `reply-line` icon in its gutter instead. A press (or Enter)
//! jumps to the original; a deleted one reads "Original message deleted" in
//! italic beside a dashed square, and one not available "Reply to an
//! earlier message", neither pressable.

use papo_sdk::Uuid;
use rune::{El, Span, col, el, icon, row, text};

use super::TimelineKey;
use super::replies::OriginalText;
use super::view::TimelineHost;
use crate::design::{
    FOCUS_RING_PT, Lead, Palette, RADIUS_AVATAR_PT, RADIUS_CONTROL_PT, REPLY_AVATAR_PT,
    REPLY_DELETED_PT, REPLY_ELBOW_PT, REPLY_LINE_ICON_PT, SPACE_SM_PT, TYPE_SMALL, WEIGHT_SEMIBOLD,
};
use crate::icons::REPLY_LINE;
use crate::rail::initial;
use crate::strings;

/// What a reply line shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ReplyLine {
    pub(super) original: OriginalText,
    /// The reply pinged the original's author: their name reads "@Name".
    pub(super) pinged: bool,
    /// The original a press jumps to, when history holds it.
    pub(super) jump: Option<Uuid>,
}

/// The reply line of a message in timeline `key`, at a density whose lead
/// column is `lead`, `lead_gap_pt` from the content column.
pub(super) fn reply_line<S: TimelineHost>(
    key: TimelineKey,
    line: &ReplyLine,
    lead: Lead,
    lead_gap_pt: f32,
    palette: &Palette,
) -> El {
    let line_pt = TYPE_SMALL.line_px();
    let content = content::<S>(key, line, palette);
    match lead {
        Lead::Avatar(size_pt) => {
            let (stroke_pt, radius_pt) = REPLY_ELBOW_PT;
            // From the avatar's centre up to the line's middle, then across
            // to it.
            let elbow = col()
                .w(size_pt / 2.0 + lead_gap_pt - SPACE_SM_PT)
                .h(line_pt)
                .justify_end()
                .a11y_hidden()
                .child(
                    el().w_full()
                        .h(line_pt / 2.0 + stroke_pt / 2.0)
                        .corners([radius_pt, 0.0, 0.0, 0.0])
                        .border_sides([stroke_pt, 0.0, 0.0, stroke_pt], palette.line_strong),
                );
            row()
                .key("reply-line")
                .w_full()
                .pl(size_pt / 2.0 - stroke_pt / 2.0)
                .gap(SPACE_SM_PT)
                .items_start()
                .child(elbow)
                .child(content)
        }
        Lead::Gutter(width_pt) => row()
            .key("reply-line")
            .w_full()
            .gap(lead_gap_pt)
            .center_y()
            .child(
                row()
                    .w(width_pt)
                    .h(line_pt)
                    .justify_end()
                    .center_y()
                    .a11y_hidden()
                    .child(icon(&REPLY_LINE, REPLY_LINE_ICON_PT).color(palette.line_strong)),
            )
            .child(content),
    }
}

/// The line itself: avatar, name and snippet, or what stands for an
/// original that is gone.
fn content<S: TimelineHost>(key: TimelineKey, line: &ReplyLine, palette: &Palette) -> El {
    let line_pt = TYPE_SMALL.line_px();
    let (lead, words, spans) = match &line.original {
        OriginalText::Message { name, snippet, .. } => {
            let name = if line.pinged {
                format!("@{name}")
            } else {
                name.clone()
            };
            let spans = vec![
                Span::new(0, name.len())
                    .weight(WEIGHT_SEMIBOLD)
                    .color(palette.text_secondary),
            ];
            let avatar = el()
                .size(REPLY_AVATAR_PT, REPLY_AVATAR_PT)
                .radius(RADIUS_AVATAR_PT / 2.0)
                .center()
                .fill(palette.surface_hover)
                .a11y_hidden()
                .child(
                    text(initial(&name))
                        .font(TYPE_SMALL)
                        .color(palette.text_secondary),
                );
            (Some(avatar), format!("{name} {snippet}"), spans)
        }
        OriginalText::Deleted => {
            let words = strings::reply_deleted().to_owned();
            let square = el()
                .size(REPLY_DELETED_PT, REPLY_DELETED_PT)
                .radius(RADIUS_AVATAR_PT / 2.0)
                .border(1.0, palette.line_strong)
                .border_dash(2.0, 2.0)
                .a11y_hidden();
            let spans = vec![Span::new(0, words.len()).italic()];
            (Some(square), words, spans)
        }
        OriginalText::Earlier => {
            let words = strings::reply_earlier().to_owned();
            let spans = vec![Span::new(0, words.len()).italic()];
            (None, words, spans)
        }
    };
    let spoken = words.clone();
    let mut content = row()
        .key("reply-original")
        .grow(1.0)
        .min_w(0.0)
        .h(line_pt)
        .gap(SPACE_SM_PT)
        .center_y()
        .children(lead)
        .child(
            text(words)
                .font(TYPE_SMALL)
                .color(palette.text_muted)
                .spans(spans)
                .truncate()
                .min_w(0.0),
        );
    if let Some(original) = line.jump {
        content = content
            .radius(RADIUS_CONTROL_PT)
            .role("link")
            .label(strings::reply_open(&spoken))
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
            .on_press_tx(move |host: &mut S, tx| host.reply_line_pressed(key, original, tx));
    }
    content
}

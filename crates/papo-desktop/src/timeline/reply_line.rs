//! The reply line over a message that replies (`docs/design.md` §11): one
//! line on the content column in `type.small`, `text.muted`: a 15 pt
//! avatar, the original's author at 600 in `text.secondary` ("@Name" when
//! the reply pinged them) and its snippet. A 2 pt `line.strong` elbow runs
//! from the message's avatar up to it; Extra compact, which has no avatar,
//! shows the `reply-line` icon in its gutter instead. A press (or Enter)
//! jumps to the original; a deleted one reads "Original message deleted" in
//! italic beside a dashed square, and one not available "Reply to an
//! earlier message", neither pressable.

use crate::emoji_images::{EmojiLook, with_images};
use papo_sdk::Uuid;
use rune::{El, Span, el, icon, row, text};

use super::TimelineKey;
use super::replies::OriginalText;
use super::view::TimelineHost;
use crate::design::{
    AVATAR_TOP, FOCUS_RING_PT, Lead, Look, RADIUS_AVATAR, RADIUS_CONTROL, REPLY_AVATAR_PT,
    REPLY_DELETED_PT, REPLY_ELBOW_PT, REPLY_LINE_ICON_PT, SPACE_SM, TYPE_SMALL, WEIGHT_SEMIBOLD,
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
    /// What the original's custom emoji draw as, by their ranges of its snippet.
    pub(super) emoji: Vec<(std::ops::Range<usize>, EmojiLook)>,
}

/// The reply line of a message in timeline `key`, at a density whose lead
/// column is `lead`, `lead_gap_pt` from the content column.
pub(super) fn reply_line<S: TimelineHost>(
    key: TimelineKey,
    line: &ReplyLine,
    lead: Lead,
    lead_gap_pt: f32,
    look: &Look,
) -> El {
    let line_pt = look.font(TYPE_SMALL).line_px();
    let content = content::<S>(key, line, look);
    match lead {
        Lead::Avatar(size_pt) => {
            let (stroke_pt, radius_pt) = REPLY_ELBOW_PT;
            // From the avatar's top up to the line's middle, then across to
            // it. The avatar sits `look.text_multiple(AVATAR_TOP)` below the line's box, so
            // the leg reaches past the box, out of flow: the line keeps its
            // height.
            let elbow_w_pt = size_pt / 2.0 + lead_gap_pt - look.space(SPACE_SM);
            let elbow = el().w(elbow_w_pt).h(line_pt).a11y_hidden().child(
                el().key("elbow")
                    .absolute(0.0, line_pt / 2.0 - stroke_pt / 2.0)
                    .size(
                        elbow_w_pt,
                        line_pt / 2.0 + stroke_pt / 2.0 + look.text_multiple(AVATAR_TOP),
                    )
                    .corners([radius_pt, 0.0, 0.0, 0.0])
                    .border_sides([stroke_pt, 0.0, 0.0, stroke_pt], look.palette.line_strong),
            );
            row()
                .key("reply-line")
                .w_full()
                .pl(size_pt / 2.0 - stroke_pt / 2.0)
                .gap(look.space(SPACE_SM))
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
                    .child(icon(&REPLY_LINE, REPLY_LINE_ICON_PT).color(look.palette.line_strong)),
            )
            .child(content),
        // No time column: the reply line's mark leads the line.
        Lead::None => row()
            .key("reply-line")
            .w_full()
            .gap(look.space(SPACE_SM))
            .center_y()
            .child(
                el().a11y_hidden()
                    .child(icon(&REPLY_LINE, REPLY_LINE_ICON_PT).color(look.palette.line_strong)),
            )
            .child(content),
    }
}

/// The line itself: avatar, name and snippet, or what stands for an
/// original that is gone.
fn content<S: TimelineHost>(key: TimelineKey, line: &ReplyLine, look: &Look) -> El {
    let line_pt = look.font(TYPE_SMALL).line_px();
    // Where the snippet starts in the words: after the name and a space.
    let mut snippet_at = None;
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
                    .color(look.palette.text_secondary),
            ];
            let avatar = el()
                .size(REPLY_AVATAR_PT, REPLY_AVATAR_PT)
                .radius(look.radius(RADIUS_AVATAR) / 2.0)
                .center()
                .fill(look.palette.surface_hover)
                .a11y_hidden()
                .child(
                    text(initial(&name))
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_secondary),
                );
            snippet_at = Some(name.len() + 1);
            (Some(avatar), format!("{name} {snippet}"), spans)
        }
        OriginalText::Deleted => {
            let words = strings::reply_deleted().to_owned();
            let square = el()
                .size(REPLY_DELETED_PT, REPLY_DELETED_PT)
                .radius(look.radius(RADIUS_AVATAR) / 2.0)
                .border(1.0, look.palette.line_strong)
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
        .gap(look.space(SPACE_SM))
        .center_y()
        .children(lead)
        .child(with_images(
            text(words)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .spans(spans)
                .truncate()
                .min_w(0.0),
            snippet_at.unwrap_or_default(),
            snippet_at.map(|_| line.emoji.clone()).unwrap_or_default(),
            look.font(TYPE_SMALL),
            look,
        ));
    if let Some(original) = line.jump {
        content = content
            .radius(look.radius(RADIUS_CONTROL))
            .role("link")
            .label(strings::reply_open(&spoken))
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| host.reply_line_pressed(key, original, tx));
    }
    content
}

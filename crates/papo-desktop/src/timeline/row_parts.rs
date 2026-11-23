//! What a message's row shows, read from the message, its neighbour, the
//! SDK's item (reply original, change in flight) and the view's state (the
//! composer's target, a jump's flash): its parts, whose hash is the row's
//! version. Drawing them is `message_row`'s.

use papo_sdk::{Message, Operation, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::operations::ReactionEmoji;
use papo_sdk_ui::{ReplyOriginal, attention, message_actions, pinned, reaction_chips};

use super::OpenTimeline;
use super::changes::ChangeState;
use super::content::mention_names;
use super::echoes::SendState;
use super::grouping::continues;
use super::hover_bar::{BAR_REACTIONS, Bar};
use super::message_row::{author_name, created_us};
use super::reactions::Chip;
use super::reading::Boundary;
use super::replies::original_text;
use super::reply_line::ReplyLine;
use super::view::Style;
use crate::design::Density;

/// A row's background (§4, Highlights: one per row, the strongest reason).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Highlight {
    None,
    /// It mentions you or everyone, or replies to you (every reply pings
    /// its author until the backend stores `notify_reply`).
    Mention,
    /// The composer points at it, or a jump just landed on it.
    Selected,
}

/// What a message row shows, read from the message and its neighbour.
#[derive(Hash)]
pub(super) struct Parts<'a> {
    pub(super) name: String,
    pub(super) time: String,
    /// The message's source, which its content is parsed from: the content
    /// changes with it.
    pub(super) body: &'a str,
    /// The names its mentions show.
    pub(super) mentions: Vec<Option<String>>,
    /// It continues the group above: no header, the within-group distance.
    pub(super) continued: bool,
    /// Extra compact shows the time once per minute.
    pub(super) show_time: bool,
    pub(super) highlight: Highlight,
    /// When it was edited, as the wire says it.
    pub(super) edited_at: Option<&'a str>,
    /// The reading boundary over the distance above it.
    pub(super) boundary: Option<Boundary>,
    /// An unsent message of yours: where its send stands.
    pub(super) sending: Option<SendState>,
    /// The reply line over it.
    pub(super) reply: Option<ReplyLine>,
    /// Your edit or deletion of it in flight, by its local identity.
    pub(super) change: Option<(Uuid, ChangeState)>,
    /// Its hover bar: a message from history has one.
    pub(super) bar: Option<Bar>,
    /// Its reaction chips, and whether the add chip ends them.
    pub(super) chips: Vec<Chip>,
    pub(super) add_chip: bool,
}

/// Where a row's message comes from.
#[derive(Clone, Copy)]
pub(super) enum Origin<'a> {
    /// The channel's history, with your edit or deletion of it in flight:
    /// the row reports where it is, for the anchor and Ctrl+A, and takes
    /// keyboard focus.
    History(Option<&'a Operation>),
    /// A local echo of yours, with where its send stands (none once sent).
    /// It is no anchor: the SDK knows no such message yet.
    Echo(Option<SendState>),
}

/// The message a row draws.
pub(super) struct RowMessage<'a> {
    /// The message's ID, or an echo's local identity.
    pub(super) id: Uuid,
    pub(super) message: &'a Message,
    pub(super) content: &'a MessageContent,
    /// Its original, when it replies.
    pub(super) reply: Option<&'a ReplyOriginal>,
    pub(super) origin: Origin<'a>,
}

/// What the row of `row_message` shows, and the content it draws (an edit
/// in flight's text in place of the message's).
pub(super) fn parts<'a>(
    open: &'a OpenTimeline,
    row_message: &RowMessage<'a>,
    previous: Option<&Message>,
    style: &Style,
) -> (Parts<'a>, &'a MessageContent) {
    let RowMessage {
        id,
        message,
        content,
        reply,
        origin,
    } = *row_message;
    let layout = style.layout;
    let boundary = open.reading.boundary(id, style.hourglass);
    // Only the divider, held for the visit, splits a group: the block's
    // edges move as the SDK covers what is read, and a split that moved
    // with them would move rows.
    let splits = boundary == Some(Boundary::NewMessages);
    let continued = layout.grouped && !splits && continues(previous, message);
    let at_us = created_us(message);
    let show_time = match previous {
        Some(previous) if layout.density == Density::ExtraCompact => {
            let minute = |at_us: Option<i64>| at_us.map(|at_us| style.time.minute_of(at_us));
            minute(created_us(previous)) != minute(at_us)
        }
        _ => true,
    };
    let (sending, history) = match origin {
        Origin::Echo(state) => (state, false),
        Origin::History(_) => (None, true),
    };
    let me = open.key.account.user();
    let change = history.then(|| open.changes.get(id)).flatten();
    let line = reply.map(|original| {
        let text = original_text(original, open);
        let author = message
            .author_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok());
        ReplyLine {
            pinged: author.is_none_or(|author| !text.by(author)),
            jump: match original {
                ReplyOriginal::Held { id, .. } => Some(*id),
                _ => None,
            },
            original: text,
        }
    });
    let attention = attention(message, reply, me);
    let flashing = open.flash.is_some_and(|(flash, _)| flash == id);
    let highlight = if style.selected == Some(id) || flashing {
        Highlight::Selected
    } else if attention.mentions_you || attention.mentions_everyone || attention.replies_to_you {
        Highlight::Mention
    } else {
        Highlight::None
    };
    // An edit in flight shows its own text.
    let edit = change.and_then(|change| change.edit.as_ref());
    let content = edit.map_or(content, |(_, parsed)| parsed.as_ref());
    let parts = Parts {
        name: author_name(message, open.resources.as_deref()),
        time: at_us.map_or_else(String::new, |at_us| style.time.time_label(at_us)),
        // An edit in flight's text stands for it: the row is redrawn when
        // it changes.
        body: edit.map_or_else(
            || message.content.as_deref().unwrap_or_default(),
            |(text, _)| text.as_str(),
        ),
        mentions: mention_names(content, open.resources.as_deref()),
        continued,
        show_time,
        highlight,
        edited_at: message.edited_at.as_deref(),
        boundary,
        sending,
        reply: line,
        change: change.map(|change| (change.local, change.state)),
        bar: match origin {
            Origin::History(pending) => Some(bar(open, id, message, pending, style)),
            Origin::Echo(_) => None,
        },
        chips: match origin {
            Origin::History(_) => chips(message, style),
            Origin::Echo(_) => Vec::new(),
        },
        add_chip: false,
    };
    // The add chip is offered where the bar offers reacting.
    let parts = Parts {
        add_chip: parts.bar.as_ref().is_some_and(|bar| !bar.emoji.is_empty()),
        ..parts
    };
    (parts, content)
}

/// What message `id`'s hover bar offers this account.
fn bar(
    open: &OpenTimeline,
    id: Uuid,
    message: &Message,
    pending: Option<&Operation>,
    style: &Style,
) -> Bar {
    let reacting = &style.reacting;
    let actions = message_actions(
        message,
        pending,
        pinned(open.pins.as_ref(), id),
        open.key.account.user(),
        reacting.capabilities,
    );
    let stretched = reacting
        .stretched
        .and_then(|(on, chosen)| (on == id).then_some(chosen));
    let emoji = if actions.react {
        let chips = reaction_chips(message, &reacting.outgoing);
        let shown = if stretched.is_some() {
            reacting.quick.len()
        } else {
            BAR_REACTIONS
        };
        reacting
            .quick
            .iter()
            .take(shown)
            .map(|emoji| {
                let unicode = ReactionEmoji::Unicode(emoji.clone());
                let mine = chips.iter().any(|chip| chip.mine && chip.emoji == unicode);
                (emoji.clone(), mine)
            })
            .collect()
    } else {
        Vec::new()
    };
    Bar {
        emoji,
        stretched,
        reply: actions.reply,
        edit: actions.edit,
    }
}

/// Message's reaction chips, and whether this account may toggle each now.
fn chips(message: &Message, style: &Style) -> Vec<Chip> {
    let reacting = &style.reacting;
    reaction_chips(message, &reacting.outgoing)
        .into_iter()
        .map(|chip| Chip {
            toggles: chip.can_toggle(reacting.capabilities),
            chip,
        })
        .collect()
}

//! The Conversations page and its Message highlights sub-page
//! (`docs/design.md` §8): When writing (Enter to send), In the conversation
//! (Show times, Show avatars, Open at the newest message, Unread
//! hourglass, Description when opening a channel, Message highlights), and
//! the mention highlight's hue swatches. Mention when replying waits for
//! B-006 and Replies' hue for `highlight.reply` (B-006 too); the Media
//! rows come with D7.
//!
//! Owns these pages' rows; the settings are `conversation_settings`'.

use super::{Context, Group, Kind, Row};
use crate::design::{Density, MENTION_HIGHLIGHT_HUE_DEGREES};
use crate::settings::{PageId, RowId};
use crate::strings;

/// The hues a highlight's swatches offer, in OKLCH degrees: amber (the
/// mention's default) first, then around the wheel.
pub(crate) const MENTION_HUES: [f32; 7] = [
    MENTION_HIGHLIGHT_HUE_DEGREES,
    25.0,
    52.0,
    152.0,
    196.0,
    256.0,
    320.0,
];

/// A switch row.
fn switch(id: RowId, title: &str, sub: Option<&str>, on: bool) -> Row {
    Row {
        id,
        title: title.to_owned(),
        sub: sub.map(str::to_owned),
        kind: Kind::Switch(on),
    }
}

pub(super) fn conversations(cx: &Context<'_>) -> Vec<Group> {
    let settings = cx.conversations;
    let mut rows = vec![switch(
        RowId::ShowTimes,
        strings::settings_show_times(),
        None,
        settings.show_times,
    )];
    // Extra compact never has avatars: the switch would do nothing there.
    if cx.appearance.messages.density != Density::ExtraCompact {
        rows.push(switch(
            RowId::ShowAvatars,
            strings::settings_show_avatars(),
            None,
            settings.show_avatars,
        ));
    }
    rows.extend([
        switch(
            RowId::OpenAtNewest,
            strings::settings_open_at_newest(),
            Some(strings::settings_open_at_newest_sub()),
            settings.open_at_newest,
        ),
        switch(
            RowId::Hourglass,
            strings::settings_hourglass(),
            Some(strings::settings_hourglass_sub()),
            settings.unread_hourglass,
        ),
        switch(
            RowId::ChannelDescription,
            strings::settings_channel_description(),
            Some(strings::settings_channel_description_sub()),
            settings.channel_description,
        ),
        Row {
            id: RowId::Highlights,
            title: strings::settings_highlights().to_owned(),
            sub: Some(strings::settings_highlights_sub().to_owned()),
            kind: Kind::SubPage(PageId::Highlights, String::new()),
        },
    ]);
    vec![
        Group {
            title: Some(strings::settings_group_writing()),
            rows: vec![switch(
                RowId::EnterSends,
                strings::settings_enter_sends(),
                Some(strings::settings_enter_sends_sub()),
                settings.enter_sends,
            )],
        },
        Group {
            title: Some(strings::settings_group_conversation()),
            rows,
        },
    ]
}

pub(super) fn highlights(cx: &Context<'_>) -> Vec<Group> {
    vec![Group {
        title: None,
        rows: vec![Row {
            id: RowId::MentionHue,
            title: strings::settings_highlight_mentions().to_owned(),
            sub: None,
            kind: Kind::Hues(cx.conversations.mention_hue_degrees),
        }],
    }]
}

/// The Conversations nav row's one line.
pub(crate) fn conversations_summary() -> &'static str {
    strings::settings_conversations_summary()
}

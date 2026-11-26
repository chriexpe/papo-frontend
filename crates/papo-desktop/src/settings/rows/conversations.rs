//! The Conversations page and its Message highlights sub-page
//! (`docs/design.md` §8): When writing (Enter to send), In the conversation
//! (Show times, Show avatars, Open at the newest message, Unread
//! hourglass, Description when opening a channel, Message highlights), and
//! the mention and reply highlights' hue swatches, Media (Image size, Video
//! size and Embed size with Custom's Max width and Max height, Animate GIFs
//! and stickers, Autoplay videos, Keep playing when hidden; §14.9, §15.7),
//! and Show link previews. Mention when replying waits for B-006.
//!
//! Owns these pages' rows; the settings are `conversation_settings`'.

use papo_sdk_ui::{CUSTOM_STEP_PT, CUSTOM_WIDTH_PT};

use super::{Context, Group, Kind, Row, Slider, Track};
use crate::design::{Density, MENTION_HIGHLIGHT_HUE_DEGREES};
use crate::keep_playing::KeepPlaying;
use crate::settings::{PageId, RowId, SizeChoice, SizeSetting};
use crate::strings;

/// The hues a highlight's swatches offer, in OKLCH degrees: amber (the
/// mention's default) first, then around the wheel.
pub(crate) const HIGHLIGHT_HUES: [f32; 7] = [
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
            title: Some(strings::settings_group_writing().to_owned()),
            rows: vec![switch(
                RowId::EnterSends,
                strings::settings_enter_sends(),
                Some(strings::settings_enter_sends_sub()),
                settings.enter_sends,
            )],
        },
        Group {
            title: Some(strings::settings_group_conversation().to_owned()),
            rows,
        },
        Group {
            title: Some(strings::settings_media().to_owned()),
            rows: media(cx),
        },
        Group {
            title: None,
            rows: vec![switch(
                RowId::LinkPreviews,
                strings::settings_previews(),
                Some(strings::settings_previews_sub()),
                settings.link_previews,
            )],
        },
    ]
}

/// The Media group (§14.9, §15.7): Image size, Video size and Embed size, each with its sliders
/// under Custom, Animate GIFs and emoji, Autoplay videos, Keep playing when hidden.
fn media(cx: &Context<'_>) -> Vec<Row> {
    let mut rows = size_rows(
        strings::settings_image_size(),
        None,
        SizeSetting::Images(cx.conversations.image_size),
    );
    rows.extend(size_rows(
        strings::settings_video_size(),
        None,
        SizeSetting::Videos(cx.conversations.video_size),
    ));
    rows.extend(size_rows(
        strings::settings_embed_size(),
        Some(strings::settings_embed_size_sub()),
        SizeSetting::Embeds(cx.conversations.embed_size),
    ));
    rows.push(switch(
        RowId::AnimateGifs,
        strings::settings_animate_gifs(),
        None,
        cx.conversations.animates_gifs(cx.reduced_motion),
    ));
    rows.push(switch(
        RowId::AutoplayVideos,
        strings::settings_autoplay(),
        Some(strings::settings_autoplay_sub()),
        cx.conversations.autoplays_videos(cx.reduced_motion),
    ));
    rows.push(Row {
        id: RowId::KeepPlaying,
        title: strings::settings_keep_playing().to_owned(),
        sub: Some(strings::settings_keep_playing_sub().to_owned()),
        kind: Kind::Choice(keep_playing_name(cx.conversations.keep_playing).to_owned()),
    });
    rows
}

/// A Keep playing when hidden choice's name.
pub(crate) fn keep_playing_name(choice: KeepPlaying) -> &'static str {
    match choice {
        KeepPlaying::Nothing => strings::settings_keep_nothing(),
        KeepPlaying::Audio => strings::settings_keep_audio(),
        KeepPlaying::Video => strings::settings_keep_video(),
        KeepPlaying::Both => strings::settings_keep_both(),
    }
}

/// A size row (`media`'s), and under Custom its Max width and Max height sliders.
fn size_rows(title: &str, sub: Option<&str>, size: SizeSetting) -> Vec<Row> {
    let media = size.media();
    let mut rows = vec![Row {
        id: media.row(),
        title: title.to_owned(),
        sub: sub.map(str::to_owned),
        kind: Kind::Choice(size_name(size.choice()).to_owned()),
    }];
    if size.choice() == SizeChoice::Custom {
        let (width, height) = size.max_size_pt();
        let (width_row, height_row) = media.slider_rows();
        let slider = |id, title: &str, value: u16, range: &std::ops::RangeInclusive<u16>| Row {
            id,
            title: title.to_owned(),
            sub: None,
            kind: Kind::Slider(Slider {
                value: f32::from(value),
                min: f32::from(*range.start()),
                max: f32::from(*range.end()),
                step: f32::from(CUSTOM_STEP_PT),
                track: Track::Plain,
            }),
        };
        rows.push(slider(
            width_row,
            strings::settings_max_width(),
            width,
            &CUSTOM_WIDTH_PT,
        ));
        rows.push(slider(
            height_row,
            strings::settings_max_height(),
            height,
            &media.custom_heights(),
        ));
    }
    rows
}

/// A size choice's name: Small, Medium, Large or Custom.
pub(crate) fn size_name(choice: SizeChoice) -> &'static str {
    match choice {
        SizeChoice::Small => strings::settings_size_small(),
        SizeChoice::Medium => strings::settings_size_medium(),
        SizeChoice::Large => strings::settings_size_large(),
        SizeChoice::Custom => strings::settings_size_custom(),
    }
}

pub(super) fn highlights(cx: &Context<'_>) -> Vec<Group> {
    vec![Group {
        title: None,
        rows: vec![
            Row {
                id: RowId::MentionHue,
                title: strings::settings_highlight_mentions().to_owned(),
                sub: None,
                kind: Kind::Hues(cx.conversations.mention_hue_degrees),
            },
            Row {
                id: RowId::ReplyHue,
                title: strings::settings_highlight_replies().to_owned(),
                sub: None,
                kind: Kind::Hues(cx.conversations.reply_hue_degrees),
            },
        ],
    }]
}

/// The Conversations nav row's one line.
pub(crate) fn conversations_summary() -> &'static str {
    strings::settings_conversations_summary()
}

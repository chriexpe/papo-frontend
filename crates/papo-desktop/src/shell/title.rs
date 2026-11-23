//! What the shown conversation is called (`docs/design.md` §10): a
//! channel's name, or a direct conversation's person (with presence) and
//! server; and the title pill at the header's left that shows it. The
//! composer's prompt says it too.
//!
//! Owns working the title out from the shell and drawing the pill; the
//! header around it is `pins_panel`'s.

use papo_sdk_ui::{PresenceMark, display_name};
use rune::{El, icon, row, text};

use super::Shell;
use crate::design::{
    AVATAR_TITLE, HEADER_PILL_HEIGHT, Look, PILL_ICON_PT, PILL_OUTLINE_PT, RADIUS_PILL, SPACE_LG,
    SPACE_SM, TYPE_BODY_STRONG, TYPE_CAPTION,
};
use crate::icons::HASH;
use crate::person_avatar::person_avatar;
use crate::strings;

/// The shown conversation's title.
pub(crate) enum Title<'a> {
    /// A server's channel, by name.
    Channel(&'a str),
    /// A direct conversation.
    Direct {
        name: String,
        server: String,
        presence: Option<PresenceMark>,
    },
}

impl Title<'_> {
    /// The conversation's name: "#general", or the person in a DM.
    pub(crate) fn name(&self) -> String {
        match self {
            Title::Channel(channel) => format!("#{channel}"),
            Title::Direct { name, .. } => name.clone(),
        }
    }

    /// The composer's prompt.
    pub(crate) fn placeholder(&self) -> String {
        match self {
            Title::Channel(channel) => strings::composer_placeholder(channel),
            Title::Direct { name, .. } => strings::composer_placeholder_direct(name),
        }
    }
}

impl Shell {
    /// The shown conversation's title; an empty channel name while none
    /// shows.
    pub(super) fn title(&self) -> Title<'_> {
        if let Some(picked) = self.nav.picked_dm() {
            let account = picked.id.account;
            return Title::Direct {
                name: display_name(picked.nickname.as_deref(), picked.username.as_deref())
                    .unwrap_or(strings::someone())
                    .to_owned(),
                server: self
                    .nav
                    .servers()
                    .get(account.server())
                    .map(|server| server.name.clone())
                    .unwrap_or_default(),
                presence: self
                    .people(account)
                    .and_then(|people| people.presence_mark(picked.user)),
            };
        }
        let channel = self
            .nav
            .selected()
            .zip(self.nav.selected_channel())
            .and_then(|(server, id)| server.text_channels.iter().find(|entry| entry.id == id))
            .map_or("", |entry| entry.name.as_str());
        Title::Channel(channel)
    }
}

/// The title pill: # and the channel's name, or the person's avatar, name
/// and server.
pub(super) fn pill(title: &Title<'_>, look: &Look) -> El {
    let pill = row()
        .key("title-pill")
        .h(look.space(HEADER_PILL_HEIGHT))
        .px(look.space(SPACE_LG))
        .gap(look.space(SPACE_SM))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .fill(look.palette.surface_pill)
        .border(PILL_OUTLINE_PT, look.palette.line_pill)
        .role("heading");
    match title {
        Title::Channel(channel) => pill
            .label((*channel).to_owned())
            .child(icon(&HASH, PILL_ICON_PT).color(look.palette.text_secondary))
            .child(
                text((*channel).to_owned())
                    .font(look.font(TYPE_BODY_STRONG))
                    .color(look.palette.text_primary),
            ),
        Title::Direct {
            name,
            server,
            presence,
        } => {
            let spoken = match presence {
                Some(mark) => strings::person_with_presence(name, *mark),
                None => name.clone(),
            };
            pill.label(strings::direct_title_label(&spoken, server))
                .child(person_avatar(
                    name,
                    AVATAR_TITLE,
                    *presence,
                    look.palette.surface_pill,
                    look,
                ))
                .child(
                    text(name.as_str())
                        .font(look.font(TYPE_BODY_STRONG))
                        .color(look.palette.text_primary)
                        .a11y_hidden(),
                )
                .child(
                    text(server.as_str())
                        .font(look.font(TYPE_CAPTION))
                        .color(look.palette.text_muted)
                        .a11y_hidden(),
                )
        }
    }
}

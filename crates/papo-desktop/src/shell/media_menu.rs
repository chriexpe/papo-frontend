//! The media menu (`docs/design.md` §14.7), beside the pointer or (from the keyboard) the
//! media (an attachment, or a link preview's picture): Open (not on audio, whose player is the
//! way in; Retry instead on audio that couldn't be read), Save image, video, audio or file with
//! its folder, Save as…, Copy image. Copy link is left out for attachments (B-024). What each
//! item does is `media_presses`'.

use std::path::Path;

use papo_sdk_ui::AttachmentKind;

use super::Shell;
use super::media_presses::MediaAction;
use crate::icons;
use crate::menu::{Entry, MenuAction, MenuItem, OpenMenu, Placement};
use crate::message_media::received::{MediaAt, media_element_key};
use crate::strings;

/// The media menu's width, in points.
const MEDIA_MENU_WIDTH_PT: f32 = 248.0;

impl Shell {
    /// The media menu for media `at` of `kind`, beside `point` (the pointer, or the media's
    /// corner for the menu key); `retry_audio` when it is audio that couldn't be read.
    pub(super) fn open_media_menu(
        &mut self,
        at: MediaAt,
        kind: AttachmentKind,
        retry_audio: bool,
        point: Option<[f32; 2]>,
    ) {
        let picture = kind.is_picture();
        let action = |action| MenuAction::Media { at, action };
        let folder = self
            .general
            .settings()
            .media_folder()
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned());
        let audio = kind == AttachmentKind::Audio;
        let (open_icon, save_label) = if picture {
            (&icons::IMAGE, strings::menu_save_image())
        } else if kind == AttachmentKind::Video {
            (&icons::VIDEO, strings::menu_save_video())
        } else if audio {
            (&icons::EXTERNAL, strings::menu_save_audio())
        } else {
            (&icons::EXTERNAL, strings::menu_save_file())
        };
        let mut entries = Vec::new();
        // Audio has no Open: its player is the way in (§14.7). One that couldn't be read is
        // offered again instead.
        if audio {
            if retry_audio {
                entries.push(Entry::Item(
                    MenuItem::plain(
                        "menu-media-retry",
                        strings::media_retry(),
                        action(MediaAction::Retry),
                        false,
                    )
                    // The circular arrow, as the editor's Rotate: the set has no other.
                    .with_icon(&icons::ROTATE),
                ));
            }
        } else {
            entries.push(Entry::Item(
                MenuItem::plain(
                    "menu-media-open",
                    strings::menu_media_open(),
                    action(MediaAction::Open),
                    false,
                )
                .with_icon(open_icon)
                .with_shortcut("O"),
            ));
        }
        // Right after Open: a conversation GIF that takes a star (§16.4).
        let key = crate::timeline::TimelineKey {
            account: at.account,
            channel: at.channel,
        };
        let favorite = (at.surface == crate::message_media::received::MediaSurface::Timeline)
            .then(|| self.media_gif_of(at))
            .flatten()
            .map(|(embed, gif)| self.favorite_item_for(key, at.message, Some(embed), &gif));
        entries.extend(favorite.map(Entry::Item));
        entries.extend([
            Entry::Item(
                MenuItem::plain(
                    "menu-media-save",
                    save_label,
                    action(MediaAction::Save),
                    false,
                )
                .with_icon(&icons::DOWNLOAD)
                .with_sub(folder),
            ),
            Entry::Item(
                MenuItem::plain(
                    "menu-media-save-as",
                    strings::menu_save_as(),
                    action(MediaAction::SaveAs),
                    false,
                )
                .with_icon(&icons::FOLDER),
            ),
        ]);
        if picture {
            entries.push(Entry::Divider);
            entries.push(Entry::Item(
                MenuItem::plain(
                    "menu-media-copy-image",
                    strings::menu_copy_image(),
                    action(MediaAction::CopyImage),
                    false,
                )
                .with_icon(&icons::COPY),
            ));
        }
        let placement = match point {
            Some(point) => Placement::AtPointer(point),
            None => Placement::BelowEnd(media_element_key(at)),
        };
        self.nav.close_accounts();
        self.nav.menu = Some(OpenMenu::with_entries(
            placement,
            media_element_key(at),
            entries,
            MEDIA_MENU_WIDTH_PT,
        ));
    }
}

//! The shell's side of link cards (`docs/design.md` §15.1, §15.6): a card's menu (§10's link
//! menu; a provider video with a player opens the player's menu, `player_presses`), the play
//! disc where a poster never plays opening the page, and a preview picture's media presses and
//! menu actions (the viewer, Save, Save as…, Copy image, Retry), as an attachment's. The card
//! items of a message's menu are `card_menu_items`.

use papo_sdk::Message;
use papo_sdk_ui::{AttachmentKind, LinkPreview};
use rune::Tx;

use super::Shell;
use super::media_presses::{MediaAction, SaveTo, SavedSource, Saving};
use crate::icons;
use crate::link_cards::{
    CardPress, MediaItem, is_provider_video, media_item, name_of, picture_key, picture_of,
    saved_source,
};
use crate::links;
use crate::menu::{MenuAction, MenuItem};
use crate::message_media::received::{MediaAt, MediaPress};
use crate::strings;
use crate::timeline::TimelineKey;

impl Shell {
    /// The link preview at media place `at`, when that place is one (past the attachments).
    pub(super) fn preview_at(&self, at: MediaAt) -> Option<LinkPreview> {
        let message = self.message_at(at)?;
        let shown = self.conversations.settings().link_previews;
        match media_item(&message, at.index, shown)? {
            MediaItem::Preview(preview) => Some(*preview),
            MediaItem::Attachment => None,
        }
    }

    /// A press on the card at `at`.
    pub(crate) fn press_card(&mut self, at: MediaAt, press: CardPress, tx: &mut Tx) {
        let Some(preview) = self.preview_at(at) else {
            return;
        };
        let Some(page) = preview.page() else {
            return;
        };
        let key = TimelineKey {
            account: at.account,
            channel: at.channel,
        };
        if !self.tabs.is_shown(key) {
            return;
        }
        match press {
            CardPress::OpenInBrowser => self.link_pressed(key, page.address.clone(), tx),
            CardPress::Menu(point) => {
                if let Some((player, _)) = crate::link_cards::player_of(at, &preview).filter(|_| {
                    at.surface == crate::message_media::received::MediaSurface::Timeline
                }) {
                    self.open_player_menu(at, player, Some(point));
                    return;
                }
                let open_label = if is_provider_video(&preview) {
                    strings::embed_open_browser()
                } else {
                    strings::link_menu_open()
                };
                self.open_link_menu_as(key, page.address.clone(), point, open_label);
            }
        }
    }

    /// A media press on a preview's picture: the viewer, its menu, Retry.
    pub(super) fn press_preview_media(
        &mut self,
        at: MediaAt,
        preview: &LinkPreview,
        press: MediaPress,
        tx: &mut Tx,
    ) {
        let Some((key, _)) = picture_key(at.account, preview) else {
            return;
        };
        match press {
            MediaPress::Open => self.open_viewer(at, tx),
            MediaPress::Menu(point) => {
                let kind =
                    picture_of(preview).map_or(AttachmentKind::Image, |picture| picture.kind);
                self.open_media_menu(at, kind, false, point);
            }
            MediaPress::Retry => {
                let jobs = self.media.retry(key);
                self.run_media_jobs(jobs);
            }
            // A preview has no file card and is never hidden.
            MediaPress::Save
            | MediaPress::CancelSave
            | MediaPress::OpenSaved
            | MediaPress::ShowSaved
            | MediaPress::Reveal => {}
        }
    }

    /// What the media menu's item does for a preview's picture.
    pub(super) fn preview_media_action(
        &mut self,
        at: MediaAt,
        preview: &LinkPreview,
        action: MediaAction,
        tx: &mut Tx,
    ) {
        match action {
            MediaAction::Open => self.open_viewer(at, tx),
            MediaAction::Save => {
                if let Some(saving) = preview_saving(at, preview) {
                    self.save_media(saving, SaveTo::MediaFolder { with_pill: true });
                }
            }
            MediaAction::SaveAs => {
                if let Some(saving) = preview_saving(at, preview) {
                    self.save_as(saving, tx);
                }
            }
            MediaAction::CopyImage => {
                if let Some((_, source)) = picture_key(at.account, preview) {
                    self.copy_image(at, source);
                }
            }
            MediaAction::Retry => {}
        }
    }
}

/// Saving a preview's picture: named after its title (else its host) with its type's
/// extension.
fn preview_saving(at: MediaAt, preview: &LinkPreview) -> Option<Saving> {
    let (key, _) = picture_key(at.account, preview)?;
    let (embed, revision) = saved_source(preview)?;
    let extension = picture_of(preview)
        .and_then(|picture| picture.mime.as_deref())
        .and_then(extension_of);
    // A title is not a path: its separators would cut it to what follows the last one.
    let base = name_of(preview).replace(['/', '\\'], "-");
    let name = match extension {
        Some(extension) => format!("{base}.{extension}"),
        None => base,
    };
    Some(Saving {
        account: at.account,
        key,
        name,
        source: SavedSource::EmbedImage { embed, revision },
    })
}

/// The file extension a relayed picture of type `mime` is saved with.
fn extension_of(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// The card items of `message`'s menu (§15.6: the keyboard reaches every card action there):
/// for each card with a page, Open link (Open in browser for a provider video) with its host
/// unless the message's text already links it, then Copy link.
pub(super) fn card_menu_items(
    key: TimelineKey,
    message: &Message,
    shown: bool,
    text_links: &[String],
) -> Vec<MenuItem> {
    let mut items = Vec::new();
    for (place, preview) in crate::link_cards::previews(message, shown)
        .iter()
        .enumerate()
    {
        let LinkPreview::Card(card) = preview else {
            continue;
        };
        let Some(page) = &card.page else {
            continue;
        };
        let provider_video = is_provider_video(preview);
        let linked = text_links
            .iter()
            .any(|link| url::Url::parse(link).is_ok_and(|link| link.as_str() == page.address));
        if provider_video || !linked {
            let label = if provider_video {
                strings::embed_open_browser()
            } else {
                strings::link_menu_open()
            };
            items.push(
                MenuItem::plain(
                    &format!("menu-card-open-{place}"),
                    label,
                    MenuAction::OpenLink {
                        key,
                        url: page.address.clone(),
                    },
                    false,
                )
                .with_icon(&icons::EXTERNAL)
                .with_sub(links::host(&page.address)),
            );
        }
        items.push(
            MenuItem::plain(
                &format!("menu-card-copy-{place}"),
                strings::link_menu_copy(),
                MenuAction::CopyLink(page.address.clone()),
                false,
            )
            .with_icon(&icons::COPY)
            .with_sub(links::host(&page.address)),
        );
    }
    items
}

#[cfg(test)]
mod tests {
    //! A saved card picture's extension.

    use super::extension_of;

    #[test]
    fn a_relayed_picture_is_saved_with_its_types_extension() {
        assert_eq!(extension_of("image/webp"), Some("webp"));
        assert_eq!(extension_of("image/jpeg"), Some("jpg"));
        assert_eq!(extension_of("image/svg+xml"), None);
    }
}

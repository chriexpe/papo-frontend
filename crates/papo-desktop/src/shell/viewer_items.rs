//! What the viewer steps through for a message (`docs/design.md` §14.8, §15.1): its pictures
//! and videos in their order, then its link previews' pictures and relayed clips, each with
//! where its whole picture comes from and its size before that decodes. Opening and stepping
//! are `viewer`'s.

use papo_sdk::{MediaSource, Message};
use papo_sdk_ui::{AttachmentKind, MediaSize};

use super::Shell;
use crate::media_store::MediaKey;
use crate::message_media::received::{attachment_id, split};
use crate::viewer::ViewerItem;

impl Shell {
    /// What the viewer steps through for `message` of `account`, in the order they are drawn:
    /// its link previews' pictures and clips, then its pictures and videos, each at its place.
    pub(super) fn viewer_items(
        &self,
        account: papo_sdk::AccountId,
        message: &Message,
    ) -> Vec<ViewerItem> {
        let attachments = message.attachments.as_deref().unwrap_or_default();
        let (pictures, _) = split(attachments);
        let attached: Vec<ViewerItem> = pictures
            .iter()
            .filter_map(|index| {
                let attachment = &attachments[*index];
                let id = attachment_id(attachment)?;
                let kind = AttachmentKind::of(attachment);
                Some(ViewerItem {
                    index: *index,
                    key: MediaKey::attachment(account, id),
                    name: attachment.original_file_name.clone().unwrap_or_default(),
                    gif: kind == AttachmentKind::Gif,
                    video: kind == AttachmentKind::Video,
                    source: MediaSource::Attachment(id),
                    size: self
                        .own_sizes
                        .get(&id)
                        .copied()
                        .unwrap_or_else(|| MediaSize::of(attachment)),
                })
            })
            .collect();
        let mut items = Vec::new();
        let shown = self.conversations.settings().link_previews;
        let first = crate::link_cards::attachment_count(message);
        for (place, preview) in crate::link_cards::previews(message, shown)
            .iter()
            .enumerate()
        {
            let index = first + place;
            let name = crate::link_cards::name_of(preview);
            if let Some((key, papo_sdk_ui::CardVideo::Clip { source, size, .. })) =
                crate::link_cards::clip_of(account, preview)
            {
                items.push(ViewerItem {
                    index,
                    key,
                    name,
                    gif: false,
                    video: true,
                    source: source.clone(),
                    size: *size,
                });
            } else if let (Some((key, source)), Some(picture)) = (
                crate::link_cards::picture_key(account, preview),
                crate::link_cards::picture_of(preview),
            ) {
                items.push(ViewerItem {
                    index,
                    key,
                    name,
                    gif: picture.kind == AttachmentKind::Gif,
                    video: false,
                    source,
                    size: picture.size,
                });
            }
        }
        items.extend(attached);
        items
    }
}

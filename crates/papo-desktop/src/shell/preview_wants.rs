//! What the shown messages' link previews want loaded (`docs/design.md` §15.1, §15.2): their
//! relayed pictures at the boxes their cards draw, each kept with the exact SDK source it is
//! fetched from (its revision whole), and their relayed clips for the video store. Fetching
//! is `received_media`'s and `videos`'.

use std::sync::Arc;

use papo_sdk::{AccountId, MediaSource};
use papo_sdk_ui::{AttachmentKind, LinkPreview, MediaBounds};

use super::Shell;
use super::videos::{Wanting, device_px};
use crate::design::Look;
use crate::media_store::Want;
use crate::video_store::VideoWant;

impl Shell {
    /// What the link previews of `messages` (each with whether its clip may only show its
    /// first frame) want: their relayed pictures, kept with their exact sources in `relayed`,
    /// and their clips.
    pub(super) fn previews_wanted(
        &mut self,
        account: AccountId,
        messages: &[(Arc<papo_sdk::Message>, bool)],
        look: &Look,
    ) -> (Vec<Want>, Vec<VideoWant>) {
        let shown = self.conversations.settings().link_previews;
        let boxes = PreviewBoxes {
            embed: self.embed_bounds(look),
            image: self.media_bounds(look),
            lead_gap_pt: crate::link_cards::rule_gap_pt(look, self.message_layout().density),
            scale: self.scale,
        };
        // Each message's previews, worked out once for its pictures and its clips.
        let previews: Vec<(Vec<LinkPreview>, bool)> = messages
            .iter()
            .map(|(message, still)| (crate::link_cards::previews(message, shown), *still))
            .collect();
        let pictures: Vec<(Want, MediaSource)> = previews
            .iter()
            .flat_map(|(previews, _)| pictures_of_previews(account, previews, &boxes))
            .collect();
        self.relayed = pictures
            .iter()
            .map(|(want, source)| (want.key, source.clone()))
            .collect();
        let video_bounds = self.video_bounds(look);
        let clips = previews
            .iter()
            .flat_map(|(previews, still)| {
                clips_wanted(
                    previews,
                    boxes.embed,
                    boxes.lead_gap_pt,
                    &Wanting {
                        account,
                        own_sizes: &self.own_sizes,
                        revealed: &self.revealed,
                        bounds: video_bounds,
                        album_bounds: boxes.image,
                        scale: boxes.scale,
                        still: *still,
                    },
                )
            })
            .collect();
        (pictures.into_iter().map(|(want, _)| want).collect(), clips)
    }
}

/// What a preview's picture box is fitted to in the shown timeline.
struct PreviewBoxes {
    /// Embed size's bounds, for cards.
    embed: MediaBounds,
    /// Image size's, for GIF links.
    image: MediaBounds,
    /// The density's lead gap, beside a card's rule.
    lead_gap_pt: f32,
    scale: f32,
}

/// The relayed pictures `previews` draw, each with the exact source it is fetched from.
fn pictures_of_previews(
    account: AccountId,
    previews: &[LinkPreview],
    boxes: &PreviewBoxes,
) -> Vec<(Want, MediaSource)> {
    previews
        .iter()
        .filter_map(|preview| {
            let picture = crate::link_cards::picture_box(
                preview,
                boxes.embed,
                boxes.image,
                boxes.lead_gap_pt,
            )?;
            let (key, source) = crate::link_cards::picture_key(account, preview)?;
            let gif = crate::link_cards::picture_of(preview)
                .is_some_and(|picture| picture.kind == AttachmentKind::Gif);
            let box_pt = picture.width_pt.max(picture.height_pt);
            Some((
                Want {
                    key,
                    // The relayed picture is the only one: no thumbnail before it.
                    thumbnail_px: None,
                    box_pt,
                    box_px: box_pt * boxes.scale,
                    gif,
                },
                source,
            ))
        })
        .collect()
}

/// What the relayed clips of `previews` want, each alone in its card.
fn clips_wanted(
    previews: &[LinkPreview],
    embed_bounds: MediaBounds,
    lead_gap_pt: f32,
    wanting: &Wanting<'_>,
) -> Vec<VideoWant> {
    previews
        .iter()
        .filter_map(|preview| {
            let (key, _) = crate::link_cards::clip_of(wanting.account, preview)?;
            let boxed = crate::link_cards::clip_box(preview, embed_bounds, lead_gap_pt)?;
            Some(VideoWant {
                key,
                box_px: device_px(boxed, wanting.scale),
                in_album: wanting.still,
                hidden: false,
            })
        })
        .collect()
}

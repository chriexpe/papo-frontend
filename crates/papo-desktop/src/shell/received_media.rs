//! The shell's side of received media (`docs/design.md` §14.4, `docs/papo-desktop.md` §5):
//! which attachments the shown timeline draws (its visible rows and a few beside them), their
//! fetches through the SDK's cache and their decodes off the UI thread, which GIFs show enough
//! to animate, and the sizes of attachments this device sent. What is kept is `media_store`'s;
//! presses are `media_presses`'.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use papo_sdk::{AccountId, Attachment, MediaSource, OperationKind, OutgoingSnapshot, Uuid};
use papo_sdk_ui::{
    AttachmentKind, HOST_HEIGHT_SHARE, MediaBounds, MediaSize, TimelineItem, media_box,
};
use rune::Tx;

use super::Shell;
use super::staged_files::decoded;
use crate::design::{Lead, Look, MESSAGE_ROW_INSET, TIMELINE_INSET};
use crate::gif_frames::gif_frames;
use crate::media_store::{Fetch, Job, MediaKey, MediaOf, Want};
use crate::message_media::received::{MediaStyle, attachment_id, is_hidden, split};

/// Rows beyond the visible ones, on each side, whose media load ahead of being seen.
const NEAR_ROWS: usize = 6;

/// The share of a GIF that must show for it to animate (§14.4: at least half).
const GIF_SHOWN_SHARE: f32 = 0.5;

/// Where the shown timeline's GIFs are and which one the pointer or keyboard is on.
#[derive(Default)]
pub(crate) struct GifPlaces {
    rects: HashMap<MediaKey, [f32; 4]>,
    hovered: Option<MediaKey>,
    /// The system's reduced motion, as the view last saw it.
    reduced_motion: bool,
}

impl Shell {
    /// The largest box media take in the shown timeline (§14.4): the Image size setting, the
    /// content column, and 0.55 × the conversation's height.
    pub(super) fn media_bounds(&self, look: &Look) -> MediaBounds {
        self.bounds_at(self.conversations.settings().image_size, look)
    }

    /// The largest box a video takes in the shown timeline (§14.4, §14.9): the Video size, the
    /// content column, and 0.55 × the conversation's height.
    pub(super) fn video_bounds(&self, look: &Look) -> MediaBounds {
        self.bounds_at(self.conversations.settings().video_size, look)
    }

    /// The largest box a link card's picture or a provider's poster takes in the shown timeline
    /// (§15.7): Embed size, the content column, and 0.55 × the conversation's height.
    pub(super) fn embed_bounds(&self, look: &Look) -> MediaBounds {
        self.bounds_of(self.conversations.settings().embed_size.max_size_pt(), look)
    }

    /// The bounds of a size `setting` in the shown timeline.
    fn bounds_at(&self, setting: papo_sdk_ui::MediaSizeSetting, look: &Look) -> MediaBounds {
        self.bounds_of(setting.max_size_pt(), look)
    }

    /// The bounds of a setting's max width and height (`max_size_pt`) in the shown timeline.
    fn bounds_of(&self, max_size_pt: (u16, u16), look: &Look) -> MediaBounds {
        let [_, _, width_pt, height_pt] = self.conversation_rect;
        let spacing = look.row_spacing(self.message_layout());
        let lead_pt = match spacing.lead {
            Lead::Avatar(size_pt) => size_pt + spacing.lead_gap_pt,
            Lead::Gutter(width_pt) => width_pt + spacing.lead_gap_pt,
            Lead::None => 0.0,
        };
        let insets_pt = 2.0 * (look.space(TIMELINE_INSET) + look.space(MESSAGE_ROW_INSET));
        // Before the conversation is laid out its size is unknown: the setting alone bounds it.
        let known = |value: f32| if value > 0.0 { value } else { f32::INFINITY };
        MediaBounds::of_max_size(
            max_size_pt,
            known(width_pt - insets_pt - lead_pt),
            known(height_pt) * HOST_HEIGHT_SHARE,
        )
    }

    /// What received media in the shown timeline draw with; `reduced_motion` as the system asks.
    pub(super) fn media_style(&self, look: &Look, reduced_motion: bool) -> MediaStyle {
        MediaStyle {
            view: self.media.view(),
            bounds: self.media_bounds(look),
            own_sizes: Arc::clone(&self.own_sizes),
            saves: self.saves.views(),
            reduced_motion,
            interactive: true,
            surface: crate::message_media::received::MediaSurface::Timeline,
            revealed: Arc::clone(&self.revealed),
            viewing: self.viewer.as_ref().map(|viewer| viewer.at),
            video_bounds: self.video_bounds(look),
            embed_bounds: self.embed_bounds(look),
            previews: self.conversations.settings().link_previews,
            videos: self.videos.view(),
            video_surfaces: self.videos.surfaces(),
            video_attention: self.videos.attention,
            video_volume_open: self.videos.volume_open,
            video_volume: self.videos.volume.get(),
            audio: self.audio.store.view(),
            audio_hover: self.audio.hover,
            players: self.players.view(self.players.store.viewer()),
        }
    }

    /// The shown timeline's rows moved: its visible and near pictures are wanted, everything
    /// else may go.
    pub(super) fn want_media(&mut self) {
        // Sizes are the same in every theme.
        let look = self.look(false);
        let bounds = self.media_bounds(&look);
        let video_bounds = self.video_bounds(&look);
        let scale = self.scale;
        let mut audio = Vec::new();
        // Your messages the timeline shows sent before their queue said so.
        let mut sent_audio = Vec::new();
        // The messages whose media show: the conversation's visible and near rows, then the
        // open pins' and the search results' whole messages (§10: media included), whose
        // videos show their first frame only (they play in the conversation).
        let mut shown: Vec<(Arc<papo_sdk::Message>, bool)> = Vec::new();
        let account = self.tabs.shown().map(|open| open.key.account);
        if let Some(open) = self.tabs.shown() {
            let visible = visible_span(open);
            let from = visible.start.saturating_sub(NEAR_ROWS);
            let to = (visible.end + NEAR_ROWS).min(open.items.len());
            for item in open.items.iter().skip(from).take(to.saturating_sub(from)) {
                // Your unsent message's audio plays as a sent one's does.
                if let TimelineItem::LocalEcho { operation, .. } = item.as_ref() {
                    sent_audio.extend(self.sent_audio(open.key.account, operation));
                    audio.extend(self.unsent_audio_wanted(open.key.account, operation));
                }
                if let TimelineItem::Message { message, .. } = item.as_ref() {
                    shown.push((Arc::clone(message), false));
                }
            }
            let pins_open = self
                .pins_panel
                .as_ref()
                .is_some_and(|panel| panel.key == open.key);
            if pins_open && let Some(pins) = open.pins.list() {
                shown.extend(pins.iter().map(|pin| (Arc::clone(&pin.message), true)));
            }
        }
        if let Some(search) = self
            .search
            .as_ref()
            .filter(|search| Some(search.account) == account)
        {
            // Only the results on screen: the panel keeps up to 32 whole messages.
            let on_screen: Vec<Uuid> = match &search.found {
                super::search::Found::Ready(results) => {
                    let range = search.hydrated.visible();
                    results
                        .items()
                        .iter()
                        .skip(range.start)
                        .take(range.len())
                        .map(|item| item.id())
                        .collect()
                }
                _ => Vec::new(),
            };
            shown.extend(
                search
                    .hydrated
                    .messages(on_screen)
                    .map(|message| (Arc::clone(message), true)),
            );
        }
        let mut wants = Vec::new();
        let mut videos = Vec::new();
        if let Some(account) = account {
            for (message, still) in &shown {
                let attachments = message.attachments.as_deref().unwrap_or_default();
                audio.extend(super::audio::audio_wanted(account, attachments));
                wants.extend(pictures_wanted(
                    account,
                    attachments,
                    &self.own_sizes,
                    &self.revealed,
                    bounds,
                    scale,
                ));
                videos.extend(super::videos::videos_wanted(
                    attachments,
                    &super::videos::Wanting {
                        account,
                        own_sizes: &self.own_sizes,
                        revealed: &self.revealed,
                        bounds: video_bounds,
                        album_bounds: bounds,
                        scale,
                        still: *still,
                    },
                ));
            }
            let (pictures, clips) = self.previews_wanted(account, &shown, &look);
            wants.extend(pictures);
            videos.extend(clips);
        }
        self.want_videos(videos);
        for (from, to) in sent_audio {
            self.audio_sent(from, to);
        }
        self.want_audio(audio);
        let wanted: HashSet<MediaKey> = wants.iter().map(|want| want.key).collect();
        self.gif_places.rects.retain(|key, _| wanted.contains(key));
        let jobs = self.media.want(wants);
        self.run_media_jobs(jobs);
        self.want_emoji();
    }

    /// Starts `jobs`: each fetches through the SDK's cache and decodes off the UI thread, then
    /// lands in the store, which may ask for more.
    pub(super) fn run_media_jobs(&mut self, jobs: Vec<Job>) {
        let (Some(sdk), Some(images)) = (self.sdk.clone(), self.images.clone()) else {
            self.media.dropped(&jobs);
            return;
        };
        for job in jobs {
            let images = Arc::clone(&images);
            let scale = self.scale;
            let updates = sdk.updates();
            // An embed's picture is fetched from the exact source its preview names.
            let source = match (job.key.of, job.fetch) {
                (MediaOf::Attachment, Fetch::Thumbnail) => MediaSource::Thumbnail(job.key.id),
                (MediaOf::Attachment, Fetch::Original | Fetch::Animation) => {
                    MediaSource::Attachment(job.key.id)
                }
                (MediaOf::EmbedPicture { .. }, _) => {
                    let Some(source) = self.relayed.get(&job.key) else {
                        self.media.dropped(&[job]);
                        continue;
                    };
                    source.clone()
                }
                // A clip's frames are the video store's.
                (MediaOf::EmbedClip, _) => {
                    self.media.dropped(&[job]);
                    continue;
                }
            };
            sdk.run(move |client| async move {
                let fetched = match client.session(job.key.account) {
                    Ok(session) => session.media().fetch(source, false).await.ok(),
                    Err(_) => None,
                };
                let (image, animated_bytes) = match fetched {
                    None => (None, 0),
                    Some(data) => {
                        let bytes = data.value.bytes;
                        match job.fetch {
                            Fetch::Animation => {
                                let frames = gif_frames(&bytes).unwrap_or(1);
                                let image = decoded(images.load(bytes)).await;
                                let animated = crate::media_store::decoded_bytes(&image, frames);
                                (Some(image), animated)
                            }
                            Fetch::Thumbnail | Fetch::Original => {
                                let image =
                                    decoded(images.load_scaled(bytes, job.box_pt, scale)).await;
                                (Some(image), 0)
                            }
                        }
                    }
                };
                updates.send(move |shell| {
                    let jobs = shell.media.landed(job, image, animated_bytes);
                    shell.run_media_jobs(jobs);
                    true
                });
            });
        }
    }

    /// A hidden (sensitive) picture is revealed: it shows, and stays shown this run. Its
    /// picture is asked for at its full size now.
    pub(super) fn reveal(&mut self, key: MediaKey) {
        if Arc::make_mut(&mut self.revealed).insert(key) {
            self.want_media();
        }
    }

    /// GIF `key` is at `rect`; which GIFs animate may change.
    fn gif_moved(&mut self, key: MediaKey, rect: [f32; 4], reduced_motion: bool) -> bool {
        self.gif_places.rects.insert(key, rect);
        self.gif_places.reduced_motion = reduced_motion;
        self.show_gifs()
    }

    /// The GIFs showing enough to animate, most shown first, to the store. Returns whether any
    /// started or stopped animating.
    pub(super) fn show_gifs(&mut self) -> bool {
        let animate = self
            .conversations
            .settings()
            .animates_gifs(self.gif_places.reduced_motion);
        let span = self
            .tabs
            .shown()
            .and_then(|open| open.tracker.shown_span_pt());
        let mut showing: Vec<(f32, MediaKey)> = match span {
            None => Vec::new(),
            Some((top_pt, bottom_pt)) => self
                .gif_places
                .rects
                .iter()
                .filter(|(key, _)| animate || self.gif_places.hovered == Some(**key))
                .filter_map(|(key, rect)| {
                    let shown = shown_share(*rect, top_pt, bottom_pt);
                    (shown >= GIF_SHOWN_SHARE).then_some((shown, *key))
                })
                .collect(),
        };
        showing.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        let before = self.media.view();
        let jobs = self
            .media
            .show_gifs(showing.into_iter().map(|(_, key)| key).collect());
        self.run_media_jobs(jobs);
        let after = self.media.view();
        self.gif_places
            .rects
            .keys()
            .any(|key| before.animates(*key) != after.animates(*key))
    }

    /// The pointer or keyboard came to (or left) GIF `key`.
    fn gif_hovered(&mut self, key: MediaKey, on: bool, reduced_motion: bool) -> bool {
        self.gif_places.reduced_motion = reduced_motion;
        if on {
            self.gif_places.hovered = Some(key);
        } else if self.gif_places.hovered == Some(key) {
            self.gif_places.hovered = None;
        }
        self.show_gifs()
    }

    /// Account `account`'s queue changed: an unsent message that was just sent leaves the
    /// sizes this device read from its files, so the received message is drawn at the same
    /// size and its row keeps its height (§9).
    pub(super) fn keep_own_sizes(&mut self, outgoing: &OutgoingSnapshot) {
        for operation in &outgoing.operations {
            if operation.state != papo_sdk::OperationState::Sent {
                continue;
            }
            let (OperationKind::Message(_), Some(sent)) =
                (operation.kind.as_ref(), operation.message.as_deref())
            else {
                continue;
            };
            let Some(entry) = self.unsent.get(operation.id) else {
                continue;
            };
            let received = sent.attachments.as_deref().unwrap_or_default();
            if received.len() != entry.files.len() {
                continue;
            }
            for (attachment, file) in received.iter().zip(entry.files.iter()) {
                if let Some(id) = attachment_id(attachment)
                    && !self.own_sizes.contains_key(&id)
                {
                    Arc::make_mut(&mut self.own_sizes).insert(id, file.media_size);
                }
            }
        }
    }
}

/// Where `open`'s visible rows are in its items now: around the rows visible at the last
/// report, found by ID, since a page arriving above moves every index without a new report;
/// the reported range before any row is known.
fn visible_span(open: &crate::timeline::OpenTimeline) -> std::ops::Range<usize> {
    let ids = open.tracker.visible_ids();
    let places: Vec<usize> = open
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item.as_ref() {
            TimelineItem::Message { id, .. } if ids.contains(id) => Some(index),
            _ => None,
        })
        .collect();
    match (places.iter().min(), places.iter().max()) {
        (Some(first), Some(last)) => *first..*last + 1,
        _ => open.visible_rows(),
    }
}

/// The share of `rect`'s height between `top_pt` and `bottom_pt`.
pub(super) fn shown_share(rect: [f32; 4], top_pt: f32, bottom_pt: f32) -> f32 {
    let [_, y, _, height] = rect;
    if height <= 0.0 {
        return 0.0;
    }
    let shown = (y + height).min(bottom_pt) - y.max(top_pt);
    (shown / height).clamp(0.0, 1.0)
}

/// What a message's pictures want: each one's thumbnail, box and kind. Only the cells an album
/// draws are wanted.
fn pictures_wanted(
    account: AccountId,
    attachments: &[Attachment],
    own_sizes: &HashMap<Uuid, MediaSize>,
    revealed: &HashSet<MediaKey>,
    bounds: MediaBounds,
    scale: f32,
) -> Vec<Want> {
    let (pictures, _) = split(attachments);
    let album = papo_sdk_ui::AlbumLayout::for_count(pictures.len());
    let shown = album.map_or(pictures.len(), papo_sdk_ui::AlbumLayout::shown);
    // Every cell is decoded for the album's box: cells are cropped from it.
    let album_box = album.map(|layout| layout.album_box(bounds));
    pictures
        .iter()
        .take(shown)
        .filter_map(|index| {
            let attachment = &attachments[*index];
            let id = attachment_id(attachment)?;
            let size = own_sizes
                .get(&id)
                .copied()
                .unwrap_or_else(|| MediaSize::of(attachment));
            let boxed = album_box.unwrap_or_else(|| media_box(size, bounds));
            let box_pt = boxed.width_pt.max(boxed.height_pt);
            let thumbnail_px = attachment
                .thumbnail_id
                .as_ref()
                .and_then(|_| {
                    let pixels = |value: Option<i64>| value.and_then(|v| u32::try_from(v).ok());
                    pixels(attachment.thumbnail_width).zip(pixels(attachment.thumbnail_height))
                })
                .filter(|(width, height)| *width > 0 && *height > 0);
            // A hidden picture draws blurred: its thumbnail is enough until it is revealed.
            let hidden = is_hidden(account, attachment, revealed);
            let thumbnail_long_px = thumbnail_px.map(|(width, height)| width.max(height));
            #[expect(clippy::cast_precision_loss, reason = "a thumbnail's side, up to 1024")]
            let wanted_px = match thumbnail_long_px {
                Some(long_px) if hidden => (box_pt * scale).min(long_px as f32),
                _ => box_pt * scale,
            };
            Some(Want {
                key: MediaKey::attachment(account, id),
                thumbnail_px,
                box_pt,
                box_px: wanted_px,
                gif: AttachmentKind::of(attachment) == AttachmentKind::Gif,
            })
        })
        .collect()
}

impl crate::message_media::received::MediaHost for Shell {
    fn media_press(
        &mut self,
        at: crate::message_media::received::MediaAt,
        press: crate::message_media::received::MediaPress,
        tx: &mut Tx,
    ) {
        self.worked_on(at.surface);
        self.press_media(at, press, tx);
    }

    fn media_moved(&mut self, key: MediaKey, rect: [f32; 4], reduced_motion: bool, tx: &mut Tx) {
        if !self.gif_moved(key, rect, reduced_motion) {
            tx.live_only();
        }
    }

    fn media_hovered(&mut self, key: MediaKey, on: bool, reduced_motion: bool, tx: &mut Tx) {
        if !self.gif_hovered(key, on, reduced_motion) {
            tx.live_only();
        }
    }

    fn saved_pill_shown(&mut self, tx: &mut Tx) {
        self.saved_pill_show(tx);
    }

    fn video_press(
        &mut self,
        at: crate::message_media::received::MediaAt,
        key: MediaKey,
        press: crate::message_media::video::VideoPress,
        tx: &mut Tx,
    ) {
        self.worked_on(at.surface);
        self.press_video(at, key, press, tx);
    }

    fn video_moved(&mut self, key: MediaKey, rect: [f32; 4], reduced_motion: bool, tx: &mut Tx) {
        if !Shell::video_moved(self, key, rect, reduced_motion) {
            tx.live_only();
        }
    }

    fn video_attention(&mut self, key: MediaKey, on: bool, tx: &mut Tx) {
        let before = self.videos.attention;
        if on {
            self.videos.attention = Some(key);
        } else if self.videos.attention == Some(key) {
            self.videos.attention = None;
        }
        if self.videos.attention == before {
            tx.live_only();
        } else {
            // The bar's seek bar moves by quarter seconds, the rest by seconds.
            self.videos.next_clock_s = None;
        }
    }

    fn audio_press(
        &mut self,
        at: crate::message_media::received::MediaAt,
        key: crate::audio_store::AudioKey,
        press: crate::message_media::audio::AudioPress,
        tx: &mut Tx,
    ) {
        // The pointer passing over a waveform is no work: an opened pin or result stays open.
        if !matches!(press, crate::message_media::audio::AudioPress::Hover(_)) {
            self.worked_on(at.surface);
        }
        self.press_audio(at, key, press, tx);
    }

    fn video_volume_open(&mut self, key: MediaKey, on: bool, tx: &mut Tx) {
        let before = self.videos.volume_open;
        if on {
            self.videos.volume_open = Some(key);
        } else if self.videos.volume_open == Some(key) {
            self.videos.volume_open = None;
        }
        if self.videos.volume_open == before {
            tx.live_only();
        }
    }
}

#[cfg(test)]
mod tests {
    //! How much of a box shows between the pills.

    use super::shown_share;

    #[test]
    fn the_share_of_a_box_between_the_edges() {
        assert!((shown_share([0.0, 100.0, 10.0, 100.0], 0.0, 500.0) - 1.0).abs() < 1e-6);
        assert!((shown_share([0.0, 450.0, 10.0, 100.0], 0.0, 500.0) - 0.5).abs() < 1e-6);
        assert!(shown_share([0.0, 600.0, 10.0, 100.0], 0.0, 500.0).abs() < 1e-6);
        assert!(shown_share([0.0, 0.0, 10.0, 0.0], 0.0, 500.0).abs() < 1e-6);
    }
}

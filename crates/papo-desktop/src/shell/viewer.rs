//! The shell's side of the viewer (`docs/design.md` §14.8): opening it on a message's picture,
//! fetching each item's whole picture (decoded in full, a GIF animating whatever the setting
//! says), Copy image, Save, Jump to message, and closing it with the keyboard back on the
//! media; its input is `shell::viewer_input`'s. One whole picture at a time: the viewer owns it and lets it
//! go when it steps or closes.

use std::sync::Arc;

use papo_sdk::Message;
use papo_sdk_ui::{MediaSize, TimelineItem};
use rune::Tx;

use super::Shell;
use super::media_presses::MediaAction;
use super::staged_files::decoded;
use crate::message_media::received::{MediaAt, MediaSurface, is_hidden};
use crate::strings;
use crate::timeline::local_time::day_label;
use crate::timeline::{author_name, created_us};
use crate::viewer::view::VIEWER_KEY;
use crate::viewer::{CLEAR_SIDE_PT, CLEAR_TOP_PT, Viewer};

/// How long the full picture may take before its ring shows, in seconds.
const RING_AFTER_S: f64 = 0.5;

/// A shape's long side when only the shape is known: large enough to fit any room.
const SHAPE_LONG_PX: f32 = 8192.0;

impl Shell {
    /// Message `message` of `account`, from the shown timeline or its pins.
    pub(super) fn message_at(&self, at: MediaAt) -> Option<Arc<Message>> {
        let open = self
            .tabs
            .shown()
            .filter(|open| open.key.account == at.account)?;
        open.items
            .iter()
            .find_map(|item| match item.as_ref() {
                TimelineItem::Message { id, message, .. } if *id == at.message => {
                    Some(Arc::clone(message))
                }
                _ => None,
            })
            .or_else(|| {
                open.pins
                    .get(at.message)
                    .map(|pin| Arc::clone(&pin.message))
            })
    }

    /// Opens the viewer on media `at`: the message's pictures, in order.
    pub(super) fn open_viewer(&mut self, at: MediaAt, tx: &mut Tx) {
        let Some(message) = self.message_at(at) else {
            return;
        };
        let items = self.viewer_items(at.account, &message);
        if items.is_empty() {
            return;
        }
        let resources = self.tabs.shown().and_then(|open| open.resources.clone());
        let author = author_name(&message, resources.as_deref());
        let when = created_us(&message).map_or_else(String::new, |at_us| {
            let day = self.local_time.day_of(at_us);
            strings::date_time(
                &day_label(day, self.today),
                &self.local_time.time_label(at_us),
            )
        });
        self.nav.menu = None;
        // The picture grows out of its box over the whole window, the pills' clear band
        // included (§14.8).
        tx.unclipped_flights();
        // The picture's place is known in the commit that opens the viewer, before the room
        // reports its layout, so the growth aims at where the picture comes to rest.
        self.viewer_room = self.expected_viewer_room();
        self.viewer = Some(Viewer::new(at, items, author, when));
        self.load_viewer_picture(tx);
        self.follow_viewer_video(tx);
        tx.focus(VIEWER_KEY, true);
    }

    /// O on a focused message (§12): the viewer on its first picture. Returns whether it has
    /// one.
    pub(super) fn open_message_media(
        &mut self,
        key: crate::timeline::TimelineKey,
        id: papo_sdk::Uuid,
        tx: &mut Tx,
    ) -> bool {
        let at = MediaAt {
            surface: MediaSurface::Timeline,
            account: key.account,
            channel: key.channel,
            message: id,
            index: 0,
        };
        let Some(message) = self.message_at(at) else {
            return false;
        };
        let Some(first) = self
            .viewer_items(at.account, &message)
            .first()
            .map(|item| item.index)
        else {
            return false;
        };
        self.open_viewer(MediaAt { index: first, ..at }, tx);
        true
    }

    /// Fetches the shown item's whole picture and decodes it in full.
    pub(super) fn load_viewer_picture(&mut self, tx: &mut Tx) {
        let (Some(sdk), Some(images)) = (self.sdk.clone(), self.images.clone()) else {
            return;
        };
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        // A video plays rather than loads a picture.
        let Some((key, source)) = viewer
            .item()
            .filter(|item| !item.video)
            .map(|item| (item.key, item.source.clone()))
        else {
            return;
        };
        viewer.asked_s = Some(tx.clock());
        tx.wake_after(std::time::Duration::from_secs_f64(RING_AFTER_S));
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let fetched = match client.session(key.account) {
                Ok(session) => session.media().fetch(source, false).await.ok(),
                Err(_) => None,
            };
            let image = match fetched {
                Some(data) => Some(decoded(images.load(data.value.bytes)).await),
                None => None,
            };
            updates.send(move |shell| {
                // Stepped or closed meanwhile: the picture goes with this closure.
                let shown = shell.viewer.as_ref().and_then(Viewer::item);
                if shown.map(|item| item.key) != Some(key) {
                    return false;
                }
                let natural_before = shell.viewer_natural();
                let Some(viewer) = shell.viewer.as_mut() else {
                    return false;
                };
                match image.filter(|image| image.error().is_none()) {
                    Some(image) => viewer.full = Some(image),
                    None => viewer.failed = true,
                }
                // A zoom made meanwhile stays, unless the picture's real size differs from
                // what it was made against (EXIF orientation, an estimated shape).
                // Pixel sides: a change of half a pixel or more is a different size.
                let natural_after = shell.viewer_natural();
                let resized = natural_after
                    .iter()
                    .zip(natural_before)
                    .any(|(after, before)| (after - before).abs() >= 0.5);
                if resized && let Some(viewer) = shell.viewer.as_mut() {
                    viewer.fit();
                }
                true
            });
        });
    }

    /// The room the viewer's stage will have: the window (the conversation reaches its bottom
    /// right corner) less the pills' clear band at the top and the margins at the sides and
    /// below.
    fn expected_viewer_room(&self) -> [f32; 2] {
        let [x, y, width, height] = self.conversation_rect;
        let window = [x + width, y + height];
        [
            (window[0] - 2.0 * CLEAR_SIDE_PT).max(0.0),
            (window[1] - CLEAR_TOP_PT - CLEAR_SIDE_PT).max(0.0),
        ]
    }

    /// The viewer's room was laid out at `rect`.
    pub(crate) fn viewer_room(&mut self, rect: [f32; 4]) {
        self.viewer_room = [rect[2], rect[3]];
    }

    /// The picture's size in pixels as far as it is known: the whole picture's once decoded,
    /// else the attachment's own size, else its shape made large enough to fit any room.
    pub(super) fn viewer_natural(&self) -> [f32; 2] {
        let Some(viewer) = &self.viewer else {
            return [1.0, 1.0];
        };
        if let Some([width, height]) = viewer
            .full
            .as_ref()
            .filter(|image| image.is_ready())
            .and_then(rune::Image::size)
        {
            #[expect(clippy::cast_precision_loss, reason = "picture sides")]
            return [width as f32, height as f32];
        }
        // A video's size as its probe read it.
        let probed = viewer
            .item()
            .filter(|item| item.video)
            .and_then(|item| self.videos.view().get(item.key).copied())
            .and_then(|video| match video.reading {
                crate::video_store::Reading::Read(info) => info.video_px,
                _ => None,
            });
        if let Some([width, height]) = probed {
            #[expect(clippy::cast_precision_loss, reason = "video sides")]
            return [width as f32, height as f32];
        }
        let size = viewer.item().map_or(MediaSize::Unknown, |item| item.size);
        #[expect(clippy::cast_precision_loss, reason = "picture sides")]
        match size {
            MediaSize::Exact {
                width_px,
                height_px,
            } => [width_px as f32, height_px as f32],
            MediaSize::Shape {
                width_px,
                height_px,
            } => {
                let long = width_px.max(height_px) as f32;
                let grow = SHAPE_LONG_PX / long;
                [width_px as f32 * grow, height_px as f32 * grow]
            }
            MediaSize::Unknown => [SHAPE_LONG_PX, SHAPE_LONG_PX * 9.0 / 16.0],
        }
    }

    /// The shown item's attachment, as the menu names it.
    pub(super) fn viewer_at(&self) -> Option<MediaAt> {
        let viewer = self.viewer.as_ref()?;
        let item = viewer.item()?;
        Some(MediaAt {
            index: item.index,
            ..viewer.at
        })
    }

    /// Copy image (Ctrl+C).
    pub(crate) fn viewer_copy(&mut self, tx: &mut Tx) {
        let video = self
            .viewer
            .as_ref()
            .and_then(Viewer::item)
            .is_some_and(|item| item.video);
        if video {
            return;
        }
        if let Some(at) = self.viewer_at() {
            self.media_action(at, MediaAction::CopyImage, tx);
        }
    }

    /// Save (Ctrl+S): into Save media to, ending with the saved pill.
    pub(crate) fn viewer_save(&mut self, tx: &mut Tx) {
        if let Some(at) = self.viewer_at() {
            self.media_action(at, MediaAction::Save, tx);
        }
    }

    /// Jump to message: the viewer closes and the message opens the way pins do.
    pub(crate) fn viewer_jump(&mut self, tx: &mut Tx) {
        let Some(viewer) = self.viewer.take() else {
            return;
        };
        self.follow_viewer_video(tx);
        if viewer.full_screen {
            tx.toggle_full_screen();
        }
        let key = crate::timeline::TimelineKey {
            account: viewer.at.account,
            channel: viewer.at.channel,
        };
        let cursor = self
            .message_at(viewer.at)
            .and_then(|message| message.cursor());
        self.open_message(key, viewer.at.message, cursor, true, tx);
    }

    /// Closes the viewer; the keyboard goes back to the media it opened from.
    pub(crate) fn close_viewer(&mut self, tx: &mut Tx) {
        let Some(viewer) = self.viewer.take() else {
            return;
        };
        self.follow_viewer_video(tx);
        if viewer.full_screen {
            tx.toggle_full_screen();
        }
        // It shrinks back into its box, crossing the pills' band.
        tx.unclipped_flights();
        tx.focus(viewer.return_focus(), true);
    }

    /// Whether the item showing is sensitive and not revealed yet.
    pub(super) fn viewer_hidden(&self) -> bool {
        let Some(at) = self.viewer_at() else {
            return false;
        };
        self.attachment_at(at)
            .is_some_and(|attachment| is_hidden(at.account, &attachment, &self.revealed))
    }

    /// A press (or Enter, Space) on a hidden item: it shows, here and in the conversation.
    pub(crate) fn viewer_reveal(&mut self) {
        let key = self
            .viewer
            .as_ref()
            .and_then(Viewer::item)
            .map(|item| item.key);
        if let Some(key) = key {
            self.reveal(key);
        }
    }

    /// A failed whole picture's Retry.
    pub(crate) fn viewer_retry(&mut self, tx: &mut Tx) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.failed = false;
        }
        self.load_viewer_picture(tx);
    }

    /// The conversation shown changed, or its account went: a viewer on another one closes,
    /// leaving the full screen it entered when `tx` is there to.
    pub(super) fn viewer_follows(&mut self, tx: Option<&mut Tx>) {
        let shown = self.tabs.shown().map(|open| open.key);
        let on = self
            .viewer
            .as_ref()
            .map(|viewer| crate::timeline::TimelineKey {
                account: viewer.at.account,
                channel: viewer.at.channel,
            });
        if on.is_none() || on == shown {
            return;
        }
        let closed = self.viewer.take();
        self.stop_viewer_video();
        if let Some(viewer) = closed
            && viewer.full_screen
            && let Some(tx) = tx
        {
            tx.toggle_full_screen();
        }
    }

    /// The whole picture has taken half a second by `clock_s`: its ring shows. Returns whether
    /// the view changed.
    pub(super) fn poll_viewer_ring(&mut self, clock_s: f64) -> bool {
        let Some(viewer) = self.viewer.as_mut() else {
            return false;
        };
        let decoded = viewer.full.as_ref().is_some_and(rune::Image::is_ready);
        let due = !decoded
            && !viewer.failed
            && viewer
                .asked_s
                .is_some_and(|asked| clock_s - asked >= RING_AFTER_S);
        if due != viewer.ring {
            viewer.ring = due;
            return true;
        }
        false
    }
}

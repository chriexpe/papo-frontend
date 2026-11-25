//! A received message's attachments (`docs/design.md` §14.4, §14.6): its pictures (JPEG, PNG,
//! WebP and GIF) in the media box, one alone or an album in their order, and every other file
//! as a card, one per line. Videos and audio are cards until they play (D7c).
//!
//! The box is reserved from what the attachment says of its size (the SDK's rules), or, for one
//! this device sent, from the size it read from the file, so the row is the height its unsent
//! row was. A picture shows `surface.hover` while it loads, then the server's thumbnail, then
//! the original when the thumbnail is smaller than the box; a GIF animates while
//! `media_store` says so. A press opens the viewer, a secondary press or the menu key the media
//! menu; a file card saves.

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{AccountId, Attachment, Uuid};
use papo_sdk_ui::{AlbumLayout, AttachmentKind, MediaBounds, MediaFit, MediaSize, media_box};
use rune::{El, Key, Tx, col, el, text};

use super::{
    album, failed, file_card_view::file_card_of, gif_badge, inner_edge, more_mark, painted,
    single_box,
};
use crate::design::{FOCUS_RING_PT, Look, MEDIA_BLOCK_GAP, TYPE_CAPTION_MEDIUM};
use crate::media_store::{MediaKey, MediaView};
use crate::saving::SaveView;
use crate::strings;

/// What a press on received media asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MediaPress {
    /// Open the viewer at the attachment.
    Open,
    /// Open the media menu, beside the pointer or (from the keyboard) beside the media.
    Menu(Option<[f32; 2]>),
    /// A picture that failed: ask for it again.
    Retry,
    /// A file card's Save, or Try again after a failed save.
    Save,
    /// A file card's ✕ while it saves.
    CancelSave,
    /// A saved file's Open, through the system.
    OpenSaved,
    /// A saved file's Show in folder.
    ShowSaved,
}

/// Where a message's media is drawn: the same message can show in more than one place at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MediaSurface {
    /// The conversation.
    Timeline,
    /// The pins panel.
    Pins,
    /// A confirmation showing the message.
    Confirm,
}

/// Which attachment of which message, in the view that draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MediaAt {
    pub(crate) surface: MediaSurface,
    pub(crate) account: AccountId,
    pub(crate) channel: Uuid,
    pub(crate) message: Uuid,
    /// Its place among the message's attachments.
    pub(crate) index: usize,
}

/// What received media asks of the app.
pub(crate) trait MediaHost: Sized + 'static {
    /// A press on attachment `at`.
    fn media_press(&mut self, at: MediaAt, press: MediaPress, tx: &mut Tx);
    /// A GIF's box is at `rect` (window points), for whether it shows (§5); `reduced_motion`
    /// is the system's, which Animate GIFs follows unless set.
    fn media_moved(&mut self, key: MediaKey, rect: [f32; 4], reduced_motion: bool, tx: &mut Tx);
    /// The pointer or the keyboard came to (or left) a GIF: with Animate GIFs off, it plays
    /// while there.
    fn media_hovered(&mut self, key: MediaKey, on: bool, reduced_motion: bool, tx: &mut Tx);
    /// The saved pill's Show in folder.
    fn saved_pill_shown(&mut self, tx: &mut Tx);
}

/// What every received attachment in one view draws with.
#[derive(Clone)]
pub(crate) struct MediaStyle {
    pub(crate) view: MediaView,
    pub(crate) bounds: MediaBounds,
    /// Sizes of attachments this device sent, read from the files.
    pub(crate) own_sizes: Arc<HashMap<Uuid, MediaSize>>,
    /// File cards' saves.
    pub(crate) saves: Arc<HashMap<MediaKey, SaveView>>,
    /// The system asks for reduced motion (Animate GIFs follows it unless set).
    pub(crate) reduced_motion: bool,
    /// Media take presses and the keyboard; off inside a confirmation, which shows the message
    /// only.
    pub(crate) interactive: bool,
    /// Where these media are drawn.
    pub(crate) surface: MediaSurface,
}

/// The key of attachment `at`'s element (a picture or a file card), unique across surfaces:
/// menus grow beside it and the keyboard comes back to it.
pub(crate) fn media_element_key(at: MediaAt) -> String {
    match at.surface {
        MediaSurface::Timeline => format!("media-{}-{}", at.message, at.index),
        MediaSurface::Pins => format!("media-pin-{}-{}", at.message, at.index),
        MediaSurface::Confirm => format!("media-confirm-{}-{}", at.message, at.index),
    }
}

impl MediaStyle {
    /// Media within `bounds`, nothing loaded, saved or sent from this device yet.
    #[cfg(test)]
    pub(crate) fn new(bounds: MediaBounds) -> MediaStyle {
        MediaStyle {
            view: MediaView::default(),
            bounds,
            own_sizes: Arc::default(),
            saves: Arc::default(),
            reduced_motion: false,
            interactive: true,
            surface: MediaSurface::Timeline,
        }
    }

    /// What `attachment` is drawn at: its size as this device read it, else as the server
    /// says.
    pub(crate) fn size_of(&self, attachment: &Attachment) -> MediaSize {
        attachment_id(attachment)
            .and_then(|id| self.own_sizes.get(&id).copied())
            .unwrap_or_else(|| MediaSize::of(attachment))
    }
}

/// An attachment's ID, when it parses.
pub(crate) fn attachment_id(attachment: &Attachment) -> Option<Uuid> {
    attachment
        .id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// The attachments of a message, split into pictures and cards, each with its place.
pub(crate) fn split(attachments: &[Attachment]) -> (Vec<usize>, Vec<usize>) {
    (0..attachments.len()).partition(|index| AttachmentKind::of(&attachments[*index]).is_picture())
}

/// The block of message `message`'s attachments at `at` (its account and channel); none when
/// it has none.
pub(crate) fn attachments<S: MediaHost>(
    at: MediaAt,
    attachments: &[Attachment],
    style: &MediaStyle,
    look: &Look,
) -> Option<El> {
    if attachments.is_empty() {
        return None;
    }
    let (pictures, cards) = split(attachments);
    let mut block = col().gap(look.space(MEDIA_BLOCK_GAP));
    let picture_at = |index: usize| MediaAt { index, ..at };
    match pictures.as_slice() {
        [] => {}
        [one] => {
            let attachment = &attachments[*one];
            let boxed = media_box(style.size_of(attachment), style.bounds);
            let base = single_box(boxed, look);
            let cell = picture::<S>(
                base,
                picture_at(*one),
                attachment,
                boxed.fit,
                0,
                style,
                look,
            );
            block = block.child(cell.child(inner_edge(look)));
        }
        many => {
            if let Some(layout) = AlbumLayout::for_count(many.len()) {
                let boxed = layout.album_box(style.bounds);
                let last = layout.shown() - 1;
                let cells = many
                    .iter()
                    .take(layout.shown())
                    .enumerate()
                    .map(|(place, index)| {
                        let more = if place == last { layout.more() } else { 0 };
                        let attachment = &attachments[*index];
                        picture::<S>(
                            el(),
                            picture_at(*index),
                            attachment,
                            MediaFit::Fill,
                            more,
                            style,
                            look,
                        )
                    })
                    .collect();
                block = block.child(album(layout, boxed, cells, many.len(), look));
            }
        }
    }
    for index in cards {
        block = block.child(file_card_of::<S>(
            picture_at(index),
            &attachments[index],
            style,
            look,
        ));
    }
    Some(block.key(format!("{}-block", media_element_key(at))))
}

/// One picture: what has loaded (or the failure with Retry), the GIF badge, "+N" when `more`
/// follow, drawn on `base` (the box, or an album's cell), focusable, opening the viewer on a
/// press and the media menu on a secondary press (Rune sends the menu key and ⇧F10 there too).
fn picture<S: MediaHost>(
    base: El,
    at: MediaAt,
    attachment: &Attachment,
    fit: MediaFit,
    more: usize,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let gif = AttachmentKind::of(attachment) == AttachmentKind::Gif;
    let key = attachment_id(attachment).map(|id| MediaKey {
        account: at.account,
        attachment: id,
    });
    let drawn = key.and_then(|key| style.view.get(key));
    let animate = key.is_some_and(|key| style.view.animates(key));
    let image = drawn.and_then(|drawn| drawn.picture(animate));
    let name = attachment.original_file_name.as_deref().unwrap_or_default();
    let label = if gif {
        strings::media_gif().to_owned()
    } else {
        strings::media_image(name)
    };
    let mut cell = painted(base, image, fit, look)
        .key(media_element_key(at))
        .role("img")
        .label(label);
    if style.interactive {
        cell = cell
            .role("button")
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| host.media_press(at, MediaPress::Open, tx))
            .on_secondary_press(move |host: &mut S, press, tx| {
                host.media_press(at, MediaPress::Menu(Some([press.x, press.y])), tx);
            })
            .on_key(move |host: &mut S, event, tx| media_key(host, at, event, tx));
    }
    if style.interactive && drawn.is_some_and(|drawn| drawn.failed) {
        let retry = text(strings::media_retry())
            .font(look.font(TYPE_CAPTION_MEDIUM))
            .color(look.palette.link)
            .role("button")
            .focusable(true)
            .on_press_tx(move |host: &mut S, tx| host.media_press(at, MediaPress::Retry, tx));
        cell = cell.child(failed(retry, look));
    }
    if gif {
        cell = cell.child(gif_badge(look));
        // Only the conversation's GIFs animate by their place in it.
        let timeline = style.interactive && at.surface == MediaSurface::Timeline;
        if let Some(key) = key.filter(|_| timeline) {
            let reduced = style.reduced_motion;
            cell = cell
                .on_layout_tx(move |host: &mut S, rect, tx| {
                    host.media_moved(key, rect, reduced, tx);
                })
                .on_hover_change(move |host: &mut S, on, tx| {
                    host.media_hovered(key, on, reduced, tx);
                })
                .on_focus_change(move |host: &mut S, on, tx| {
                    host.media_hovered(key, on, reduced, tx);
                });
        }
    }
    if more > 0 {
        cell = cell.child(more_mark(more, look));
    }
    cell
}

/// Keys on focused media: Enter or Space opens the viewer. (The menu key and ⇧F10 reach the
/// secondary press.)
pub(crate) fn media_key<S: MediaHost>(
    host: &mut S,
    at: MediaAt,
    event: &rune::KeyEvent,
    tx: &mut Tx,
) -> bool {
    let mods = event.mods;
    if mods.ctrl || mods.cmd || mods.alt || mods.logo || mods.shift {
        return false;
    }
    let opens = match &event.key {
        Key::Enter => true,
        Key::Text(space) => space == " ",
        _ => false,
    };
    if opens {
        host.media_press(at, MediaPress::Open, tx);
    }
    opens
}

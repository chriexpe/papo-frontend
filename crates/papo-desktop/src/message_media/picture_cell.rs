//! One picture cell in a message (`docs/design.md` §14.4): what has loaded, or the failure
//! with Retry, the GIF badge and "+N", focusable, opening the viewer on a press and the media
//! menu on a secondary press, the same for an attachment's picture and a link preview's
//! relayed one. Which picture it is and its box are the caller's.

use rune::{El, Key, Tx, text};

use super::received::{MediaAt, MediaHost, MediaPress, MediaStyle, MediaSurface};
use super::received::{media_element_key, viewer_match_id};
use super::{Layer, failed, gif_badge, more_mark, painted};
use crate::design::{FOCUS_RING_PT, Look, TYPE_CAPTION_MEDIUM};
use crate::media_store::MediaKey;
use crate::strings;
use papo_sdk_ui::MediaFit;

/// What a picture cell shows, apart from where it comes from: an attachment's, or a link
/// preview's relayed picture.
pub(crate) struct Shown {
    /// Its key in the store; none for an attachment without an ID.
    pub(crate) key: Option<MediaKey>,
    pub(crate) gif: bool,
    /// Sensitive and not revealed: blurred, still, and revealed by a press.
    pub(crate) hidden: bool,
    /// What assistive technology reads.
    pub(crate) label: String,
}

/// One picture: what has loaded (or the failure with Retry), the GIF badge, "+N" when `more`
/// follow, drawn on `base` (the box, or an album's cell), focusable, opening the viewer on a
/// press and the media menu on a secondary press (Rune sends the menu key and ⇧F10 there too).
pub(crate) fn picture_cell<S: MediaHost>(
    base: El,
    at: MediaAt,
    shown: &Shown,
    fit: MediaFit,
    more: usize,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let key = shown.key;
    let hidden = shown.hidden;
    let drawn = key.and_then(|key| style.view.get(key));
    // A hidden GIF stays still.
    let animate = !hidden && key.is_some_and(|key| style.view.animates(key));
    let image = drawn.and_then(|drawn| drawn.picture(animate));
    let layer = Layer {
        hidden,
        in_viewer: style.viewing == Some(at),
        matched: style.interactive.then(|| viewer_match_id(at)),
    };
    // A press on a hidden picture reveals it; the viewer and the menu still open it on purpose.
    let press = if hidden {
        MediaPress::Reveal
    } else {
        MediaPress::Open
    };
    let mut cell = painted(base, image, fit, layer, look)
        .key(media_element_key(at))
        .role("img")
        .label(shown.label.clone());
    if style.interactive {
        cell = cell
            .role("button")
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| host.media_press(at, press, tx))
            .on_secondary_press(move |host: &mut S, press, tx| {
                host.media_press(at, MediaPress::Menu(Some([press.x, press.y])), tx);
            })
            .on_key(move |host: &mut S, event, tx| media_key(host, at, press, event, tx));
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
    if shown.gif {
        cell = cell.child(gif_badge(look));
        // Only the conversation's GIFs animate by their place in it.
        let timeline = style.interactive && at.surface == MediaSurface::Timeline && !hidden;
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

/// Keys on focused media: Enter or Space does what a press does (`press`: open the viewer, or
/// reveal a hidden picture). (The menu key and ⇧F10 reach the secondary press.)
pub(crate) fn media_key<S: MediaHost>(
    host: &mut S,
    at: MediaAt,
    press: MediaPress,
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
        host.media_press(at, press, tx);
    }
    opens
}

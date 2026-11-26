//! The layers a provider player's box draws over its poster (`docs/design.md` §15.3): the
//! ask's panel of dark glass, the loading ring, a live player's two glass buttons, the note on
//! a released poster and the failure. Pure; which shows when is `player`'s.

use rune::edit::KeyEvent;
use rune::{Color, El, LoopSpec, col, el, icon, row, text};

use super::{Consent, PlayerEvent, ask_play_key, page_key, pressed_by_keys};
use crate::design::{
    FOCUS_RING_PT, Look, MEDIA_BADGE_ALPHA, MEDIA_BADGE_RADIUS, PLAYER_ASK_ALPHA,
    PLAYER_ASK_BLUR_PT, PLAYER_ASK_GAP_PT, PLAYER_ASK_PAD_PT, PLAYER_ASK_RGB, PLAYER_BUTTON_GAP_PT,
    PLAYER_BUTTON_GHOST_ALPHA, PLAYER_BUTTON_PAD_PT, PLAYER_BUTTON_PT,
    PLAYER_BUTTON_SECONDARY_ALPHA, PLAYER_FAIL_GAP_PT, PLAYER_FAIL_ICON_PT, PLAYER_GLASS_ALPHA,
    PLAYER_GLASS_BLUR_PT, PLAYER_GLASS_HOVER_ALPHA, PLAYER_GLASS_ICON_PT, PLAYER_GLASS_PT,
    PLAYER_GLASS_RGB, PLAYER_NOTE_PAD_PT, PLAYER_RING_ICON_PT, PLAYER_RING_PERIOD_S,
    PLAYER_RING_PT, RADIUS_CONTROL, TYPE_CAPTION_MEDIUM, TYPE_SMALL, TYPE_SMALL_MEDIUM,
    VIDEO_MARK_INSET_PT, VIDEO_PLAY_ALPHA,
};
use crate::icons::{EXTERNAL, IMAGE_OFF, SPINNER};
use crate::message_media::received::MediaAt;
use crate::provider_players::PlayerKey;
use crate::strings;
use crate::timeline::TimelineHost;

/// The ask: a panel of dark glass at the box's foot with what loading tells the provider, and
/// Play (accent, focused), Always allow {provider}, Cancel. Esc cancels.
pub(super) fn ask<S: TimelineHost>(at: MediaAt, key: PlayerKey, provider: &str, look: &Look) -> El {
    let [r, g, b] = PLAYER_ASK_RGB.map(|channel| f32::from(channel) / 255.0);
    let button = |label: String, fill: Color, ink: Color, consent: Consent| {
        let button = row()
            .h(PLAYER_BUTTON_PT)
            .px(PLAYER_BUTTON_PAD_PT)
            .items_center()
            .radius(look.radius(RADIUS_CONTROL))
            .fill(fill)
            .role("button")
            .focusable(true)
            .label(label.clone())
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| {
                host.player_event(at, key, PlayerEvent::Consent(consent), tx);
            })
            .child(text(label).font(look.font(TYPE_SMALL_MEDIUM)).color(ink));
        let name = match consent {
            Consent::Once => ask_play_key(key),
            Consent::Always => format!("{}-always", page_key(key)),
            Consent::Cancel => format!("{}-cancel", page_key(key)),
        };
        pressed_by_keys::<S>(button, at, key, PlayerEvent::Consent(consent)).key(name)
    };
    let buttons = row()
        .gap(PLAYER_BUTTON_GAP_PT)
        .flex_wrap()
        .child(button(
            strings::embed_play().to_owned(),
            look.palette.accent,
            look.palette.accent_on,
            Consent::Once,
        ))
        .child(button(
            strings::embed_always(provider),
            Color::WHITE.alpha(PLAYER_BUTTON_SECONDARY_ALPHA),
            Color::WHITE,
            Consent::Always,
        ))
        .child(button(
            strings::embed_cancel().to_owned(),
            Color::TRANSPARENT,
            Color::WHITE.alpha(PLAYER_BUTTON_GHOST_ALPHA),
            Consent::Cancel,
        ));
    el().pin(Some(0.0), None, Some(0.0), Some(0.0)).child(
        col()
            .w_full()
            .padding(PLAYER_ASK_PAD_PT)
            .gap(PLAYER_ASK_GAP_PT)
            .fill(Color {
                r,
                g,
                b,
                a: PLAYER_ASK_ALPHA,
            })
            .backdrop_blur(PLAYER_ASK_BLUR_PT)
            .role("group")
            .label(strings::embed_ask_label(provider))
            .on_key(move |host: &mut S, event: &KeyEvent, tx| {
                if event.key == rune::Key::Escape {
                    host.player_event(at, key, PlayerEvent::Consent(Consent::Cancel), tx);
                    return true;
                }
                false
            })
            .child(
                text(strings::embed_consent(provider))
                    .font(look.font(TYPE_SMALL))
                    .color(Color::WHITE)
                    .w_full()
                    .wrap(),
            )
            .child(buttons),
    )
}

/// Loading: the 36 pt disc with its turning ring.
pub(super) fn ring() -> El {
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .a11y_hidden()
        .child(
            el().size(PLAYER_RING_PT, PLAYER_RING_PT)
                .radius(PLAYER_RING_PT / 2.0)
                .center()
                .fill(Color::BLACK.alpha(VIDEO_PLAY_ALPHA))
                .child(
                    icon(&SPINNER, PLAYER_RING_ICON_PT)
                        .color(Color::WHITE)
                        .looping(LoopSpec::rotation(PLAYER_RING_PERIOD_S)),
                ),
        )
}

/// A 28 pt round glass button beside the live page.
pub(super) fn glass_button<S: TimelineHost>(
    at: MediaAt,
    key: PlayerKey,
    name: &str,
    shape: &'static rune::Icon,
    label: &'static str,
    event: PlayerEvent,
    look: &Look,
) -> El {
    let glass = |alpha: f32| {
        let [r, g, b] = PLAYER_GLASS_RGB.map(|channel| f32::from(channel) / 255.0);
        Color { r, g, b, a: alpha }
    };
    let button = el()
        .key(format!("{}-{name}", page_key(key)))
        .size(PLAYER_GLASS_PT, PLAYER_GLASS_PT)
        .radius(PLAYER_GLASS_PT / 2.0)
        .center()
        .fill(glass(PLAYER_GLASS_ALPHA))
        .hover(glass(PLAYER_GLASS_HOVER_ALPHA))
        .backdrop_blur(PLAYER_GLASS_BLUR_PT)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |host: &mut S, tx| host.player_event(at, key, event, tx))
        .child(icon(shape, PLAYER_GLASS_ICON_PT).color(Color::WHITE));
    pressed_by_keys::<S>(button, at, key, event)
}

/// The note at a released poster's foot: "Stopped when it left the view".
pub(super) fn note(look: &Look) -> El {
    el().pin(
        Some(VIDEO_MARK_INSET_PT),
        None,
        None,
        Some(VIDEO_MARK_INSET_PT),
    )
    .a11y_hidden()
    .child(
        text(strings::embed_stopped())
            .font(look.font(TYPE_CAPTION_MEDIUM))
            .color(Color::WHITE)
            .px(PLAYER_NOTE_PAD_PT.0)
            .py(PLAYER_NOTE_PAD_PT.1)
            .radius(look.radius(MEDIA_BADGE_RADIUS))
            .fill(Color::BLACK.alpha(MEDIA_BADGE_ALPHA)),
    )
}

/// A player that couldn't load: `image-off`, the words, Retry and Open in browser, in the box.
/// Read in a polite live region.
pub(super) fn failed<S: TimelineHost>(at: MediaAt, key: PlayerKey, look: &Look) -> El {
    let button = |name: &str, label: &'static str, shape: Option<&'static rune::Icon>, event| {
        let mut button = row()
            .key(format!("{}-{name}", page_key(key)))
            .h(PLAYER_BUTTON_PT)
            .px(PLAYER_BUTTON_PAD_PT)
            .gap(PLAYER_BUTTON_GAP_PT * 0.75)
            .items_center()
            .radius(look.radius(RADIUS_CONTROL))
            .fill(look.palette.surface_hover)
            .border(1.0, look.palette.line_subtle)
            .role("button")
            .focusable(true)
            .label(label)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| host.player_event(at, key, event, tx));
        if let Some(shape) = shape {
            button = button.child(icon(shape, 14.0).color(look.palette.text_primary));
        }
        let button = button.child(
            text(label)
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(look.palette.text_primary),
        );
        pressed_by_keys::<S>(button, at, key, event)
    };
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(look.palette.surface_code)
        .center()
        .child(
            col()
                .items_center()
                .gap(PLAYER_FAIL_GAP_PT)
                .padding(16.0)
                .child(icon(&IMAGE_OFF, PLAYER_FAIL_ICON_PT).color(look.palette.text_muted))
                .child(
                    text(strings::embed_failed())
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_muted)
                        .live_region(rune::LiveRegion::Polite),
                )
                .child(
                    row()
                        .gap(PLAYER_BUTTON_GAP_PT)
                        .child(button(
                            "retry",
                            strings::embed_retry(),
                            None,
                            PlayerEvent::Retry,
                        ))
                        .child(button(
                            "browser",
                            strings::embed_open_browser(),
                            Some(&EXTERNAL),
                            PlayerEvent::OpenInBrowser,
                        )),
                ),
        )
}

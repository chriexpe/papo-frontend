//! A provider video's box, in every state of `docs/design.md` §15.3, drawn: the poster with its
//! chip and play disc (Open in browser when players are off or WPE is missing, §15.7), the ask
//! on the dimmed poster, loading with its ring after half a second, the live page with its two
//! glass buttons on hover or focus, the poster with its note once released, and the failure.
//! Every state keeps the box reserved at Embed size, so no state moves the row. Pure.
//!
//! The live page is a `NativeSurface` element: it scrolls and clips with its row, under the
//! pills, with menus and dialogs above it. It forwards the pointer and keys to the page (the
//! first Esc stays Papo's) and reports where it is, for the out-of-view rule.

use std::collections::HashMap;
use std::sync::Arc;

use rune::edit::KeyEvent;
use rune::{Color, Cursor, El, NativeSurface, SurfaceFit, Tx, el, icon, row};

use crate::design::{
    FOCUS_RING_PT, Look, PLAYER_DIM_ALPHA, PLAYER_GLASS_GAP_PT, PROVIDER_CHIP_INSET_PT,
    PROVIDER_PLAY_HOVER_ALPHA, VIDEO_PLAY_ALPHA, VIDEO_PLAY_ICON_PT, VIDEO_PLAY_PT,
};
use crate::icons::{EXTERNAL, PLAY, X};
use crate::message_media::received::MediaAt;
use crate::provider_players::{Phase, PlayerKey};
use crate::strings;
use crate::timeline::TimelineHost;
use crate::web_view::{PageInput, PointerButton};

mod overlays;

use overlays::{ask, failed, glass_button, note, ring};

/// What a provider player's box asks of the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PlayerEvent {
    /// The play disc, or Play on a stopped player.
    Play,
    /// The ask's answer.
    Consent(Consent),
    /// Close player: the poster returns.
    Close,
    /// Open in browser: the page's address, by §9's link rules.
    OpenInBrowser,
    /// A failure's Retry.
    Retry,
    /// Input for the live page.
    Input(PageInput),
    /// The live page took the keyboard, or lost it.
    Focus(bool),
    /// The pointer came over the box, or left it.
    Hover(bool),
    /// The box's menu, beside the pointer (window points).
    Menu([f32; 2]),
}

/// The ask's three answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Consent {
    /// Play, once.
    Once,
    /// Always allow the site, then play.
    Always,
    Cancel,
}

/// What the players show, for every box.
#[derive(Clone, Default)]
pub(crate) struct PlayersView {
    /// Players can load: switched on, and WPE WebKit was found (or isn't known missing yet).
    /// Without, the disc reads Open in browser and nothing is loaded.
    pub(crate) loadable: bool,
    pub(crate) shown: Arc<HashMap<PlayerKey, PlayerShown>>,
}

/// One player's look.
#[derive(Clone)]
pub(crate) struct PlayerShown {
    pub(crate) phase: Phase,
    /// Loading has taken half a second: the ring shows.
    pub(crate) ring: bool,
    /// The live page's frames.
    pub(crate) surface: Option<NativeSurface>,
    /// The pointer or keyboard is on it: its glass buttons show.
    pub(crate) chrome: bool,
    /// The cursor the page asked for.
    pub(crate) cursor: Cursor,
    /// The viewer has the player: the box shows its dimmed poster.
    pub(crate) in_viewer: bool,
}

/// What a box is drawn from.
pub(crate) struct PlayerBox<'a> {
    pub(crate) at: MediaAt,
    pub(crate) key: PlayerKey,
    /// The provider's name, the allowlist's own.
    pub(crate) provider: &'a str,
    pub(crate) title: Option<&'a str>,
    pub(crate) players: &'a PlayersView,
}

/// The element key of player `key`'s live page.
pub(crate) fn page_key(key: PlayerKey) -> String {
    format!("player-{}-{}", key.message, key.embed)
}

/// The element key of the ask's Play, which takes the focus when the ask opens.
pub(crate) fn ask_play_key(key: PlayerKey) -> String {
    format!("player-ask-{}-{}", key.message, key.embed)
}

/// The player's box: `holder`, the reserved box, holding `poster` (its picture and label), the
/// layers of the player's current state over it, and `chip` naming the provider above any dim
/// (the live page covers it).
pub(crate) fn player<S: TimelineHost>(
    holder: El,
    poster: El,
    chip: El,
    shown: &PlayerBox<'_>,
    look: &Look,
) -> El {
    let at = shown.at;
    let key = shown.key;
    let state = shown.players.shown.get(&key);
    let layout = move |host: &mut S, rect: [f32; 4]| host.player_moved(key, rect);
    let holder = holder.on_screen_layout(layout);
    match state {
        None => holder.child(poster).child(chip).child(play_disc::<S>(
            at,
            key,
            shown.players.loadable,
            look,
        )),
        Some(state) => match state.phase {
            _ if state.in_viewer => holder.child(poster).child(dim::<S>()).child(chip),
            Phase::Asking => holder
                .child(poster)
                .child(dim::<S>())
                .child(chip)
                .child(ask::<S>(at, key, shown.provider, look)),
            Phase::Loading { .. } => {
                let built = holder.child(poster).child(dim::<S>()).child(chip);
                if state.ring {
                    built.child(ring())
                } else {
                    built
                }
            }
            Phase::Playing => match &state.surface {
                Some(surface) => holder
                    .child(poster)
                    .child(live::<S>(shown, state, surface, look)),
                None => holder.child(poster).child(dim::<S>()).child(chip),
            },
            Phase::Stopped => holder
                .child(poster.label(strings::embed_stopped_label()))
                .child(chip)
                .child(play_disc::<S>(at, key, shown.players.loadable, look))
                .child(note(look)),
            Phase::Failed => holder.child(poster).child(failed::<S>(at, key, look)),
        },
    }
}

/// `button`'s Enter and Space press it, before the message around it takes them (Enter opens
/// its menu, Space plays its video).
pub(super) fn pressed_by_keys<S: TimelineHost>(
    button: El,
    at: MediaAt,
    key: PlayerKey,
    event: PlayerEvent,
) -> El {
    button.on_key(move |host: &mut S, pressed: &KeyEvent, tx| {
        let modified =
            pressed.mods.cmd || pressed.mods.ctrl || pressed.mods.alt || pressed.mods.logo;
        let presses = matches!(&pressed.key, rune::Key::Enter)
            || matches!(&pressed.key, rune::Key::Text(typed) if typed == " ");
        if presses && !modified {
            host.player_event(at, key, event, tx);
            return true;
        }
        false
    })
}

/// The poster under the ask and loading: dimmed to 42 %.
fn dim<S: TimelineHost>() -> El {
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(PLAYER_DIM_ALPHA))
        .a11y_hidden()
        // Under the ask or loading, a press on the poster opens nothing.
        .on_background_press(|_: &mut S| {})
}

/// The 48 pt play disc: it plays, or with players off or WPE missing, opens the page in the
/// browser and says so.
fn play_disc<S: TimelineHost>(at: MediaAt, key: PlayerKey, loadable: bool, look: &Look) -> El {
    let (label, event) = if loadable {
        (strings::embed_play(), PlayerEvent::Play)
    } else {
        (strings::embed_open_browser(), PlayerEvent::OpenInBrowser)
    };
    let disc = el()
        .key(format!("{}-play", page_key(key)))
        .size(VIDEO_PLAY_PT, VIDEO_PLAY_PT)
        .radius(VIDEO_PLAY_PT / 2.0)
        .center()
        .fill(Color::BLACK.alpha(VIDEO_PLAY_ALPHA))
        .hover(Color::BLACK.alpha(PROVIDER_PLAY_HOVER_ALPHA))
        .role("button")
        .focusable(true)
        .label(label)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |host: &mut S, tx| host.player_event(at, key, event, tx))
        .child(icon(&PLAY, VIDEO_PLAY_ICON_PT).color(Color::WHITE));
    let disc = pressed_by_keys::<S>(disc, at, key, event);
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .child(disc)
}

/// The live page: its frames fill the box; on hover or focus its two glass buttons show.
fn live<S: TimelineHost>(
    shown: &PlayerBox<'_>,
    state: &PlayerShown,
    surface: &NativeSurface,
    look: &Look,
) -> El {
    let at = shown.at;
    let key = shown.key;
    let label = strings::embed_player_label(shown.provider, shown.title);
    let page = page::<S>(at, key, surface, state.cursor, label, look).pin(
        Some(0.0),
        Some(0.0),
        Some(0.0),
        Some(0.0),
    );
    let mut live = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK)
        .child(page);
    if state.chrome {
        live = live.child(
            el().pin(
                None,
                Some(PROVIDER_CHIP_INSET_PT),
                Some(PROVIDER_CHIP_INSET_PT),
                None,
            )
            .child(
                row()
                    .gap(PLAYER_GLASS_GAP_PT)
                    .child(glass_button::<S>(
                        at,
                        key,
                        "open",
                        &EXTERNAL,
                        strings::embed_open_browser(),
                        PlayerEvent::OpenInBrowser,
                        look,
                    ))
                    .child(glass_button::<S>(
                        at,
                        key,
                        "close",
                        &X,
                        strings::embed_close_player(),
                        PlayerEvent::Close,
                        look,
                    )),
            ),
        );
    }
    live
}

/// Player `key`'s page as an element, wherever it is drawn (its box or the viewer): its
/// frames, its cursor, and the pointer, keys and focus forwarded to it (the first Esc and the
/// Menu key stay Papo's); a secondary click opens the card's menu. Spoken as a region named
/// `label`.
pub(crate) fn page<S: TimelineHost>(
    at: MediaAt,
    key: PlayerKey,
    surface: &NativeSurface,
    cursor: Cursor,
    label: String,
    look: &Look,
) -> El {
    let send = move |host: &mut S, input: PageInput, tx: &mut Tx| {
        host.player_event(at, key, PlayerEvent::Input(input), tx);
    };
    el().key(page_key(key))
        .native_surface(surface, SurfaceFit::Fill)
        .cursor(cursor)
        .focusable(true)
        .role("region")
        .label(label)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_hover_change(move |host: &mut S, inside, tx| {
            host.player_event(at, key, PlayerEvent::Hover(inside), tx);
            if !inside {
                send(host, PageInput::Leave, tx);
            }
        })
        .on_pointer_move_local(move |host: &mut S, pointer, tx| {
            send(
                host,
                PageInput::Move {
                    x: pointer.x,
                    y: pointer.y,
                },
                tx,
            );
        })
        .on_drag(move |host: &mut S, drag, tx| {
            let (x, y) = (drag.x, drag.y);
            let input = match drag.phase {
                rune::Phase::Down => PageInput::Down {
                    x,
                    y,
                    button: PointerButton::Primary,
                },
                rune::Phase::Move => PageInput::Move { x, y },
                rune::Phase::Up => PageInput::Up {
                    x,
                    y,
                    button: PointerButton::Primary,
                },
            };
            send(host, input, tx);
        })
        .on_secondary_press(move |host: &mut S, press, tx| {
            host.player_event(at, key, PlayerEvent::Menu([press.x, press.y]), tx);
        })
        .on_focus_change(move |host: &mut S, focused, tx| {
            host.player_event(at, key, PlayerEvent::Focus(focused), tx);
        })
        .on_key(move |host: &mut S, event: &KeyEvent, tx| host.player_key(at, key, event, tx))
}

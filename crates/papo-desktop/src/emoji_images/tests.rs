//! The store's fetch, failure and eviction rules, decoded sizes, and lines that never grow.

use super::*;
use crate::design::Look;

fn account() -> AccountId {
    let server = papo_sdk::ServerAddress::parse("https://emoji.invalid")
        .unwrap()
        .id();
    AccountId::new(server, Uuid::from_u128(1))
}

fn emoji(id: u128, created_at: &str) -> CustomEmoji {
    CustomEmoji {
        id: Uuid::from_u128(id),
        name: format!("e{id}"),
        format: "png".into(),
        created_at: created_at.into(),
    }
}

fn pixels() -> Pixels {
    let pixels = Pixels::new();
    pixels.set(1, 1, vec![0; 4]);
    pixels
}

#[test]
fn an_emoji_is_fetched_once_and_draws_when_it_lands() {
    let mut images = EmojiImages::default();
    let gato = emoji(1, "t1");
    let jobs = images.want(account(), [&gato]);
    assert_eq!(jobs.len(), 1);
    assert!(images.want(account(), [&gato]).is_empty(), "asked once");
    let key = jobs[0].key;
    assert_eq!(images.looks().look(key), EmojiLook::Loading);
    assert!(images.landed(&jobs[0], Ok((pixels(), 1.0))));
    assert!(matches!(images.looks().look(key), EmojiLook::Ready { .. }));
}

#[test]
fn a_failed_emoji_shows_its_name_and_a_recreated_one_is_fetched_again() {
    let mut images = EmojiImages::default();
    let jobs = images.want(account(), [&emoji(1, "t1")]);
    images.landed(&jobs[0], Err(EmojiFailure::Decode));
    assert_eq!(images.looks().look(jobs[0].key), EmojiLook::Failed);
    let again = images.want(account(), [&emoji(1, "t2")]);
    assert_eq!(again.len(), 1, "a new revision is a new image");
    assert!(
        !images.landed(&jobs[0], Ok((pixels(), 1.0))),
        "the old fetch is stale"
    );
    assert_eq!(images.looks().look(jobs[0].key), EmojiLook::Loading);
}

#[test]
fn the_least_recently_wanted_settled_images_go_first() {
    let mut images = EmojiImages::default();
    let all: Vec<CustomEmoji> = (0..=MAX_KEPT as u128).map(|id| emoji(id, "t")).collect();
    for one in &all[..MAX_KEPT] {
        let jobs = images.want(account(), [one]);
        images.landed(&jobs[0], Ok((pixels(), 1.0)));
    }
    let first = EmojiKey {
        account: account(),
        id: all[0].id,
    };
    images.want(account(), [&all[MAX_KEPT]]);
    assert_eq!(images.entries.len(), MAX_KEPT);
    assert_eq!(
        images.looks().look(first),
        EmojiLook::Loading,
        "evicted: asked again when wanted"
    );
}

#[test]
fn images_wanted_now_are_never_evicted() {
    let mut images = EmojiImages::default();
    let all: Vec<CustomEmoji> = (0..MAX_KEPT as u128 + 10)
        .map(|id| emoji(id, "t"))
        .collect();
    let jobs = images.want(account(), &all);
    for job in &jobs {
        images.landed(job, Ok((pixels(), 1.0)));
    }
    assert!(
        images.want(account(), &all).is_empty(),
        "nothing asked twice, nothing flickers"
    );
}

#[test]
fn a_failed_fetch_is_asked_again_after_a_reconnect_and_a_failed_decode_is_not() {
    let mut images = EmojiImages::default();
    let (fetched, decoded) = (emoji(1, "t"), emoji(2, "t"));
    let jobs = images.want(account(), [&fetched, &decoded]);
    images.landed(&jobs[0], Err(EmojiFailure::Fetch));
    images.landed(&jobs[1], Err(EmojiFailure::Decode));
    assert_eq!(images.looks().look(jobs[0].key), EmojiLook::Failed);
    assert!(images.reconnected(account()));
    let again = images.want(account(), [&fetched, &decoded]);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].key, jobs[0].key);
}

#[test]
fn an_emoji_the_catalog_lost_shows_its_name() {
    let mut images = EmojiImages::default();
    let key = EmojiKey {
        account: account(),
        id: Uuid::from_u128(5),
    };
    assert!(images.unknown(account(), [key.id]));
    assert_eq!(images.looks().look(key), EmojiLook::Failed);
    assert!(!images.unknown(account(), [key.id]), "said once");
}

/// The height of a line of body text holding `:gato:`, drawn as `look` when given.
fn line_height(look: Option<EmojiLook>, jumbo: bool, scale: f32) -> f32 {
    use rune::{Cx, El, Env, Runtime, col, text};
    struct State(Option<EmojiLook>, bool);
    fn view(state: &State, _: &mut Cx<State>) -> El {
        let ui = Look::new(crate::design::Theme::Graphite);
        let font = ui.font(if state.1 {
            crate::design::TYPE_JUMBO
        } else {
            crate::design::TYPE_BODY
        });
        let mut line = text("oi :gato: tudo").key("line").font(font);
        if let Some(look) = &state.0 {
            let place = EmojiPlace {
                font,
                jumbo: state.1,
                hidden: false,
            };
            if let Some(image) = inline_image(look, place, &ui) {
                line = line.inline_image(3..9, image);
            }
        }
        col().key("root").child(line)
    }
    let env = Env {
        system_dark: true,
        scale,
    };
    let mut rt = Runtime::with_text(
        State(look, jumbo),
        view,
        (600.0, 200.0),
        env,
        crate::fonts::text_system(),
    );
    rt.frame(0.0);
    rt.elements()
        .into_iter()
        .find(|element| element.path.ends_with("line"))
        .unwrap()
        .rect[3]
}

#[test]
fn an_emoji_never_grows_its_line_loading_or_drawn() {
    let image = pixels_of(64, 64);
    for (jumbo, scale) in [false, true]
        .into_iter()
        .flat_map(|jumbo| [1.0, 1.25, 1.5, 2.0, 3.0].map(|scale| (jumbo, scale)))
    {
        let plain = line_height(None, jumbo, scale);
        for look in [
            EmojiLook::Loading,
            EmojiLook::Ready {
                pixels: image.clone(),
                aspect: 1.0,
            },
            EmojiLook::Ready {
                pixels: image.clone(),
                aspect: 2.5,
            },
        ] {
            let with = line_height(Some(look.clone()), jumbo, scale);
            assert!(
                (with - plain).abs() < 0.01,
                "jumbo {jumbo} at {scale}×, {look:?}: {with} against {plain}"
            );
        }
    }
}

fn pixels_of(width: u32, height: u32) -> Pixels {
    let pixels = Pixels::new();
    pixels.set(width, height, vec![255; (width * height * 4) as usize]);
    pixels
}

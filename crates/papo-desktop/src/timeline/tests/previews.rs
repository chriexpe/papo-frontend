//! Link cards in the conversation (`docs/design.md` §15.1): a card's parts and its spoken
//! label, a GIF link drawn as media, Show link previews off drawing nothing, and a preview that
//! arrives late growing its row without moving the message being read.

use std::sync::Arc;

use eyeball_im::VectorDiff;
use imbl::Vector;
use papo_sdk::{Generation, Message, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{TimelineItem, TimelineUpdate};
use rune::Runtime;
use serde_json::{Value, json};

use super::{FRAME_S, HOLD_PT, Host, rect, runtime, until_idle};
use crate::design::MessageLayout;
use crate::timeline::anchor::Anchor;
use crate::timeline::rows::{anchor_key, message_key};

/// Message `id` by Ana saying `body`, with `embeds` (wire JSON).
fn with_embeds(id: u128, seconds: u32, body: &str, embeds: &Value) -> Arc<TimelineItem> {
    let id = Uuid::from_u128(id);
    let wire = json!({
        "id": id.to_string(),
        "author_id": "ana",
        "content": body,
        "created_at": format!("2026-11-20T14:{:02}:{:02}Z", seconds / 60 % 60, seconds % 60),
        "embeds": embeds.clone(),
    });
    let message: Message = serde_json::from_value(wire).unwrap();
    Arc::new(TimelineItem::Message {
        reply: None,
        pending: None,
        id,
        message: Arc::new(message),
        content: Arc::new(MessageContent::parse(body)),
    })
}

/// An article's embed: provider, title, description, a footer, and no picture.
fn article(n: u128) -> Value {
    json!({
        "id": Uuid::from_u128(5_000 + n).to_string(),
        "source_type": "link",
        "fetch_method": "opengraph",
        "site_name": "Royal Society of Camelot",
        "url": "https://camelot.example/papers/swallow",
        "title": "Unladen Airspeed Velocity",
        "description": "High-speed parchment telemetry confirms 43 wing-beats a second.",
        "footer": {"text": "6 min read"},
        "fields": [{"position": 0, "name": "Bird", "value": "European"}],
    })
}

fn tree(rt: &Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

fn set(rt: &mut Runtime<Host>, index: usize, item: Arc<TimelineItem>) {
    rt.update(move |host, _tx| {
        host.open.update(
            TimelineUpdate {
                version: 2,
                generation: Generation::for_test(1),
                diffs: vec![VectorDiff::Set { index, value: item }],
            },
            true,
        );
    });
}

#[test]
fn a_card_draws_its_parts_and_is_read_as_a_link_preview() {
    let link = "https://camelot.example/papers/swallow";
    let rt = runtime(
        std::iter::once(with_embeds(1, 0, link, &json!([article(1)]))).collect(),
        MessageLayout::default(),
    );
    let spoken = tree(&rt);
    assert!(
        spoken.contains("Link preview, Royal Society of Camelot: Unladen Airspeed Velocity"),
        "{spoken}"
    );
    for part in ["High-speed parchment", "6 min read", "Bird", "European"] {
        assert!(spoken.contains(part), "{part}: {spoken}");
    }
}

#[test]
fn a_gif_link_is_media_with_its_badge_and_no_card() {
    let gif = json!({
        "id": Uuid::from_u128(5_100).to_string(),
        "fetch_method": "gif",
        "provider": "KLIPY",
        "url": "https://klipy.com/gifs/a-gif",
        "title": "A GIF",
        "thumbnail": {"mime_type": "image/gif", "width": 320, "height": 326},
    });
    let rt = runtime(
        std::iter::once(with_embeds(1, 0, "look", &json!([gif]))).collect(),
        MessageLayout::default(),
    );
    let spoken = tree(&rt);
    assert!(spoken.contains("GIF"), "{spoken}");
    assert!(!spoken.contains("Link preview"), "no card: {spoken}");
}

#[test]
fn show_link_previews_off_draws_and_reserves_nothing() {
    let items: Vector<_> =
        std::iter::once(with_embeds(1, 0, "the link", &json!([article(1)]))).collect();
    let mut rt = runtime(items, MessageLayout::default());
    let with_card = rt
        .presented(&message_key(Uuid::from_u128(1)))
        .map(|(rect, _)| rect[3])
        .unwrap();
    rt.update(|host, _tx| host.media.previews = false);
    until_idle(&mut rt);
    let without = rt
        .presented(&message_key(Uuid::from_u128(1)))
        .map(|(rect, _)| rect[3])
        .unwrap();
    assert!(!tree(&rt).contains("Link preview"));
    assert!(without + 40.0 < with_card, "{without} vs {with_card}");
}

/// `count` messages of plain words.
fn history(count: u128) -> Vector<Arc<TimelineItem>> {
    (0..count)
        .map(|n| {
            let seconds = u32::try_from(n * 60 % 3_600).unwrap();
            with_embeds(n + 1_000, seconds, "a line of words", &json!([]))
        })
        .collect()
}

#[test]
fn a_late_preview_grows_its_row_without_moving_the_message_being_read() {
    let mut rt = runtime(history(120), MessageLayout::default());
    for _ in 0..4 {
        rt.wheel(400.0, 300.0, 400.0);
        rt.frame(FRAME_S);
    }
    until_idle(&mut rt);
    let viewport = format!("timeline-{}-{}", Uuid::nil(), Uuid::from_u128(7));
    let Anchor::Message { id, offset_pt } = rt.state.open.current_anchor() else {
        panic!("reading, not following the newest");
    };
    let read = rt
        .state
        .open
        .items
        .iter()
        .position(|item| matches!(item.as_ref(), TimelineItem::Message { id: m, .. } if *m == id))
        .unwrap();
    // A message above the one being read gets its card.
    let above = read - 3;
    let TimelineItem::Message { id: grown, .. } = rt.state.open.items[above].as_ref() else {
        panic!("a message");
    };
    let grown_id = grown.as_u128();
    let before = rt
        .presented(&message_key(*grown))
        .map(|(rect, _)| rect[3])
        .unwrap();
    set(
        &mut rt,
        above,
        with_embeds(grown_id, 0, "a line of words", &json!([article(1)])),
    );
    until_idle(&mut rt);
    let after = rt
        .presented(&message_key(Uuid::from_u128(grown_id)))
        .map(|(rect, _)| rect[3])
        .unwrap();
    assert!(
        after > before + 40.0,
        "the row grew by its card: {before} → {after}"
    );
    let now_pt = rect(&rt, &anchor_key(id))[1] - rect(&rt, &viewport)[1];
    let moved = now_pt - offset_pt;
    assert!(
        moved.abs() < HOLD_PT,
        "the message being read moved {moved}"
    );
}

#[test]
fn at_the_bottom_a_late_preview_keeps_the_newest_in_view() {
    let mut rt = runtime(history(40), MessageLayout::default());
    assert_eq!(rt.state.open.current_anchor(), Anchor::Newest);
    let last = rt.state.open.items.len() - 1;
    set(
        &mut rt,
        last,
        with_embeds(1_039, 0, "a line of words", &json!([article(1)])),
    );
    until_idle(&mut rt);
    let viewport = rect(
        &rt,
        &format!("timeline-{}-{}", Uuid::nil(), Uuid::from_u128(7)),
    );
    let newest = rect(&rt, &message_key(Uuid::from_u128(1_039)));
    let gap = (viewport[1] + viewport[3]) - (newest[1] + newest[3]);
    assert!(gap.abs() < 24.0, "the newest stays at the bottom: {gap}");
}

/// Frames until message `n`'s row stops changing height after a preview arrives on it.
fn frames_to_grow(reduced_motion: bool) -> usize {
    let mut rt = runtime(history(10), MessageLayout::default());
    rt.update(move |host, _tx| host.media.reduced_motion = reduced_motion);
    until_idle(&mut rt);
    let id = message_key(Uuid::from_u128(1_009));
    let height = |rt: &Runtime<Host>| rt.presented(&id).map(|(rect, _)| rect[3]).unwrap();
    let start = height(&rt);
    set(
        &mut rt,
        9,
        with_embeds(1_009, 0, "a line of words", &json!([article(1)])),
    );
    let mut last = height(&rt);
    for frame in 1..=60 {
        rt.frame(FRAME_S);
        let now = height(&rt);
        if (now - last).abs() < 0.01 && now > start + 40.0 {
            return frame;
        }
        last = now;
    }
    panic!("the row never settled");
}

#[test]
fn a_late_preview_grows_in_160_ms_and_at_once_under_reduced_motion() {
    let animated = frames_to_grow(false);
    assert!((6..=16).contains(&animated), "{animated} frames at 60 fps");
    let reduced = frames_to_grow(true);
    assert!(reduced <= 2, "{reduced} frames under reduced motion");
}

#[test]
fn the_rule_keeps_the_densitys_lead_gap_without_avatars_or_times() {
    let layout = MessageLayout {
        avatars: false,
        times: false,
        ..MessageLayout::default()
    };
    let rt = runtime(
        std::iter::once(with_embeds(1, 0, "the link", &json!([article(1)]))).collect(),
        layout,
    );
    let card = rt
        .elements()
        .into_iter()
        .find(|element| element.path.ends_with("-card"))
        .expect("the card");
    let parts = rt
        .elements()
        .into_iter()
        .find(|element| element.path == format!("{}/#1", card.path))
        .expect("its parts");
    let gap = parts.rect[0] - (card.rect[0] + crate::design::CARD_RULE_PT);
    assert!((gap - 10.0).abs() < 0.5, "rule to parts: {gap}");
}

#[test]
fn inline_extra_compact_cards_start_on_the_content_column_whatever_the_names_width() {
    let layout = MessageLayout {
        density: crate::design::Density::ExtraCompact,
        stacked: false,
        ..MessageLayout::default()
    };
    let short = with_embeds(1, 0, "the link", &json!([article(1)]));
    let mut long = with_embeds(2, 120, "the link", &json!([article(2)]));
    if let TimelineItem::Message { message, .. } = Arc::make_mut(&mut long) {
        Arc::make_mut(message).author_id = Some("someone-with-a-much-longer-name".into());
    }
    let rt = runtime([short, long].into_iter().collect(), layout);
    let cards: Vec<f32> = rt
        .elements()
        .into_iter()
        .filter(|element| element.path.ends_with("-card"))
        .map(|element| element.rect[0])
        .collect();
    assert_eq!(cards.len(), 2);
    assert!((cards[0] - cards[1]).abs() < 0.5, "cards at {cards:?}");
    let name = super::part(&rt, 2, "name");
    assert!(
        (cards[1] - name[0]).abs() < 0.5,
        "the card starts where the name does"
    );
}

//! Where a message's previews sit among its media, and what they are named.

use papo_sdk::{Attachment, Embed, EmbedMedia, Message, Uuid};
use papo_sdk_ui::LinkPreview;

use super::{MediaItem, media_item, name_of, previews, provider_of};

fn embed(n: u128, title: Option<&str>) -> Embed {
    Embed {
        id: Some(Uuid::from_u128(n).to_string()),
        fetch_method: Some("opengraph".into()),
        url: Some("https://news.example/story".into()),
        title: title.map(str::to_owned),
        description: Some("What happened.".into()),
        thumbnail: Some(EmbedMedia {
            mime_type: Some("image/webp".into()),
            ..EmbedMedia::default()
        }),
        ..Embed::default()
    }
}

fn message(attachments: usize, embeds: Vec<Embed>) -> Message {
    Message {
        attachments: Some(
            (0..attachments)
                .map(|n| Attachment {
                    id: Some(Uuid::from_u128(100 + n as u128).to_string()),
                    ..Attachment::default()
                })
                .collect(),
        ),
        embeds: Some(embeds),
        ..Message::default()
    }
}

#[test]
fn previews_follow_the_attachments_among_a_messages_media() {
    let message = message(2, vec![embed(1, Some("One")), embed(2, Some("Two"))]);
    assert!(matches!(
        media_item(&message, 1, true),
        Some(MediaItem::Attachment)
    ));
    let Some(MediaItem::Preview(preview)) = media_item(&message, 3, true) else {
        panic!("the second preview");
    };
    assert_eq!(preview.embed(), Uuid::from_u128(2));
    assert!(media_item(&message, 4, true).is_none());
}

#[test]
fn show_link_previews_off_draws_and_reserves_nothing() {
    let message = message(0, vec![embed(1, Some("One"))]);
    assert_eq!(previews(&message, false), Vec::new());
    assert!(media_item(&message, 0, false).is_none());
    assert_eq!(previews(&message, true).len(), 1);
}

#[test]
fn a_preview_is_named_by_its_title_else_its_host() {
    let titled = previews(&message(0, vec![embed(1, Some("A story"))]), true);
    assert_eq!(name_of(&titled[0]), "A story");
    let untitled = previews(&message(0, vec![embed(1, None)]), true);
    assert_eq!(name_of(&untitled[0]), "news.example");
    let LinkPreview::Card(_) = &untitled[0] else {
        panic!("a card");
    };
    assert_eq!(
        provider_of(&untitled[0]),
        "news.example",
        "no provider: its host"
    );
}

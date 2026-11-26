//! The previews a message draws and each card's parts, from embeds shaped as the server sends
//! them.

use super::*;
use crate::media_box::{EmbedSizeSetting, MediaSizeSetting};
use crate::provider_player::PlayerProvider;
use papo_sdk::{EmbedAuthor, EmbedField, EmbedFooter};

fn id(n: u128) -> String {
    Uuid::from_u128(n).to_string()
}

fn media(mime: Option<&str>, width: Option<i64>, height: Option<i64>) -> EmbedMedia {
    EmbedMedia {
        url: Some("https://site.example/media".into()),
        mime_type: mime.map(str::to_owned),
        width,
        height,
        ..Default::default()
    }
}

/// An article as an `opengraph` crawl gives it.
fn article(n: u128) -> Embed {
    Embed {
        id: Some(id(n)),
        source_type: Some("link".into()),
        fetch_method: Some("opengraph".into()),
        site_name: Some("Windows Forum".into()),
        url: Some("https://windowsforum.com/news/a-story.1/".into()),
        title: Some("A story".into()),
        description: Some("What happened.".into()),
        thumbnail: Some(media(Some("image/webp"), Some(1536), Some(1024))),
        fetched_at: Some("2026-11-25T10:00:00Z".into()),
        ..Default::default()
    }
}

/// A provider video as `YouTube`'s `oembed` gives it: the player page has no media type.
fn provider_video(n: u128) -> Embed {
    Embed {
        id: Some(id(n)),
        fetch_method: Some("oembed".into()),
        provider: Some("YouTube".into()),
        url: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()),
        title: Some("A video".into()),
        author: Some(EmbedAuthor {
            name: Some("A channel".into()),
            url: Some("https://www.youtube.com/@channel".into()),
            ..Default::default()
        }),
        thumbnail: Some(media(Some("image/webp"), Some(384), Some(288))),
        video: Some(EmbedMedia {
            url: Some("https://www.youtube.com/embed/dQw4w9WgXcQ".into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn message(embeds: Vec<Embed>) -> Message {
    Message {
        embeds: Some(embeds),
        ..Default::default()
    }
}

fn card(preview: LinkPreview) -> LinkCard {
    match preview {
        LinkPreview::Card(card) => *card,
        LinkPreview::Media(_) => panic!("a card was expected"),
    }
}

#[test]
fn an_article_shows_its_parts_and_its_relayed_picture() {
    let card = card(preview(&article(1)).unwrap());
    assert_eq!(card.provider.as_deref(), Some("Windows Forum"));
    assert_eq!(card.title.as_deref(), Some("A story"));
    assert_eq!(card.description.as_deref(), Some("What happened."));
    let page = card.page.unwrap();
    assert_eq!(page.host, "windowsforum.com");
    let picture = card.picture.unwrap();
    assert_eq!(
        picture.source,
        MediaSource::EmbedImage {
            embed: Uuid::from_u128(1),
            revision: "2026-11-25T10:00:00Z".into(),
        }
    );
    assert_eq!(picture.size, MediaSize::exact(1536, 1024));
    assert_eq!(picture.kind, AttachmentKind::Image);
    assert_eq!(picture.mime.as_deref(), Some("image/webp"));
    assert!(card.video.is_none());
}

#[test]
fn a_text_only_card_draws_without_a_picture() {
    let mut repository = article(2);
    repository.site_name = Some("GitHub".into());
    repository.thumbnail = None;
    let card = card(preview(&repository).unwrap());
    assert!(card.picture.is_none());
    assert_eq!(card_box(&card, bounds(EmbedSizeSetting::Medium)), None);
}

#[test]
fn the_provider_name_comes_before_the_site_name_and_the_author_follows_it() {
    let card = card(preview(&provider_video(3)).unwrap());
    assert_eq!(card.provider.as_deref(), Some("YouTube"));
    assert_eq!(card.author.as_deref(), Some("A channel"));
}

#[test]
fn a_video_without_a_type_is_the_providers_player_with_a_poster() {
    let card = card(preview(&provider_video(4)).unwrap());
    let Some(CardVideo::Provider(Some(player))) = &card.video else {
        panic!("an allowlisted player was expected: {:?}", card.video);
    };
    assert_eq!(player.provider, PlayerProvider::YouTube);
    assert_eq!(
        player.address(),
        "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1"
    );
    assert!(card.picture.is_some(), "the poster");
}

#[test]
fn a_player_address_off_the_allowlist_is_never_loaded() {
    // A custom or webhook embed may carry any address as its video, on any page.
    for address in [
        "https://evil.example/embed/dQw4w9WgXcQ",
        "https://player.vimeo.com/video/76979871",
    ] {
        let mut embed = provider_video(6);
        embed.url = Some("https://news.example/story".into());
        embed.video = Some(EmbedMedia {
            url: Some(address.into()),
            ..Default::default()
        });
        let card = card(preview(&embed).unwrap());
        assert_eq!(card.video, Some(CardVideo::Provider(None)), "{address}");
        let shown = format!("{card:?}");
        assert!(!shown.contains("evil.example") && !shown.contains("player.vimeo"));
    }
    // On a provider's own page, the player is the page's, whatever the wire's address says.
    let mut embed = provider_video(7);
    embed.video = Some(EmbedMedia {
        url: Some("https://evil.example/embed/dQw4w9WgXcQ".into()),
        ..Default::default()
    });
    let Some(CardVideo::Provider(Some(player))) = card(preview(&embed).unwrap()).video else {
        panic!("the page's player");
    };
    assert_eq!(
        player.address(),
        "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1"
    );
}

#[test]
fn a_providers_page_without_a_player_from_the_server_names_one() {
    let mut tiktok = article(9);
    tiktok.fetch_method = Some("oembed".into());
    tiktok.provider = Some("TikTok".into());
    tiktok.url = Some("https://www.tiktok.com/@scout2015/video/6718335390845095173".into());
    let tiktok_card = card(preview(&tiktok).unwrap());
    let Some(CardVideo::Provider(Some(player))) = &tiktok_card.video else {
        panic!("a TikTok player: {:?}", tiktok_card.video);
    };
    assert_eq!(player.provider, PlayerProvider::TikTok);
    assert_eq!(tiktok_card.track, None);
    let mut spotify = article(10);
    spotify.url = Some("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT".into());
    let track = card(preview(&spotify).unwrap());
    assert_eq!(track.video, None, "a track is not a video");
    assert!(track.picture.is_some(), "its cover");
    assert_eq!(
        track.track.map(|player| player.provider),
        Some(PlayerProvider::Spotify)
    );
}

#[test]
fn the_players_name_is_the_allowlists_not_the_embeds() {
    let mut embed = provider_video(8);
    embed.provider = Some("Your Bank".into());
    let card = card(preview(&embed).unwrap());
    let Some(CardVideo::Provider(Some(player))) = &card.video else {
        panic!("an allowlisted player was expected");
    };
    assert_eq!(player.provider.name(), "YouTube");
    assert_eq!(
        card.provider.as_deref(),
        Some("Your Bank"),
        "the card's own line"
    );
}

#[test]
fn a_relayed_video_is_a_clip_sized_by_itself_else_by_its_poster() {
    let mut post = article(5);
    post.provider = Some("X".into());
    post.thumbnail = Some(media(Some("image/webp"), Some(576), Some(1024)));
    post.video = Some(media(Some("video/mp4"), None, None));
    let LinkCard { video, .. } = card(preview(&post).unwrap());
    assert_eq!(
        video,
        Some(CardVideo::Clip {
            source: MediaSource::EmbedVideo(Uuid::from_u128(5)),
            size: MediaSize::exact(576, 1024),
            mime: "video/mp4".into(),
        })
    );
    post.video = Some(media(Some("Video/WebM"), Some(1280), Some(720)));
    let LinkCard { video, .. } = card(preview(&post).unwrap());
    assert!(matches!(
        video,
        Some(CardVideo::Clip { size, ref mime, .. })
            if size == MediaSize::exact(1280, 720) && mime == "video/webm"
    ));
}

#[test]
fn a_gif_link_is_media_not_a_card() {
    let gif = Embed {
        id: Some(id(6)),
        fetch_method: Some("gif".into()),
        provider: Some("KLIPY".into()),
        url: Some("https://klipy.com/gifs/a-gif".into()),
        title: Some("A GIF".into()),
        thumbnail: Some(media(Some("image/gif"), Some(320), Some(326))),
        ..Default::default()
    };
    let LinkPreview::Media(media) = preview(&gif).unwrap() else {
        panic!("media was expected");
    };
    assert_eq!(media.picture.kind, AttachmentKind::Gif);
    assert_eq!(media.picture.size, MediaSize::exact(320, 326));
    assert_eq!(media.title.as_deref(), Some("A GIF"));
    let mut no_picture = gif;
    no_picture.thumbnail = None;
    assert_eq!(preview(&no_picture), None);
}

#[test]
fn an_embed_with_nothing_to_show_is_no_preview() {
    let empty = Embed {
        id: Some(id(7)),
        fetch_method: Some("opengraph".into()),
        url: Some("https://intranet.example/doc/55".into()),
        site_name: Some("Intranet".into()),
        footer: Some(EmbedFooter {
            text: Some("Footer only".into()),
            ..Default::default()
        }),
        title: Some("   ".into()),
        ..Default::default()
    };
    assert_eq!(preview(&empty), None);
    let mut no_id = article(8);
    no_id.id = None;
    assert_eq!(preview(&no_id), None);
}

#[test]
fn a_message_draws_at_most_three_previews_in_link_order_skipping_empty_ones() {
    let mut empty = article(10);
    empty.title = None;
    empty.description = None;
    empty.thumbnail = None;
    let previews = link_previews(&message(vec![
        article(11),
        empty,
        provider_video(12),
        article(13),
        article(14),
    ]));
    let ids: Vec<Uuid> = previews.iter().map(LinkPreview::embed).collect();
    assert_eq!(
        ids,
        [11, 12, 13].map(Uuid::from_u128).to_vec(),
        "the fourth link stays text"
    );
    assert_eq!(link_previews(&Message::default()), Vec::new());
}

#[test]
fn fields_keep_their_order_up_to_six_and_drop_empty_ones() {
    let mut status = article(20);
    status.color = Some("#4ADE80".into());
    status.fields = Some(
        (0..8)
            .rev()
            .map(|position| EmbedField {
                position: Some(position),
                name: Some(if position == 1 {
                    String::new()
                } else {
                    format!("Label {position}")
                }),
                value: Some(format!("Value {position}")),
                inline: Some(true),
            })
            .collect(),
    );
    let card = card(preview(&status).unwrap());
    let labels: Vec<&str> = card.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Label 0", "Label 2", "Label 3", "Label 4", "Label 5", "Label 6"
        ]
    );
    assert_eq!(card.colour, Some([0x4a, 0xde, 0x80]));
}

#[test]
fn only_a_six_digit_hex_colour_is_taken() {
    for value in ["4ADE80", "#4ADE8", "#4ADE80FF", "#GGGGGG", "red"] {
        assert_eq!(colour(value), None, "{value}");
    }
    assert_eq!(colour(" #00ff7f "), Some([0, 255, 127]));
}

#[test]
fn only_an_http_page_is_offered_and_never_the_embeds_other_addresses() {
    let mut unsafe_page = article(30);
    unsafe_page.url = Some("javascript:alert(1)".into());
    let card = card(preview(&unsafe_page).unwrap());
    assert!(card.page.is_none(), "the card still shows, without a link");
    let shown = format!("{:?}", preview(&provider_video(31)).unwrap());
    for hidden in ["/embed/", "/@channel", "site.example/media"] {
        assert!(!shown.contains(hidden), "{hidden} is never handed out");
    }
}

fn bounds(setting: EmbedSizeSetting) -> MediaBounds {
    MediaBounds::of_max_size(setting.max_size_pt(), 900.0, 1000.0)
}

#[test]
fn a_provider_poster_is_reserved_as_the_players_16_by_9() {
    let card = card(preview(&provider_video(40)).unwrap());
    let medium = card_box(&card, bounds(EmbedSizeSetting::Medium)).unwrap();
    assert_eq!((medium.width_pt, medium.height_pt), (448.0, 252.0));
    let small = card_box(&card, bounds(EmbedSizeSetting::Small)).unwrap();
    assert_eq!((small.width_pt, small.height_pt), (320.0, 180.0));
    assert_eq!(small.fit, MediaFit::Fill);
}

#[test]
fn a_picture_keeps_its_shape_never_enlarged_and_is_1_91_to_1_without_a_size() {
    let mut picture = article(41);
    let medium = bounds(EmbedSizeSetting::Medium);
    let large = card_box(&card(preview(&picture).unwrap()), medium).unwrap();
    assert_eq!((large.width_pt, large.height_pt), (448.0, 299.0));
    picture.thumbnail = Some(media(Some("image/webp"), Some(200), Some(100)));
    let small = card_box(&card(preview(&picture).unwrap()), medium).unwrap();
    assert_eq!((small.width_pt, small.height_pt), (200.0, 100.0));
    picture.thumbnail = Some(media(Some("image/webp"), None, None));
    let unsized_box = card_box(&card(preview(&picture).unwrap()), medium).unwrap();
    assert_eq!(
        (unsized_box.width_pt, unsized_box.height_pt),
        (448.0, 235.0)
    );
    assert_eq!(unsized_box.fit, MediaFit::Fill);
}

#[test]
fn the_box_is_the_smaller_of_the_column_and_the_setting() {
    let card = card(preview(&provider_video(42)).unwrap());
    let narrow = MediaBounds::of_max_size(EmbedSizeSetting::Large.max_size_pt(), 300.0, 1000.0);
    let fitted = card_box(&card, narrow).unwrap();
    assert_eq!((fitted.width_pt, fitted.height_pt), (300.0, 169.0));
    let short = MediaBounds::of_max_size(EmbedSizeSetting::Large.max_size_pt(), 900.0, 120.0);
    let fitted = card_box(&card, short).unwrap();
    assert_eq!((fitted.width_pt, fitted.height_pt), (213.0, 120.0));
}

#[test]
fn embed_size_is_apart_from_image_size_and_custom_has_its_own_heights() {
    assert_eq!(EmbedSizeSetting::default().max_size_pt(), (448, 336));
    assert_eq!(EmbedSizeSetting::Large.max_size_pt(), (600, 450));
    assert_ne!(
        EmbedSizeSetting::Medium.max_size_pt(),
        MediaSizeSetting::Medium.max_size_pt()
    );
    assert_eq!(EmbedSizeSetting::custom(100, 100).max_size_pt(), (240, 160));
    assert_eq!(
        EmbedSizeSetting::custom(1000, 1000).max_size_pt(),
        (720, 640)
    );
    assert_eq!(EmbedSizeSetting::custom(333, 171).max_size_pt(), (336, 168));
    let stored = EmbedSizeSetting::Custom {
        max_width_pt: 5,
        max_height_pt: 5000,
    };
    assert_eq!(stored.max_size_pt(), (240, 640));
}

#[test]
fn a_thumbnail_the_server_holds_no_picture_for_draws_no_picture() {
    let mut declared = article(50);
    declared.thumbnail = Some(EmbedMedia {
        url: Some("https://site.example/picture.png".into()),
        ..Default::default()
    });
    let card = card(preview(&declared).unwrap());
    assert!(
        card.picture.is_none(),
        "no stored type: the server holds no bytes"
    );
}

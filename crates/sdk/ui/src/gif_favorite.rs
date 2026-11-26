//! The GIF a conversation message carries that a star can keep (`docs/design.md` §16.4): a GIF
//! that arrived as a GIPHY link, whoever sent it, which the server made a `gif` embed. A
//! favorite is GIPHY's ID, so it is the same GIF on every server; its addresses are GIPHY's
//! own for that ID, not the link as one sender wrote it. Starring, its looks and its keys are
//! the client's; keeping the favorite is `ExpressionService::favorite_gif`'s.

use papo_sdk::{Embed, Gif, Message};

/// The embed kind the server gives a GIF link.
const FETCH_METHOD_GIF: &str = "gif";
/// GIPHY's grid rendition is this many pixels wide (`fixed_width`).
const PREVIEW_WIDTH_PX: u32 = 200;
/// GIPHY's IDs are letters and digits, this long at most.
const MAX_ID_CHARS: usize = 64;
/// The longest title a kept GIF holds, in bytes (`Gif`'s own bound): a longer one is cut.
const MAX_TITLE_BYTES: usize = 4096;

/// The GIPHY GIF `message` carries, ready to keep as a favorite; none when it carries none
/// (another provider's GIF, a link the server didn't make a `gif` embed, or no GIF at all).
#[must_use]
pub fn favorite_gif(message: &Message) -> Option<Gif> {
    message
        .embeds
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find_map(favorite_gif_of)
}

/// The GIF of `embed`, when it is a `gif` embed of a GIPHY link: for a client that draws each
/// of a message's previews with its own star.
#[must_use]
pub fn favorite_gif_of(embed: &Embed) -> Option<Gif> {
    if embed.fetch_method.as_deref() != Some(FETCH_METHOD_GIF) {
        return None;
    }
    let id = giphy_id(embed.url.as_deref()?)?;
    let (width, height) = embed
        .thumbnail
        .as_ref()
        .and_then(|picture| Some((picture.width?, picture.height?)))
        .and_then(|(width, height)| Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?)))
        .filter(|(width, height)| *width > 0 && *height > 0)
        .unwrap_or((0, 0));
    let preview_height = if width > 0 {
        u32::try_from(u64::from(height) * u64::from(PREVIEW_WIDTH_PX) / u64::from(width))
            .unwrap_or(0)
    } else {
        0
    };
    let gif = Gif {
        url: format!("https://media.giphy.com/media/{id}/giphy.gif"),
        page_url: format!("https://giphy.com/gifs/{id}"),
        title: cut(embed.title.as_deref().unwrap_or_default(), MAX_TITLE_BYTES),
        width,
        height,
        preview_url: format!("https://media.giphy.com/media/{id}/200w.gif"),
        preview_width: if preview_height > 0 {
            PREVIEW_WIDTH_PX
        } else {
            0
        },
        preview_height,
        id,
    };
    Some(gif)
}

/// `text` cut to at most `max_bytes`, at a character's edge.
fn cut(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// GIPHY's ID in `address`: a page (`giphy.com/gifs/a-title-ID`, `/clips/…`, `/embed/ID`) or a
/// media address (`media2.giphy.com/media/ID/giphy.gif`, `…/media/v1.…/ID/giphy.gif`).
fn giphy_id(address: &str) -> Option<String> {
    let url = url::Url::parse(address).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "https" || !(host == "giphy.com" || host.ends_with(".giphy.com")) {
        return None;
    }
    let parts: Vec<&str> = url
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect();
    let candidate = match parts.as_slice() {
        ["gifs" | "clips" | "embed", tail, ..] => tail.rsplit('-').next()?,
        ["media", rest @ ..] => {
            // The last part is the file; versioned paths put a `v1.` token before the ID.
            let (_, folders) = rest.split_last()?;
            folders.iter().rev().find(|part| !part.starts_with("v1."))?
        }
        _ => return None,
    };
    let valid = !candidate.is_empty()
        && candidate.chars().count() <= MAX_ID_CHARS
        && candidate.chars().all(|ch| ch.is_ascii_alphanumeric());
    valid.then(|| candidate.to_owned())
}

#[cfg(test)]
mod tests {
    use papo_sdk::{Embed, EmbedMedia, Message};

    use super::{favorite_gif, favorite_gif_of, giphy_id};

    fn gif_embed(url: &str) -> Embed {
        Embed {
            id: Some("00000000-0000-4000-8000-000000000006".into()),
            fetch_method: Some("gif".into()),
            url: Some(url.into()),
            title: Some("Football GIF".into()),
            thumbnail: Some(EmbedMedia {
                mime_type: Some("image/gif".into()),
                width: Some(480),
                height: Some(270),
                ..EmbedMedia::default()
            }),
            ..Embed::default()
        }
    }

    #[test]
    fn giphys_id_comes_from_its_pages_and_media_addresses() {
        for (address, id) in [
            (
                "https://giphy.com/gifs/ReocLd5xHpvanCOQ7Y",
                "ReocLd5xHpvanCOQ7Y",
            ),
            (
                "https://giphy.com/gifs/football-thanksgiving-ReocLd5xHpvanCOQ7Y",
                "ReocLd5xHpvanCOQ7Y",
            ),
            (
                "https://media1.giphy.com/media/ReocLd5xHpvanCOQ7Y/giphy.gif",
                "ReocLd5xHpvanCOQ7Y",
            ),
            (
                "https://media1.giphy.com/media/v1.Y2lkPTc5/ReocLd5xHpvanCOQ7Y/giphy.gif?cid=1",
                "ReocLd5xHpvanCOQ7Y",
            ),
        ] {
            assert_eq!(giphy_id(address).as_deref(), Some(id), "{address}");
        }
        for address in [
            "https://klipy.com/gifs/a-gif",
            "http://giphy.com/gifs/abc",
            "https://notgiphy.com/gifs/abc",
            "https://giphy.com/search/cats",
        ] {
            assert_eq!(giphy_id(address), None, "{address}");
        }
    }

    #[test]
    fn a_giphy_gif_in_a_message_is_kept_by_giphys_own_addresses_and_sizes() {
        let message = Message {
            content: Some(
                "olha https://media1.giphy.com/media/v1.x/ReocLd5xHpvanCOQ7Y/giphy.gif".into(),
            ),
            embeds: Some(vec![gif_embed("https://giphy.com/gifs/ReocLd5xHpvanCOQ7Y")]),
            ..Message::default()
        };
        let gif = favorite_gif(&message).expect("a GIPHY GIF");
        assert_eq!(gif.id, "ReocLd5xHpvanCOQ7Y");
        assert_eq!(
            gif.url,
            "https://media.giphy.com/media/ReocLd5xHpvanCOQ7Y/giphy.gif"
        );
        assert_eq!(gif.page_url, "https://giphy.com/gifs/ReocLd5xHpvanCOQ7Y");
        assert_eq!(
            gif.preview_url,
            "https://media.giphy.com/media/ReocLd5xHpvanCOQ7Y/200w.gif"
        );
        assert_eq!((gif.width, gif.height), (480, 270));
        assert_eq!((gif.preview_width, gif.preview_height), (200, 112));
        assert_eq!(gif.title, "Football GIF");
        assert!(gif.message().is_ok(), "valid to keep and to send");
    }

    #[test]
    fn another_providers_gif_or_a_plain_link_takes_no_star() {
        let klipy = Message {
            embeds: Some(vec![gif_embed("https://klipy.com/gifs/a-gif")]),
            ..Message::default()
        };
        assert_eq!(favorite_gif(&klipy), None);
        let mut card = gif_embed("https://giphy.com/gifs/abc123");
        card.fetch_method = Some("opengraph".into());
        let linked = Message {
            embeds: Some(vec![card]),
            ..Message::default()
        };
        assert_eq!(favorite_gif(&linked), None);
        assert_eq!(favorite_gif(&Message::default()), None);
    }

    #[test]
    fn each_embed_is_its_own_gif_and_a_long_title_is_cut_to_keep() {
        let first = gif_embed("https://giphy.com/gifs/aaa111");
        let mut second = gif_embed("https://giphy.com/gifs/bbb222");
        second.title = Some("é".repeat(3000));
        assert_eq!(favorite_gif_of(&first).unwrap().id, "aaa111");
        let long = favorite_gif_of(&second).unwrap();
        assert_eq!(long.id, "bbb222");
        assert!(long.title.len() <= 4096);
        assert!(long.message().is_ok(), "valid to keep");
    }
}

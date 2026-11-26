//! Each allowlisted provider's addresses: the pages a message links (from which a player is
//! derived), its player pages (the server's, and what a player navigates to), and the player
//! address a client loads. Every id is checked character by character before it is kept, so a
//! rebuilt address can't carry anything the pattern didn't allow.

use url::Url;

use super::PlayerProvider;

pub(super) const YOUTUBE_PLAYER_HOST: &str = "www.youtube.com";
pub(super) const TIKTOK_PLAYER_HOST: &str = "www.tiktok.com";
pub(super) const SPOTIFY_PLAYER_HOST: &str = "open.spotify.com";
pub(super) const SOUNDCLOUD_PLAYER_HOST: &str = "w.soundcloud.com";

/// `TikTok`'s player is upright (width over height).
pub(super) const TIKTOK_ASPECT: f32 = 9.0 / 16.0;

/// What a Spotify link may open.
const SPOTIFY_KINDS: [&str; 5] = ["track", "album", "playlist", "episode", "show"];
/// `SoundCloud` paths whose first part is the site's own page, not an artist.
const SOUNDCLOUD_RESERVED: [&str; 12] = [
    "charts",
    "discover",
    "messages",
    "notifications",
    "pages",
    "people",
    "search",
    "settings",
    "stations",
    "tags",
    "upload",
    "you",
];

/// The provider and id of a page a message links, when it names an allowlisted provider's
/// video or track.
pub(super) fn page(url: &Url) -> Option<(PlayerProvider, String)> {
    let parts = parts(url);
    let parts = parts.as_slice();
    match url.host_str()? {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
            let id = match parts {
                ["watch"] => url
                    .query_pairs()
                    .find(|(name, _)| name == "v")
                    .map(|(_, value)| value.into_owned())?,
                ["shorts" | "embed", id] => (*id).to_owned(),
                _ => return None,
            };
            youtube_id(&id).then_some((PlayerProvider::YouTube, id))
        }
        "youtu.be" => match parts {
            [id] if youtube_id(id) => Some((PlayerProvider::YouTube, (*id).to_owned())),
            _ => None,
        },
        "tiktok.com" | "www.tiktok.com" | "m.tiktok.com" => match parts {
            [user, "video", id] if user.starts_with('@') && digits(id, 15..=20) => {
                Some((PlayerProvider::TikTok, (*id).to_owned()))
            }
            _ => None,
        },
        "open.spotify.com" => spotify(parts),
        "soundcloud.com" | "www.soundcloud.com" | "m.soundcloud.com" => soundcloud(parts),
        _ => None,
    }
}

/// The provider and id of a player page: where a loaded player stays.
pub(super) fn player(url: &Url) -> Option<(PlayerProvider, String)> {
    let parts = parts(url);
    match (url.host_str()?, parts.as_slice()) {
        (YOUTUBE_PLAYER_HOST, ["embed", id]) if youtube_id(id) => {
            Some((PlayerProvider::YouTube, (*id).to_owned()))
        }
        (TIKTOK_PLAYER_HOST, ["embed", "v2", id] | ["embed", id]) if digits(id, 15..=20) => {
            Some((PlayerProvider::TikTok, (*id).to_owned()))
        }
        (SPOTIFY_PLAYER_HOST, ["embed", rest @ ..]) => spotify(rest),
        (SOUNDCLOUD_PLAYER_HOST, ["player"]) => {
            let page = url
                .query_pairs()
                .find(|(name, _)| name == "url")
                .and_then(|(_, page)| Url::parse(&page).ok())?;
            let parts = self::parts(&page);
            matches!(page.host_str(), Some("soundcloud.com"))
                .then(|| soundcloud(parts.as_slice()))
                .flatten()
        }
        _ => None,
    }
}

/// `YouTube`'s player page as the server derives it: `https://www.youtube.com/embed/{id}`.
pub(super) fn youtube_player(url: &Url) -> Option<String> {
    match player(url)? {
        (PlayerProvider::YouTube, id) => Some(id),
        _ => None,
    }
}

/// The address a client loads for `provider`'s player of `id`.
pub(super) fn player_address(provider: PlayerProvider, id: &str) -> String {
    match provider {
        PlayerProvider::YouTube => format!("https://{YOUTUBE_PLAYER_HOST}/embed/{id}?autoplay=1"),
        PlayerProvider::TikTok => format!("https://{TIKTOK_PLAYER_HOST}/embed/v2/{id}?autoplay=1"),
        PlayerProvider::Spotify => format!("https://{SPOTIFY_PLAYER_HOST}/embed/{id}"),
        PlayerProvider::SoundCloud => {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("url", &format!("https://soundcloud.com/{id}"))
                .append_pair("auto_play", "true")
                .finish();
            format!("https://{SOUNDCLOUD_PLAYER_HOST}/player/?{query}")
        }
    }
}

/// A Spotify path (after any `intl-xx` part): `{kind}/{id}`, the id 22 letters and digits.
fn spotify(parts: &[&str]) -> Option<(PlayerProvider, String)> {
    let parts = match parts {
        [locale, rest @ ..] if locale.starts_with("intl-") => rest,
        _ => parts,
    };
    match parts {
        [kind, id]
            if SPOTIFY_KINDS.contains(kind)
                && id.len() == 22
                && id.bytes().all(|byte| byte.is_ascii_alphanumeric()) =>
        {
            Some((PlayerProvider::Spotify, format!("{kind}/{id}")))
        }
        _ => None,
    }
}

/// A `SoundCloud` path: `{artist}/{track}` or `{artist}/sets/{set}`.
fn soundcloud(parts: &[&str]) -> Option<(PlayerProvider, String)> {
    let id = match parts {
        [artist, "sets", set] if slug(artist) && slug(set) => format!("{artist}/sets/{set}"),
        [artist, track] if slug(artist) && slug(track) && *track != "sets" => {
            format!("{artist}/{track}")
        }
        _ => return None,
    };
    let artist = parts.first()?;
    (!SOUNDCLOUD_RESERVED.contains(artist)).then_some((PlayerProvider::SoundCloud, id))
}

/// The path's parts, without empty ones (a trailing `/`).
fn parts(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|segments| segments.filter(|part| !part.is_empty()).collect())
        .unwrap_or_default()
}

/// `YouTube`'s 11-character video id.
fn youtube_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Only digits, as many as `count` allows.
fn digits(id: &str, count: std::ops::RangeInclusive<usize>) -> bool {
    count.contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
}

/// A `SoundCloud` name in a path: letters, digits, `-` and `_`, at most 100.
fn slug(part: &str) -> bool {
    (1..=100).contains(&part.len())
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

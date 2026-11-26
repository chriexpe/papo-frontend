//! Which providers' players a client may load, and the one address of each it loads
//! (`docs/design.md` §15.3, §15.4): the player descriptor. A client never loads an address the
//! wire hands it as is: a custom or webhook embed can carry any. A player is either the server's
//! player page (`YouTube`'s `video.url`) or derived from the card's page address, the link the
//! message already holds, and only when that address parses as an allowlisted provider's,
//! strictly. Either way the address loaded is rebuilt from the parsed id. Deriving makes no
//! request: nothing reaches a provider before the person presses play.
//!
//! The allowlist holds the providers whose player plays in the desktop's web view (WPE
//! 2.54 on Linux): Vimeo's refuses it ("Unable to play video") and has no entry.
//!
//! Owns the allowlist, the parsing (each provider's patterns are `pages`'), the address loaded
//! and whether a later address is still that player. Loading, consent, the allowed sites and
//! the browser are the client's.

use url::Url;

mod pages;

/// The shape a video player is reserved at (width over height) when its provider's isn't
/// known: the server sends no player ratio.
pub const PLAYER_ASPECT: f32 = 16.0 / 9.0;

/// A provider whose player a client may load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlayerProvider {
    /// `YouTube`'s embed player.
    YouTube,
    /// `TikTok`'s embed player, upright.
    TikTok,
    /// Spotify's embed player (tracks, albums, playlists, episodes, shows).
    Spotify,
    /// `SoundCloud`'s widget (tracks and sets).
    SoundCloud,
}

/// What a provider's player plays: a video fills the card's picture box, a track is the strip
/// of §15.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlayerKind {
    /// A video, in the card's picture box.
    Video,
    /// A track, in the strip under the card's cover row.
    Audio,
}

impl PlayerProvider {
    /// Every allowlisted provider, in a stable order.
    pub const ALL: [PlayerProvider; 4] = [
        PlayerProvider::YouTube,
        PlayerProvider::TikTok,
        PlayerProvider::Spotify,
        PlayerProvider::SoundCloud,
    ];

    /// The provider's name, as consent and the allowed sites show it. It is the allowlist's
    /// own, never the embed's `provider` text, so a custom embed cannot name another site.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PlayerProvider::YouTube => "YouTube",
            PlayerProvider::TikTok => "TikTok",
            PlayerProvider::Spotify => "Spotify",
            PlayerProvider::SoundCloud => "SoundCloud",
        }
    }

    /// The host its player loads from, as the allowed sites show it.
    #[must_use]
    pub fn host(self) -> &'static str {
        match self {
            PlayerProvider::YouTube => pages::YOUTUBE_PLAYER_HOST,
            PlayerProvider::TikTok => pages::TIKTOK_PLAYER_HOST,
            PlayerProvider::Spotify => pages::SPOTIFY_PLAYER_HOST,
            PlayerProvider::SoundCloud => pages::SOUNDCLOUD_PLAYER_HOST,
        }
    }

    /// A stable key for storing a choice about this provider (an allowed site).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            PlayerProvider::YouTube => "youtube",
            PlayerProvider::TikTok => "tiktok",
            PlayerProvider::Spotify => "spotify",
            PlayerProvider::SoundCloud => "soundcloud",
        }
    }

    /// What its player plays.
    #[must_use]
    pub fn kind(self) -> PlayerKind {
        match self {
            PlayerProvider::YouTube | PlayerProvider::TikTok => PlayerKind::Video,
            PlayerProvider::Spotify | PlayerProvider::SoundCloud => PlayerKind::Audio,
        }
    }

    /// The provider stored under `key`, if it is still allowlisted.
    #[must_use]
    pub fn from_key(key: &str) -> Option<PlayerProvider> {
        PlayerProvider::ALL
            .into_iter()
            .find(|provider| provider.key() == key)
    }
}

/// A provider's player, as a client loads it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderPlayer {
    /// Whose player it is.
    pub provider: PlayerProvider,
    /// The provider's own identifier for what it plays (a video id, Spotify's `track/{id}`,
    /// `SoundCloud`'s `{user}/{track}`), already checked character by character.
    id: String,
    /// A video player's shape, width over height (`TikTok`'s is upright).
    pub aspect: f32,
}

impl ProviderPlayer {
    /// The player for the server's player page (an embed's `video.url` without a media type):
    /// only `YouTube`'s, exactly as the server derives it.
    #[must_use]
    pub fn from_embed_address(address: &str) -> Option<ProviderPlayer> {
        let url = checked(address)?;
        pages::youtube_player(&url).map(|id| ProviderPlayer::new(PlayerProvider::YouTube, id))
    }

    /// The player a card's page address (the link in the message) names, for an allowlisted
    /// provider's page: a `YouTube` watch, short or `youtu.be` link, a `TikTok` video, a Spotify track, album, playlist, episode or show, a `SoundCloud` track or set.
    #[must_use]
    pub fn from_page_address(address: &str) -> Option<ProviderPlayer> {
        let url = checked(address)?;
        let (provider, id) = pages::page(&url)?;
        Some(ProviderPlayer::new(provider, id))
    }

    fn new(provider: PlayerProvider, id: String) -> ProviderPlayer {
        let aspect = match provider {
            PlayerProvider::TikTok => pages::TIKTOK_ASPECT,
            _ => PLAYER_ASPECT,
        };
        ProviderPlayer {
            provider,
            id,
            aspect,
        }
    }

    /// What its player plays.
    #[must_use]
    pub fn kind(&self) -> PlayerKind {
        self.provider.kind()
    }

    /// The address a client loads once you pressed play: the player page, rebuilt from the
    /// parsed id, asked to start at once where the provider allows (the press was the
    /// request to play; Spotify's player waits for a press of its own).
    #[must_use]
    pub fn address(&self) -> String {
        pages::player_address(self.provider, &self.id)
    }

    /// Whether the page at `address` is still this player, so a navigation there stays in it.
    /// Anything else (the provider's own site, a sign-in, an ad) belongs in the browser.
    #[must_use]
    pub fn keeps(&self, address: &str) -> bool {
        checked(address)
            .and_then(|url| pages::player(&url))
            .is_some_and(|(provider, id)| provider == self.provider && id == self.id)
    }
}

/// `address` as a URL a player may come from: `https`, with no port, user or password.
fn checked(address: &str) -> Option<Url> {
    let url = Url::parse(address.trim()).ok()?;
    let plain = url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    plain.then_some(url)
}

#[cfg(test)]
mod tests;

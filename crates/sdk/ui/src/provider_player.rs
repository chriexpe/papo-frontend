//! Which providers' players a client may load, and the one address of each it loads
//! (`docs/design.md` §15.3): the player descriptor. A provider video's embed carries the
//! player page the server derived, but a custom or webhook embed can carry any address, so a
//! client never loads the wire's `video.url` itself. Only an address that parses as an
//! allowlisted provider's player becomes a [`ProviderPlayer`], rebuilt from its parsed parts.
//!
//! Owns the allowlist, the parsing, the address loaded and whether a later address is still
//! that player. Loading, consent, the allowed sites and the browser are the client's.
//!
//! The allowlist holds only what the server sends: `YouTube`, the one provider whose player page
//! it derives (`https://www.youtube.com/embed/{id}`). Other oEmbed providers' cards come
//! without a player, so they have no entry.

use url::Url;

/// The shape a provider's player is reserved at (width over height): the server sends no
/// player ratio.
pub const PLAYER_ASPECT: f32 = 16.0 / 9.0;

/// A provider whose player a client may load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlayerProvider {
    /// `YouTube`'s embed player.
    YouTube,
}

impl PlayerProvider {
    /// Every allowlisted provider, in a stable order.
    pub const ALL: [PlayerProvider; 1] = [PlayerProvider::YouTube];

    /// The provider's name, as consent and the allowed sites show it. It is the allowlist's
    /// own, never the embed's `provider` text, so a custom embed cannot name another site.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PlayerProvider::YouTube => "YouTube",
        }
    }

    /// The host its player loads from, as the allowed sites show it.
    #[must_use]
    pub fn host(self) -> &'static str {
        match self {
            PlayerProvider::YouTube => YOUTUBE_HOST,
        }
    }

    /// A stable key for storing a choice about this provider (an allowed site).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            PlayerProvider::YouTube => "youtube",
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

/// A provider video's player, as a client loads it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderPlayer {
    /// Whose player it is.
    pub provider: PlayerProvider,
    /// The player's own identifier for the video (`YouTube`'s 11-character id).
    video: String,
    /// The player's shape, width over height.
    pub aspect: f32,
}

impl ProviderPlayer {
    /// The player for the embed's `video.url`, when it is an allowlisted provider's player
    /// page exactly: `https`, the provider's host with no port or user, and its player path.
    #[must_use]
    pub fn from_embed_address(address: &str) -> Option<ProviderPlayer> {
        let url = Url::parse(address.trim()).ok()?;
        if url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.host_str() != Some(YOUTUBE_HOST)
        {
            return None;
        }
        let video = youtube_video(&url)?;
        Some(ProviderPlayer {
            provider: PlayerProvider::YouTube,
            video: video.to_owned(),
            aspect: PLAYER_ASPECT,
        })
    }

    /// The address a client loads once you pressed play: the player page, rebuilt from its
    /// parts, asked to start at once (the press was the request to play).
    #[must_use]
    pub fn address(&self) -> String {
        match self.provider {
            PlayerProvider::YouTube => {
                format!("https://{YOUTUBE_HOST}/embed/{}?autoplay=1", self.video)
            }
        }
    }

    /// Whether the page at `address` is still this player, so a navigation there stays in it.
    /// Anything else (the provider's own site, a sign-in, an ad) belongs in the browser.
    #[must_use]
    pub fn keeps(&self, address: &str) -> bool {
        ProviderPlayer::from_embed_address(address)
            .is_some_and(|player| player.provider == self.provider && player.video == self.video)
    }
}

const YOUTUBE_HOST: &str = "www.youtube.com";

/// The video id of a `YouTube` player page, `/embed/{id}`, whatever its query says.
fn youtube_video(url: &Url) -> Option<&str> {
    let video = url.path().strip_prefix("/embed/")?;
    let valid = video.len() == 11
        && video
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    valid.then_some(video)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_providers_exact_player_page_is_a_player() {
        let player =
            ProviderPlayer::from_embed_address("https://www.youtube.com/embed/dQw4w9WgXcQ")
                .unwrap();
        assert_eq!(player.provider, PlayerProvider::YouTube);
        assert!((player.aspect - 16.0 / 9.0).abs() < f32::EPSILON);
        for refused in [
            "http://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com.evil.example/embed/dQw4w9WgXcQ",
            "https://evil.example/embed/dQw4w9WgXcQ",
            "https://www.youtube.com:8443/embed/dQw4w9WgXcQ",
            "https://user@www.youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/embed/abc",
            "https://www.youtube.com/embed/dQw4w9WgXcQ/more",
            "https://www.youtube.com/embed/dQw4w9WgX%2F",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(
                ProviderPlayer::from_embed_address(refused),
                None,
                "{refused}"
            );
        }
    }

    #[test]
    fn the_loaded_address_is_rebuilt_never_the_wires() {
        let player = ProviderPlayer::from_embed_address(
            " https://www.youtube.com/embed/dQw4w9WgXcQ?origin=https://evil.example#x ",
        )
        .unwrap();
        assert_eq!(
            player.address(),
            "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1"
        );
    }

    #[test]
    fn the_player_keeps_its_own_page_and_nothing_else() {
        let player =
            ProviderPlayer::from_embed_address("https://www.youtube.com/embed/dQw4w9WgXcQ")
                .unwrap();
        assert!(player.keeps(&player.address()));
        assert!(player.keeps("https://www.youtube.com/embed/dQw4w9WgXcQ?start=30"));
        for elsewhere in [
            "https://www.youtube.com/embed/aaaaaaaaaaa",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://accounts.google.com/ServiceLogin",
            "about:blank",
        ] {
            assert!(!player.keeps(elsewhere), "{elsewhere}");
        }
    }

    #[test]
    fn a_stored_key_names_its_provider_while_it_is_allowlisted() {
        for provider in PlayerProvider::ALL {
            assert_eq!(PlayerProvider::from_key(provider.key()), Some(provider));
        }
        assert_eq!(PlayerProvider::from_key("vimeo"), None);
        assert_eq!(PlayerProvider::YouTube.name(), "YouTube");
        assert_eq!(PlayerProvider::YouTube.host(), "www.youtube.com");
    }
}

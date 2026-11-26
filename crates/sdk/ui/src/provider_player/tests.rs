//! Which addresses become players, and the one address each loads.

use super::*;

fn page(address: &str) -> Option<(PlayerProvider, String)> {
    ProviderPlayer::from_page_address(address).map(|player| (player.provider, player.address()))
}

#[test]
fn only_the_servers_exact_youtube_player_page_is_taken_from_the_wire() {
    let player =
        ProviderPlayer::from_embed_address("https://www.youtube.com/embed/dQw4w9WgXcQ").unwrap();
    assert_eq!(player.provider, PlayerProvider::YouTube);
    assert!((player.aspect - 16.0 / 9.0).abs() < f32::EPSILON);
    for refused in [
        "http://www.youtube.com/embed/dQw4w9WgXcQ",
        "https://youtube.com/embed/dQw4w9WgXcQ",
        "https://www.youtube.com.evil.example/embed/dQw4w9WgXcQ",
        "https://www.youtube.com:8443/embed/dQw4w9WgXcQ",
        "https://user@www.youtube.com/embed/dQw4w9WgXcQ",
        "https://www.youtube.com/embed/abc",
        "https://www.youtube.com/embed/dQw4w9WgX%2F",
        // Other player pages are never taken from the wire.
        "https://www.tiktok.com/embed/v2/6718335390845095173",
        "https://player.vimeo.com/video/76979871",
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
fn each_providers_pages_name_their_player() {
    let cases = [
        (
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10",
            PlayerProvider::YouTube,
            "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1",
        ),
        (
            "https://youtu.be/dQw4w9WgXcQ",
            PlayerProvider::YouTube,
            "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1",
        ),
        (
            "https://m.youtube.com/shorts/dQw4w9WgXcQ",
            PlayerProvider::YouTube,
            "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1",
        ),
        (
            "https://www.tiktok.com/@scout2015/video/6718335390845095173",
            PlayerProvider::TikTok,
            "https://www.tiktok.com/embed/v2/6718335390845095173?autoplay=1",
        ),
        (
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            PlayerProvider::Spotify,
            "https://open.spotify.com/embed/track/4cOdK2wGLETKBW3PvgPWqT",
        ),
        (
            "https://open.spotify.com/intl-pt/album/1A2GTWGtFfWp7KSQTwWOyo",
            PlayerProvider::Spotify,
            "https://open.spotify.com/embed/album/1A2GTWGtFfWp7KSQTwWOyo",
        ),
        (
            "https://soundcloud.com/forss/flickermood",
            PlayerProvider::SoundCloud,
            "https://w.soundcloud.com/player/?url=https%3A%2F%2Fsoundcloud.com%2Fforss%2Fflickermood&auto_play=true",
        ),
        (
            "https://soundcloud.com/forss/sets/soulhack",
            PlayerProvider::SoundCloud,
            "https://w.soundcloud.com/player/?url=https%3A%2F%2Fsoundcloud.com%2Fforss%2Fsets%2Fsoulhack&auto_play=true",
        ),
    ];
    for (address, provider, player) in cases {
        assert_eq!(
            page(address),
            Some((provider, player.to_owned())),
            "{address}"
        );
    }
}

#[test]
fn anything_off_the_patterns_names_no_player() {
    for address in [
        // Vimeo's player refuses WPE WebKit: no entry.
        "https://vimeo.com/76979871",
        "https://www.tiktok.com/@scout2015",
        "https://www.tiktok.com/@scout2015/video/123",
        "https://vm.tiktok.com/ZMabcdef/",
        "https://open.spotify.com/artist/0gxyHStUsqpMadRV0Di1Qt",
        "https://open.spotify.com/track/short",
        "https://soundcloud.com/forss",
        "https://soundcloud.com/discover/sets/new",
        "https://soundcloud.com/forss/flick%20ermood",
        "https://on.soundcloud.com/abc",
        "https://evil.example/forss/flickermood",
        "https://www.youtube.com/watch?v=short",
    ] {
        assert_eq!(page(address), None, "{address}");
    }
}

#[test]
fn a_player_keeps_its_own_page_and_nothing_else() {
    for (address, elsewhere) in [
        (
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        ),
        (
            "https://www.tiktok.com/@a/video/6718335390845095173",
            "https://www.tiktok.com/@a",
        ),
        (
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
        ),
        (
            "https://soundcloud.com/forss/flickermood",
            "https://soundcloud.com/forss/flickermood",
        ),
    ] {
        let player = ProviderPlayer::from_page_address(address).unwrap();
        assert!(player.keeps(&player.address()), "{address}");
        assert!(!player.keeps(elsewhere), "{address} -> {elsewhere}");
        assert!(!player.keeps("about:blank"));
    }
    let youtube = ProviderPlayer::from_page_address("https://youtu.be/dQw4w9WgXcQ").unwrap();
    assert!(youtube.keeps("https://www.youtube.com/embed/dQw4w9WgXcQ?start=30"));
    assert!(!youtube.keeps("https://www.youtube.com/embed/aaaaaaaaaaa"));
}

#[test]
fn kinds_shapes_and_stored_keys() {
    let tiktok =
        ProviderPlayer::from_page_address("https://www.tiktok.com/@a/video/6718335390845095173")
            .unwrap();
    assert!((tiktok.aspect - 9.0 / 16.0).abs() < f32::EPSILON, "upright");
    assert_eq!(tiktok.kind(), PlayerKind::Video);
    assert_eq!(PlayerProvider::Spotify.kind(), PlayerKind::Audio);
    assert_eq!(PlayerProvider::SoundCloud.kind(), PlayerKind::Audio);
    for provider in PlayerProvider::ALL {
        assert_eq!(PlayerProvider::from_key(provider.key()), Some(provider));
    }
    assert_eq!(PlayerProvider::from_key("dailymotion"), None);
    assert_eq!(PlayerProvider::SoundCloud.host(), "w.soundcloud.com");
}

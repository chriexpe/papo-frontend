//! The contrast floors of `docs/design.md` §4, for every theme.

use rune::Color;

use super::colour::{self, contrast};
use super::lightness_rule;
use super::*;

const THEMES: [Theme; 4] = [Theme::Oled, Theme::Graphite, Theme::Light, Theme::SoftLight];

#[test]
fn text_meets_its_contrast_floor_on_every_surface_it_sits_on() {
    for theme in THEMES {
        let palette = Palette::new(theme);
        for (surface, name) in [
            (palette.surface_content, "content"),
            (palette.surface_pill, "pill"),
            (palette.surface_sidebar, "sidebar"),
            (palette.surface_rail, "rail"),
            (palette.surface_hover, "hover"),
            (palette.surface_selected, "selected"),
            (palette.surface_code, "code"),
            (palette.highlight_mention, "mention highlight"),
        ] {
            let primary = contrast(palette.text_primary, surface);
            let secondary = contrast(palette.text_secondary, surface);
            let muted = contrast(palette.text_muted, surface);
            assert!(
                primary >= 13.0,
                "{theme:?} text.primary on {name}: {primary:.2}:1"
            );
            assert!(
                secondary >= 7.0,
                "{theme:?} text.secondary on {name}: {secondary:.2}:1"
            );
            assert!(muted >= 4.5, "{theme:?} text.muted on {name}: {muted:.2}:1");
        }
    }
}

#[test]
fn links_and_feedback_lines_are_readable_on_the_screens_they_sit_on() {
    for theme in THEMES {
        let palette = Palette::new(theme);
        for (colour, name) in [
            (palette.link, "link"),
            (palette.danger, "danger"),
            (palette.warning, "warning"),
            (palette.success, "success"),
        ] {
            let ratio = contrast(colour, palette.surface_content);
            assert!(ratio >= 4.5, "{theme:?} {name} on content: {ratio:.2}:1");
        }
    }
}

#[test]
fn text_on_the_accent_is_whichever_of_black_and_white_reads_better() {
    for theme in THEMES {
        let palette = Palette::new(theme);
        let other = if palette.accent_on == Color::BLACK {
            Color::WHITE
        } else {
            Color::BLACK
        };
        assert!(
            contrast(palette.accent_on, palette.accent) >= contrast(other, palette.accent),
            "{theme:?}"
        );
    }
}

#[test]
fn neutral_surfaces_are_grey() {
    for theme in THEMES {
        let pill = Palette::new(theme).surface_pill;
        assert!(
            (pill.r - pill.g).abs() < 1e-4 && (pill.g - pill.b).abs() < 1e-4,
            "{theme:?}: {pill:?}"
        );
    }
}

#[test]
fn oled_content_is_true_black() {
    let content = Palette::new(Theme::Oled).surface_content;
    assert_eq!((content.r, content.g, content.b), (0.0, 0.0, 0.0));
}

#[test]
fn a_mention_count_keeps_its_contrast() {
    // White on the mock's red, as chosen: under the 4.5:1 text floor
    // (3.42:1 dark, 4.36:1 light), so it is held to 3:1 instead.
    for theme in THEMES {
        let palette = Palette::new(theme);
        let ratio = contrast(palette.mention_on, palette.mention);
        assert!(
            ratio >= 3.0,
            "{theme:?} count on the mention badge: {ratio:.2}:1"
        );
    }
}

#[test]
fn code_tokens_and_mentions_are_readable_where_they_sit() {
    for theme in THEMES {
        let palette = Palette::new(theme);
        let code = palette.code;
        for (colour, name) in [
            (code.keyword, "keyword"),
            (code.string, "string"),
            (code.number, "number"),
            (code.type_name, "type"),
            (code.function, "function"),
        ] {
            let ratio = contrast(colour, palette.surface_code);
            assert!(ratio >= 4.5, "{theme:?} {name} on code: {ratio:.2}:1");
        }
        let mention = contrast(palette.link, palette.accent_soft);
        assert!(
            mention >= 4.5,
            "{theme:?} mention on its chip: {mention:.2}:1"
        );
        let link = contrast(palette.link, palette.highlight_mention);
        assert!(
            link >= 4.5,
            "{theme:?} link on the mention highlight: {link:.2}:1"
        );
        let link = contrast(palette.link, palette.surface_selected);
        assert!(
            link >= 4.5,
            "{theme:?} link on the selected highlight: {link:.2}:1"
        );
    }
}

/// The surfaces a mark or a ring can sit on.
fn mark_surfaces(palette: &Palette) -> [(Color, &'static str); 6] {
    [
        (palette.surface_content, "content"),
        (palette.surface_pill, "pill"),
        (palette.surface_sidebar, "sidebar"),
        (palette.surface_rail, "rail"),
        (palette.surface_hover, "hover"),
        (palette.surface_selected, "selected"),
    ]
}

#[test]
fn presence_search_and_focus_colours_hold_three_to_one_where_they_sit() {
    for theme in THEMES {
        let palette = Palette::new(theme);
        for (colour, name) in [
            (palette.presence_online, "presence.online"),
            (palette.presence_away, "presence.away"),
            (palette.presence_busy, "presence.busy"),
            (palette.presence_offline, "presence.offline"),
            (palette.search_match, "search.match"),
            (palette.focus_ring, "focus.ring"),
        ] {
            for (surface, on) in mark_surfaces(&palette) {
                let ratio = contrast(colour, surface);
                assert!(ratio >= 3.0, "{theme:?} {name} on {on}: {ratio:.2}:1");
            }
        }
    }
}

#[test]
fn presence_colours_keep_their_authored_lightness_and_hue() {
    // Rune clamps a colour sRGB cannot show, which could move its
    // lightness; only the chroma may give way.
    for theme in THEMES {
        let palette = Palette::new(theme);
        let dark = theme.is_dark();
        for (colour, token) in [
            (palette.presence_online, colour::PRESENCE_ONLINE),
            (palette.presence_away, colour::PRESENCE_AWAY),
        ] {
            let [lightness, a, b, _] = colour.to_oklab();
            let authored = if dark { token.dark } else { token.light };
            let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
            assert!(
                (lightness - authored).abs() < 1e-3,
                "{theme:?}: L {lightness}"
            );
            assert!(
                (hue - token.hue_degrees).abs() < 0.5,
                "{theme:?}: hue {hue}"
            );
            assert!(a.hypot(b) <= token.chroma + 1e-3, "{theme:?}");
        }
    }
}

#[test]
fn a_role_coloured_name_reads_on_the_sidebar_and_a_hovered_row() {
    for theme in THEMES {
        let palette = Palette::new(theme);
        for hue in (0..360).step_by(5) {
            #[expect(clippy::cast_precision_loss, reason = "whole degrees below 360")]
            let hue = hue as f32;
            for chroma in [0.0, 0.05, 0.12, 0.25] {
                for lightness in [0.1, 0.5, 0.9] {
                    let role = rune::oklch(lightness, chroma, hue);
                    let name = lightness_rule::role_name(role, &palette);
                    for (surface, on) in [
                        (palette.surface_sidebar, "sidebar"),
                        (palette.surface_hover, "hover"),
                    ] {
                        let ratio = contrast(name, surface);
                        assert!(
                            ratio >= 4.5,
                            "{theme:?} role L {lightness} C {chroma} h {hue} on {on}: {ratio:.2}:1"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_role_colour_that_already_reads_keeps_its_lightness() {
    let palette = Palette::new(Theme::Graphite);
    let role = rune::oklch(0.85, 0.08, 300.0);
    let name = lightness_rule::role_name(role, &palette);
    assert!((name.to_oklab()[0] - role.to_oklab()[0]).abs() < 1e-3);
}

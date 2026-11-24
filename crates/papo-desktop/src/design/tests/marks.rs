//! The colours drawn on surfaces, swept as the tokens are: code tokens and
//! mentions, presence, search and focus marks, and role-coloured names,
//! each readable where it sits.

use rune::Color;

use super::super::colour::contrast;
use super::super::*;
use super::{THEMES, sweep};

#[test]
fn code_tokens_and_mentions_are_readable_where_they_sit() {
    for (theme, palette) in sweep() {
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
    for (theme, palette) in sweep() {
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
                    let name = floors::role_name(role, &palette);
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
    let name = floors::role_name(role, &palette);
    assert!((name.to_oklab()[0] - role.to_oklab()[0]).abs() < 1e-3);
}

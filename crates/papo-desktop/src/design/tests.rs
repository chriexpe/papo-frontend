//! The contrast floors of `docs/design.md` §4, for every theme.

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

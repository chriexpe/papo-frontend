//! The contrast floors of `docs/design.md` §4, for every theme at every
//! setting: each accent, every 5° of background hue, and the tint from none
//! to full (strength × vividness, which is all the tint depends on), with a
//! second colour so the chrome's gradient is measured too.

use rune::Color;

use super::colour::{MENTION_HIGHLIGHT_HUE_DEGREES, contrast};
use super::floors;
use super::*;

mod marks;

const THEMES: [Theme; 4] = Theme::ALL;

/// One palette of the sweep, named in a failure.
#[derive(Debug)]
#[expect(dead_code, reason = "read through Debug, in failure messages")]
struct Case {
    theme: Theme,
    accent: Accent,
    hue_degrees: f32,
    tint: f32,
}

/// Every palette the settings can make, as far as contrast goes.
fn sweep() -> impl Iterator<Item = (Case, Palette)> {
    THEMES.into_iter().flat_map(|theme| {
        Accent::ALL.into_iter().flat_map(move |accent| {
            (0..360).step_by(5).flat_map(move |hue| {
                [0.0, 0.5, 1.0].into_iter().map(move |tint| {
                    #[expect(clippy::cast_precision_loss, reason = "whole degrees below 360")]
                    let hue_degrees = hue as f32;
                    let colours = [
                        BackgroundColour {
                            hue_degrees,
                            vividness: tint,
                        },
                        BackgroundColour {
                            hue_degrees: hue_degrees + 180.0,
                            vividness: 1.0,
                        },
                    ];
                    let background = Background::new(&colours, 1.0);
                    let case = Case {
                        theme,
                        accent,
                        hue_degrees,
                        tint,
                    };
                    // The mention highlight is the user's hue too: the
                    // sweep's hue tries every one.
                    (case, Palette::with(theme, accent, &background, hue_degrees))
                })
            })
        })
    })
}

#[test]
fn text_meets_its_contrast_floor_on_every_surface_it_sits_on() {
    for (theme, palette) in sweep() {
        for (surface, name) in [
            (palette.surface_content, "content"),
            (palette.surface_pill, "pill"),
            (palette.surface_sidebar, "sidebar"),
            (palette.surface_rail, "rail"),
            (palette.surface_hover, "hover"),
            (palette.surface_selected, "selected"),
            (palette.surface_code, "code"),
            (palette.highlight_mention, "mention highlight"),
            (palette.surface_tile, "settings tile"),
            (palette.surface_panel, "settings panel"),
        ]
        .into_iter()
        .chain(
            palette
                .rail_fill
                .colours()
                .iter()
                .map(|c| (*c, "rail gradient")),
        )
        .chain(
            palette
                .sidebar_fill
                .colours()
                .iter()
                .map(|c| (*c, "sidebar gradient")),
        ) {
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
    for (theme, palette) in sweep() {
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
    for (theme, palette) in sweep() {
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
fn with_no_background_colour_the_neutrals_are_grey() {
    for theme in THEMES {
        let palette = Palette::with(
            theme,
            Accent::Blue,
            &Background::new(&[], 0.4),
            MENTION_HIGHLIGHT_HUE_DEGREES,
        );
        let pill = palette.surface_pill;
        assert!(
            (pill.r - pill.g).abs() < 1e-4 && (pill.g - pill.b).abs() < 1e-4,
            "{theme:?}: {pill:?}"
        );
    }
}

#[test]
fn oled_stays_true_black_under_any_background_colour() {
    for (case, palette) in sweep().filter(|(case, _)| case.theme == Theme::Oled) {
        for surface in [
            palette.surface_content,
            palette.surface_sidebar,
            palette.surface_rail,
        ] {
            assert_eq!(
                (surface.r, surface.g, surface.b),
                (0.0, 0.0, 0.0),
                "{case:?}"
            );
        }
    }
}

#[test]
fn the_selected_row_shows_against_its_pane_on_every_theme() {
    // Graphite's selected row is 1.15:1 against its sidebar, a light
    // theme's with Teal about 1.10:1; OLED's used to be 1.01:1, invisible
    // (#208).
    for (case, palette) in sweep() {
        let ratio = contrast(palette.surface_selected, palette.surface_sidebar);
        assert!(ratio >= 1.05, "{case:?} selected on sidebar: {ratio:.3}:1");
    }
}

#[test]
fn a_second_colour_makes_a_gradient_across_the_chrome_only() {
    let two = [
        BackgroundColour {
            hue_degrees: 290.0,
            vividness: 0.55,
        },
        BackgroundColour {
            hue_degrees: 20.0,
            vividness: 0.55,
        },
    ];
    let palette = Palette::with(
        Theme::Graphite,
        Accent::Blue,
        &Background::new(&two, 0.5),
        MENTION_HIGHLIGHT_HUE_DEGREES,
    );
    assert_eq!(
        palette.rail_fill.gradient().map(|stops| stops.len()),
        Some(2)
    );
    assert_eq!(
        palette.sidebar_fill.gradient().map(|stops| stops.len()),
        Some(2)
    );
    let one = Palette::with(
        Theme::Graphite,
        Accent::Blue,
        &Background::new(&two[..1], 0.5),
        MENTION_HIGHLIGHT_HUE_DEGREES,
    );
    assert_eq!(one.rail_fill.gradient(), None);
    assert_eq!(
        palette.surface_content, one.surface_content,
        "the message list keeps the first colour"
    );
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
    for (theme, palette) in sweep() {
        let ratio = contrast(palette.mention_on, palette.mention);
        assert!(
            ratio >= 3.0,
            "{theme:?} count on the mention badge: {ratio:.2}:1"
        );
    }
}

#[test]
fn where_the_design_values_already_read_they_stand() {
    // Blue at the default background colour is what §4 measured: nothing
    // is fitted, so the link, the selected row and the surfaces are the
    // authored ones.
    for theme in THEMES {
        let palette = Palette::new(theme);
        let dark = theme.is_dark();
        let [link_lightness, ..] = palette.link.to_oklab();
        let authored = if dark { 0.76 } else { 0.50 };
        assert!(
            (link_lightness - authored).abs() < 1e-3,
            "{theme:?} link L {link_lightness}"
        );
        let [_, a, b, _] = palette.surface_content.to_oklab();
        let background = Background::default();
        let (chroma, _) = background.tint(
            background.first(),
            palette.surface_content.to_oklab()[0],
            false,
        );
        assert!(
            (a.hypot(b) - chroma).abs() < 2e-3,
            "{theme:?} content keeps its tint"
        );
    }
}

#[test]
fn a_link_moves_from_its_target_only_as_far_as_it_must() {
    let mut moved = 0;
    for (case, palette) in sweep() {
        let dark = case.theme.is_dark();
        let target = if dark { 0.76 } else { 0.50 };
        let [lightness, ..] = palette.link.to_oklab();
        if (lightness - target).abs() < 1e-3 {
            continue;
        }
        moved += 1;
        // One step back towards the target breaks a floor.
        let back = if dark {
            lightness - 0.002
        } else {
            lightness + 0.002
        };
        // The colour the fitting tried there: the authored chroma, as much
        // of it as sRGB holds at that lightness.
        let hue = case.accent.hue_degrees();
        let back = if dark {
            back.max(target)
        } else {
            back.min(target)
        };
        let closer = rune::oklch(back, floors::in_gamut(back, 0.15, hue), hue);
        let surfaces = [
            palette.surface_content,
            palette.surface_pill,
            palette.surface_hover,
            palette.surface_sidebar,
            palette.surface_code,
            palette.surface_selected,
            palette.accent_soft,
        ];
        assert!(
            surfaces
                .iter()
                .any(|surface| contrast(closer, *surface) < 4.5),
            "{case:?}: link at L {lightness} could sit closer to {target}"
        );
    }
    assert!(moved > 0, "some accent and tint need a fitted link");
}

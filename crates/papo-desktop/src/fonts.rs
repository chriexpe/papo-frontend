//! The fonts the app sets its text in (`docs/design.md` §2 and §9): Inter
//! for everything, with its italic faces for emphasis, and JetBrains Mono for
//! code. Both are under the SIL Open Font License, beside the files in
//! `assets/fonts/`.
//!
//! Owns only the font files and the family names. Rune registers them in
//! place of its own bundled Inter, so a window and a headless test lay text
//! out in the same faces; a glyph these lack falls back to the system's
//! fonts.

/// Inter 4.1 (Regular, Medium, SemiBold and their italics; a mention in
/// italic text is weight 500) and JetBrains Mono 2.304 Regular.
pub(crate) const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Italic.ttf"),
    include_bytes!("../assets/fonts/Inter-MediumItalic.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBoldItalic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
];

/// The sans and mono families, as Rune's `AppSpec::families` names them.
pub(crate) const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");

/// A text system over [`FONTS`], for headless runs.
#[cfg(test)]
pub(crate) fn text_system() -> rune::text::TextSystem {
    rune::text::TextSystem::new(FONTS, FAMILIES)
}

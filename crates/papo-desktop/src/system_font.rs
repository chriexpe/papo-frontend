//! The system's interface font, for "Use the system font" (`docs/design.md`
//! §2): the family the desktop names through the settings portal (GNOME's
//! `font-name`, KDE's `font`), when it is installed; else the family the
//! system resolves `system-ui` to (fontconfig on Linux, the OS's font API
//! elsewhere). KDE writes its font only once the user changes it, so a
//! default KDE desktop names none and fontconfig answers.
//!
//! Owns reading and resolving the name, once, when the SDK client opens.
//! The shell holds it (`Shell::system_font_found`) and the look applies it.

use fontique::{Collection, CollectionOptions, GenericFamily};
use papo_sdk::ClientHandle;
use zbus::OwnedVariant;

use crate::sdk_bridge::Updates;
use crate::shell::Shell;

/// The desktop's settings, through the portal (`org.freedesktop.portal.Settings`).
#[zbus::proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Settings {
    fn read_one(&self, namespace: &str, key: &str) -> zbus::Result<OwnedVariant>;
}

/// Where each desktop keeps its interface font, and how it is written.
const DESKTOP_FONTS: [(&str, &str, Format); 2] = [
    ("org.gnome.desktop.interface", "font-name", Format::Pango),
    ("org.kde.kdeglobals.General", "font", Format::Qt),
];

/// How a desktop writes a font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// A Pango description: "Cantarell 11", "Noto Sans Bold Italic 10".
    Pango,
    /// A Qt font string: "Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1".
    Qt,
}

/// Pango's style, weight, variant and stretch words, which follow the
/// family in a description (Pango's `pango_font_description_from_string`).
const PANGO_STYLE_WORDS: &[&str] = &[
    "normal",
    "roman",
    "oblique",
    "italic",
    "small-caps",
    "all-small-caps",
    "petite-caps",
    "all-petite-caps",
    "unicase",
    "title-caps",
    "thin",
    "ultra-light",
    "extra-light",
    "light",
    "semi-light",
    "demi-light",
    "book",
    "regular",
    "medium",
    "semi-bold",
    "demi-bold",
    "bold",
    "ultra-bold",
    "extra-bold",
    "heavy",
    "black",
    "ultra-heavy",
    "extra-heavy",
    "ultra-condensed",
    "extra-condensed",
    "condensed",
    "semi-condensed",
    "semi-expanded",
    "expanded",
    "extra-expanded",
    "ultra-expanded",
    "not-rotated",
    "south",
    "upside-down",
    "north",
    "rotated-left",
    "east",
    "rotated-right",
    "west",
];

/// Finds the system's interface font and hands it to the shell; nothing
/// when no installed family can be named.
pub(crate) async fn run(_client: ClientHandle, updates: Updates<Shell>) {
    let named = desktop_font().await;
    let found = tokio::task::spawn_blocking(move || {
        let mut fonts = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        resolve(named.as_deref(), &mut fonts)
    })
    .await;
    match found {
        Ok(Some(family)) => {
            log::debug!("the system font is {family}");
            updates.send_tx(move |shell, tx| shell.system_font_found(family, tx));
        }
        Ok(None) => log::info!("no system font: the system names no installed family"),
        Err(error) => log::warn!("no system font: its lookup failed: {error}"),
    }
}

/// The family the desktop names, if it names one.
async fn desktop_font() -> Option<String> {
    let bus = zbus::connection::Builder::session()
        .build()
        .await
        .inspect_err(|error| log::info!("no desktop font: no session bus: {error}"))
        .ok()?;
    let settings = SettingsProxy::new(&bus).await.ok()?;
    for (namespace, key, format) in DESKTOP_FONTS {
        // A desktop that keeps no such key answers with an error.
        let Ok(value) = settings.read_one(namespace, key).await else {
            continue;
        };
        let Ok(written) = String::try_from(value) else {
            continue;
        };
        if let Some(family) = family_of(&written, format) {
            return Some(family);
        }
    }
    None
}

/// The family `written` names, in `format`; none when it names none.
fn family_of(written: &str, format: Format) -> Option<String> {
    let family = match format {
        Format::Qt => written
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned(),
        Format::Pango => pango_family(written),
    };
    (!family.is_empty()).then_some(family)
}

/// A Pango description's family: what is left once its size (the last word,
/// when a number) and then its style words are taken off the end. A family
/// list ("Cantarell, Noto Sans 11") gives its first family.
fn pango_family(description: &str) -> String {
    let mut words: Vec<&str> = description.split_whitespace().collect();
    if words.len() > 1
        && let Some(last) = words.last()
        && last.trim_end_matches("px").parse::<f32>().is_ok()
    {
        words.pop();
    }
    while words.len() > 1
        && let Some(last) = words.last()
        && PANGO_STYLE_WORDS.contains(&last.to_ascii_lowercase().as_str())
    {
        words.pop();
    }
    let family = words.join(" ");
    family
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// The family to use: the desktop's, by its installed name, when it is
/// installed; else the family the system resolves `system-ui` to.
fn resolve(named: Option<&str>, fonts: &mut Collection) -> Option<String> {
    let installed = named.and_then(|name| {
        let id = fonts.family_id(name)?;
        fonts.family_name(id).map(str::to_owned)
    });
    if installed.is_none()
        && let Some(name) = named
    {
        log::info!("the desktop's font {name:?} is not installed; asking the system for its own");
    }
    installed.or_else(|| {
        let id = fonts.generic_families(GenericFamily::SystemUi).next()?;
        fonts.family_name(id).map(str::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use super::{Format, family_of};

    #[test]
    fn a_gnome_font_names_its_family_without_size_or_style() {
        let family = |written| family_of(written, Format::Pango);
        assert_eq!(family("Cantarell 11").as_deref(), Some("Cantarell"));
        assert_eq!(family("Adwaita Sans 11").as_deref(), Some("Adwaita Sans"));
        assert_eq!(
            family("Noto Sans Bold Italic 10").as_deref(),
            Some("Noto Sans")
        );
        assert_eq!(family("Ubuntu Semi-Bold 10.5").as_deref(), Some("Ubuntu"));
        assert_eq!(family("Inter 14px").as_deref(), Some("Inter"));
        assert_eq!(
            family("Cantarell, Noto Sans 11").as_deref(),
            Some("Cantarell")
        );
    }

    #[test]
    fn a_family_that_is_only_style_words_is_kept_whole() {
        // "Black" is a style word, but a lone word is the family.
        assert_eq!(
            family_of("Black 12", Format::Pango).as_deref(),
            Some("Black")
        );
    }

    #[test]
    fn a_kde_font_names_the_family_before_its_first_comma() {
        let family = |written| family_of(written, Format::Qt);
        assert_eq!(
            family("Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1").as_deref(),
            Some("Noto Sans")
        );
        assert_eq!(family("Hack,9,-1,5,50,0,0,0,0,0").as_deref(), Some("Hack"));
    }

    #[test]
    fn an_empty_setting_names_no_family() {
        assert_eq!(family_of("", Format::Qt), None);
        assert_eq!(family_of("  ", Format::Pango), None);
    }
}

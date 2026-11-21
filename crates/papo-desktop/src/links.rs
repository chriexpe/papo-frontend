//! Opening a link from a message (`docs/design.md` §9, Links): at once when
//! its host is the server's own or a trusted domain, and for every `mailto:`
//! link; anything else asks first, in a confirmation with a box that trusts
//! the host from then on.
//!
//! The trusted domains are a device preference, kept by the desktop in its
//! settings file: the SDK keeps no device preferences. Until the settings
//! screen exists (D6), "Confirm external links" is on and the list holds
//! only what the box adds. Opening is Rune's (`Tx::open_url`).

use std::collections::BTreeSet;

use rune::prefs::Prefs;
use rune::{El, Span, col, el, icon, rich, row, text};

use crate::design::{
    CONFIRM_WIDTH_PT, FIELD_HEIGHT_PT, FIELD_OUTLINE_PT, FOCUS_RING_PT, PILL_OUTLINE_PT,
    POPUP_PADDING_PT, POPUP_SHADOW_PT, Palette, RADIUS_CONTROL_PT, RADIUS_FIELD_PT,
    RADIUS_POPUP_PT, SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, SPACE_XXL_PT, TASK_BOX_OUTLINE_PT,
    TASK_BOX_PT, TASK_BOX_RADIUS_PT, TASK_CHECK_PT, TYPE_BODY_STRONG, TYPE_CODE, TYPE_DISPLAY,
    TYPE_SMALL, WEIGHT_SEMIBOLD,
};
use crate::shell::Shell;
use crate::strings;
use crate::window::APP_ID;

/// The settings file's key for the trusted domains.
const TRUSTED_KEY: &str = "trusted_domains";

/// What a press on a link does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Open,
    /// Ask first: the link goes to `host`, which is not trusted.
    Ask {
        host: String,
    },
    /// Not a link the app opens (the SDK passes only safe ones).
    Refuse,
}

/// What pressing `url` does, in a server whose host is `server_host`.
pub(crate) fn decide(url: &str, server_host: Option<&str>, trusted: &Trusted) -> Decision {
    let Ok(parsed) = url::Url::parse(url) else {
        return Decision::Refuse;
    };
    match parsed.scheme() {
        // Only opens the mail app, which asks before sending anything.
        "mailto" => Decision::Open,
        "http" | "https" => {
            let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
                return Decision::Refuse;
            };
            if server_host == Some(host.as_str()) || trusted.contains(&host) {
                Decision::Open
            } else {
                Decision::Ask { host }
            }
        }
        _ => Decision::Refuse,
    }
}

/// The hosts whose links open without asking: this device's preference.
pub(crate) struct Trusted {
    hosts: BTreeSet<String>,
    prefs: Prefs,
}

impl Default for Trusted {
    /// None, kept nowhere: before the settings file is read, and in tests.
    fn default() -> Trusted {
        Trusted {
            hosts: BTreeSet::new(),
            prefs: Prefs::memory(),
        }
    }
}

impl Trusted {
    /// The trusted domains in this user's settings file.
    pub(crate) fn load() -> Trusted {
        let prefs = Prefs::open(APP_ID);
        let hosts = prefs.get(TRUSTED_KEY).unwrap_or_default();
        Trusted { hosts, prefs }
    }

    pub(crate) fn contains(&self, host: &str) -> bool {
        self.hosts.contains(host)
    }

    /// Trusts `host` from now on, and saves the list. A failed save keeps
    /// it trusted until the app closes.
    pub(crate) fn add(&mut self, host: String) {
        if !self.hosts.insert(host) {
            return;
        }
        if let Err(error) = self.prefs.set(TRUSTED_KEY, &self.hosts) {
            log::warn!("the trusted domains were not saved: {error}");
        }
    }
}

/// A link waiting for the reader's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Prompt {
    pub(crate) url: String,
    pub(crate) host: String,
    /// The "Always open links from {host}" box.
    pub(crate) always: bool,
}

/// "Open this link?" over the dimmed app: the address with its host in
/// bold, the box, Cancel and Open. Escape and Cancel keep it closed.
pub(crate) fn confirm(prompt: &Prompt, palette: &Palette) -> El {
    let (drop, blur) = POPUP_SHADOW_PT;
    let cancel = button("open-link-cancel", strings::link_cancel(), palette)
        .border(FIELD_OUTLINE_PT, palette.line_strong)
        .on_press(Shell::cancel_link)
        .child(
            text(strings::link_cancel())
                .font(TYPE_BODY_STRONG)
                .color(palette.text_primary),
        );
    let open = button("open-link-confirm", strings::link_open(), palette)
        .fill(palette.accent)
        .default_button()
        .on_press_tx(Shell::confirm_link)
        .child(
            text(strings::link_open())
                .font(TYPE_BODY_STRONG)
                .color(palette.accent_on),
        );
    let frame = col()
        .key("open-link")
        .w(CONFIRM_WIDTH_PT)
        .padding(POPUP_PADDING_PT)
        .gap(SPACE_LG_PT)
        .radius(RADIUS_POPUP_PT)
        .fill(palette.surface_content)
        .border(PILL_OUTLINE_PT, palette.line_subtle)
        .shadow(drop, blur, palette.popup_shadow)
        .role("alertdialog")
        .label(strings::link_title())
        .modal(false, Shell::cancel_link)
        .child(
            text(strings::link_title())
                .font(TYPE_DISPLAY)
                .color(palette.text_primary)
                .role("heading")
                .level(2)
                .wrap(),
        )
        .child(address(prompt, palette))
        .child(always_box(prompt, palette))
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(SPACE_MD_PT)
                .child(cancel)
                .child(open),
        );
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .padding(SPACE_XXL_PT)
        .fill(palette.scrim)
        .child(frame)
}

/// The whole address in mono, its host in bold.
fn address(prompt: &Prompt, palette: &Palette) -> El {
    let start = prompt.url.find("://").map_or(0, |at| at + 3);
    let host = prompt.url[start..]
        .to_ascii_lowercase()
        .find(&prompt.host)
        .map(|at| start + at..start + at + prompt.host.len());
    let spans = host
        .map(|host| {
            vec![
                Span::new(host.start, host.end)
                    .weight(WEIGHT_SEMIBOLD)
                    .color(palette.text_primary),
            ]
        })
        .unwrap_or_default();
    el().key("address")
        .w_full()
        .py(SPACE_SM_PT * 1.5)
        .px(SPACE_MD_PT)
        .radius(RADIUS_CONTROL_PT)
        .fill(palette.surface_code)
        .child(
            rich(prompt.url.clone(), spans)
                .font(TYPE_CODE)
                .color(palette.text_secondary)
                .wrap_anywhere()
                .w_full()
                .selectable(true)
                .selection_color(palette.text_selection),
        )
}

/// "Always open links from {host}", with the task box (§9), here one that
/// toggles: hover, a focus ring, Space.
fn always_box(prompt: &Prompt, palette: &Palette) -> El {
    let label = strings::link_always(&prompt.host);
    let square = el()
        .size(TASK_BOX_PT, TASK_BOX_PT)
        .radius(TASK_BOX_RADIUS_PT)
        .center();
    let square = if prompt.always {
        square
            .fill(palette.accent)
            .child(icon(&crate::icons::CHECK, TASK_CHECK_PT).color(palette.accent_on))
    } else {
        square.border(TASK_BOX_OUTLINE_PT, palette.line_strong)
    };
    row()
        .key("open-link-always")
        .gap(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .hover(palette.surface_hover)
        .role("checkbox")
        .checked(prompt.always)
        .label(label.clone())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
        .on_press(Shell::toggle_trust_link)
        .child(square)
        .child(
            text(label)
                .font(TYPE_SMALL)
                .color(palette.text_secondary)
                .wrap(),
        )
}

fn button(key: &'static str, label: &'static str, palette: &Palette) -> El {
    row()
        .key(key)
        .h(FIELD_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .center()
        .radius(RADIUS_FIELD_PT)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
}

#[cfg(test)]
mod tests {
    use super::{Decision, Trusted, decide};

    #[test]
    fn the_servers_own_links_and_mail_open_at_once() {
        let trusted = Trusted::default();
        let server = Some("papo.example.com");
        assert_eq!(
            decide("https://papo.example.com/invite", server, &trusted),
            Decision::Open
        );
        assert_eq!(
            decide("https://PAPO.example.com/", server, &trusted),
            Decision::Open,
            "hosts compare without case"
        );
        assert_eq!(
            decide("mailto:lia@example.com", server, &trusted),
            Decision::Open
        );
    }

    #[test]
    fn other_hosts_ask_until_trusted() {
        let mut trusted = Trusted::default();
        let server = Some("papo.example.com");
        let ask = Decision::Ask {
            host: "lumen.example.org".to_owned(),
        };
        assert_eq!(
            decide("https://lumen.example.org/guide", server, &trusted),
            ask
        );
        trusted.add("lumen.example.org".to_owned());
        assert_eq!(
            decide("https://lumen.example.org/guide", server, &trusted),
            Decision::Open
        );
        assert_eq!(
            decide("https://docs.lumen.example.org/", server, &trusted),
            Decision::Ask {
                host: "docs.lumen.example.org".to_owned()
            },
            "a subdomain is a host of its own"
        );
    }

    #[test]
    fn a_host_hidden_behind_userinfo_is_the_real_host() {
        let trusted = Trusted::default();
        assert_eq!(
            decide(
                "https://papo.example.com@evil.example.net/",
                Some("papo.example.com"),
                &trusted
            ),
            Decision::Ask {
                host: "evil.example.net".to_owned()
            }
        );
    }

    #[test]
    fn other_schemes_never_open() {
        let trusted = Trusted::default();
        assert_eq!(
            decide("file:///etc/passwd", None, &trusted),
            Decision::Refuse
        );
        assert_eq!(decide("not a url", None, &trusted), Decision::Refuse);
    }
}

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
use rune::{El, Span, el, icon, rich, row, text};
use url::Position;

use crate::confirm::{Question, confirm as ask};
use crate::design::{
    FOCUS_RING_PT, Look, RADIUS_CONTROL, SPACE_MD, SPACE_SM, TASK_BOX, TASK_BOX_OUTLINE_PT,
    TASK_BOX_RADIUS, TASK_CHECK, TYPE_CODE, TYPE_SMALL, WEIGHT_SEMIBOLD,
};
use crate::device_settings::{SettingsFile, Stored, Write};
use crate::shell::Shell;
use crate::strings;

/// The settings file's key for the trusted domains.
const TRUSTED_KEY: &str = "trusted_domains";

/// The `mailto:` fields a mail app is given: what to fill in, never an
/// `attach` or another parameter that reaches the reader's files.
const MAIL_FIELDS: [&str; 4] = ["subject", "body", "cc", "bcc"];

/// What a press on a link does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Open this address: the link's own, or a `mailto:` link without the
    /// fields a mail app should not take from a message.
    Open(String),
    /// Ask first: the link goes to `host`, which is not trusted.
    Ask { host: String },
    /// Not a link the app opens (the SDK passes only safe ones).
    Refuse,
}

/// The host `url` goes to, as a menu names it; none for an address with
/// none (`mailto:`).
pub(crate) fn host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// What pressing `url` does, in a server whose host is `server_host`.
pub(crate) fn decide(url: &str, server_host: Option<&str>, trusted: &Trusted) -> Decision {
    let Ok(parsed) = url::Url::parse(url) else {
        return Decision::Refuse;
    };
    match parsed.scheme() {
        // Only opens the mail app, which asks before sending anything.
        "mailto" => Decision::Open(mail_only(&parsed)),
        "http" | "https" => {
            let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else {
                return Decision::Refuse;
            };
            if server_host == Some(host.as_str()) || trusted.contains(&host) {
                Decision::Open(url.to_owned())
            } else {
                Decision::Ask { host }
            }
        }
        _ => Decision::Refuse,
    }
}

/// `mail` with only the [`MAIL_FIELDS`] of its query, as written.
fn mail_only(mail: &url::Url) -> String {
    let kept: Vec<&str> = mail
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|field| {
            // The name as a mail app decodes it, so `%61ttach` is `attach`.
            url::form_urlencoded::parse(field.as_bytes())
                .next()
                .is_some_and(|(name, _)| {
                    MAIL_FIELDS
                        .iter()
                        .any(|kept| name.eq_ignore_ascii_case(kept))
                })
        })
        .collect();
    let mut mail = mail.clone();
    mail.set_query((!kept.is_empty()).then(|| kept.join("&")).as_deref());
    mail.set_fragment(None);
    mail.into()
}

/// The hosts whose links open without asking: this device's preference,
/// changed from the link confirmation's box and, with D6b, from General ›
/// Trusted domains. A change shows at once and is written off the UI
/// thread; a failed write takes the list back to what the file holds.
pub(crate) struct Trusted {
    stored: Stored<BTreeSet<String>>,
    file: SettingsFile,
}

impl Default for Trusted {
    /// None, kept nowhere: before the settings file is read, and in tests.
    fn default() -> Trusted {
        Trusted::read(&Prefs::memory())
    }
}

impl Trusted {
    /// The trusted domains in `prefs`, this device's settings file.
    pub(crate) fn read(prefs: &Prefs) -> Trusted {
        Trusted {
            stored: Stored::new(prefs.get(TRUSTED_KEY).unwrap_or_default()),
            file: SettingsFile::of(prefs),
        }
    }

    pub(crate) fn contains(&self, host: &str) -> bool {
        self.stored.shown().contains(host)
    }

    /// Trusts `host` from now on; the list to write now, unless a write is
    /// running (it then follows) or the host was trusted already.
    pub(crate) fn add(&mut self, host: String) -> Option<BTreeSet<String>> {
        let mut hosts = self.stored.shown().clone();
        hosts.insert(host);
        self.stored.change(hosts)
    }

    /// The write of `hosts` landed (`ok`) or did not, which takes the list
    /// back to what the file holds; the list to write next.
    pub(crate) fn written(
        &mut self,
        hosts: BTreeSet<String>,
        ok: bool,
    ) -> Option<BTreeSet<String>> {
        self.stored.written(hosts, ok)
    }

    /// The write that saves `hosts`.
    pub(crate) fn write_of(&self, hosts: &BTreeSet<String>) -> Write {
        self.file
            .write(vec![(TRUSTED_KEY, serde_json::json!(hosts))])
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

/// "Open this link?" over the dimmed app, in the one shape of a
/// confirmation (`docs/design.md` §10): the address with its host in bold,
/// the box as the foot row's note, Cancel (focused) and Open. Escape,
/// Return and Cancel keep it closed.
pub(crate) fn confirm(prompt: &Prompt, look: &Look) -> El {
    let question = Question {
        key: "open-link",
        title: strings::link_title(),
        body: None,
        shown: Some(address(prompt, look)),
        note: Some(always_box(prompt, look)),
        cancel_key: "open-link-cancel",
        cancel: strings::link_cancel(),
        confirm_key: "open-link-confirm",
        confirm: strings::link_open(),
        danger: true,
    };
    ask(question, Shell::cancel_link, Shell::confirm_link, look)
}

/// The whole address in mono, as the URL parser reads it, its host in
/// bold: the host that opens, never a look-alike inside the user name.
fn address(prompt: &Prompt, look: &Look) -> El {
    let parsed = url::Url::parse(&prompt.url).ok();
    let shown = parsed
        .as_ref()
        .map_or_else(|| prompt.url.clone(), |url| url.as_str().to_owned());
    let host = parsed.as_ref().map(|url| {
        let start = url[..Position::BeforeHost].len();
        start..start + url[Position::BeforeHost..Position::AfterHost].len()
    });
    let spans = host
        .map(|host| {
            vec![
                Span::new(host.start, host.end)
                    .weight(WEIGHT_SEMIBOLD)
                    .color(look.palette.text_primary),
            ]
        })
        .unwrap_or_default();
    el().key("address")
        .w_full()
        .py(look.space(SPACE_SM) * 1.5)
        .px(look.space(SPACE_MD))
        .radius(look.radius(RADIUS_CONTROL))
        .fill(look.palette.surface_code)
        .child(
            rich(shown, spans)
                .font(look.font(TYPE_CODE))
                .color(look.palette.text_secondary)
                .wrap_anywhere()
                .w_full()
                .selectable(true)
                .selection_color(look.palette.text_selection),
        )
}

/// "Always open links from {host}", with the task box (§9), here one that
/// toggles: hover, a focus ring, Space.
fn always_box(prompt: &Prompt, look: &Look) -> El {
    let label = strings::link_always(&prompt.host);
    let square = el()
        .size(look.text_multiple(TASK_BOX), look.text_multiple(TASK_BOX))
        .radius(look.radius(TASK_BOX_RADIUS))
        .center();
    let square = if prompt.always {
        square.fill(look.palette.accent).child(
            icon(&crate::icons::CHECK, look.text_multiple(TASK_CHECK))
                .color(look.palette.accent_on),
        )
    } else {
        square.border(TASK_BOX_OUTLINE_PT, look.palette.line_strong)
    };
    // As wide as the foot row leaves it, so the label wraps beside Cancel
    // instead of running under it.
    row()
        .key("open-link-always")
        .w_full()
        .min_w(0.0)
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.palette.surface_hover)
        .role("checkbox")
        .checked(prompt.always)
        .label(label.clone())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::toggle_trust_link)
        .child(square)
        .child(
            text(label)
                .key("open-link-always-label")
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_secondary)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

#[cfg(test)]
mod tests {
    use super::{Decision, Trusted, decide};

    fn open(url: &str) -> Decision {
        Decision::Open(url.to_owned())
    }

    #[test]
    fn the_servers_own_links_and_mail_open_at_once() {
        let trusted = Trusted::default();
        let server = Some("papo.example.com");
        let invite = "https://papo.example.com/invite";
        assert_eq!(decide(invite, server, &trusted), open(invite));
        let upper = "https://PAPO.example.com/";
        assert_eq!(
            decide(upper, server, &trusted),
            open(upper),
            "hosts compare without case"
        );
        let mail = "mailto:lia@example.com?subject=Hi%20there&body=See%20you";
        assert_eq!(decide(mail, server, &trusted), open(mail));
    }

    #[test]
    fn mail_never_takes_fields_that_reach_the_readers_files() {
        let trusted = Trusted::default();
        assert_eq!(
            decide(
                "mailto:lia@example.com?attach=/etc/passwd&subject=Hi&%61ttachment=~/.ssh/id_ed25519",
                None,
                &trusted
            ),
            open("mailto:lia@example.com?subject=Hi")
        );
        assert_eq!(
            decide("mailto:lia@example.com?attach=/etc/passwd", None, &trusted),
            open("mailto:lia@example.com")
        );
    }

    #[test]
    fn other_hosts_ask_until_trusted() {
        let mut trusted = Trusted::default();
        let server = Some("papo.example.com");
        let guide = "https://lumen.example.org/guide";
        let ask = Decision::Ask {
            host: "lumen.example.org".to_owned(),
        };
        assert_eq!(decide(guide, server, &trusted), ask);
        let _ = trusted.add("lumen.example.org".to_owned());
        assert_eq!(decide(guide, server, &trusted), open(guide));
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

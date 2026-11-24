//! Opening a link from a message (`docs/design.md` §9, Links): at once when
//! its host is the server's own or a trusted domain, and for every `mailto:`
//! link; anything else asks first, in a confirmation with a box that trusts
//! the host from then on.
//!
//! The trusted domains are a device preference (`trusted_domains`), and so
//! is "Confirm external links" (`general_settings`): off, every web link
//! opens at once. Opening is Rune's (`Tx::open_url`).

use rune::{El, Span, el, rich};
use url::Position;

use crate::confirm::{Question, confirm as ask};
use crate::design::{Look, RADIUS_CONTROL, SPACE_MD, SPACE_SM, TYPE_CODE, WEIGHT_SEMIBOLD};
use crate::shell::Shell;
use crate::strings;
use crate::trusted_domains::Trusted;

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

/// What pressing `url` does, in a server whose host is `server_host`;
/// `confirm` is "Confirm external links".
pub(crate) fn decide(
    url: &str,
    server_host: Option<&str>,
    trusted: &Trusted,
    confirm: bool,
) -> Decision {
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
            if !confirm || server_host == Some(host.as_str()) || trusted.contains(&host) {
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
    crate::checkbox::checkbox(
        "open-link-always",
        strings::link_always(&prompt.host),
        prompt.always,
        Shell::toggle_trust_link,
        look,
    )
}

#[cfg(test)]
mod tests {
    use super::{Decision, decide};
    use crate::trusted_domains::Trusted;

    fn open(url: &str) -> Decision {
        Decision::Open(url.to_owned())
    }

    #[test]
    fn the_servers_own_links_and_mail_open_at_once() {
        let trusted = Trusted::default();
        let server = Some("papo.example.com");
        let invite = "https://papo.example.com/invite";
        assert_eq!(decide(invite, server, &trusted, true), open(invite));
        let upper = "https://PAPO.example.com/";
        assert_eq!(
            decide(upper, server, &trusted, true),
            open(upper),
            "hosts compare without case"
        );
        let mail = "mailto:lia@example.com?subject=Hi%20there&body=See%20you";
        assert_eq!(decide(mail, server, &trusted, true), open(mail));
    }

    #[test]
    fn mail_never_takes_fields_that_reach_the_readers_files() {
        let trusted = Trusted::default();
        assert_eq!(
            decide(
                "mailto:lia@example.com?attach=/etc/passwd&subject=Hi&%61ttachment=~/.ssh/id_ed25519",
                None,
                &trusted,
                true
            ),
            open("mailto:lia@example.com?subject=Hi")
        );
        assert_eq!(
            decide(
                "mailto:lia@example.com?attach=/etc/passwd",
                None,
                &trusted,
                true
            ),
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
        assert_eq!(decide(guide, server, &trusted, true), ask);
        let _ = trusted.add("lumen.example.org".to_owned());
        assert_eq!(decide(guide, server, &trusted, true), open(guide));
        assert_eq!(
            decide("https://docs.lumen.example.org/", server, &trusted, true),
            Decision::Ask {
                host: "docs.lumen.example.org".to_owned()
            },
            "a subdomain is a host of its own"
        );
    }

    #[test]
    fn with_confirm_external_links_off_every_web_link_opens() {
        let trusted = Trusted::default();
        let guide = "https://lumen.example.org/guide";
        assert_eq!(decide(guide, None, &trusted, false), open(guide));
        assert_eq!(
            decide("ftp://lumen.example.org/", None, &trusted, false),
            Decision::Refuse,
            "still only web and mail links"
        );
    }

    #[test]
    fn a_host_hidden_behind_userinfo_is_the_real_host() {
        let trusted = Trusted::default();
        assert_eq!(
            decide(
                "https://papo.example.com@evil.example.net/",
                Some("papo.example.com"),
                &trusted,
                true
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
            decide("file:///etc/passwd", None, &trusted, true),
            Decision::Refuse
        );
        assert_eq!(decide("not a url", None, &trusted, true), Decision::Refuse);
    }
}

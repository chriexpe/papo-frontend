//! Shared search grammar, filter autocomplete and stable-identity resolution.
use crate::state::{ChannelKind, Store};

/// Busca dentro da pastilha: campo em cima, resultados embaixo.
#[derive(Clone, Debug)]
pub struct SearchShortcut {
    pub label: String,
    pub replacement: String,
}

pub fn search_fragment(query: &str) -> (&str, usize) {
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in query.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            start = index + ch.len_utf8();
        }
    }
    (&query[start..], start)
}

pub fn search_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in query.chars() {
        if escaped {
            token.push(ch);
            escaped = false;
        } else if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(ch);
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

#[derive(Clone, Copy, Debug, Default)]
struct SearchFilterUsage {
    author: bool,
    channel: bool,
    mentions: bool,
    attachment: bool,
    link: bool,
}

fn search_filter_usage(query: &str) -> SearchFilterUsage {
    let mut usage = SearchFilterUsage::default();

    for token in search_tokens(query) {
        let Some((prefix, value)) = token.split_once(':') else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        match prefix.to_lowercase().as_str() {
            "de" | "from" | "author" => usage.author = true,
            "em" | "in" | "channel" => usage.channel = true,
            "mentions" | "menciona" | "mention" => usage.mentions = true,
            "tem" | "has" => match value.to_lowercase().as_str() {
                "link" => usage.link = true,
                "anexo" | "arquivo" | "attachment" | "file" => usage.attachment = true,
                _ => {}
            },
            _ => {}
        }
    }

    usage
}

pub fn search_shortcuts(
    store: &Store,
    query: &str,
    explicit_attachment: bool,
) -> Vec<SearchShortcut> {
    let (fragment, start) = search_fragment(query);
    let lower = fragment.to_lowercase();
    let mut usage = search_filter_usage(&query[..start]);
    usage.attachment |= explicit_attachment;
    let mut out = Vec::new();

    if !lower.contains(':') {
        let candidates = [
            ("de:", "autor", !usage.author),
            ("em:", "canal", !usage.channel),
            ("tem:", "conteúdo", !(usage.link && usage.attachment)),
            ("mentions:", "menção", !usage.mentions),
        ];
        for (token, hint, available) in candidates {
            if available && !lower.is_empty() && token.starts_with(&lower) {
                out.push(SearchShortcut {
                    label: format!("{token}  {hint}"),
                    replacement: token.to_owned(),
                });
            }
        }
        return out;
    }

    let Some((prefix, raw_value)) = fragment.split_once(':') else {
        return out;
    };
    let prefix_lower = prefix.to_lowercase();
    let value = search_tokens(raw_value)
        .into_iter()
        .next()
        .unwrap_or_default();
    let needle = value.trim_start_matches(['@', '#']).to_lowercase();

    match prefix_lower.as_str() {
        "de" | "from" | "author" if !usage.author => {
            let mut members: Vec<_> = store
                .members
                .iter()
                .filter(|member| {
                    needle.is_empty()
                        || member.name.to_lowercase().contains(&needle)
                        || member.username.to_lowercase().contains(&needle)
                })
                .collect();
            members.sort_by_key(|member| {
                let name = member.name.to_lowercase();
                let username = member.username.to_lowercase();
                (
                    !name.starts_with(&needle),
                    !username.starts_with(&needle),
                    name,
                )
            });
            for member in members.into_iter().take(6) {
                out.push(SearchShortcut {
                    label: format!("de: @{}  {}", member.username, member.name),
                    replacement: format!("de:@{}", member.username),
                });
            }
        }
        "em" | "in" | "channel" if !usage.channel => {
            let mut channels: Vec<_> = store
                .channels
                .iter()
                .filter(|channel| {
                    channel.kind != ChannelKind::Direct
                        && (needle.is_empty() || channel.name.to_lowercase().contains(&needle))
                })
                .collect();
            channels.sort_by_key(|channel| {
                let name = channel.name.to_lowercase();
                (!name.starts_with(&needle), name)
            });
            for channel in channels.into_iter().take(6) {
                out.push(SearchShortcut {
                    label: format!("em: #{}", channel.name),
                    replacement: format!(
                        "em:{}",
                        serde_json::to_string(&format!("#{}", channel.name)).unwrap()
                    ),
                });
            }
        }
        "mentions" | "menciona" | "mention" if !usage.mentions => {
            let mut members: Vec<_> = store
                .members
                .iter()
                .filter(|member| {
                    needle.is_empty()
                        || member.name.to_lowercase().contains(&needle)
                        || member.username.to_lowercase().contains(&needle)
                })
                .collect();
            members.sort_by_key(|member| {
                let name = member.name.to_lowercase();
                let username = member.username.to_lowercase();
                (
                    !name.starts_with(&needle),
                    !username.starts_with(&needle),
                    name,
                )
            });
            for member in members.into_iter().take(6) {
                out.push(SearchShortcut {
                    label: format!("mentions: @{}  {}", member.username, member.name),
                    replacement: format!("mentions:@{}", member.username),
                });
            }
        }
        "tem" | "has" => {
            for value in ["link", "anexo", "arquivo"] {
                let available = match value {
                    "link" => !usage.link,
                    "anexo" | "arquivo" => !usage.attachment,
                    _ => true,
                };
                if available && (needle.is_empty() || value.starts_with(&needle)) {
                    out.push(SearchShortcut {
                        label: format!("tem: {value}"),
                        replacement: format!("tem:{value}"),
                    });
                }
            }
        }
        _ => {}
    }
    out
}

pub fn replace_search_fragment(query: &str, replacement: &str) -> String {
    let (_, start) = search_fragment(query);
    let mut next = query[..start].to_owned();
    next.push_str(replacement);
    if !replacement.ends_with(':') {
        next.push(' ');
    }
    next
}

fn search_member_id(store: &Store, raw: &str) -> Option<String> {
    let value = raw.trim_start_matches('@');
    store
        .members
        .iter()
        .find(|member| {
            member.username.eq_ignore_ascii_case(value)
                || member.name.eq_ignore_ascii_case(value)
                || member.id == value
        })
        .map(|member| member.id.clone())
}

fn search_channel_id(store: &Store, raw: &str) -> Option<String> {
    let value = raw.trim_start_matches('#');
    store
        .channels
        .iter()
        .find(|channel| channel.name.eq_ignore_ascii_case(value) || channel.id == value)
        .map(|channel| channel.id.clone())
}

pub fn search_request(
    query: &str,
    store: &Store,
    attachments: bool,
    oldest_first: bool,
    scope: Option<&str>,
) -> Option<crate::api::models::SearchRequest> {
    let mut text = Vec::new();
    let mut author = None;
    let mut channel_id = scope.map(str::to_owned);
    let mut mention = None;
    let mut contains_attachment = attachments.then_some(true);
    let mut has = None;

    let tokens = search_tokens(query);
    for token in &tokens {
        let Some((prefix, value)) = token.split_once(':') else {
            text.push(token.as_str());
            continue;
        };
        match prefix.to_lowercase().as_str() {
            "de" | "from" | "author" => {
                if author.is_none() {
                    author = Some(search_member_id(store, value)?);
                }
            }
            "em" | "in" | "channel" => {
                if channel_id.is_none() {
                    channel_id = Some(search_channel_id(store, value)?);
                }
            }
            "mentions" | "menciona" | "mention" => {
                if mention.is_none() {
                    mention = Some(search_member_id(store, value)?);
                }
            }
            "tem" | "has" => match value.to_lowercase().as_str() {
                "" => return None,
                "link" => has = Some("link".to_owned()),
                "anexo" | "arquivo" | "attachment" | "file" => contains_attachment = Some(true),
                _ => text.push(token.as_str()),
            },
            _ => text.push(token.as_str()),
        }
    }

    let text = text.join(" ");
    if text.trim().is_empty()
        && author.is_none()
        && channel_id.is_none()
        && mention.is_none()
        && contains_attachment.is_none()
        && has.is_none()
    {
        return None;
    }

    Some(crate::api::models::SearchRequest {
        text: (!text.trim().is_empty()).then(|| text.trim().to_owned()),
        author,
        channel_id,
        mention,
        has,
        order: Some(if oldest_first { "asc" } else { "desc" }.to_owned()),
        date_start: None,
        date_end: None,
        contains_attachment,
    })
}

/// Byte range of the token at the caret, including a quoted value's suffix.
pub fn search_token_range(query: &str, caret: usize) -> (usize, usize) {
    let caret = caret.min(query.len());
    let (_, start) = search_fragment(&query[..caret]);
    let mut quoted = false;
    let mut escaped = false;
    for (i, ch) in query[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            return (start, start + i);
        }
    }
    (start, query.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{models, net::Update};
    use serde_json::json;
    fn store() -> Store {
        let mut s = Store::default();
        s.apply(Update::Users(
            serde_json::from_value(json!([
                {"id":"u1","username":"ana","nickname":"Ana"},
                {"id":"u2","username":"other","nickname":"Ana"}
            ]))
            .unwrap(),
        ));
        s.apply(Update::Channels(
            serde_json::from_value::<Vec<models::Channel>>(json!([
                {"id":"c1","name":"🧪 Design room","type":"text"},
                {"id":"dm","name":"Private","type":"direct"}
            ]))
            .unwrap(),
        ));
        s
    }
    #[test]
    fn quoted_channels_and_stacked_aliases_resolve_stable_ids() {
        let s = store();
        let r = search_request(
            "hello from:@ana in:\"#🧪 Design room\" has:link tem:arquivo mentions:@other",
            &s,
            false,
            false,
            None,
        )
        .unwrap();
        assert_eq!(r.text.as_deref(), Some("hello"));
        assert_eq!(r.author.as_deref(), Some("u1"));
        assert_eq!(r.channel_id.as_deref(), Some("c1"));
        assert_eq!(r.mention.as_deref(), Some("u2"));
        assert_eq!(r.has.as_deref(), Some("link"));
        assert_eq!(r.contains_attachment, Some(true));
        assert!(search_request("de:@missing", &s, false, false, None).is_none());
        assert!(search_request("em:#missing", &s, false, false, None).is_none());
    }
    #[test]
    fn shortcuts_stack_without_repeating_single_value_filters() {
        let s = store();
        assert_eq!(replace_search_fragment("hello d", "de:"), "hello de:");
        assert_eq!(
            replace_search_fragment("hello de:", "de:@ana"),
            "hello de:@ana "
        );
        assert!(
            !search_shortcuts(&s, "de:@ana d", false)
                .iter()
                .any(|h| h.replacement == "de:")
        );
        let hints = search_shortcuts(&s, "tem:link tem:", false);
        assert!(hints.iter().any(|h| h.replacement == "tem:anexo"));
        assert!(!hints.iter().any(|h| h.replacement == "tem:link"));
        assert!(search_shortcuts(&s, "tem:link tem:anexo t", false).is_empty());
    }
    #[test]
    fn quoted_completion_and_cursor_ranges_preserve_surrounding_text() {
        let s = store();
        let hints = search_shortcuts(&s, "hi em:\"#🧪 Des", false);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].replacement, "em:\"#🧪 Design room\"");
        let query = "🙂 em:\"#🧪 Design room\" tem:link";
        let caret = "🙂 em:\"#🧪 Des".len();
        let (start, end) = search_token_range(query, caret);
        assert_eq!(&query[start..end], "em:\"#🧪 Design room\"");
        assert_eq!(&query[end..], " tem:link");
    }
    #[test]
    fn global_search_does_not_inherit_the_selected_direct_message() {
        let mut s = store();
        s.selected_channel = "dm".into();
        assert!(
            search_request("hello", &s, false, false, None)
                .unwrap()
                .channel_id
                .is_none()
        );
        assert_eq!(
            search_request("hello", &s, false, true, Some("dm"))
                .unwrap()
                .channel_id
                .as_deref(),
            Some("dm")
        );
        assert!(search_request(" ", &s, false, false, None).is_none());
    }
}

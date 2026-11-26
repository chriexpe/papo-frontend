//! A reply's original in words (`docs/design.md` §11, the reply line's
//! rules), for the reply line and the reply bar alike: who wrote it and its
//! snippet, or that it was deleted, or that it is not available. What to
//! show is the SDK's (`ReplyOriginal`, `snippet`); this names people and
//! kinds in the user's language.

use papo_sdk::{MentionTarget, Message, ResourceSnapshot, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{ReplyOriginal, Snippet, SnippetPart, display_name, snippet};

use super::OpenTimeline;
use super::message_row::author_name;
use crate::strings;

/// What stands for a hidden spoiler in a one-line snippet: its text never
/// shows there.
const HIDDEN_SPOILER: &str = "▮▮▮";

/// A reply's original, in words.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OriginalText {
    /// A message the conversation can show.
    Message {
        /// Its backend ID, or an echo's local identity.
        id: Uuid,
        /// Its author's user ID, when it has one.
        author: Option<Uuid>,
        /// Its author's name.
        name: String,
        /// The message in one line.
        snippet: String,
        /// Its custom emoji: the bytes of `snippet` each one's `:name:` takes.
        emoji: Vec<(std::ops::Range<usize>, Uuid)>,
    },
    /// History knows it was deleted.
    Deleted,
    /// Not available, and not known deleted.
    Earlier,
}

impl OriginalText {
    /// Whether `user` wrote it.
    pub(crate) fn by(&self, user: Uuid) -> bool {
        matches!(self, OriginalText::Message { author: Some(author), .. } if *author == user)
    }
}

/// `original` in words; an echo's original is found among `open`'s echoes.
pub(crate) fn original_text(original: &ReplyOriginal, open: &OpenTimeline) -> OriginalText {
    let resources = open.resources.as_deref();
    match original {
        ReplyOriginal::Held {
            id,
            message,
            content,
        } => message_text(*id, message, content, resources),
        ReplyOriginal::Local { local } => match open.echoes.get(*local) {
            Some(echo) => message_text(*local, &echo.message, &echo.content, resources),
            None => OriginalText::Earlier,
        },
        ReplyOriginal::Deleted => OriginalText::Deleted,
        _ => OriginalText::Earlier,
    }
}

fn message_text(
    id: Uuid,
    message: &Message,
    content: &MessageContent,
    resources: Option<&ResourceSnapshot>,
) -> OriginalText {
    let line = snippet_line(&snippet(message, content), resources);
    OriginalText::Message {
        id,
        author: message
            .author_id
            .as_deref()
            .and_then(|author| Uuid::parse_str(author).ok())
            .filter(|_| message.webhook_id.is_none() && message.external_author.is_none()),
        name: author_name(message, resources),
        snippet: line.text,
        emoji: line.emoji,
    }
}

/// A snippet in one line: its text and where its custom emoji are in it.
pub(crate) struct SnippetLine {
    pub(crate) text: String,
    pub(crate) emoji: Vec<(std::ops::Range<usize>, Uuid)>,
}

/// A message's snippet in one line: mentions by name, custom emoji by their
/// `:name:` (drawn as their images over it), a hidden spoiler as blocks,
/// media and files by their kind.
pub(crate) fn snippet_line(snippet: &Snippet, resources: Option<&ResourceSnapshot>) -> SnippetLine {
    let Snippet::Text(parts) = snippet else {
        return SnippetLine {
            text: snippet_text(snippet, resources),
            emoji: Vec::new(),
        };
    };
    let mut line = SnippetLine {
        text: String::new(),
        emoji: Vec::new(),
    };
    for part in parts {
        let start = line.text.len();
        line.text.push_str(&part_text(part, resources));
        if let SnippetPart::CustomEmoji { id, .. } = part {
            line.emoji.push((start..line.text.len(), *id));
        }
    }
    line
}

/// A message's snippet in one line of text: mentions by name, a hidden
/// spoiler as blocks, media and files by their kind.
fn snippet_text(snippet: &Snippet, resources: Option<&ResourceSnapshot>) -> String {
    match snippet {
        Snippet::Text(parts) => parts
            .iter()
            .map(|part| part_text(part, resources))
            .collect(),
        Snippet::Gif => strings::reply_gif().to_owned(),
        Snippet::Image => strings::reply_image().to_owned(),
        Snippet::Video => strings::reply_video().to_owned(),
        Snippet::Audio => strings::reply_audio().to_owned(),
        Snippet::File { name } => strings::reply_file(name),
        Snippet::Link { title, domain } => match (title, domain) {
            (Some(title), Some(domain)) => format!("{title} · {domain}"),
            (Some(only), None) | (None, Some(only)) => only.clone(),
            (None, None) => String::new(),
        },
        _ => String::new(),
    }
}

fn part_text(part: &SnippetPart, resources: Option<&ResourceSnapshot>) -> String {
    match part {
        SnippetPart::Text(text) => text.clone(),
        SnippetPart::Mention(MentionTarget::User(id)) => {
            let name = resources
                .and_then(|resources| resources.users.get(*id))
                .and_then(|user| display_name(user.nickname.as_deref(), user.username.as_deref()))
                .unwrap_or_else(|| strings::someone());
            format!("@{name}")
        }
        SnippetPart::Mention(MentionTarget::Everyone) => "@everyone".to_owned(),
        SnippetPart::CustomEmoji { raw, .. } => raw.clone(),
        SnippetPart::Spoiler => HIDDEN_SPOILER.to_owned(),
        _ => String::new(),
    }
}

/// The original of a reply being written, in words: from `open`'s row for
/// message `id` when the conversation shows it, else from what the SDK said
/// (`known`), else none while that is looked up.
pub(crate) fn reply_text(
    open: Option<&OpenTimeline>,
    id: Uuid,
    known: Option<&ReplyOriginal>,
) -> Option<OriginalText> {
    if let Some(open) = open {
        let shown = open.items.iter().find_map(|item| match item.as_ref() {
            papo_sdk_ui::TimelineItem::Message {
                id: current,
                message,
                content,
                ..
            } if *current == id => Some((message, content)),
            _ => None,
        });
        if let Some((message, content)) = shown {
            return Some(message_text(
                id,
                message,
                content,
                open.resources.as_deref(),
            ));
        }
        if let Some(known) = known {
            return Some(original_text(known, open));
        }
    }
    match known {
        Some(ReplyOriginal::Deleted) => Some(OriginalText::Deleted),
        Some(_) => Some(OriginalText::Earlier),
        None => None,
    }
}

impl OpenTimeline {
    /// The name of message `id`'s author; "Someone" when the conversation
    /// does not show it.
    pub(crate) fn author_of(&self, id: Uuid) -> String {
        self.items
            .iter()
            .find_map(|item| match item.as_ref() {
                papo_sdk_ui::TimelineItem::Message {
                    id: current,
                    message,
                    ..
                } if *current == id => Some(author_name(message, self.resources.as_deref())),
                _ => None,
            })
            .unwrap_or_else(|| strings::someone().to_owned())
    }
}

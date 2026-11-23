//! A search's results as a client lists them: the pages loaded so far, in the backend's order
//! (newest first unless the query asks otherwise), each hit drawn as a message from what it carries (author, time, channel,
//! text; a hit has no attachments, embeds or reactions), the next page's cursor, and the count
//! line's number, exact only once the last page is in. Requests and their cancellation are
//! `SearchService`'s; the panel, paging trigger and opening a hit are the client's.

use std::collections::HashSet;
use std::sync::Arc;

use papo_sdk::{Message, MessageKey, SearchCursor, SearchHit, SearchPage, Uuid};

use super::local_days::SearchWindow;
use crate::message_content::MessageContent;

/// One result.
#[derive(Clone)]
pub struct SearchResultItem {
    /// The hit, for `SearchService::target` when it is opened.
    pub hit: SearchHit,
    /// The message as far as the hit carries it: id, channel, author, text, time, a webhook's
    /// display identity. Everything else is absent, not empty.
    pub message: Message,
    /// The text, parsed.
    pub content: Arc<MessageContent>,
    /// Its channel.
    pub channel: Uuid,
    /// The channel's name as the backend sent it.
    pub channel_name: Option<String>,
    /// The author's username as the backend sent it, for an author the roster no longer holds.
    pub author_username: Option<String>,
    /// Its place in history.
    pub key: MessageKey,
}

impl SearchResultItem {
    /// The hit as a result; none when its identity is malformed.
    #[must_use]
    pub fn of(hit: &SearchHit) -> Option<Self> {
        let result = &hit.result;
        let id = result.id.as_deref()?;
        let created_at = result.created_at.as_deref()?;
        let key = MessageKey::parse(created_at, id).ok()?;
        let channel = Uuid::parse_str(result.channel_id.as_deref()?).ok()?;
        let message = Message {
            id: result.id.clone(),
            channel_id: result.channel_id.clone(),
            author_id: result.author_id.clone(),
            content: result.content.clone(),
            created_at: result.created_at.clone(),
            webhook_id: result.webhook_id.clone(),
            external_author: result.external_author.clone(),
            ..Message::default()
        };
        Some(Self {
            hit: hit.clone(),
            content: Arc::new(MessageContent::parse(
                result.content.as_deref().unwrap_or_default(),
            )),
            message,
            channel,
            channel_name: result.channel_name.clone(),
            author_username: result.author_username.clone(),
            key,
        })
    }

    /// The message's id.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.key.message_id
    }
}

/// How many results there are, as far as is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchCount {
    /// Every page is in: exactly this many.
    Exact(usize),
    /// More pages exist: at least this many (the ones loaded).
    AtLeast(usize),
}

/// The results loaded for one query.
#[derive(Clone, Default)]
pub struct SearchResults {
    /// The instants kept: local date filters' exact days.
    window: SearchWindow,
    items: Vec<Arc<SearchResultItem>>,
    /// The listed messages' ids.
    listed: HashSet<Uuid>,
    next: Option<SearchCursor>,
    complete: bool,
}

impl SearchResults {
    /// The results of a query's first page.
    #[must_use]
    pub fn first(page: &SearchPage) -> Self {
        Self::first_within(page, SearchWindow::default())
    }

    /// The results of a query's first page, kept to `window` (from
    /// [`crate::localize_search`]).
    #[must_use]
    pub fn first_within(page: &SearchPage, window: SearchWindow) -> Self {
        let mut results = Self {
            window,
            ..Self::default()
        };
        results.append(page);
        results
    }

    /// Adds the next page. A hit already listed (the backend's pages overlap only on a bug),
    /// malformed or outside the window is left out.
    pub fn append(&mut self, page: &SearchPage) {
        for hit in &page.hits {
            let Some(item) = SearchResultItem::of(hit) else {
                continue;
            };
            if !self.window.holds(item.key.created_at_us) {
                continue;
            }
            if !self.listed.insert(item.id()) {
                continue;
            }
            self.items.push(Arc::new(item));
        }
        self.next.clone_from(&page.next);
        self.complete = page.next.is_none();
    }

    /// The results, in the backend's order.
    #[must_use]
    pub fn items(&self) -> &[Arc<SearchResultItem>] {
        &self.items
    }

    /// The cursor of the next page; none once the last is in.
    #[must_use]
    pub fn next(&self) -> Option<&SearchCursor> {
        self.next.as_ref()
    }

    /// Whether the pages so far kept nothing while more exist (the window
    /// trimmed them all): the next page is wanted at once, as no row can
    /// come near to ask for it.
    #[must_use]
    pub fn wants_next(&self) -> bool {
        self.items.is_empty() && self.next.is_some()
    }

    /// The count line's number.
    #[must_use]
    pub fn count(&self) -> SearchCount {
        if self.complete {
            SearchCount::Exact(self.items.len())
        } else {
            SearchCount::AtLeast(self.items.len())
        }
    }
}

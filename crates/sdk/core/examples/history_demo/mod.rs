//! Owns bounded example paging and connection observations; leaves fixture mutations and SDK policy outside.

use papo_api::{HistoryCursor, HistoryOrder, HistoryQuery};
use papo_sdk::{ConnectionState, ConnectionStatus, HistoryHandle, HistorySource};
use std::{collections::BTreeSet, error::Error, time::Duration};
use tokio::sync::watch;

pub(super) type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

pub(super) struct Traversal {
    pub ids: Vec<String>,
    pub server_pages: usize,
    pub cache_pages: usize,
}

pub(super) async fn traverse(
    history: &HistoryHandle,
    order: HistoryOrder,
    offline: bool,
) -> Result<Traversal> {
    let mut cursor: Option<HistoryCursor> = None;
    let mut seen = BTreeSet::new();
    let mut result = Traversal {
        ids: Vec::new(),
        server_pages: 0,
        cache_pages: 0,
    };
    loop {
        let page = history
            .page(HistoryQuery {
                cursor: cursor.as_ref(),
                order,
                limit: Some(100),
            })
            .await?;
        if offline && page.source != HistorySource::Cache {
            return Err("offline page used server".into());
        }
        match page.source {
            HistorySource::Cache => result.cache_pages += 1,
            HistorySource::Server => result.server_pages += 1,
        }
        if let Some(gap) = &page.gap
            && (offline
                || !page.has_more
                || gap.order != order
                || page.next_cursor.as_ref() != Some(&gap.cursor))
        {
            return Err("unexpected history gap".into());
        }
        let more = page.has_more;
        let next = page.next_cursor.clone();
        for message in page.messages {
            let id = message.id.as_ref().ok_or("missing message identity")?;
            if !seen.insert(id.clone()) {
                return Err("duplicate history identity".into());
            }
            result.ids.push(id.clone());
        }
        if !more {
            return Ok(result);
        }
        if next.is_none() || next == cursor {
            return Err("history cursor made no progress".into());
        }
        cursor = next;
        if result.ids.len() > 100_000 {
            return Err("example history budget exceeded".into());
        }
    }
}

pub(super) async fn wait_for(
    status: &mut watch::Receiver<ConnectionStatus>,
    predicate: impl Fn(ConnectionState) -> bool,
) -> Result<ConnectionStatus> {
    tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            let current = *status.borrow_and_update();
            if predicate(current.state) {
                return Ok::<_, Box<dyn Error>>(current);
            }
            status.changed().await?;
        }
    })
    .await?
}

pub(super) fn demonstration_password() -> String {
    std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + index)))
        .chain(std::iter::once('!'))
        .collect()
}

//! Catch-up's questions to history, answered only through `HistorySnapshot::coverage()`:
//! point resolution, range completeness, and the first unseen message from someone else.
//! Interval rules live in `catch_up`; span extraction and coalescing in `catch_up_spans`.

use papo_api::{HistoryCursor, Uuid};

use crate::{
    HistoryCoverage, HistorySnapshot, MessageKey, ReadingError, authored_by_other,
    catch_up::ReadSpan, catch_up_spans::span_contains,
};

/// One snapshot as seen by one account's catch-up rules.
pub(crate) struct CatchUpHistory<'a> {
    pub snapshot: &'a HistorySnapshot,
    coverage: HistoryCoverage<'a>,
    account_user: Uuid,
}

impl<'a> CatchUpHistory<'a> {
    pub fn new(snapshot: &'a HistorySnapshot, account_user: Uuid) -> Result<Self, ReadingError> {
        Ok(Self {
            snapshot,
            coverage: snapshot.coverage()?,
            account_user,
        })
    }

    pub fn resolve(&self, point: MessageKey) -> Result<MessageKey, ReadingError> {
        Ok(self.coverage.resolve(point)?.key)
    }

    pub fn index_of(&self, point: MessageKey) -> Result<Option<usize>, ReadingError> {
        Ok(self.coverage.resolve(point)?.index)
    }

    /// True when any held message sits strictly in `(a, b)`.
    pub fn has_entry_between(&self, a: MessageKey, b: MessageKey) -> Result<bool, ReadingError> {
        Ok(!self.coverage.range(Some(a), Some(b))?.messages.is_empty())
    }

    /// True when history proves that no unknown message lies strictly in `(a, b)`.
    pub fn is_complete(&self, a: Option<MessageKey>, b: MessageKey) -> bool {
        self.coverage.is_complete(a, Some(b))
    }

    /// The first held message from someone else strictly in `(a, b)` outside `covered_spans`.
    pub fn first_unseen_foreign(
        &self,
        a: Option<MessageKey>,
        b: MessageKey,
        covered_spans: &[ReadSpan],
    ) -> Result<Option<(MessageKey, Option<HistoryCursor>)>, ReadingError> {
        debug_assert!(covered_spans.windows(2).all(|w| w[0].end < w[1].start));
        let range = self.coverage.range(a, Some(b))?.messages;
        let mut index = range.start;
        while index < range.end {
            let message = &self.snapshot.messages[index];
            let point = MessageKey::from_message(message)?;
            let span_index = covered_spans.partition_point(|span| span.end < point);
            if let Some(&span) = covered_spans.get(span_index)
                && span_contains(span, point)
            {
                let after_span = self.coverage.range(Some(span.end), None)?.messages.start;
                index = after_span.max(index + 1);
                continue;
            }
            if authored_by_other(message, self.account_user) {
                return Ok(Some((point, message.cursor())));
            }
            index += 1;
        }
        Ok(None)
    }

    /// True when `(a, b)` is complete and holds no message from someone else outside
    /// `covered_spans`. Completeness is knowledge only; exposure comes from the spans.
    pub fn no_unseen_foreign(
        &self,
        a: Option<MessageKey>,
        b: MessageKey,
        covered_spans: &[ReadSpan],
    ) -> Result<bool, ReadingError> {
        if a.is_some_and(|a| a >= b) {
            return Ok(true);
        }
        if !self.is_complete(a, b) {
            return Ok(false);
        }
        Ok(self.first_unseen_foreign(a, b, covered_spans)?.is_none())
    }
}

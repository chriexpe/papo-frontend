//! The panel's search (`docs/design.md` §8): every row of every page whose
//! title, summary or value holds what was typed, ignoring case and
//! accents ("aparencia" finds "Aparência"), grouped by the page it lives
//! on.
//!
//! Owns only the matching; the rows are `rows`', the results' look is
//! `view`'s.

use super::PageId;
use super::rows::{self, Context, Group, Row};

/// The rows of `page` that match `query`, as one group; none when nothing
/// does.
fn matches(page: PageId, query: &str, cx: &Context<'_>) -> Vec<Row> {
    let wanted = fold(query);
    rows::page(page, cx)
        .into_iter()
        .flat_map(|group: Group| group.rows)
        .filter(|row| {
            rows::searchable(row)
                .iter()
                .any(|words| fold(words).contains(&wanted))
        })
        .collect()
}

/// Every page's matching rows, page by page, the pages with none left out.
pub(super) fn results(query: &str, cx: &Context<'_>) -> Vec<(PageId, Vec<Row>)> {
    PageId::ALL
        .into_iter()
        .map(|page| (page, matches(page, query, cx)))
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// `words` lower-cased with the accents of Latin letters taken off, so a
/// search needs neither.
pub(super) fn fold(words: &str) -> String {
    words
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

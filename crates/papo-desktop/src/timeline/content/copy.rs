//! The Text copy (`docs/design.md` §9, Selection and copy): a selection in
//! the timeline copied as the text it shows, without Markdown syntax. A
//! hidden spoiler copies as `||text||`, so it stays hidden when pasted into
//! Papo; a list item takes its drawn marker; table cells side by side take a
//! tab. A selection across messages puts "{author} — {time}" before each
//! message's text, with a blank line between messages.
//!
//! The copy lays each message out again ([`Doc`]) from the items the view
//! drew, and finds the selected texts by the keys the view gave them.

use std::ops::Range;
use std::sync::Arc;

use imbl::{HashMap, Vector};
use papo_sdk::{ResourceSnapshot, Uuid};
use papo_sdk_ui::TimelineItem;
use rune::TextSelection;

use super::doc::{Joint, Mark, Run};
use super::{Context, Doc};
use crate::strings;
use crate::timeline::local_time::LocalTime;
use crate::timeline::message_row::{author_name, created_us};

/// What a copy reads: the timeline as the view drew it.
pub(crate) struct Source {
    pub(crate) items: Vector<Arc<TimelineItem>>,
    pub(crate) resources: Option<Arc<ResourceSnapshot>>,
    /// The spoilers revealed in each message.
    pub(crate) revealed: HashMap<Uuid, Vec<u32>>,
    /// Whether the view draws one to three emoji large.
    pub(crate) jumbo: bool,
    pub(crate) time: LocalTime,
    pub(crate) today: i64,
}

/// One message's selected texts: (text index, selected bytes).
type Selected = (Uuid, Vec<(usize, Range<usize>)>);

/// The text `selection` copies as; none when it holds no message text.
pub(crate) fn text_of(source: &Source, selection: &TextSelection) -> Option<String> {
    let mut messages: Vec<Selected> = Vec::new();
    let mut texts = 0;
    for selected in selection.texts().iter().filter(|text| text.is_selected()) {
        let Some((id, index)) = locate(selected.path()) else {
            continue;
        };
        texts += 1;
        match messages.last_mut() {
            Some((last, runs)) if *last == id => runs.push((index, selected.range())),
            _ => messages.push((id, vec![(index, selected.range())])),
        }
    }
    if messages.is_empty() {
        return None;
    }
    let headed = messages.len() > 1;
    let pieces: Vec<String> = messages
        .iter()
        .filter_map(|(id, selected)| {
            let (message, content) = find(&source.items, *id)?;
            let revealed = source.revealed.get(id).map_or(&[][..], Vec::as_slice);
            let doc = Doc::new(
                content,
                &Context {
                    resources: source.resources.as_deref(),
                    revealed,
                    jumbo: source.jumbo,
                },
            );
            let text = message_text(&doc.runs(), selected, texts > 1);
            if !headed {
                return Some(text);
            }
            let author = author_name(message, source.resources.as_deref());
            let time = created_us(message)
                .map(|at_us| source.time.moment_label(at_us, source.today))
                .unwrap_or_default();
            Some(format!("{}\n{text}", strings::copy_header(&author, &time)))
        })
        .collect();
    Some(pieces.join("\n\n"))
}

/// One message's selected texts joined: a line between texts, a tab
/// between cells side by side. A text taken from its start along with
/// others gets its prefix (a list item's marker).
fn message_text(runs: &[&Run], selected: &[(usize, Range<usize>)], several: bool) -> String {
    let mut text = String::new();
    for (n, (index, range)) in selected.iter().enumerate() {
        let Some(run) = runs.get(*index) else {
            continue;
        };
        if n > 0 {
            text.push(match run.joint {
                Joint::Line => '\n',
                Joint::Tab => '\t',
            });
        }
        if several
            && range.start == 0
            && let Some(prefix) = &run.prefix
        {
            text.push_str(prefix);
        }
        text.push_str(&run_text(run, range.clone()));
    }
    text
}

/// Bytes `range` of `run` as a copy takes them: without chips' padding,
/// past the end of its own text (the "(edited)" marker) nothing, and each
/// hidden spoiler it touches whole, as `||text||`.
pub(super) fn run_text(run: &Run, range: Range<usize>) -> String {
    let end = range.end.min(run.text.len());
    let start = range.start.min(end);
    join(run, start..end, |words| format!("||{words}||"))
}

/// What assistive technology reads for `run`, when that differs from its
/// words: without chips' padding, and each hidden spoiler as "Hidden
/// spoiler. Press to reveal." rather than its words. None reads the words
/// as they are.
pub(super) fn spoken(run: &Run) -> Option<String> {
    let padded = run
        .marks
        .iter()
        .any(|mark| matches!(mark, Mark::Pad { .. }));
    padded.then(|| {
        join(run, 0..run.text.len(), |_| {
            strings::spoiler_hidden().to_owned()
        })
    })
}

/// Bytes `range` of `run` without chips' padding, each hidden spoiler it
/// touches taken whole and written by `hidden` from its words.
fn join(run: &Run, range: Range<usize>, hidden: impl Fn(&str) -> String) -> String {
    let mut spoilers: Vec<Range<usize>> = run
        .hidden()
        .filter(|spoiler| spoiler.start < range.end && range.start < spoiler.end)
        .collect();
    spoilers.sort_by_key(|spoiler| spoiler.start);
    let mut text = String::new();
    let mut at = range.start;
    for spoiler in spoilers {
        // A spoiler inside one already taken.
        if spoiler.end <= at {
            continue;
        }
        text.push_str(&unpadded(run, at..spoiler.start.max(at)));
        text.push_str(&hidden(&unpadded(run, spoiler.clone())));
        at = spoiler.end;
    }
    text.push_str(&unpadded(run, at..range.end.max(at)));
    text
}

/// Bytes `range` of `run` less its chips' padding.
fn unpadded(run: &Run, range: Range<usize>) -> String {
    let mut text = String::new();
    let mut at = range.start;
    let mut pads: Vec<&Range<usize>> = run
        .marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::Pad { range: pad, .. } if pad.start < range.end && range.start < pad.end => {
                Some(pad)
            }
            _ => None,
        })
        .collect();
    pads.sort_by_key(|pad| pad.start);
    for pad in pads {
        text.push_str(run.text.get(at..pad.start.max(at)).unwrap_or_default());
        at = at.max(pad.end);
    }
    text.push_str(run.text.get(at..range.end.max(at)).unwrap_or_default());
    text
}

/// The message and text index a selected text's path names: its row key
/// `m{id}` and its own key `t{index}`.
fn locate(path: &str) -> Option<(Uuid, usize)> {
    let id = message_of(path)?;
    let index = path.rsplit('/').next()?.strip_prefix('t')?.parse().ok()?;
    Some((id, index))
}

/// The message whose row an element's path runs through: its row key
/// `m{id}`.
pub(crate) fn message_of(path: &str) -> Option<Uuid> {
    path.split('/').find_map(|segment| {
        let id = segment.strip_prefix('m')?;
        Uuid::parse_str(id).ok()
    })
}

fn find(
    items: &Vector<Arc<TimelineItem>>,
    id: Uuid,
) -> Option<(
    &papo_sdk::Message,
    &papo_sdk_ui::message_content::MessageContent,
)> {
    items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id: found,
            message,
            content,
        } if *found == id => Some((message.as_ref(), content.as_ref())),
        _ => None,
    })
}

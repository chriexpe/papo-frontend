//! The @ picker (`docs/design.md` §3, "Composer surfaces"): it grows upward
//! out of the text pill, as wide as it, up to six rows and then it scrolls.
//! Each row: avatar, name, @username; @everyone first where allowed. ↑ and
//! ↓ move, Enter or Tab picks, Escape closes; the keyboard stays in the
//! field throughout.
//!
//! Its state is the query being typed and the people it found; the shell
//! finds them in the account's roster. The view is pure.

use rune::{Align, Cx, El, ScrollSpec, Side, col, row, text};

use super::mentions::{Candidate, Query, Target};
use crate::channel_pane::avatar;
use crate::design::{
    MENU_ROW_HEIGHT_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT, RADIUS_PILL_PT, SPACE_LG_PT,
    SPACE_SM_PT, TYPE_BODY, TYPE_SMALL,
};
use crate::shell::Shell;
use crate::strings;
use crate::timeline::TimelineKey;

/// The element key of the open picker.
pub(crate) const PICKER_KEY: &str = "mention-picker";

/// The rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 6.0;

/// The picker over one field's @ query.
#[derive(Debug, PartialEq)]
pub(crate) struct Picker {
    pub(super) key: TimelineKey,
    pub(super) query: Query,
    pub(super) candidates: Vec<Candidate>,
    /// The highlighted row: what Enter or Tab picks.
    pub(super) selected: usize,
}

impl Picker {
    /// Moves the highlight `by` rows, wrapping round the ends.
    pub(super) fn step(&mut self, by: isize) {
        let count = self.candidates.len();
        if count == 0 {
            return;
        }
        let count = isize::try_from(count).unwrap_or(isize::MAX);
        let at = isize::try_from(self.selected).unwrap_or(0);
        self.selected = usize::try_from((at + by).rem_euclid(count)).unwrap_or(0);
    }

    /// The highlighted row's key.
    pub(crate) fn chosen_key(&self) -> Option<String> {
        self.chosen().map(|candidate| row_key(candidate.target))
    }

    /// Who the highlighted row mentions.
    pub(crate) fn chosen_target(&self) -> Option<Target> {
        self.chosen().map(|candidate| candidate.target)
    }

    pub(super) fn chosen(&self) -> Option<&Candidate> {
        self.candidates.get(self.selected)
    }
}

/// The element key of the row of `target`, by who it mentions, so a row
/// keeps its identity as the query narrows. The field names the highlighted
/// one as its active descendant, since the keyboard stays in the field.
pub(crate) fn row_key(target: Target) -> String {
    match target {
        Target::Everyone => "mention-everyone".to_owned(),
        Target::User(id) => format!("mention-{id}"),
    }
}

/// Draws `picker` above the pill keyed `pill_key`, as wide as it.
pub(crate) fn view(picker: &Picker, pill_key: &str, palette: &Palette, cx: &Cx<Shell>) -> El {
    let width = cx.layout_size(pill_key).map(|[width, _]| width);
    let rows = picker
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| row_for(candidate, index == picker.selected, palette));
    let list = col()
        .key("mention-rows")
        .w_full()
        .max_h(MENU_ROW_HEIGHT_PT * VISIBLE_ROWS)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(rows);
    let mut panel = col()
        .key(PICKER_KEY)
        .padding(SPACE_SM_PT)
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role("listbox")
        .label(strings::mention_picker())
        .anchored(pill_key, Side::Top, Align::Start, SPACE_SM_PT)
        .child(list);
    if let Some(width) = width {
        panel = panel.w(width);
    }
    panel
}

fn row_for(candidate: &Candidate, selected: bool, palette: &Palette) -> El {
    let target = candidate.target;
    let label = match &candidate.username {
        Some(username) if *username != candidate.name => {
            format!("{} @{username}", candidate.name)
        }
        _ => candidate.name.clone(),
    };
    let handle = match (&candidate.target, &candidate.username) {
        // A person with no nickname shows their username once, as the name.
        (Target::User(_), Some(username)) if *username == candidate.name => None,
        (Target::User(_), Some(username)) => Some(format!("@{username}")),
        (Target::Everyone, _) => Some(strings::mention_everyone_hint().to_owned()),
        (Target::User(_), None) => None,
    };
    let mut line = row()
        .key(row_key(target))
        .w_full()
        .h(MENU_ROW_HEIGHT_PT)
        .px(SPACE_SM_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .role("option")
        .label(label)
        .on_press_tx(move |shell: &mut Shell, tx| shell.mention_picked(target, tx))
        .child(avatar(&candidate.name, None, palette))
        .child(
            text(candidate.name.clone())
                .font(TYPE_BODY)
                .color(palette.text_primary)
                .truncate(),
        )
        .children(handle.map(|handle| {
            text(handle)
                .font(TYPE_SMALL)
                .color(palette.text_muted)
                .truncate()
        }))
        .child(row().grow(1.0))
        .pr(SPACE_LG_PT);
    // `surface.hover` is too close to the pill's own surface to show.
    if selected {
        line = line.fill(palette.surface_selected);
    }
    line
}

#[cfg(test)]
mod tests {
    use papo_sdk::Uuid;

    use super::{Candidate, Picker, Query, Target};
    use crate::timeline::TimelineKey;

    fn picker(rows: usize) -> Picker {
        Picker {
            key: TimelineKey {
                account: papo_sdk::AccountId::new(
                    papo_sdk::ServerAddress::parse("https://papo.example.invalid")
                        .unwrap()
                        .id(),
                    Uuid::nil(),
                ),
                channel: Uuid::from_u128(1),
            },
            query: Query {
                range: 0..1,
                text: String::new(),
            },
            candidates: (0..rows)
                .map(|n| Candidate {
                    target: Target::User(Uuid::from_u128(n as u128)),
                    name: format!("p{n}"),
                    username: None,
                })
                .collect(),
            selected: 0,
        }
    }

    #[test]
    fn the_arrows_wrap_round_the_ends() {
        let mut picker = picker(3);
        picker.step(-1);
        assert_eq!(picker.selected, 2);
        picker.step(1);
        assert_eq!(picker.selected, 0);
    }
}

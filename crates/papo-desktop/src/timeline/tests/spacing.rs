//! The spacing model of `docs/design.md` §5 at every density: distances
//! between groups and within one, and one content column per message.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk_ui::TimelineItem;

use super::{DENSITIES, assert_near, bottom, has_part, message, part, runtime};
use crate::design::{AVATAR_TOP_PT, Density, Lead, MESSAGE_BLOCK_GAP_PT, MessageLayout};

/// Ana twice in a minute, Bia, then Bia again half an hour later.
fn conversation() -> Vector<Arc<TimelineItem>> {
    Vector::from(vec![
        message(1, "ana", 0, "A little more like you."),
        message(2, "ana", 60, "Messages stay together."),
        message(3, "bia", 120, "oi"),
        message(4, "bia", 1920, "still here"),
    ])
}

fn layouts() -> Vec<MessageLayout> {
    let mut layouts = Vec::new();
    for density in DENSITIES {
        for grouped in [true, false] {
            for stacked in [true, false] {
                if density != Density::ExtraCompact && !stacked {
                    continue;
                }
                layouts.push(MessageLayout {
                    density,
                    grouped,
                    stacked,
                });
            }
        }
    }
    layouts
}

#[test]
fn rows_sit_at_the_spacing_of_their_density() {
    for layout in layouts() {
        let rt = runtime(conversation(), layout);
        let spacing = layout.density.spacing();
        let continued = if layout.grouped {
            spacing.within_group_pt
        } else {
            spacing.between_groups_pt
        };
        let rows: Vec<_> = (1..=4).map(|n| part(&rt, n, "message")).collect();
        let what = format!("{layout:?}");
        assert_near(rows[1][1] - bottom(rows[0]), continued, &what);
        assert_near(
            rows[2][1] - bottom(rows[1]),
            spacing.between_groups_pt,
            &what,
        );
        assert_near(
            rows[3][1] - bottom(rows[2]),
            spacing.between_groups_pt,
            &what,
        );
    }
}

#[test]
fn every_block_starts_on_its_messages_content_column() {
    for layout in layouts() {
        let rt = runtime(conversation(), layout);
        let what = format!("{layout:?}");
        let column = part(&rt, 1, "content")[0];
        for n in 1..=4 {
            let content = part(&rt, n, "content");
            assert_near(content[0], column, &what);
            let inline = layout.density == Density::ExtraCompact && !layout.stacked;
            if inline {
                // The name leads the line; the text keeps one column, a
                // hidden name's width reserved.
                assert_near(part(&rt, n, "body")[0], part(&rt, 1, "body")[0], &what);
                continue;
            }
            assert_near(part(&rt, n, "body")[0], column, &what);
            for header in ["header", "name"] {
                if has_part(&rt, n, header) {
                    let header = part(&rt, n, header);
                    assert_near(header[0], column, &what);
                    let gap = part(&rt, n, "body")[1] - bottom(header);
                    assert_near(gap, MESSAGE_BLOCK_GAP_PT, &what);
                }
            }
        }
    }
}

#[test]
fn an_avatar_sits_below_the_row_padding_and_adds_no_height() {
    for density in [Density::Compact, Density::Normal, Density::Comfortable] {
        let layout = MessageLayout {
            density,
            ..MessageLayout::default()
        };
        let rt = runtime(conversation(), layout);
        let spacing = density.spacing();
        let Lead::Avatar(size_pt) = spacing.lead else {
            panic!("{density:?} leads with an avatar");
        };
        let what = format!("{density:?}");
        // Bia's "oi": a header and one line, the shortest row.
        let row = part(&rt, 3, "message");
        let avatar = part(&rt, 3, "avatar");
        assert_near(
            avatar[1] - row[1] - spacing.row_padding_pt,
            AVATAR_TOP_PT,
            &what,
        );
        let content = part(&rt, 3, "content");
        assert_near(
            row[3],
            2.0 * spacing.row_padding_pt + content[3].max(size_pt),
            &format!("{what}: the row's height"),
        );
        assert!(
            bottom(avatar) <= bottom(row) - spacing.row_padding_pt + 1.0,
            "{what}: the avatar stays inside its row"
        );
    }
}

#[test]
fn a_continued_message_hides_its_header_only_when_grouped() {
    let grouped = runtime(conversation(), MessageLayout::default());
    assert!(has_part(&grouped, 1, "header"));
    assert!(!has_part(&grouped, 2, "header"));
    assert!(has_part(&grouped, 3, "header"));
    assert!(has_part(&grouped, 4, "header"), "half an hour later");
    let separate = runtime(
        conversation(),
        MessageLayout {
            grouped: false,
            ..MessageLayout::default()
        },
    );
    assert!(has_part(&separate, 2, "header"));
}

#[test]
fn a_loading_row_s_placeholders_line_up_with_the_messages() {
    use papo_sdk_ui::TimelineDirection;
    for layout in layouts() {
        let items = Vector::from(vec![
            message(1, "ana", 0, "above the gap"),
            Arc::new(TimelineItem::Loading(TimelineDirection::Newer)),
        ]);
        let rt = runtime(items, layout);
        let placeholder = rt
            .elements()
            .into_iter()
            .find(|element| {
                element.path.ends_with("skeleton-0/lead")
                    || element.path.contains("skeleton-0/") && element.path.ends_with("/lead")
            })
            .unwrap_or_else(|| panic!("{layout:?}: no placeholder"));
        let lead = part(&rt, 1, "lead");
        assert_near(placeholder.rect[0], lead[0], &format!("{layout:?}: lead x"));
        assert_near(
            placeholder.rect[2],
            lead[2],
            &format!("{layout:?}: lead width"),
        );
    }
}

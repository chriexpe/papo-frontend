//! What a badge says: unread, a mention count, or nothing, from the SDK's
//! badge values (`docs/design.md` §3, Badges).
//!
//! Owns turning `has_unread` and `UnreadCount` into what the rail and the
//! channel list show and what assistive technology hears, without
//! inventing anything: unknown unread reads as nothing, and a count the SDK
//! gives only as a lower bound shows as "n+". Also draws the mention pill
//! both the rail and the channel list use. The SDK owns the evidence and the
//! counting; the views own where a badge sits.

use papo_sdk::{ChannelBadges, UnreadCount};
use papo_sdk_ui::BadgeAggregate;
use rune::{Color, El, el, row, text};

use crate::design::{MENTION_PILL_PT, MENTION_PILL_RING_PT, Palette, SPACE_SM_PT, TYPE_BADGE};
use crate::strings;

/// The largest count a badge spells out; more shows as "99+".
const MAX_SHOWN_COUNT: u64 = 99;
/// The most characters a count shows in a circle; more take a pill.
const CIRCLE_MAX_CHARS: usize = 2;

/// What one channel's or one server's badge shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Badge {
    /// Read, or not known to be unread.
    #[default]
    Nothing,
    /// Unread, with no mention of the person.
    Unread,
    /// Mentions of the person; implies unread.
    Mentions(Count),
}

/// A mention count as the SDK gave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Count {
    /// At least 1.
    value: u64,
    /// The SDK knows only that there are this many or more.
    at_least: bool,
}

impl Badge {
    /// The badge for unread evidence `has_unread` and mention count
    /// `mentions`. A count of zero, exact or not, shows no number.
    pub(crate) fn new(has_unread: Option<bool>, mentions: UnreadCount) -> Badge {
        let count = match mentions {
            UnreadCount::Exact(value) => Some((value, false)),
            UnreadCount::AtLeast(value) => Some((value, true)),
            // A kind of count added later: no number is claimed.
            _ => None,
        };
        match count {
            Some((value, at_least)) if value > 0 => Badge::Mentions(Count { value, at_least }),
            _ if has_unread == Some(true) => Badge::Unread,
            _ => Badge::Nothing,
        }
    }

    pub(crate) fn of_channel(badges: &ChannelBadges) -> Badge {
        Badge::new(badges.has_unread, badges.mention_count)
    }

    /// A server's badge from its account's aggregate. An aggregate over an
    /// incomplete channel list already carries its count as a lower bound.
    pub(crate) fn of_aggregate(aggregate: &BadgeAggregate) -> Badge {
        Badge::new(aggregate.has_unread, aggregate.mention_count)
    }

    /// Several accounts' badges as one: their mentions summed (a lower bound
    /// when any is), else unread when any is.
    pub(crate) fn sum(badges: impl IntoIterator<Item = Badge>) -> Badge {
        let mut total: Option<Count> = None;
        let mut unread = false;
        for badge in badges {
            unread |= badge.is_unread();
            if let Some(count) = badge.mentions() {
                total = Some(match total {
                    Some(sum) => Count {
                        value: sum.value.saturating_add(count.value),
                        at_least: sum.at_least || count.at_least,
                    },
                    None => count,
                });
            }
        }
        match total {
            Some(count) => Badge::Mentions(count),
            None if unread => Badge::Unread,
            None => Badge::Nothing,
        }
    }

    /// Whether the row or tile reads as unread: bold, with its dot or bar.
    pub(crate) fn is_unread(self) -> bool {
        !matches!(self, Badge::Nothing)
    }

    /// The mention count, when there is one.
    pub(crate) fn mentions(self) -> Option<Count> {
        match self {
            Badge::Mentions(count) => Some(count),
            Badge::Nothing | Badge::Unread => None,
        }
    }

    /// What assistive technology adds to the row's or tile's name.
    pub(crate) fn spoken(self) -> Option<String> {
        match self {
            Badge::Nothing => None,
            Badge::Unread => Some(strings::badge_unread().to_owned()),
            Badge::Mentions(count) if count.at_least => {
                Some(strings::badge_mentions_at_least(count.value))
            }
            Badge::Mentions(count) => Some(strings::badge_mentions(count.value)),
        }
    }

    /// `name`, followed by what the badge says.
    pub(crate) fn label(self, name: &str) -> String {
        match self.spoken() {
            Some(spoken) => strings::badge_labelled(name, &spoken),
            None => name.to_owned(),
        }
    }
}

impl Count {
    /// A count known exactly, at least 1.
    pub(crate) fn exact(value: u64) -> Count {
        Count {
            value: value.max(1),
            at_least: false,
        }
    }

    /// The pill's text: the count, "n+" for a lower bound, "99+" past 99.
    pub(crate) fn shown(self) -> String {
        if self.value > MAX_SHOWN_COUNT {
            return strings::badge_count_more(MAX_SHOWN_COUNT);
        }
        if self.at_least {
            strings::badge_count_more(self.value)
        } else {
            self.value.to_string()
        }
    }
}

/// The red badge with the count: a circle for one or two characters ("3",
/// "3+"), a pill for more ("99+"). `ring` outlines it in the colour of what
/// it sits on, where it overlaps another shape (a rail tile's corner); the
/// ring is outside the badge, so the count keeps all of it. Assistive
/// technology hears the count through the row's or tile's label.
pub(crate) fn mention_pill(count: Count, ring: Option<Color>, palette: &Palette) -> El {
    count_pill(count, (palette.mention, palette.mention_on), ring)
}

/// [`mention_pill`]'s shape in other colours: `fill` behind `ink`. ↓'s
/// arrivals take the accent, so they never read as mentions.
pub(crate) fn count_pill(count: Count, (fill, ink): (Color, Color), ring: Option<Color>) -> El {
    let shown = count.shown();
    let badge = row()
        .h(MENTION_PILL_PT)
        .center()
        .radius(MENTION_PILL_PT / 2.0)
        .fill(fill)
        .a11y_hidden();
    let badge = if shown.chars().count() <= CIRCLE_MAX_CHARS {
        badge.w(MENTION_PILL_PT)
    } else {
        badge.px(SPACE_SM_PT)
    };
    let badge = badge.child(text(shown).font(TYPE_BADGE).color(ink));
    match ring {
        Some(ring) => el()
            .padding(MENTION_PILL_RING_PT)
            .radius(MENTION_PILL_PT / 2.0 + MENTION_PILL_RING_PT)
            .fill(ring)
            .a11y_hidden()
            .child(badge),
        None => badge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_win_over_unread_and_imply_it() {
        let badge = Badge::new(Some(true), UnreadCount::Exact(3));
        assert_eq!(badge.mentions().unwrap().shown(), "3");
        assert!(badge.is_unread());
        assert_eq!(Badge::new(Some(true), UnreadCount::Exact(0)), Badge::Unread);
    }

    #[test]
    fn an_unknown_unread_shows_nothing() {
        assert_eq!(Badge::new(None, UnreadCount::AtLeast(0)), Badge::Nothing);
        assert_eq!(
            Badge::new(Some(false), UnreadCount::Exact(0)),
            Badge::Nothing
        );
        assert!(!Badge::Nothing.is_unread());
        assert_eq!(Badge::Nothing.label("general"), "general");
    }

    #[test]
    fn a_lower_bound_shows_with_a_plus_and_never_more_than_given() {
        let badge = Badge::new(None, UnreadCount::AtLeast(3));
        assert_eq!(badge.mentions().unwrap().shown(), "3+");
        assert!(
            badge.is_unread(),
            "a mention is unread even when unread is unknown"
        );
        assert_eq!(Badge::new(None, UnreadCount::AtLeast(0)).mentions(), None);
    }

    #[test]
    fn counts_past_ninety_nine_cap() {
        assert_eq!(
            Badge::new(None, UnreadCount::Exact(99))
                .mentions()
                .unwrap()
                .shown(),
            "99"
        );
        assert_eq!(
            Badge::new(None, UnreadCount::Exact(100))
                .mentions()
                .unwrap()
                .shown(),
            "99+"
        );
        assert_eq!(
            Badge::new(None, UnreadCount::AtLeast(250))
                .mentions()
                .unwrap()
                .shown(),
            "99+"
        );
    }

    #[test]
    fn assistive_technology_hears_the_badge_after_the_name() {
        assert_eq!(Badge::Unread.label("general"), "general, unread messages");
        let one = Badge::new(None, UnreadCount::Exact(1));
        assert_eq!(one.label("general"), "general, 1 mention");
        let three = Badge::new(None, UnreadCount::Exact(3));
        assert_eq!(three.label("general"), "general, 3 mentions");
        let at_least = Badge::new(None, UnreadCount::AtLeast(3));
        assert_eq!(at_least.label("general"), "general, at least 3 mentions");
    }

    #[test]
    fn a_server_aggregate_reads_like_a_channel() {
        let aggregate = BadgeAggregate {
            mention_count: UnreadCount::AtLeast(2),
            has_unread: None,
        };
        assert_eq!(
            Badge::of_aggregate(&aggregate).mentions().unwrap().shown(),
            "2+"
        );
    }

    #[test]
    fn several_accounts_sum_their_mentions_else_say_unread() {
        let three = Badge::new(Some(true), UnreadCount::Exact(3));
        let some = Badge::new(Some(true), UnreadCount::AtLeast(2));
        assert_eq!(
            Badge::sum([three, Badge::Unread])
                .mentions()
                .unwrap()
                .shown(),
            "3"
        );
        assert_eq!(Badge::sum([three, some]).mentions().unwrap().shown(), "5+");
        assert_eq!(Badge::sum([Badge::Nothing, Badge::Unread]), Badge::Unread);
        assert_eq!(Badge::sum([]), Badge::Nothing);
    }
}

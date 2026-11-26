//! Where ← and → are on a quick-reaction strip (`docs/design.md` §10, §16.2): More emoji at its
//! left end, then the quick reactions. Shared by the stretched hover bar and the message menu's
//! strip; what each spot does and how it draws is theirs.

/// A spot on the strip.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum StripSpot {
    /// More emoji: the picker, for a reaction.
    More,
    /// The quick reaction at this index.
    Emoji(usize),
}

impl StripSpot {
    /// Where a strip opens: on its first emoji, as before More emoji joined it.
    pub(crate) const FIRST: StripSpot = StripSpot::Emoji(0);

    /// One step right (`right`) or left over a strip of `count` emoji, wrapping round the ends:
    /// left of the first emoji is More emoji, right of the last is More emoji again.
    #[must_use]
    pub(crate) fn step(self, count: usize, right: bool) -> StripSpot {
        let spots = count + 1;
        let at = match self {
            StripSpot::More => 0,
            StripSpot::Emoji(index) => (index + 1).min(count),
        };
        let next = if right {
            (at + 1) % spots
        } else {
            (at + spots - 1) % spots
        };
        match next {
            0 => StripSpot::More,
            spot => StripSpot::Emoji(spot - 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StripSpot;

    #[test]
    fn more_emoji_sits_left_of_the_first_emoji_and_the_ends_wrap() {
        assert_eq!(StripSpot::FIRST.step(8, false), StripSpot::More);
        assert_eq!(StripSpot::More.step(8, true), StripSpot::Emoji(0));
        assert_eq!(StripSpot::Emoji(7).step(8, true), StripSpot::More);
        assert_eq!(StripSpot::More.step(8, false), StripSpot::Emoji(7));
        assert_eq!(StripSpot::Emoji(2).step(8, true), StripSpot::Emoji(3));
        assert_eq!(
            StripSpot::More.step(0, true),
            StripSpot::More,
            "only More emoji"
        );
    }
}

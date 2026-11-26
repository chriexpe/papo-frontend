//! The emoji picker's icons (`docs/design.md` §16.2, §10): its category strip's, one per
//! Unicode category without one elsewhere, More emoji's, and the GIF tab's.

use rune::Icon;

use super::LINE_VIEW;

line_icon!(
    /// More emoji: the quick-reaction strip's left end, which opens the picker.
    SMILE_WINK_PLUS,
    "smile-wink-plus"
);

line_icon!(
    /// Animals and nature, in the emoji picker's category strip.
    LEAF,
    "leaf"
);
line_icon!(
    /// Food and drink, in the emoji picker's category strip.
    CUP,
    "cup"
);
line_icon!(
    /// Activities, in the emoji picker's category strip.
    BALL,
    "ball"
);
line_icon!(
    /// Travel and places, in the emoji picker's category strip.
    CAR,
    "car"
);
line_icon!(
    /// Objects, in the emoji picker's category strip.
    BULB,
    "bulb"
);
line_icon!(
    /// Symbols, in the emoji picker's category strip.
    HEART,
    "heart"
);
line_icon!(
    /// Flags, in the emoji picker's category strip.
    FLAG,
    "flag"
);
line_icon!(
    /// The GIF tab, the pill's GIF button.
    GIF,
    "gif"
);
line_icon!(
    /// The Favorites chip.
    STAR,
    "star"
);
line_icon!(
    /// The Trending chip.
    TRENDING,
    "trending"
);

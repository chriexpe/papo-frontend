//! The app's icons (`docs/design.md` §13): Papo's line icons on a 24-unit
//! grid and its presence marks on a 10-unit grid, each SVG file in
//! `assets/icons/` embedded whole. Rune reads the root `<svg>` as a group
//! and passes its paint attributes down, and a body that names no colour of
//! its own draws in the colour of the element that shows it, so the icons
//! stay neutral (§1).
//!
//! Owns only the shapes and the check that every committed file is one
//! Rune draws that way; sizes and colours come from `design` through the
//! views that use them. `assets/icons/` mirrors the whole design handoff
//! (`docs/papo-icons/`); a file is declared here by the slice that first
//! draws it.

use rune::Icon;

/// The line icons' grid, in SVG units square.
const LINE_VIEW: f32 = 24.0;
/// The presence marks' grid, in SVG units square.
const PRESENCE_VIEW: f32 = 10.0;

/// A line icon from `assets/icons/Icons/{file}.svg`.
macro_rules! line_icon {
    ($(#[$doc:meta])* $name:ident, $file:literal) => {
        $(#[$doc])*
        pub(crate) static $name: Icon = Icon {
            name: concat!("papo-", $file),
            view: LINE_VIEW,
            body: include_str!(concat!("../assets/icons/Icons/", $file, ".svg")),
        };
    };
}

/// A presence mark from `assets/icons/Presence/{file}.svg`.
macro_rules! presence_mark {
    ($(#[$doc:meta])* $name:ident, $file:literal) => {
        $(#[$doc])*
        pub(crate) static $name: Icon = Icon {
            name: concat!("papo-presence-", $file),
            view: PRESENCE_VIEW,
            body: include_str!(concat!("../assets/icons/Presence/", $file, ".svg")),
        };
    };
}

line_icon!(
    /// Add reaction, the add chip.
    SMILE_PLUS,
    "smile-plus"
);
line_icon!(
    /// View reactions.
    SMILE,
    "smile"
);
line_icon!(
    /// More: the message menu.
    MORE,
    "more"
);
line_icon!(
    /// Pin message, pins.
    PIN,
    "pin"
);
line_icon!(
    /// Unpin message.
    PIN_OFF,
    "pin-off"
);
line_icon!(
    /// Copy text, Copy link.
    COPY,
    "copy"
);
line_icon!(
    /// Copy as Markdown.
    MARKDOWN,
    "markdown"
);
line_icon!(
    /// Copy ID.
    ID,
    "id"
);
line_icon!(
    /// Delete message.
    TRASH,
    "trash"
);
line_icon!(
    /// Open link.
    EXTERNAL,
    "external"
);
line_icon!(
    /// Back: the reactions list to its menu.
    ARROW_LEFT,
    "arrow-left"
);
line_icon!(
    /// Close, cancel, clear.
    X,
    "x"
);
line_icon!(
    /// Add server (rail), Add account.
    PLUS,
    "plus"
);
line_icon!(
    /// Server settings (the server card's ⚙).
    GEAR,
    "gear"
);
line_icon!(
    /// Accounts on this server (the server pill's ⇄).
    SWITCH,
    "switch"
);
line_icon!(
    /// The current choice, a done task, Save edit.
    CHECK,
    "check"
);
line_icon!(
    /// A text channel.
    HASH,
    "hash"
);
line_icon!(
    /// A voice channel.
    SPEAKER,
    "speaker"
);
line_icon!(
    /// Open the unread catch-up block (⌛).
    HOURGLASS,
    "hourglass"
);
line_icon!(
    /// Jump to the newest message (↓).
    ARROW_DOWN,
    "arrow-down"
);
line_icon!(
    /// The "Unread" chip that ends a skipped block.
    ARROW_UP,
    "arrow-up"
);
line_icon!(
    /// The reply bar's mention switch.
    AT,
    "at"
);
line_icon!(
    /// Waiting: an arc, turned while it shows.
    SPINNER,
    "spinner"
);
line_icon!(
    /// Send, on the accent.
    SEND,
    "send"
);
line_icon!(
    /// Queued or sending: the time slot of an unsent message or change.
    CLOCK,
    "clock"
);
line_icon!(
    /// A failed or unknown send, in the danger colour.
    WARNING,
    "warning"
);
line_icon!(
    /// Reply (the reply bar).
    REPLY,
    "reply"
);
line_icon!(
    /// Edit message (the edit bar).
    EDIT,
    "edit"
);
line_icon!(
    /// Direct messages: the rail's Direct.
    DIRECT,
    "direct"
);
line_icon!(
    /// A reply line in Extra compact, where no elbow fits.
    REPLY_LINE,
    "reply-line"
);

presence_mark!(
    /// Available: a filled disc.
    PRESENCE_ONLINE,
    "online"
);
presence_mark!(
    /// Away: a crescent.
    PRESENCE_AWAY,
    "away"
);
presence_mark!(
    /// Busy: a disc with a bar cut out.
    PRESENCE_BUSY,
    "busy"
);
presence_mark!(
    /// Offline: a ring.
    PRESENCE_OFFLINE,
    "offline"
);

#[cfg(test)]
mod tests;

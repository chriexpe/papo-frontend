//! The app's icons: line drawings on a 16-unit grid, drawn in the colour of
//! the element that shows them, so they stay neutral (`docs/design.md` §1).
//!
//! Owns only the shapes; sizes and colours come from `design` through the
//! views that use them.

use rune::Icon;

/// Close, cancel, clear: a cross. From `papo-icons` (`docs/design.md` §13).
pub(crate) static CROSS: Icon = Icon {
    name: "papo-cross",
    view: 24.0,
    body: r#"<path d="M6 6l12 12M18 6 6 18" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Add: a plus.
pub(crate) static PLUS: Icon = Icon {
    name: "papo-plus",
    view: 16.0,
    body: r#"<path d="M8 3V13M3 8H13" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>"#,
};

/// Options: a gear.
pub(crate) static GEAR: Icon = Icon {
    name: "papo-gear",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="2.2" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M8 1.8V3.6M8 12.4V14.2M1.8 8H3.6M12.4 8H14.2M3.6 3.6L4.9 4.9M11.1 11.1L12.4 12.4M3.6 12.4L4.9 11.1M11.1 4.9L12.4 3.6" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none"/>"#,
};

/// Switch account: two arrows, one each way.
pub(crate) static SWAP: Icon = Icon {
    name: "papo-swap",
    view: 16.0,
    body: r#"<path d="M3 5.5H12.5M10 3L12.5 5.5L10 8M13 10.5H3.5M6 8L3.5 10.5L6 13" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// The current choice, a met requirement, Save edit: a check mark. From `papo-icons` (`docs/design.md` §13).
pub(crate) static CHECK: Icon = Icon {
    name: "papo-check",
    view: 24.0,
    body: r#"<path d="M5 12.5l4.5 4.5L19 7.5" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// A text channel: a hash.
pub(crate) static HASH: Icon = Icon {
    name: "papo-hash",
    view: 16.0,
    body: r#"<path d="M6.5 2.5L5 13.5M11 2.5L9.5 13.5M2.5 6H13.5M2 10.5H13" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none"/>"#,
};

/// A voice channel: a speaker.
pub(crate) static SPEAKER: Icon = Icon {
    name: "papo-speaker",
    view: 16.0,
    body: r#"<path d="M2.5 6H5L8.5 3V13L5 10H2.5Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" fill="none"/><path d="M11 5.5C12 6.4 12 9.6 11 10.5M12.8 3.8C14.8 5.8 14.8 10.2 12.8 12.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" fill="none"/>"#,
};

/// Waiting: an arc that turns.
pub(crate) static SPINNER: Icon = Icon {
    name: "papo-spinner",
    view: 16.0,
    body: r#"<path d="M14 8A6 6 0 1 1 8 2" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>"#,
};

/// Skipped unread messages: an hourglass.
pub(crate) static HOURGLASS: Icon = Icon {
    name: "papo-hourglass",
    view: 16.0,
    body: r#"<path d="M4 2H12M4 14H12M5 2C5 5.5 8 6.5 8 8C8 9.5 5 10.5 5 14M11 2C11 5.5 8 6.5 8 8C8 9.5 11 10.5 11 14" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// To the newest message: an arrow down.
pub(crate) static ARROW_DOWN: Icon = Icon {
    name: "papo-arrow-down",
    view: 16.0,
    body: r#"<path d="M8 3V13M3.5 8.5L8 13L12.5 8.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Send: an arrow pointing right, the mock's ➤.
pub(crate) static SEND: Icon = Icon {
    name: "papo-send",
    view: 16.0,
    body: r#"<path d="M3 8H13M8.5 3.5L13 8L8.5 12.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Waiting to be sent or saved: a clock. From `papo-icons` (`docs/design.md` §13).
pub(crate) static CLOCK: Icon = Icon {
    name: "papo-clock",
    view: 24.0,
    body: r#"<circle cx="12" cy="12" r="8.5" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M12 7.5V12l3 2" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Did not go through: a warning triangle. From `papo-icons` (`docs/design.md` §13).
pub(crate) static WARNING: Icon = Icon {
    name: "papo-warning",
    view: 24.0,
    body: r#"<path d="M10.3 5 3.2 17.5A2 2 0 0 0 5 20.5h14a2 2 0 0 0 1.8-3L13.7 5a2 2 0 0 0-3.4 0z" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M12 10v4" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M12 17h.01" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Reply: an arrow turning back. From `papo-icons` (`docs/design.md` §13).
pub(crate) static REPLY: Icon = Icon {
    name: "papo-reply",
    view: 24.0,
    body: r#"<path d="M9.5 6 4 11.5 9.5 17" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M4.5 11.5H14a6 6 0 0 1 6 6v1" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// Edit a message: a pencil. From `papo-icons` (`docs/design.md` §13).
pub(crate) static EDIT: Icon = Icon {
    name: "papo-edit",
    view: 24.0,
    body: r#"<path d="M15 4.5l4.5 4.5L9 19.5H4.5V15z" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M12.5 7l4.5 4.5" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

/// A reply line in Extra compact, where no elbow fits. From `papo-icons` (`docs/design.md` §13).
pub(crate) static REPLY_LINE: Icon = Icon {
    name: "papo-reply-line",
    view: 24.0,
    body: r#"<path d="M7 19.5V13a5 5 0 0 1 5-5h7.5" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M16 4.5 19.5 8 16 11.5" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

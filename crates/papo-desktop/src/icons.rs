//! The app's icons: line drawings on a 16-unit grid, drawn in the colour of
//! the element that shows them, so they stay neutral (`docs/design.md` §1).
//!
//! Owns only the shapes; sizes and colours come from `design` through the
//! views that use them.

use rune::Icon;

/// Close: a cross.
pub(crate) static CROSS: Icon = Icon {
    name: "papo-cross",
    view: 16.0,
    body: r#"<path d="M4 4L12 12M12 4L4 12" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>"#,
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

/// The current choice: a check mark.
pub(crate) static CHECK: Icon = Icon {
    name: "papo-check",
    view: 16.0,
    body: r#"<path d="M3 8.5L6.5 12L13 4.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
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

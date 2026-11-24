//! A channel's description (`docs/design.md` §8, Conversations rows):
//! when a channel with a topic opens, its title pill stretches to the
//! right and the topic scrolls beside the name, past a hairline, for six
//! seconds (with "Description when opening a channel" on), on one line,
//! scrolling when it is longer than the header leaves it; while the
//! pointer is on the pill the strip shows whatever the setting. A press on
//! the pill opens the whole topic, wrapped, in a panel grown from it, which
//! an outside press or Escape closes, as the menus do.
//!
//! Owns when the strip shows, its scrolling and the panel's body; the
//! title pill around it is `title`'s, the topic the channel's
//! (`server_list::ChannelEntry`).

use std::time::Duration;

use rune::{Easing, El, LoopSpec, Tx, col, el, row, text};

use super::Shell;
use crate::design::{Look, PANE_EDGE_PT, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_SMALL};
use crate::menu::{OpenMenu, Placement};

/// How long the strip shows when a channel opens.
const STRIP_S: f64 = 6.0;
/// How fast a long topic scrolls, in points a second.
const SCROLL_PT_PER_S: f32 = 36.0;
/// The soft fade at the strip's ends, in points.
const STRIP_FADE_PT: f32 = 12.0;
/// The panel's narrowest, in points; its widest is a quarter of the
/// window.
const PANEL_LEAST_PT: f32 = 280.0;
/// The panel's padding either side of the topic, in points.
const PANEL_PADDING_PT: f32 = 16.0;
/// The menu's own padding around the panel's body, both sides, at the
/// largest spacing (`space.sm` at ×1.25), so a topic that fits never wraps.
const MENU_PADDING_PT: f32 = 20.0;
/// Between the topic and its repeat while it scrolls.
const REPEAT_GAP: &str = "    ·    ";

/// The description's state for the shown channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Description {
    /// Until when the strip shows after the channel opened, on the window's
    /// clock.
    until_s: Option<f64>,
    /// The pointer is on the title pill.
    hovered: bool,
    /// The topic's width on one line, once laid out.
    topic_pt: Option<f32>,
    /// The strip's width, as the header leaves it.
    strip_pt: Option<f32>,
    /// The window's width, from the header's right edge.
    window_pt: Option<f32>,
}

impl Shell {
    /// The shown channel's topic; none in a direct conversation or for a
    /// channel without one.
    pub(crate) fn shown_topic(&self) -> Option<&str> {
        let server = self.nav.selected()?;
        let channel = self.nav.selected_channel()?;
        server
            .text_channels
            .iter()
            .find(|entry| entry.id == channel)?
            .topic
            .as_deref()
    }

    /// A conversation opened: with the setting on and a topic, the strip
    /// shows for six seconds.
    pub(super) fn description_opened(&mut self, tx: &mut Tx) {
        // The window's width holds across channels; the topic's does not.
        self.description = Description {
            window_pt: self.description.window_pt,
            ..Description::default()
        };
        if !self.conversations.settings().channel_description || self.shown_topic().is_none() {
            return;
        }
        self.description.until_s = Some(tx.clock() + STRIP_S);
        tx.wake_after(Duration::from_secs_f64(STRIP_S));
    }

    /// The window's poll: the strip's six seconds ending. True when it
    /// went.
    pub(super) fn poll_description(&mut self, clock_s: f64) -> bool {
        if self
            .description
            .until_s
            .is_some_and(|until_s| until_s <= clock_s)
        {
            self.description.until_s = None;
            return !self.description.hovered;
        }
        false
    }

    /// The pointer entered or left the title pill.
    pub(crate) fn title_hovered(&mut self, on: bool) {
        self.description.hovered = on;
    }

    /// The strip's topic was laid out this wide.
    pub(crate) fn topic_measured(&mut self, width_pt: f32) {
        if self.description.topic_pt != Some(width_pt) {
            self.description.topic_pt = Some(width_pt);
        }
    }

    /// The topic's measured width, for tests.
    #[cfg(test)]
    pub(crate) fn description_width(&self) -> Option<f32> {
        self.description.topic_pt
    }

    /// Whether the strip shows now.
    pub(super) fn strip_shown(&self) -> bool {
        self.shown_topic().is_some()
            && (self.description.hovered || self.description.until_s.is_some())
    }

    /// A press on the title pill: the whole topic in a panel under it, or
    /// nothing for a channel without one. The panel is as wide as the topic
    /// on one line, between its narrowest and a quarter of the window;
    /// past that the topic wraps.
    pub(crate) fn open_description(&mut self, tx: &mut Tx) {
        let Some(topic) = self.shown_topic().map(str::to_owned) else {
            return;
        };
        let most_pt = self
            .description
            .window_pt
            .map_or(PANEL_LEAST_PT, |window_pt| {
                (window_pt / 4.0).max(PANEL_LEAST_PT)
            });
        let width_pt = self.description.topic_pt.map_or(most_pt, |topic_pt| {
            (topic_pt + PANEL_PADDING_PT * 2.0 + MENU_PADDING_PT).clamp(PANEL_LEAST_PT, most_pt)
        });
        let mut menu = OpenMenu::with_entries(
            Placement::Below(super::title::TITLE_PILL_KEY.to_owned()),
            super::title::TITLE_PILL_KEY.to_owned(),
            Vec::new(),
            width_pt,
        );
        menu.description = Some(topic);
        self.nav.menu = Some(menu);
        tx.focus(crate::menu::MENU_KEY, true);
    }

    /// The header was laid out at `rect`: its right edge is the window's.
    pub(crate) fn header_measured(&mut self, rect: [f32; 4]) {
        let window_pt = rect[0] + rect[2];
        if self.description.window_pt != Some(window_pt) {
            self.description.window_pt = Some(window_pt);
        }
    }

    /// The strip's window was laid out this wide.
    pub(crate) fn strip_measured(&mut self, width_pt: f32) {
        if self.description.strip_pt != Some(width_pt) {
            self.description.strip_pt = Some(width_pt);
        }
    }

    /// The topic beside the name, on one line: as wide as the topic, or as
    /// the header leaves it before the actions pill; a topic longer than
    /// that scrolls.
    pub(super) fn strip(&self, topic: &str, look: &Look) -> El {
        let font = look.font(TYPE_SMALL);
        let topic = one_line(topic);
        let measured = text(topic.clone())
            .font(font)
            .color(look.palette.text_secondary)
            .max_lines(1)
            .on_layout(|shell: &mut Shell, rect: [f32; 4]| shell.topic_measured(rect[2]));
        let long = match (self.description.topic_pt, self.description.strip_pt) {
            (Some(topic_pt), Some(strip_pt)) => topic_pt > strip_pt + 1.0,
            _ => false,
        };
        let mut line = row().child(measured);
        if let (true, Some(topic_pt)) = (long, self.description.topic_pt) {
            // The topic and its repeat, sliding by half their width: the
            // repeat lands where the topic began, so the loop is seamless.
            let period_s = f64::from(topic_pt / SCROLL_PT_PER_S);
            line = line
                .child(
                    text(format!("{REPEAT_GAP}{topic}{REPEAT_GAP}"))
                        .font(font)
                        .color(look.palette.text_secondary)
                        .max_lines(1)
                        .a11y_hidden(),
                )
                .looping(LoopSpec::slide(period_s, [0.0, 0.0], [-0.5, 0.0]).easing(Easing::LINEAR));
        }
        // As wide as the topic, shrinking to what the header leaves it.
        let mut window = el()
            .shrink()
            .on_layout(|shell: &mut Shell, rect: [f32; 4]| shell.strip_measured(rect[2]))
            .child(line);
        if long {
            window = window.clip_fade(STRIP_FADE_PT, 0.0, STRIP_FADE_PT, 0.0);
        } else {
            window = window.clip_fade(0.0, 0.0, 0.0, 0.0);
        }
        row()
            .key("title-description")
            .shrink()
            .gap(look.space(SPACE_MD))
            .center_y()
            .child(
                el().w(PANE_EDGE_PT)
                    .h(look.font(TYPE_SMALL).line_px())
                    .fill(look.palette.line_subtle)
                    .a11y_hidden(),
            )
            .child(window)
    }
}

/// `topic` on one line: its line breaks and runs of spaces as one space.
fn one_line(topic: &str) -> String {
    topic.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The open description panel's body: the whole topic, wrapped.
pub(super) fn body(shell: &Shell, look: &Look) -> Option<El> {
    let topic = shell.nav.menu.as_ref()?.description.as_ref()?;
    Some(
        col()
            .w_full()
            .px(PANEL_PADDING_PT)
            .py(look.space(SPACE_MD))
            .gap(look.space(SPACE_SM))
            .role("note")
            .label(crate::strings::channel_description_open())
            .child(
                text(topic.clone())
                    .font(look.font(TYPE_BODY))
                    .color(look.palette.text_primary)
                    .wrap(),
            ),
    )
}

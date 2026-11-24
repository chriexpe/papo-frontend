//! A channel's description (`docs/design.md` §8, Conversations rows):
//! when a channel with a topic opens, its title pill stretches to the
//! right and the topic scrolls beside the name, past a hairline, for six
//! seconds (with "Description when opening a channel" on); while the
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
use crate::design::{Look, PANE_EDGE_PT, SPACE_LG, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_SMALL};
use crate::menu::{OpenMenu, Placement};

/// How long the strip shows when a channel opens.
const STRIP_S: f64 = 6.0;
/// The strip's widest, in points; a longer topic scrolls inside it.
const STRIP_MOST_PT: f32 = 320.0;
/// How fast a long topic scrolls, in points a second.
const SCROLL_PT_PER_S: f32 = 36.0;
/// The soft fade at the strip's ends, in points.
const STRIP_FADE_PT: f32 = 12.0;
/// The panel's width, in points.
const PANEL_WIDTH_PT: f32 = 360.0;
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
        self.description = Description::default();
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

    /// Whether the strip shows now.
    pub(super) fn strip_shown(&self) -> bool {
        self.shown_topic().is_some()
            && (self.description.hovered || self.description.until_s.is_some())
    }

    /// A press on the title pill: the whole topic in a panel under it, or
    /// nothing for a channel without one.
    pub(crate) fn open_description(&mut self, tx: &mut Tx) {
        let Some(topic) = self.shown_topic().map(str::to_owned) else {
            return;
        };
        let mut menu = OpenMenu::with_entries(
            Placement::Below(super::title::TITLE_PILL_KEY.to_owned()),
            super::title::TITLE_PILL_KEY.to_owned(),
            Vec::new(),
            PANEL_WIDTH_PT,
        );
        menu.description = Some(topic);
        self.nav.menu = Some(menu);
        tx.focus(crate::menu::MENU_KEY, true);
    }

    /// The topic beside the name: as wide as the topic up to the strip's
    /// widest, scrolling when it is longer.
    pub(super) fn strip(&self, topic: &str, look: &Look) -> El {
        let font = look.font(TYPE_SMALL);
        let measured = text(topic.to_owned())
            .font(font)
            .color(look.palette.text_secondary)
            .single_line()
            .on_layout(|shell: &mut Shell, rect: [f32; 4]| shell.topic_measured(rect[2]));
        let long = self
            .description
            .topic_pt
            .is_some_and(|width_pt| width_pt > STRIP_MOST_PT);
        let mut line = row().child(measured);
        if let (true, Some(width_pt)) = (long, self.description.topic_pt) {
            // The topic and its repeat, sliding by half their width: the
            // repeat lands where the topic began, so the loop is seamless.
            let period_s = f64::from((width_pt * 2.0) / SCROLL_PT_PER_S);
            line = line
                .child(
                    text(format!("{REPEAT_GAP}{topic}{REPEAT_GAP}"))
                        .font(font)
                        .color(look.palette.text_secondary)
                        .single_line()
                        .a11y_hidden(),
                )
                .looping(LoopSpec::slide(period_s, [0.0, 0.0], [-0.5, 0.0]).easing(Easing::LINEAR));
        }
        let width_pt = self
            .description
            .topic_pt
            .map_or(STRIP_MOST_PT, |width_pt| width_pt.min(STRIP_MOST_PT));
        let mut window = el().w(width_pt).child(line);
        if long {
            window = window.clip_fade(STRIP_FADE_PT, 0.0, STRIP_FADE_PT, 0.0);
        }
        row()
            .key("title-description")
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

/// The open description panel's body: the whole topic, wrapped.
pub(super) fn body(shell: &Shell, look: &Look) -> Option<El> {
    let topic = shell.nav.menu.as_ref()?.description.as_ref()?;
    Some(
        col()
            .w_full()
            .px(look.space(SPACE_LG))
            .py(look.space(SPACE_MD))
            .gap(look.space(SPACE_SM))
            .role("note")
            .label(crate::strings::channel_description_open())
            .child(
                text(topic.clone())
                    .font(look.font(TYPE_BODY))
                    .color(look.palette.text_primary),
            ),
    )
}

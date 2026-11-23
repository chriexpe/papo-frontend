//! Where the keyboard's moves through a server go (`docs/design.md` §12):
//! back and forward through the channels shown in this server (Alt+← and
//! Alt+→), and the previous or next unread channel (Alt+⇧↑ and Alt+⇧↓).
//!
//! Owns each server's history of shown channels, newest last, bounded. A
//! channel shown again by going back or forward moves along the history
//! instead of adding to it; any other pick adds the channel left and clears
//! what was ahead, as a browser does.

use crate::server_list::ServerEntry;

use super::Navigation;

/// The most channels a server's history keeps behind and ahead.
const HISTORY_LIMIT: usize = 50;

/// One server's shown channels.
#[derive(Clone, Debug, Default)]
pub(crate) struct ChannelHistory {
    back: Vec<String>,
    forward: Vec<String>,
}

impl ChannelHistory {
    /// Channel `left` was left for another by a pick: it is where Back goes.
    pub(super) fn left(&mut self, left: String) {
        if self.back.last() != Some(&left) {
            self.back.push(left);
            if self.back.len() > HISTORY_LIMIT {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }
}

impl Navigation {
    /// Alt+← (`forward` false) or Alt+→: the channel back or forward in
    /// the selected server's history, shown; the one left goes the other
    /// way. Channels the server no longer has are skipped. Returns the
    /// channel now shown, if it moved.
    pub(crate) fn step_history(&mut self, forward: bool) -> Option<String> {
        let server = self.selected()?.clone();
        let current = self.selected_channel()?.to_owned();
        let history = self.history.entry(server.id).or_default();
        loop {
            let next = if forward {
                history.forward.pop()?
            } else {
                history.back.pop()?
            };
            if next == current || !has_channel(&server, &next) {
                continue;
            }
            // The channel left is where the other way goes.
            if forward {
                history.back.push(current);
            } else {
                history.forward.push(current);
            }
            self.channels.insert(server.id, next.clone());
            return Some(next);
        }
    }

    /// The selected server's nearest unread channel before (`by` -1) or
    /// after (`by` 1) the selected one, in the list's order, wrapping round
    /// the ends; none when no other channel is unread.
    pub(crate) fn unread_channel_beside(&self, by: isize) -> Option<&str> {
        let server = self.selected()?;
        let selected = self.selected_channel()?;
        let channels = &server.text_channels;
        let from = channels.iter().position(|channel| channel.id == selected)?;
        let count = channels.len();
        (1..count)
            .map(|step| {
                let at = if by < 0 {
                    (from + count - step) % count
                } else {
                    (from + step) % count
                };
                &channels[at]
            })
            .find(|channel| channel.badge.is_unread())
            .map(|channel| channel.id.as_str())
    }
}

/// Whether `server` still has channel `id`.
fn has_channel(server: &ServerEntry, id: &str) -> bool {
    server
        .text_channels
        .iter()
        .chain(&server.voice_channels)
        .any(|channel| channel.id == id)
}

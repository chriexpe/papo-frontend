//! What the WebView thread says to the shell (`docs/design.md` §15.3, §15.5): WPE ready or
//! missing (then every poster's button opens the page), a page's first frame, its failure, its
//! sound starting (other media go quiet), its full-screen request (the viewer takes it), a click
//! that leaves the player (Papo's link rules), the cursor it wants, and which sites hold data.

use papo_sdk_ui::PlayerProvider;
use rune::{Cursor, Tx};

use super::Shell;
use super::players::Web;
use crate::web_view::{PageEvent, PageId, WebEvent};

impl Shell {
    /// What the WebView thread says. Returns whether views change.
    pub(crate) fn web_event(&mut self, event: WebEvent, tx: &mut Tx) -> bool {
        match event {
            WebEvent::Ready => {
                if let Web::Starting(views) =
                    std::mem::replace(&mut self.players.web, Web::NotStarted)
                {
                    self.players.web = Web::Ready(views);
                }
                false
            }
            WebEvent::Unavailable(_) => {
                self.players.web = Web::Unavailable;
                let commands = self.players.store.release_all();
                self.run_player_commands(commands);
                true
            }
            WebEvent::Page(page, event) => self.page_event(page, event, tx),
            WebEvent::SiteData { site, has_data } => {
                let Some(provider) = PlayerProvider::from_key(&site) else {
                    return false;
                };
                if has_data {
                    self.players.sites_with_data.insert(provider)
                } else {
                    self.players.sites_with_data.remove(&provider)
                }
            }
        }
    }

    fn page_event(&mut self, page: PageId, event: PageEvent, tx: &mut Tx) -> bool {
        let Some(key) = self.players.store.key_of(page) else {
            return false;
        };
        match event {
            PageEvent::Painted => {
                self.players.ringing.remove(&key);
                self.players.store.painted(page)
            }
            PageEvent::Failed => {
                let commands = self.players.store.failed(page);
                self.run_player_commands(commands);
                true
            }
            PageEvent::Sound(sounding) => {
                let change = self.players.store.sound(page, sounding);
                if change.quiet_others {
                    self.quiet_other_media();
                }
                self.run_player_commands(change.commands);
                false
            }
            PageEvent::FullScreen => {
                self.open_player_viewer(key, tx);
                true
            }
            PageEvent::OpenInBrowser(address) => {
                // The page left the player for the provider's own site: Papo's link rules.
                let timeline = crate::timeline::TimelineKey {
                    account: key.account,
                    channel: key.channel,
                };
                self.link_pressed(timeline, address, tx);
                true
            }
            PageEvent::Cursor(name) => {
                let cursor = cursor_of(&name);
                self.players.cursors.insert(key, cursor) != Some(cursor)
            }
        }
    }
}

/// The cursor a page asks for by CSS name, among the ones Papo draws.
fn cursor_of(name: &str) -> Cursor {
    match name {
        "pointer" => Cursor::Pointer,
        "text" | "vertical-text" => Cursor::Text,
        "grab" => Cursor::Grab,
        "grabbing" => Cursor::Grabbing,
        "not-allowed" | "no-drop" => Cursor::NotAllowed,
        "move" | "all-scroll" => Cursor::Move,
        "ew-resize" | "col-resize" => Cursor::ResizeEw,
        "ns-resize" | "row-resize" => Cursor::ResizeNs,
        _ => Cursor::Default,
    }
}

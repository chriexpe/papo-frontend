//! What the viewer draws, gathered from the shell (`docs/design.md` §14.8): its always-dark look
//! (Graphite's on Light and Soft light, OLED's on OLED), the picture (the whole one once
//! decoded, else the timeline's copy), its size, its room, and the ring.

use rune::{Cx, El};

use super::Shell;
use crate::design::Theme;
use crate::viewer::view::{Shown, view as viewer};

/// The viewer, while open.
pub(super) fn view(shell: &Shell, cx: &mut Cx<Shell>) -> Option<El> {
    let open = shell.viewer.as_ref()?;
    let item = open.item()?;
    let system_dark = cx.system_dark();
    let mut appearance = shell.shown_appearance();
    if !appearance.theme_or(system_dark).is_dark() {
        appearance.theme = Some(Theme::Graphite);
    }
    let look = super::appearance::look_of(
        &appearance,
        shell.conversations.settings().mention_hue_degrees,
        system_dark,
        shell.system_font,
    );
    let full = open
        .full
        .as_ref()
        .filter(|image| image.is_ready() && image.error().is_none());
    // A GIF animates here whatever the setting says: the whole picture is its animation.
    let still = shell
        .media
        .view()
        .get(item.key)
        .and_then(|drawn| drawn.picture(true).cloned());
    let picture = full.cloned().or(still);
    let shown = Shown {
        viewer: open,
        picture: picture.as_ref(),
        natural: shell.viewer_natural(),
        room: shell.viewer_room,
        ring: open.ring && !open.full.as_ref().is_some_and(rune::Image::is_ready),
        saved_pill: shell.saves.pill().map(|pill| pill.name.as_str()),
    };
    Some(viewer(&shown, &look))
}

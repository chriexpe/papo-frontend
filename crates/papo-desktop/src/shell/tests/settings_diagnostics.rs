//! Settings › Diagnostics in the shell (`docs/design.md` §8): Clear media
//! cache asks in its row and Cancel puts it back; the page shows the
//! connection the live watch posts.

use super::navigation::click;
use super::settings::open;
use super::*;
use crate::settings::{PageId, RowId};

#[test]
fn clear_media_cache_asks_first_and_cancel_puts_the_row_back() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &format!("settings-nav-{:?}", PageId::Diagnostics));
    click(&mut rt, &RowId::ClearCache.key());
    assert_eq!(
        rt.state.settings().and_then(|panel| panel.asking),
        Some(RowId::ClearCache)
    );
    assert!(tree(&rt).contains("Clear the cache?"));
    click(&mut rt, &format!("{}-cancel", RowId::ClearCache.key()));
    assert_eq!(rt.state.settings().and_then(|panel| panel.asking), None);
    assert!(!tree(&rt).contains("Clear the cache?"));
}

#[test]
fn diagnostics_shows_the_version() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &format!("settings-nav-{:?}", PageId::Diagnostics));
    assert!(tree(&rt).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_failed_clear_says_so_under_its_row_with_try_again() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &format!("settings-nav-{:?}", PageId::Diagnostics));
    rt.update(|shell, _tx| {
        if let Some(panel) = shell.settings.as_mut() {
            panel.saved(
                RowId::ClearCache,
                crate::settings::Target::ClearCache,
                false,
            );
        }
    });
    rt.frame(FRAME_S);
    let shown = tree(&rt);
    assert!(shown.contains("Couldn't clear the media cache."), "{shown}");
    assert!(!shown.contains("Couldn't save this setting."));
    assert!(rt.find("settings-retry").is_some());
}

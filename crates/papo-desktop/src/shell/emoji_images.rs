//! The shell's side of custom emoji (`docs/design.md` §9, Emoji): each account's catalog as
//! content resolves against it, the emoji the shown views name (the conversation's rows near
//! the view and their replies, the open pins, the search results on screen, the field), their
//! images fetched through the SDK's media store and decoded off the UI thread.
//!
//! What is kept and asked for is `emoji_images`'; the catalog and the image bytes are the SDK's.

use std::collections::BTreeSet;
use std::sync::Arc;

use papo_sdk::{AccountId, CustomEmoji, EmojiCatalog, Uuid};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::CustomEmojis;
use rune::Pixels;

use super::Shell;
use crate::emoji_images::{DECODE_MAX_PX, EmojiJob};
use crate::timeline::content::custom_emoji_ids;

/// Rows past the visible ones whose emoji are fetched too, so a short scroll finds them drawn.
const NEAR_ROWS: usize = 12;

impl Shell {
    /// Account `account`'s custom emoji as content resolves against them; none before its
    /// catalog is posted.
    pub(crate) fn custom_emojis(&self, account: AccountId) -> CustomEmojis {
        self.activity
            .get(&account)
            .map(|activity| activity.custom.clone())
            .unwrap_or_default()
    }

    /// Account `account`'s catalog, when posted.
    fn catalog(&self, account: AccountId) -> Option<&Arc<EmojiCatalog>> {
        self.activity.get(&account)?.emojis.as_ref()
    }

    /// Account `account`'s catalog changed: its open timelines, the search over it and the
    /// field resolve against the new one, and its emoji are asked for.
    pub(super) fn custom_emojis_changed(&mut self, account: AccountId) {
        self.timelines_follow_emojis();
        let emojis = self.custom_emojis(account);
        if let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.account == account)
            && let super::search::Found::Ready(results) = &mut search.found
        {
            results.set_custom_emojis(&emojis);
        }
        // A draft restored before the catalog arrived binds its `:name:` now.
        if let Some(key) = self.composer.shown().map(|(key, _)| key)
            && key.account == account
        {
            self.composer.bind_emojis(key, &emojis, false);
        }
        self.want_emoji();
    }

    /// Every open timeline resolves against its account's current emoji.
    pub(super) fn timelines_follow_emojis(&mut self) {
        let accounts: Vec<AccountId> = self.tabs.open_accounts().collect();
        for account in accounts {
            let emojis = self.custom_emojis(account);
            for open in self.tabs.of_account_mut(account) {
                open.set_emojis(emojis.clone());
            }
        }
    }

    /// Asks for the images of the custom emoji the shown views name.
    pub(super) fn want_emoji(&mut self) {
        let Some(account) = self.tabs.shown().map(|open| open.key.account) else {
            return;
        };
        let ids = self.shown_emoji(account);
        let Some(catalog) = self.catalog(account).cloned() else {
            return;
        };
        let wanted: Vec<&CustomEmoji> = ids.iter().filter_map(|id| catalog.get(*id)).collect();
        let jobs = self.emoji_images.want(account, wanted);
        self.run_emoji_jobs(jobs);
    }

    /// The custom emoji the views of `account` name now.
    fn shown_emoji(&self, account: AccountId) -> BTreeSet<Uuid> {
        let mut ids = BTreeSet::new();
        if let Some(open) = self.tabs.shown() {
            let visible = open.visible_rows();
            let from = visible.start.saturating_sub(NEAR_ROWS);
            let to = (visible.end + NEAR_ROWS).min(open.items.len());
            for item in open.items.iter().skip(from).take(to.saturating_sub(from)) {
                match item.as_ref() {
                    TimelineItem::Message {
                        id, content, reply, ..
                    } => {
                        // An edit in flight shows its own text.
                        let shown = open
                            .changes
                            .get(*id)
                            .and_then(|change| change.edit.as_ref())
                            .map_or(content, |(_, parsed)| parsed);
                        ids.extend(custom_emoji_ids(shown));
                        if let Some(papo_sdk_ui::ReplyOriginal::Held { content, .. }) = reply {
                            ids.extend(custom_emoji_ids(content));
                        }
                    }
                    TimelineItem::LocalEcho { id, .. } => {
                        if let Some(echo) = open.echoes.get(*id) {
                            ids.extend(custom_emoji_ids(&echo.content));
                        }
                    }
                    _ => {}
                }
            }
            let pins_open = self
                .pins_panel
                .as_ref()
                .is_some_and(|panel| panel.key == open.key);
            if pins_open && let Some(pins) = open.pins.list() {
                for pin in pins {
                    ids.extend(custom_emoji_ids(&pin.content));
                }
            }
        }
        if let Some(search) = self
            .search
            .as_ref()
            .filter(|search| search.account == account)
            && let super::search::Found::Ready(results) = &search.found
        {
            let range = search.hydrated.visible();
            for item in results.items().iter().skip(range.start).take(range.len()) {
                ids.extend(custom_emoji_ids(&item.content));
            }
        }
        ids.extend(self.composer.emoji_ids());
        ids
    }

    /// Starts `jobs`: each reads the emoji's image through the SDK's media store and decodes
    /// it off the UI thread, then lands in the store.
    fn run_emoji_jobs(&mut self, jobs: Vec<EmojiJob>) {
        if jobs.is_empty() {
            return;
        }
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        for job in jobs {
            let updates = sdk.updates();
            sdk.run(move |client| async move {
                let fetched = match client.session(job.key.account) {
                    Ok(session) => session.media().fetch(job.source.clone(), false).await.ok(),
                    Err(_) => None,
                };
                let image = match fetched {
                    Some(data) => {
                        let bytes = data.value.bytes;
                        tokio::task::spawn_blocking(move || decode(&bytes))
                            .await
                            .ok()
                            .flatten()
                    }
                    None => None,
                };
                updates.send(move |shell| shell.emoji_images.landed(&job, image));
            });
        }
    }
}

/// An emoji's image decoded: its first frame (animated emoji are not in D11, §16), fitted
/// within [`DECODE_MAX_PX`] square, and its aspect. None when it doesn't decode.
fn decode(bytes: &[u8]) -> Option<(Pixels, f32)> {
    let image = image::load_from_memory(bytes).ok()?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return None;
    }
    let image = if width > DECODE_MAX_PX || height > DECODE_MAX_PX {
        image.resize(
            DECODE_MAX_PX,
            DECODE_MAX_PX,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = Pixels::new();
    pixels.set(width, height, rgba.into_raw());
    #[expect(
        clippy::cast_precision_loss,
        reason = "image sides are at most DECODE_MAX_PX"
    )]
    let aspect = width as f32 / height as f32;
    Some((pixels, aspect))
}

#[cfg(test)]
mod tests {
    use super::decode;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn an_emoji_decodes_within_the_largest_box_keeping_its_shape() {
        let (pixels, aspect) = decode(&png(400, 200)).unwrap();
        assert_eq!(pixels.size(), (160, 80));
        assert!((aspect - 2.0).abs() < 1e-3);
        let (small, _) = decode(&png(32, 32)).unwrap();
        assert_eq!(small.size(), (32, 32), "a small one keeps its size");
        assert!(decode(b"not an image").is_none());
    }
}

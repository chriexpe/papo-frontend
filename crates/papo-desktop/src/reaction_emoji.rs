//! A reaction's emoji as the desktop shows it (`docs/design.md` §10, Reactions): a Unicode one
//! as its glyph, a custom one as its image with its `:name:` as its words. Shared by the hover
//! bar, the quick-reaction strip, the chips and the reactions list, so they draw alike. What a
//! custom emoji is called is the account's catalog's; its image is `emoji_images`'.

use papo_sdk::AccountId;
use papo_sdk_ui::message_content::CustomEmojis;
use papo_sdk_ui::operations::ReactionEmoji;
use rune::{El, text};

use crate::design::{Look, TYPE_BODY, emoji_box};
use crate::emoji_images::{EmojiKey, EmojiLook, EmojiLooks, emoji_square};

/// How wide a custom emoji's name may run when its image failed, where nothing else bounds it
/// (a chip, a tab of the reactions list): long names are common.
pub(crate) const FAILED_NAME_MAX_PT: f32 = 120.0;

/// What one account's reaction emoji are called and draw as.
#[derive(Clone)]
pub(crate) struct ReactionEmojiLook {
    pub(crate) account: AccountId,
    pub(crate) names: CustomEmojis,
    pub(crate) images: EmojiLooks,
}

impl ReactionEmojiLook {
    /// `emoji` as a row draws it; none for a custom emoji the catalog doesn't have (it has no
    /// name to show, so it is left out).
    pub(crate) fn shown(&self, emoji: &ReactionEmoji) -> Option<ShownReaction> {
        match emoji {
            ReactionEmoji::Unicode(glyph) => Some(ShownReaction::unicode(glyph)),
            ReactionEmoji::Custom(id) => {
                let name = self.names.get(*id)?;
                Some(
                    ShownReaction {
                        emoji: emoji.clone(),
                        name: format!(":{}:", name.name),
                        image: None,
                    }
                    .drawn(self.account, &self.images),
                )
            }
        }
    }
}

/// A reaction emoji as drawn: part of a memoized row's version, so the row redraws when a custom
/// one's image lands.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct ShownReaction {
    pub(crate) emoji: ReactionEmoji,
    /// What it is called in words: the glyph, or `:name:` (spoken, the chip's key, its menu).
    pub(crate) name: String,
    /// A custom one's image as it stands; none for a Unicode one.
    pub(crate) image: Option<EmojiLook>,
}

impl ShownReaction {
    /// A Unicode emoji: its glyph, which is also what it is called.
    pub(crate) fn unicode(glyph: &str) -> ShownReaction {
        ShownReaction {
            emoji: ReactionEmoji::Unicode(glyph.to_owned()),
            name: glyph.to_owned(),
            image: None,
        }
    }

    /// The same emoji with a custom one's image as `images` hold it now for `account`: a
    /// surface that keeps its emoji while open (the menu's strip) draws the image that landed
    /// since.
    pub(crate) fn drawn(mut self, account: AccountId, images: &EmojiLooks) -> ShownReaction {
        if let ReactionEmoji::Custom(id) = self.emoji {
            self.image = Some(images.look(EmojiKey { account, id }));
        }
        self
    }

    /// The emoji drawn at the body text's size, as a Unicode one's glyph is (§10: "emoji at
    /// body size"); a custom one's name, when its image failed, cut to `max_pt` wide.
    pub(crate) fn glyph(&self, max_pt: f32, look: &Look) -> El {
        let font = look.font(TYPE_BODY);
        match &self.image {
            None => text(self.name.clone()).font(font).a11y_hidden(),
            Some(image) => {
                let (side_pt, _) = emoji_box(font, true);
                emoji_square(image, side_pt, &self.name, max_pt, look)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use papo_sdk::{CustomEmoji, EmojiCatalog, ServerAddress, Uuid};

    use super::*;

    fn look() -> ReactionEmojiLook {
        let account = AccountId::new(
            ServerAddress::parse("https://emoji.invalid").unwrap().id(),
            Uuid::from_u128(1),
        );
        let catalog = EmojiCatalog {
            revision: 3,
            emojis: vec![CustomEmoji {
                id: Uuid::from_u128(7),
                name: "gato".into(),
                format: "png".into(),
                created_at: "t".into(),
            }],
        };
        ReactionEmojiLook {
            account,
            names: CustomEmojis::new(&catalog),
            images: EmojiLooks::default(),
        }
    }

    #[test]
    fn a_custom_reaction_is_named_and_drawn_by_its_image_and_an_unknown_one_is_left_out() {
        let gato = look()
            .shown(&ReactionEmoji::Custom(Uuid::from_u128(7)))
            .unwrap();
        assert_eq!(gato.name, ":gato:");
        assert_eq!(
            gato.image,
            Some(EmojiLook::Loading),
            "its square until it lands"
        );
        assert_eq!(
            look().shown(&ReactionEmoji::Custom(Uuid::from_u128(8))),
            None
        );
        let cheese = look().shown(&ReactionEmoji::Unicode("🧀".into())).unwrap();
        assert_eq!((cheese.name.as_str(), cheese.image), ("🧀", None));
    }
}

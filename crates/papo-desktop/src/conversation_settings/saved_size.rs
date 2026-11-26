//! How a size setting is saved in the device's settings file, whichever media it bounds: a
//! preset's name, or Custom's sides. Reading and writing the file is `device_settings`'.

use papo_sdk_ui::{EmbedSizeSetting, MediaSizeSetting};
use rune::prefs::Prefs;
use serde_json::{Value, json};

/// A size setting as it is saved, whichever media it bounds: a preset's name, or Custom's
/// sides as `{"custom": [width, height]}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SavedSize {
    Small,
    Medium,
    Large,
    Custom(u16, u16),
}

impl SavedSize {
    pub(super) fn of_media(size: MediaSizeSetting) -> SavedSize {
        match size {
            MediaSizeSetting::Small => SavedSize::Small,
            MediaSizeSetting::Medium => SavedSize::Medium,
            MediaSizeSetting::Large => SavedSize::Large,
            MediaSizeSetting::Custom {
                max_width_pt,
                max_height_pt,
            } => SavedSize::Custom(max_width_pt, max_height_pt),
        }
    }

    pub(super) fn of_embed(size: EmbedSizeSetting) -> SavedSize {
        match size {
            EmbedSizeSetting::Small => SavedSize::Small,
            EmbedSizeSetting::Medium => SavedSize::Medium,
            EmbedSizeSetting::Large => SavedSize::Large,
            EmbedSizeSetting::Custom {
                max_width_pt,
                max_height_pt,
            } => SavedSize::Custom(max_width_pt, max_height_pt),
        }
    }

    /// As an Image size or Video size (a Custom size comes back within its ranges).
    pub(super) fn media(self) -> MediaSizeSetting {
        match self {
            SavedSize::Small => MediaSizeSetting::Small,
            SavedSize::Medium => MediaSizeSetting::Medium,
            SavedSize::Large => MediaSizeSetting::Large,
            SavedSize::Custom(width, height) => MediaSizeSetting::custom(width, height),
        }
    }

    /// As an Embed size (a Custom size comes back within its ranges).
    pub(super) fn embed(self) -> EmbedSizeSetting {
        match self {
            SavedSize::Small => EmbedSizeSetting::Small,
            SavedSize::Medium => EmbedSizeSetting::Medium,
            SavedSize::Large => EmbedSizeSetting::Large,
            SavedSize::Custom(width, height) => EmbedSizeSetting::custom(width, height),
        }
    }

    pub(super) fn value(self) -> Value {
        match self {
            SavedSize::Small => json!("small"),
            SavedSize::Medium => json!("medium"),
            SavedSize::Large => json!("large"),
            SavedSize::Custom(width, height) => json!({ "custom": [width, height] }),
        }
    }
}

/// The size saved under `key`; none when it is missing or anything else.
pub(super) fn saved_size(prefs: &Prefs, key: &str) -> Option<SavedSize> {
    match prefs.get::<Value>(key)? {
        Value::String(name) => match name.as_str() {
            "small" => Some(SavedSize::Small),
            "medium" => Some(SavedSize::Medium),
            "large" => Some(SavedSize::Large),
            _ => None,
        },
        Value::Object(object) => {
            let sides = object.get("custom")?.as_array()?;
            let side = |index: usize| {
                sides
                    .get(index)?
                    .as_u64()
                    .map(|side| u16::try_from(side).unwrap_or(u16::MAX))
            };
            Some(SavedSize::Custom(side(0)?, side(1)?))
        }
        _ => None,
    }
}

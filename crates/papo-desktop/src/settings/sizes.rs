//! The media size settings as the settings rows see them (`docs/design.md` §14.9, §15.7):
//! which media one bounds, its rows, its choice and its sides. The values are the SDK's
//! (`MediaSizeSetting`, `EmbedSizeSetting`); their rows are `rows`'.

use super::RowId;

/// Which media a size setting bounds (§14.9, §15.7: images, videos and embeds each have their
/// own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizedMedia {
    Images,
    Videos,
    Embeds,
}

impl SizedMedia {
    /// Its size row.
    pub(crate) fn row(self) -> RowId {
        match self {
            SizedMedia::Images => RowId::ImageSize,
            SizedMedia::Videos => RowId::VideoSize,
            SizedMedia::Embeds => RowId::EmbedSize,
        }
    }

    /// Its Custom size's Max width and Max height rows.
    pub(crate) fn slider_rows(self) -> (RowId, RowId) {
        match self {
            SizedMedia::Images => (RowId::MediaMaxWidth, RowId::MediaMaxHeight),
            SizedMedia::Videos => (RowId::VideoMaxWidth, RowId::VideoMaxHeight),
            SizedMedia::Embeds => (RowId::EmbedMaxWidth, RowId::EmbedMaxHeight),
        }
    }

    /// The media whose Custom Max width or Max height `row` is.
    pub(crate) fn of_slider(row: RowId) -> Option<SizedMedia> {
        [SizedMedia::Images, SizedMedia::Videos, SizedMedia::Embeds]
            .into_iter()
            .find(|media| {
                let (width, height) = media.slider_rows();
                row == width || row == height
            })
    }

    /// The heights its Custom size may take, in points (Embed size's start lower).
    pub(crate) fn custom_heights(self) -> std::ops::RangeInclusive<u16> {
        match self {
            SizedMedia::Images | SizedMedia::Videos => papo_sdk_ui::CUSTOM_HEIGHT_PT,
            SizedMedia::Embeds => papo_sdk_ui::EMBED_CUSTOM_HEIGHT_PT,
        }
    }
}

/// One media's size setting, whichever it bounds: Image size and Video size share their
/// presets, Embed size has its own (§15.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizeSetting {
    Images(papo_sdk_ui::MediaSizeSetting),
    Videos(papo_sdk_ui::MediaSizeSetting),
    Embeds(papo_sdk_ui::EmbedSizeSetting),
}

/// A size setting's choice, apart from the media it bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizeChoice {
    Small,
    Medium,
    Large,
    Custom,
}

impl SizeSetting {
    /// The media it bounds.
    pub(crate) fn media(self) -> SizedMedia {
        match self {
            SizeSetting::Images(_) => SizedMedia::Images,
            SizeSetting::Videos(_) => SizedMedia::Videos,
            SizeSetting::Embeds(_) => SizedMedia::Embeds,
        }
    }

    /// Which choice it is.
    pub(crate) fn choice(self) -> SizeChoice {
        use papo_sdk_ui::{EmbedSizeSetting as E, MediaSizeSetting as M};
        match self {
            SizeSetting::Images(M::Small)
            | SizeSetting::Videos(M::Small)
            | SizeSetting::Embeds(E::Small) => SizeChoice::Small,
            SizeSetting::Images(M::Medium)
            | SizeSetting::Videos(M::Medium)
            | SizeSetting::Embeds(E::Medium) => SizeChoice::Medium,
            SizeSetting::Images(M::Large)
            | SizeSetting::Videos(M::Large)
            | SizeSetting::Embeds(E::Large) => SizeChoice::Large,
            SizeSetting::Images(M::Custom { .. })
            | SizeSetting::Videos(M::Custom { .. })
            | SizeSetting::Embeds(E::Custom { .. }) => SizeChoice::Custom,
        }
    }

    /// Its max width and max height, in points.
    pub(crate) fn max_size_pt(self) -> (u16, u16) {
        match self {
            SizeSetting::Images(size) | SizeSetting::Videos(size) => size.max_size_pt(),
            SizeSetting::Embeds(size) => size.max_size_pt(),
        }
    }

    /// The same media's setting at `choice`; Custom at `width` × `height`, brought into its
    /// ranges.
    pub(crate) fn with(self, choice: SizeChoice, width: u16, height: u16) -> SizeSetting {
        use papo_sdk_ui::{EmbedSizeSetting as E, MediaSizeSetting as M};
        let media = |choice| match choice {
            SizeChoice::Small => M::Small,
            SizeChoice::Medium => M::Medium,
            SizeChoice::Large => M::Large,
            SizeChoice::Custom => M::custom(width, height),
        };
        match self {
            SizeSetting::Images(_) => SizeSetting::Images(media(choice)),
            SizeSetting::Videos(_) => SizeSetting::Videos(media(choice)),
            SizeSetting::Embeds(_) => SizeSetting::Embeds(match choice {
                SizeChoice::Small => E::Small,
                SizeChoice::Medium => E::Medium,
                SizeChoice::Large => E::Large,
                SizeChoice::Custom => E::custom(width, height),
            }),
        }
    }
}

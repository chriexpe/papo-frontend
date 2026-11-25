//! How a decoded picture's YCbCr values map to colour: matrix, range and transfer, in Papo's own
//! terms, so nothing this crate exposes is a GStreamer type. Read from GStreamer's caps where
//! frames are copied; drawing (a shader, a CPU conversion) belongs to the host.

use gstreamer_video::{VideoColorMatrix, VideoColorRange, VideoColorimetry, VideoTransferFunction};

/// A decoded picture's colour description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoColour {
    /// How Cb and Cr mix into red, green and blue.
    pub matrix: ColourMatrix,
    /// Which values are black and white.
    pub range: ColourRange,
    /// The curve from encoded values to light.
    pub transfer: ColourTransfer,
}

/// The YCbCr matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourMatrix {
    /// ITU-R BT.601 (SD video, JPEG).
    Bt601,
    /// ITU-R BT.709 (HD video).
    Bt709,
    /// ITU-R BT.2020 (UHD, HDR video).
    Bt2020,
    /// Not stated, or one Papo doesn't name.
    Unknown,
}

/// The value range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourRange {
    /// 0–255: full range (JPEG, screen captures).
    Full,
    /// 16–235 luma, 16–240 chroma: limited ("video") range, most video.
    Limited,
    /// Not stated.
    Unknown,
}

/// The transfer curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourTransfer {
    /// BT.709's curve; BT.601 and BT.2020 (10 and 12 bit) use the same one.
    Bt709,
    /// sRGB's curve.
    Srgb,
    /// Any other curve (PQ, HLG, gamma).
    Other,
}

impl VideoColour {
    /// The colour a caps `colorimetry` field states (`"bt709"`, `"bt601"`, `"1:4:7:1"`), for a
    /// client's own pipeline; what can't be read is unknown, as a stream that states nothing.
    /// Initialises GStreamer if nothing has yet (its parser needs it).
    #[must_use]
    pub fn from_caps_string(colorimetry: &str) -> Self {
        if crate::init_gstreamer().is_err() {
            return Self::UNKNOWN;
        }
        colorimetry
            .parse::<VideoColorimetry>()
            .map_or(Self::UNKNOWN, |colorimetry| Self::of(&colorimetry))
    }

    /// Nothing stated.
    const UNKNOWN: Self = Self {
        matrix: ColourMatrix::Unknown,
        range: ColourRange::Unknown,
        transfer: ColourTransfer::Other,
    };

    /// The colour GStreamer's caps state.
    pub(crate) fn of(colorimetry: &VideoColorimetry) -> Self {
        let matrix = match colorimetry.matrix() {
            VideoColorMatrix::Bt601 => ColourMatrix::Bt601,
            VideoColorMatrix::Bt709 => ColourMatrix::Bt709,
            VideoColorMatrix::Bt2020 => ColourMatrix::Bt2020,
            _ => ColourMatrix::Unknown,
        };
        let range = match colorimetry.range() {
            VideoColorRange::Range0_255 => ColourRange::Full,
            VideoColorRange::Range16_235 => ColourRange::Limited,
            _ => ColourRange::Unknown,
        };
        let transfer = match colorimetry.transfer() {
            VideoTransferFunction::Bt601
            | VideoTransferFunction::Bt709
            | VideoTransferFunction::Bt202010
            | VideoTransferFunction::Bt202012 => ColourTransfer::Bt709,
            VideoTransferFunction::Srgb => ColourTransfer::Srgb,
            _ => ColourTransfer::Other,
        };
        Self {
            matrix,
            range,
            transfer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer_video::VideoColorPrimaries;

    #[test]
    fn caps_colour_maps_to_papo_terms() {
        let sd = VideoColorimetry::new(
            VideoColorRange::Range16_235,
            VideoColorMatrix::Bt601,
            VideoTransferFunction::Bt601,
            VideoColorPrimaries::Smpte170m,
        );
        assert_eq!(
            VideoColour::of(&sd),
            VideoColour {
                matrix: ColourMatrix::Bt601,
                range: ColourRange::Limited,
                transfer: ColourTransfer::Bt709,
            }
        );
        let jpeg = VideoColorimetry::new(
            VideoColorRange::Range0_255,
            VideoColorMatrix::Bt601,
            VideoTransferFunction::Srgb,
            VideoColorPrimaries::Bt709,
        );
        assert_eq!(VideoColour::of(&jpeg).range, ColourRange::Full);
        assert_eq!(VideoColour::of(&jpeg).transfer, ColourTransfer::Srgb);
        let hdr = VideoColorimetry::new(
            VideoColorRange::Unknown,
            VideoColorMatrix::Rgb,
            VideoTransferFunction::Smpte2084,
            VideoColorPrimaries::Bt2020,
        );
        let hdr = VideoColour::of(&hdr);
        assert_eq!(hdr.matrix, ColourMatrix::Unknown);
        assert_eq!(hdr.range, ColourRange::Unknown);
        assert_eq!(hdr.transfer, ColourTransfer::Other);
    }

    #[test]
    fn caps_strings_read_like_the_caps_they_came_from() {
        assert_eq!(
            VideoColour::from_caps_string("bt709"),
            VideoColour {
                matrix: ColourMatrix::Bt709,
                range: ColourRange::Limited,
                transfer: ColourTransfer::Bt709,
            }
        );
        assert_eq!(
            VideoColour::from_caps_string("bt601").matrix,
            ColourMatrix::Bt601
        );
        assert_eq!(
            VideoColour::from_caps_string("not colour"),
            VideoColour::UNKNOWN
        );
    }
}

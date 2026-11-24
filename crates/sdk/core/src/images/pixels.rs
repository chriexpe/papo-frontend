//! The pixel route, used only when pixels must change: orientation that is not upright, a turn,
//! a crop, or a fixed output size. Decoding runs under hard limits; the result keeps the colour
//! profile and nothing else, so it carries no metadata by construction.

use super::{
    edit::{CropRect, ImageFormat},
    error::{ImageError, UnsupportedFormat},
};
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageReader, Limits,
    imageops::FilterType, metadata::Orientation,
};
use std::io::{BufRead, Seek, Write};

/// Decoded pixels the route accepts: 64 megapixels (256 MiB as RGBA).
const MAX_PIXELS: u64 = 64_000_000;
const MAX_DIMENSION: u32 = 16_384;
const MAX_DECODER_ALLOC_BYTES: u64 = 768 * 1024 * 1024;
const JPEG_QUALITY: u8 = 92;
/// Qualities tried in turn when a fixed-size upload must fit its byte cap.
const FIXED_JPEG_QUALITIES: [u8; 4] = [90, 80, 70, 55];

/// A picture ready to edit: pixels in display orientation plus its colour profile.
pub(super) struct Picture {
    pub(super) pixels: DynamicImage,
    pub(super) icc_profile: Option<Vec<u8>>,
}

fn library_format(format: ImageFormat) -> image::ImageFormat {
    match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Gif => image::ImageFormat::Gif,
        ImageFormat::Webp => image::ImageFormat::WebP,
    }
}

fn convert(error: image::ImageError) -> ImageError {
    match error {
        image::ImageError::IoError(error) => super::container::source_error(&error),
        image::ImageError::Limits(_) => ImageError::TooLarge,
        image::ImageError::Unsupported(_) => ImageError::Unsupported(UnsupportedFormat::Other),
        _ => ImageError::Malformed,
    }
}

/// Opens a decoder with the route's limits applied before any pixel is read.
fn decoder<'a, R: 'a + BufRead + Seek>(
    src: R,
    format: ImageFormat,
    limited: bool,
) -> Result<impl ImageDecoder + 'a, ImageError> {
    let mut decoder = ImageReader::with_format(src, library_format(format))
        .into_decoder()
        .map_err(convert)?;
    if !limited {
        return Ok(decoder);
    }
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let mut limits = Limits::no_limits();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODER_ALLOC_BYTES);
    decoder.set_limits(limits).map_err(convert)?;
    Ok(decoder)
}

/// Size and orientation from the file's headers, without decoding pixels and without the pixel
/// limits (a panorama can be cleaned though it could not be decoded). A header that cannot be
/// read fails closed: an orientation not known is never taken for upright.
pub(super) fn headers<R: BufRead + Seek>(
    src: R,
    format: ImageFormat,
) -> Result<(u32, u32, Orientation), ImageError> {
    let mut decoder = decoder(src, format, false)?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder.orientation().map_err(|_| ImageError::Malformed)?;
    Ok((width, height, orientation))
}

/// Decodes the first frame and turns it upright.
pub(super) fn open<R: BufRead + Seek>(src: R, format: ImageFormat) -> Result<Picture, ImageError> {
    let mut decoder = decoder(src, format, true)?;
    let orientation = decoder.orientation().map_err(|_| ImageError::Malformed)?;
    let icc_profile = decoder.icc_profile().ok().flatten();
    let mut pixels = DynamicImage::from_decoder(decoder).map_err(convert)?;
    pixels.apply_orientation(orientation);
    Ok(Picture {
        pixels,
        icc_profile,
    })
}

/// Applies clockwise quarter turns, then the crop, in the picture as the viewer sees it.
pub(super) fn edit(
    mut pixels: DynamicImage,
    quarter_turns: u8,
    crop: Option<CropRect>,
) -> Result<DynamicImage, ImageError> {
    pixels = match quarter_turns % 4 {
        1 => pixels.rotate90(),
        2 => pixels.rotate180(),
        3 => pixels.rotate270(),
        _ => pixels,
    };
    let Some(rect) = crop else {
        return Ok(pixels);
    };
    let fits_horizontally = rect
        .x
        .checked_add(rect.width)
        .is_some_and(|edge| edge <= pixels.width());
    let fits_vertically = rect
        .y
        .checked_add(rect.height)
        .is_some_and(|edge| edge <= pixels.height());
    if rect.width == 0 || rect.height == 0 || !fits_horizontally || !fits_vertically {
        return Err(ImageError::InvalidEdit);
    }
    Ok(pixels.crop_imm(rect.x, rect.y, rect.width, rect.height))
}

/// Writes `pixels` in the same format family the file had, at high quality.
///
/// JPEG stays JPEG (quality 92), PNG stays PNG and WebP stays WebP (lossless, the only mode the
/// locked encoder has). A GIF has no still encoder here, so an edited still GIF becomes a PNG.
/// Returns the format written.
pub(super) fn encode_like(
    picture: &Picture,
    source: ImageFormat,
    out: &mut dyn Write,
) -> Result<ImageFormat, ImageError> {
    let format = if source == ImageFormat::Gif {
        ImageFormat::Png
    } else {
        source
    };
    write(
        &picture.pixels,
        picture.icc_profile.clone(),
        format,
        JPEG_QUALITY,
        out,
    )?;
    Ok(format)
}

/// Resizes to cover `width` × `height` (centred) and encodes under `max_bytes`: PNG when there is
/// transparency, else JPEG at the highest quality that fits.
pub(super) fn encode_fixed(
    picture: &Picture,
    width: u32,
    height: u32,
    max_bytes: usize,
) -> Result<(Vec<u8>, ImageFormat), ImageError> {
    let pixels = picture
        .pixels
        .resize_to_fill(width, height, FilterType::Lanczos3);
    let transparent = pixels.color().has_alpha()
        && pixels
            .to_rgba8()
            .pixels()
            .any(|pixel| pixel.0[3] != u8::MAX);
    let candidates: &[(ImageFormat, u8)] = if transparent {
        &[(ImageFormat::Png, 0)]
    } else {
        &[
            (ImageFormat::Jpeg, FIXED_JPEG_QUALITIES[0]),
            (ImageFormat::Jpeg, FIXED_JPEG_QUALITIES[1]),
            (ImageFormat::Jpeg, FIXED_JPEG_QUALITIES[2]),
            (ImageFormat::Jpeg, FIXED_JPEG_QUALITIES[3]),
        ]
    };
    for (format, quality) in candidates {
        let mut bytes = Vec::new();
        write(
            &pixels,
            picture.icc_profile.clone(),
            *format,
            *quality,
            &mut bytes,
        )?;
        if bytes.len() <= max_bytes {
            return Ok((bytes, *format));
        }
    }
    Err(ImageError::TooLarge)
}

fn write(
    pixels: &DynamicImage,
    icc_profile: Option<Vec<u8>>,
    format: ImageFormat,
    quality: u8,
    out: &mut dyn Write,
) -> Result<(), ImageError> {
    let (width, height) = (pixels.width(), pixels.height());
    match format {
        ImageFormat::Jpeg => {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(out, quality);
            keep_profile(&mut encoder, icc_profile);
            let rgb = pixels.to_rgb8();
            encoder
                .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                .map_err(convert)
        }
        ImageFormat::Png => {
            let mut encoder = image::codecs::png::PngEncoder::new(out);
            keep_profile(&mut encoder, icc_profile);
            let rgba = pixels.to_rgba8();
            let opaque = rgba.pixels().all(|pixel| pixel.0[3] == u8::MAX);
            if opaque {
                encoder
                    .write_image(
                        pixels.to_rgb8().as_raw(),
                        width,
                        height,
                        ExtendedColorType::Rgb8,
                    )
                    .map_err(convert)
            } else {
                encoder
                    .write_image(rgba.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .map_err(convert)
            }
        }
        ImageFormat::Webp => {
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(out);
            keep_profile(&mut encoder, icc_profile);
            if pixels.color().has_alpha() {
                encoder
                    .write_image(
                        pixels.to_rgba8().as_raw(),
                        width,
                        height,
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(convert)
            } else {
                encoder
                    .write_image(
                        pixels.to_rgb8().as_raw(),
                        width,
                        height,
                        ExtendedColorType::Rgb8,
                    )
                    .map_err(convert)
            }
        }
        ImageFormat::Gif => Err(ImageError::Unsupported(UnsupportedFormat::Other)),
    }
}

/// An encoder that cannot carry the profile simply writes without it; the pixels are the same.
fn keep_profile(encoder: &mut impl ImageEncoder, icc_profile: Option<Vec<u8>>) {
    if let Some(profile) = icc_profile {
        let _ = encoder.set_icc_profile(profile);
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic pictures only; no file from a camera is needed to prove the geometry.

    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn gradient(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
            Rgb([
                u8::try_from(x % 256).unwrap(),
                u8::try_from(y % 256).unwrap(),
                7,
            ])
        }))
    }

    #[test]
    fn a_quarter_turn_swaps_the_sides_clockwise() {
        let turned = edit(gradient(4, 2), 1, None).unwrap();
        assert_eq!((turned.width(), turned.height()), (2, 4));
        // The top-left pixel of the source ends up top-right after a clockwise turn.
        assert_eq!(turned.to_rgb8().get_pixel(1, 0).0, [0, 0, 7]);
    }

    #[test]
    fn four_turns_are_the_identity() {
        let source = gradient(5, 3);
        assert_eq!(
            edit(source.clone(), 4, None).unwrap().to_rgb8(),
            source.to_rgb8()
        );
    }

    #[test]
    fn a_crop_is_taken_after_the_turn() {
        let crop = CropRect {
            x: 0,
            y: 1,
            width: 2,
            height: 2,
        };
        let cropped = edit(gradient(4, 2), 1, Some(crop)).unwrap();
        assert_eq!((cropped.width(), cropped.height()), (2, 2));
    }

    #[test]
    fn empty_and_out_of_bounds_crops_are_refused() {
        for rect in [
            CropRect {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            CropRect {
                x: 3,
                y: 0,
                width: 2,
                height: 1,
            },
            CropRect {
                x: u32::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
        ] {
            assert_eq!(
                edit(gradient(4, 2), 0, Some(rect)).map(|_| ()),
                Err(ImageError::InvalidEdit)
            );
        }
    }

    #[test]
    fn a_fixed_upload_is_exactly_the_target_size_and_a_jpeg_when_opaque() {
        let picture = Picture {
            pixels: gradient(300, 200),
            icc_profile: None,
        };
        let (bytes, format) = encode_fixed(&picture, 64, 64, 2 * 1024 * 1024).unwrap();
        assert_eq!(format, ImageFormat::Jpeg);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (64, 64));
    }

    #[test]
    fn transparency_keeps_a_fixed_upload_a_png() {
        let pixels = DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 40, Rgba([1, 2, 3, 0])));
        let (bytes, format) = encode_fixed(
            &Picture {
                pixels,
                icc_profile: None,
            },
            32,
            32,
            usize::MAX,
        )
        .unwrap();
        assert_eq!(format, ImageFormat::Png);
        assert!(image::load_from_memory(&bytes).unwrap().color().has_alpha());
    }

    #[test]
    fn a_fixed_upload_that_cannot_fit_its_cap_is_a_typed_error() {
        let picture = Picture {
            pixels: gradient(300, 200),
            icc_profile: None,
        };
        assert_eq!(
            encode_fixed(&picture, 64, 64, 10).map(|_| ()),
            Err(ImageError::TooLarge)
        );
    }

    #[test]
    fn a_still_gif_edit_is_written_as_a_png() {
        let picture = Picture {
            pixels: gradient(8, 8),
            icc_profile: None,
        };
        let mut out = Vec::new();
        assert_eq!(
            encode_like(&picture, ImageFormat::Gif, &mut out),
            Ok(ImageFormat::Png)
        );
        assert!(out.starts_with(b"\x89PNG"));
    }

    #[test]
    fn the_colour_profile_survives_a_re_encode() {
        // An sRGB-like profile is not parsed by the encoder; the bytes only need to round-trip.
        let profile = vec![7_u8; 128];
        let picture = Picture {
            pixels: gradient(8, 8),
            icc_profile: Some(profile.clone()),
        };
        let mut out = Vec::new();
        encode_like(&picture, ImageFormat::Png, &mut out).unwrap();
        let mut decoder =
            ImageReader::with_format(std::io::Cursor::new(out), image::ImageFormat::Png)
                .into_decoder()
                .unwrap();
        assert_eq!(decoder.icc_profile().unwrap(), Some(profile));
    }
}

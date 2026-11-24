//! End-to-end checks of the image path on synthetic files: every fixture is built in code, with
//! sentinel text standing for GPS, camera and author data, and each result is read back from its
//! own bytes.

use super::*;
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageReader, Rgb, RgbImage,
};
use std::io::Cursor;

const SECRET: &[u8] = b"GPS-SECRET-48.8584N-2.2945E";

fn gradient(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            u8::try_from(x * 8 % 256).unwrap(),
            u8::try_from(y * 8 % 256).unwrap(),
            90,
        ])
    })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A TIFF block with one orientation entry, then the secret as unreferenced trailing bytes.
fn exif(orientation: u8) -> Vec<u8> {
    let mut bytes = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    bytes.extend_from_slice(&[orientation, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend_from_slice(SECRET);
    bytes
}

fn jpeg(width: u32, height: u32, orientation: Option<u8>, profile: Option<Vec<u8>>) -> Vec<u8> {
    let mut plain = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 90);
    if let Some(profile) = profile {
        encoder.set_icc_profile(profile).unwrap();
    }
    encoder
        .write_image(
            gradient(width, height).as_raw(),
            width,
            height,
            ExtendedColorType::Rgb8,
        )
        .unwrap();
    let mut file = plain[..2].to_vec();
    let mut metadata = Vec::new();
    for (marker, payload) in [
        (0xE1, orientation.map_or_else(|| exif(1), exif)),
        (
            0xE1,
            [b"http://ns.adobe.com/xap/1.0/\0".as_slice(), SECRET].concat(),
        ),
        (0xFE, SECRET.to_vec()),
        (0xE0, [b"JFXX\0\x10".as_slice(), SECRET].concat()),
    ] {
        metadata.extend_from_slice(&[0xFF, marker]);
        metadata.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        metadata.extend_from_slice(&payload);
    }
    file.extend(metadata);
    file.extend_from_slice(&plain[2..]);
    file.extend_from_slice(SECRET);
    file
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    bytes.extend_from_slice(&body);
    bytes.extend_from_slice(&crc32(&body).to_be_bytes());
    bytes
}

fn png(width: u32, height: u32, profile: Option<Vec<u8>>) -> Vec<u8> {
    let mut plain = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut plain);
    if let Some(profile) = profile {
        encoder.set_icc_profile(profile).unwrap();
    }
    encoder
        .write_image(
            gradient(width, height).as_raw(),
            width,
            height,
            ExtendedColorType::Rgb8,
        )
        .unwrap();
    // Insert text, time and EXIF chunks after IHDR (8 signature + 25 header bytes).
    let mut file = plain[..33].to_vec();
    file.extend(png_chunk(
        *b"tEXt",
        &[b"Author\0".as_slice(), SECRET].concat(),
    ));
    file.extend(png_chunk(*b"eXIf", &exif(1)[6..]));
    file.extend(png_chunk(*b"tIME", &[7, 232, 11, 24, 1, 2, 3]));
    file.extend_from_slice(&plain[33..]);
    file.extend_from_slice(SECRET);
    file
}

fn gif(frames: u8) -> Vec<u8> {
    let mut plain = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut plain);
        for index in 0..frames {
            let frame = image::Frame::new(
                DynamicImage::ImageRgb8(gradient(8 + u32::from(index), 8)).to_rgba8(),
            );
            encoder.encode_frame(frame).unwrap();
        }
    }
    let palette = if plain[10] & 0x80 == 0 {
        0
    } else {
        3 << ((plain[10] & 7) + 1)
    };
    let at = 13 + palette;
    let mut file = plain[..at].to_vec();
    file.extend_from_slice(&[0x21, 0xFE, u8::try_from(SECRET.len()).unwrap()]);
    file.extend_from_slice(SECRET);
    file.push(0);
    file.extend_from_slice(&[0x21, 0xFF, 11]);
    file.extend_from_slice(b"XMP DataXMP");
    file.extend_from_slice(&[u8::try_from(SECRET.len()).unwrap()]);
    file.extend_from_slice(SECRET);
    file.push(0);
    file.extend_from_slice(&plain[at..]);
    file.extend_from_slice(SECRET);
    file
}

fn webp_chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = kind.to_vec();
    bytes.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(data);
    if data.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}

fn webp(width: u32, height: u32) -> Vec<u8> {
    let mut plain = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut plain)
        .write_image(
            gradient(width, height).as_raw(),
            width,
            height,
            ExtendedColorType::Rgb8,
        )
        .unwrap();
    assert_eq!(
        &plain[12..16],
        b"VP8L",
        "the fixture assumes a simple lossless file"
    );
    let bitstream = plain[12..].to_vec();
    let mut canvas = vec![0x08_u8, 0, 0, 0];
    canvas.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    canvas.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend(webp_chunk(*b"VP8X", &canvas));
    body.extend(bitstream);
    body.extend(webp_chunk(*b"EXIF", &exif(1)[6..]));
    body.extend(webp_chunk(*b"XMP ", SECRET));
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    file.extend(body);
    file
}

fn run(file: &[u8], edit: &AttachmentEdit) -> Result<(Vec<u8>, ImageFormat), ImageError> {
    let mut out = Vec::new();
    let format = apply(&mut Cursor::new(file.to_vec()), &mut out, edit)?;
    Ok((out, format))
}

fn anonymize() -> AttachmentEdit {
    AttachmentEdit {
        anonymize: Anonymize::Required,
        ..AttachmentEdit::default()
    }
}

fn decode(bytes: &[u8]) -> DynamicImage {
    image::load_from_memory(bytes).unwrap()
}

fn profile_of(bytes: &[u8], format: image::ImageFormat) -> Option<Vec<u8>> {
    ImageReader::with_format(Cursor::new(bytes.to_vec()), format)
        .into_decoder()
        .unwrap()
        .icc_profile()
        .unwrap()
}

#[test]
fn a_jpeg_keeps_its_exact_pixels_and_loses_everything_else() {
    let file = jpeg(32, 24, None, None);
    let (out, format) = run(&file, &anonymize()).unwrap();
    assert_eq!(format, ImageFormat::Jpeg);
    assert!(!contains(&out, SECRET) && !contains(&out, b"Exif") && !contains(&out, b"JFXX"));
    // No recompression: the decoded pixels are identical to the original's.
    assert_eq!(decode(&out).to_rgb8(), decode(&file).to_rgb8());
    assert!(out.len() < file.len());
}

#[test]
fn a_jpeg_keeps_its_colour_profile() {
    let profile = vec![42_u8; 300];
    let file = jpeg(16, 16, None, Some(profile.clone()));
    let (out, _) = run(&file, &anonymize()).unwrap();
    assert_eq!(profile_of(&out, image::ImageFormat::Jpeg), Some(profile));
}

#[test]
fn a_rotated_jpeg_is_turned_upright_and_the_orientation_tag_goes() {
    let file = jpeg(32, 24, Some(6), None);
    let (out, _) = run(&file, &anonymize()).unwrap();
    let picture = decode(&out);
    assert_eq!((picture.width(), picture.height()), (24, 32));
    assert!(!contains(&out, SECRET) && !contains(&out, b"Exif"));
    let mut decoder = ImageReader::with_format(Cursor::new(out), image::ImageFormat::Jpeg)
        .into_decoder()
        .unwrap();
    assert_eq!(
        decoder.orientation().unwrap(),
        image::metadata::Orientation::NoTransforms
    );
}

#[test]
fn a_png_keeps_pixels_and_profile_and_loses_text_time_and_exif() {
    let profile = vec![9_u8; 200];
    let file = png(20, 10, Some(profile.clone()));
    let (out, format) = run(&file, &anonymize()).unwrap();
    assert_eq!(format, ImageFormat::Png);
    for gone in [SECRET, b"tEXt", b"eXIf", b"tIME"] {
        assert!(!contains(&out, gone), "{gone:?} survived");
    }
    assert_eq!(decode(&out).to_rgb8(), decode(&file).to_rgb8());
    assert_eq!(profile_of(&out, image::ImageFormat::Png), Some(profile));
}

#[test]
fn an_animated_gif_keeps_every_frame_and_loses_comments_and_xmp() {
    let file = gif(3);
    let (out, format) = run(&file, &anonymize()).unwrap();
    assert_eq!(format, ImageFormat::Gif);
    assert!(!contains(&out, SECRET) && !contains(&out, b"XMP Data"));
    let frames = |bytes: &[u8]| {
        use image::AnimationDecoder;
        image::codecs::gif::GifDecoder::new(Cursor::new(bytes.to_vec()))
            .unwrap()
            .into_frames()
            .count()
    };
    assert_eq!(frames(&out), 3);
    assert_eq!(frames(&file), 3);
}

#[test]
fn a_webp_loses_exif_and_xmp_and_stays_decodable() {
    let file = webp(16, 8);
    let (out, format) = run(&file, &anonymize()).unwrap();
    assert_eq!(format, ImageFormat::Webp);
    assert!(!contains(&out, SECRET) && !contains(&out, b"EXIF") && !contains(&out, b"XMP "));
    assert_eq!(decode(&out).to_rgb8(), decode(&file).to_rgb8());
}

#[test]
fn anonymizing_twice_changes_nothing_more() {
    for file in [
        jpeg(16, 16, None, None),
        png(16, 16, None),
        gif(2),
        webp(16, 8),
    ] {
        let (once, _) = run(&file, &anonymize()).unwrap();
        let (twice, _) = run(&once, &anonymize()).unwrap();
        assert_eq!(once, twice);
    }
}

#[test]
fn formats_the_path_cannot_clean_are_refused_not_sent() {
    let heic = [b"\0\0\0\x18ftypheic".as_slice(), &[0; 32]].concat();
    assert_eq!(
        run(&heic, &anonymize()).map(|_| ()),
        Err(ImageError::Unsupported(UnsupportedFormat::Heic))
    );
    let tiff = [b"II*\0\x08\0\0\0".as_slice(), &[0; 32]].concat();
    assert_eq!(
        run(&tiff, &anonymize()).map(|_| ()),
        Err(ImageError::Unsupported(UnsupportedFormat::Tiff))
    );
    let mp4 = [b"\0\0\0\x18ftypisom".as_slice(), &[0; 32]].concat();
    assert_eq!(
        run(&mp4, &anonymize()).map(|_| ()),
        Err(ImageError::NotAnImage)
    );
    assert_eq!(
        run(b"%PDF-1.7 ...", &anonymize()).map(|_| ()),
        Err(ImageError::NotAnImage)
    );
}

#[test]
fn the_extension_is_never_trusted() {
    // Bytes of a PNG are cleaned as a PNG whatever the caller named them.
    let (out, format) = run(&png(8, 8, None), &anonymize()).unwrap();
    assert_eq!(format, ImageFormat::Png);
    assert!(out.starts_with(b"\x89PNG"));
}

#[test]
fn a_crop_re_encodes_from_pixels_and_carries_no_metadata() {
    let file = jpeg(32, 24, None, None);
    let edit = AttachmentEdit {
        crop: Some(CropRect {
            x: 4,
            y: 2,
            width: 16,
            height: 10,
        }),
        ..AttachmentEdit::default()
    };
    let (out, format) = run(&file, &edit).unwrap();
    assert_eq!(format, ImageFormat::Jpeg);
    let picture = decode(&out);
    assert_eq!((picture.width(), picture.height()), (16, 10));
    assert!(!contains(&out, SECRET) && !contains(&out, b"Exif"));
}

#[test]
fn a_turn_and_crop_follow_the_picture_as_shown() {
    let file = png(32, 16, None);
    let edit = AttachmentEdit {
        quarter_turns: 1,
        crop: Some(CropRect {
            x: 0,
            y: 0,
            width: 16,
            height: 32,
        }),
        ..AttachmentEdit::default()
    };
    let (out, _) = run(&file, &edit).unwrap();
    let picture = decode(&out);
    assert_eq!((picture.width(), picture.height()), (16, 32));
}

#[test]
fn an_animation_cannot_be_cropped_or_turned() {
    let edit = AttachmentEdit {
        quarter_turns: 1,
        ..AttachmentEdit::default()
    };
    assert_eq!(run(&gif(2), &edit).map(|_| ()), Err(ImageError::Animated));
}

#[test]
fn a_crop_outside_the_picture_is_an_invalid_edit() {
    let edit = AttachmentEdit {
        crop: Some(CropRect {
            x: 30,
            y: 0,
            width: 10,
            height: 10,
        }),
        ..AttachmentEdit::default()
    };
    assert_eq!(
        run(&png(32, 24, None), &edit).map(|_| ()),
        Err(ImageError::InvalidEdit)
    );
}

#[test]
fn inspect_reports_the_displayed_size_and_animation() {
    let rotated = inspect(&mut Cursor::new(jpeg(32, 24, Some(6), None))).unwrap();
    assert_eq!(
        (rotated.width, rotated.height, rotated.animated),
        (24, 32, false)
    );
    assert_eq!(rotated.format, ImageFormat::Jpeg);
    let animated = inspect(&mut Cursor::new(gif(2))).unwrap();
    assert!(animated.animated);
    assert_eq!(
        inspect(&mut Cursor::new(b"\0\0\0\x18ftypheic".to_vec())).map(|_| ()),
        Err(ImageError::Unsupported(UnsupportedFormat::Heic))
    );
}

#[test]
fn a_picture_past_the_pixel_limit_is_refused_before_decoding() {
    let mut header = vec![0_u8; 13];
    header[..4].copy_from_slice(&10_000_u32.to_be_bytes());
    header[4..8].copy_from_slice(&10_000_u32.to_be_bytes());
    header[8] = 8;
    header[9] = 2;
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    file.extend(png_chunk(*b"IHDR", &header));
    file.extend(png_chunk(*b"IDAT", &[0; 4]));
    file.extend(png_chunk(*b"IEND", &[]));
    let edit = AttachmentEdit {
        quarter_turns: 1,
        ..AttachmentEdit::default()
    };
    assert_eq!(run(&file, &edit).map(|_| ()), Err(ImageError::TooLarge));
}

#[test]
fn a_fixed_upload_is_exact_size_re_encoded_and_under_its_cap() {
    let file = jpeg(640, 480, Some(6), Some(vec![5_u8; 64]));
    let upload = fixed(&mut Cursor::new(file), FixedImage::Avatar, 0, None).unwrap();
    assert_eq!(upload.format, ImageFormat::Jpeg);
    assert!(upload.bytes.len() <= FixedImage::Avatar.max_bytes());
    let picture = decode(&upload.bytes);
    assert_eq!((picture.width(), picture.height()), (512, 512));
    assert!(!contains(&upload.bytes, SECRET) && !contains(&upload.bytes, b"Exif"));
    assert_eq!(
        profile_of(&upload.bytes, image::ImageFormat::Jpeg),
        Some(vec![5_u8; 64])
    );
}

#[test]
fn a_banner_is_three_to_one() {
    let upload = fixed(
        &mut Cursor::new(png(300, 300, None)),
        FixedImage::Banner,
        0,
        None,
    )
    .unwrap();
    let picture = decode(&upload.bytes);
    assert_eq!((picture.width(), picture.height()), (1500, 500));
}

#[test]
fn an_animated_picture_cannot_become_a_fixed_upload() {
    assert_eq!(
        fixed(&mut Cursor::new(gif(2)), FixedImage::Avatar, 0, None).map(|_| ()),
        Err(ImageError::Animated)
    );
}

#[test]
fn only_a_cleanable_image_reads_as_one_to_clean_and_a_known_other_image_is_refused() {
    let mut photo = Cursor::new(b"\xFF\xD8\xFF\xE0rest of a jpeg".to_vec());
    assert_eq!(cleans_kind(&mut photo), Ok(true));
    let mut pdf = Cursor::new(b"%PDF-1.7 minutes".to_vec());
    assert_eq!(cleans_kind(&mut pdf), Ok(false));
    let mut heic = Cursor::new(b"\0\0\0\x18ftypheic\0\0\0\0mif1".to_vec());
    assert_eq!(
        cleans_kind(&mut heic),
        Err(ImageError::Unsupported(UnsupportedFormat::Heic))
    );
}

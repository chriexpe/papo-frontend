//! Prepara uma imagem para subir: encolhe até caber e converte.
//!
//! O servidor é exigente com figurinha — 512 px e 256 KB — e ninguém tem um
//! arquivo desse tamanho à mão. Em vez de recusar o que o usuário escolheu,
//! o cliente encolhe: reduz o lado maior, recodifica, e se ainda não couber
//! reduz de novo, até um piso a partir do qual não vale mais a pena.
//!
//! WebP sem perdas é o destino das imagens paradas: para arte chapada, que é
//! o que figurinha costuma ser, ele fica bem menor que PNG. GIF animado
//! continua GIF, quadro a quadro — não há como escrever WebP animado aqui, e
//! virar imagem parada perderia justamente o que faz dele um GIF.

use std::io::{Cursor, Read};
use std::path::Path;

use image::codecs::gif::{GifDecoder, GifEncoder};
use image::codecs::webp::WebPEncoder;
use image::{AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageEncoder, ImageFormat};

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SOURCE_PIXELS: u64 = 40_000_000;
const MAX_ANIMATION_PIXELS: u64 = 64_000_000;

#[derive(Clone, Copy, Debug)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
}

fn source_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_SOURCE_BYTES {
        return Err("Choose an image up to 32 MB".into());
    }
    let mut raw = Vec::new();
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() as u64 > MAX_SOURCE_BYTES {
        return Err("Choose an image up to 32 MB".into());
    }
    Ok(raw)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err("Choose an image up to 40 megapixels".into());
    }
    Ok(())
}

/// Dimensions follow EXIF orientation, matching Android's image preview.
pub fn inspect_source(path: &Path) -> Result<SourceInfo, String> {
    let raw = source_bytes(path)?;
    let mut decoder = image::ImageReader::new(Cursor::new(&raw))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let (mut width, mut height) = decoder.dimensions();
    validate_dimensions(width, height)?;
    use image::metadata::Orientation;
    if matches!(
        decoder.orientation().map_err(|e| e.to_string())?,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        std::mem::swap(&mut width, &mut height);
    }
    Ok(SourceInfo { width, height })
}

/// Imagem pronta para o corpo da requisição.
#[derive(Clone, Debug)]
pub struct Prepared {
    /// Conteúdo em base64, como o backend recebe.
    pub blob: String,
    /// `WEBP` ou `GIF`, os dois nomes do enum do contrato que usamos.
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    /// O arquivo foi reduzido para caber.
    pub shrunk: bool,
}

/// Abaixo disto a figurinha já não se reconhece; melhor dizer que não deu.
const FLOOR: u32 = 64;

/// Recorte escolhido no editor, em frações da imagem original (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Crop {
    /// Em pixels da imagem, sem sair dela e com pelo menos 1×1.
    fn pixels(self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let clamp = |value: f32| value.clamp(0.0, 1.0);
        let x = (clamp(self.x) * width as f32).round() as u32;
        let y = (clamp(self.y) * height as f32).round() as u32;
        let w = ((clamp(self.width) * width as f32).round() as u32)
            .clamp(1, width.saturating_sub(x).max(1));
        let h = ((clamp(self.height) * height as f32).round() as u32)
            .clamp(1, height.saturating_sub(y).max(1));
        (x.min(width - 1), y.min(height - 1), w, h)
    }
}

/// Encolhe e converte até caber em `max_side` pixels e `max_bytes` bytes.
pub fn fit(path: &Path, max_side: u32, max_bytes: usize) -> Result<Prepared, String> {
    fit_cropped(path, None, max_side, max_side, max_bytes)
}

/// Recorta (quando pedido), encolhe até caber na caixa `max_width` ×
/// `max_height` e converte até caber em `max_bytes`. É o caminho da foto,
/// do banner e do ícone do servidor: qualquer imagem entra, o que sai é o
/// que o servidor aceita.
pub fn fit_cropped(
    path: &Path,
    crop: Option<Crop>,
    max_width: u32,
    max_height: u32,
    max_bytes: usize,
) -> Result<Prepared, String> {
    if max_width == 0 || max_height == 0 || max_bytes == 0 {
        return Err("Choose a valid output size".into());
    }
    if crop.is_some_and(|crop| {
        [crop.x, crop.y, crop.width, crop.height]
            .iter()
            .any(|v| !v.is_finite())
            || crop.width <= 0.0
            || crop.height <= 0.0
    }) {
        return Err("Choose a valid crop".into());
    }
    let raw = source_bytes(path)?;
    let format = image::guess_format(&raw).map_err(|_| "formato não reconhecido".to_owned())?;
    let mut decoder = image::ImageReader::new(Cursor::new(&raw))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    validate_dimensions(width, height)?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;

    if matches!(
        format,
        ImageFormat::Gif | ImageFormat::WebP | ImageFormat::Png
    ) && let Some(prepared) =
        animated_gif(&raw, crop, max_width, max_height, max_bytes, orientation)?
    {
        return Ok(prepared);
    }

    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    image.apply_orientation(orientation);
    let original = (image.width(), image.height());
    let image = match crop {
        Some(crop) => {
            let (x, y, w, h) = crop.pixels(image.width(), image.height());
            image.crop_imm(x, y, w, h)
        }
        None => image,
    };
    // Foto sem transparência vira JPEG quando o WebP sem perdas não cabe:
    // ruído de câmera não comprime sem perdas, e um banner de 1536 px
    // passaria dos 2 MB antes de encolher até ficar borrado.
    let opaque =
        !image.color().has_alpha() || image.to_rgba8().pixels().all(|pixel| pixel[3] == 255);
    let (mut width, mut height) = (max_width, max_height);
    loop {
        let scaled = scale_to(&image, width, height);
        let mut encoded = webp(&scaled)?;
        let mut format = "WEBP";
        if encoded.len() > max_bytes && opaque {
            encoded = jpeg(&scaled)?;
            format = "JPEG";
        }
        if encoded.len() <= max_bytes || width.max(height) <= FLOOR {
            if encoded.len() > max_bytes {
                return Err(format!(
                    "não coube em {} KB nem a {FLOOR} px",
                    max_bytes / 1024
                ));
            }
            return Ok(Prepared {
                blob: encode64(&encoded),
                format,
                width: scaled.width(),
                height: scaled.height(),
                bytes: encoded.len(),
                shrunk: (scaled.width(), scaled.height()) != original,
            });
        }
        width = (width * 3 / 4).max(FLOOR);
        height = (height * 3 / 4).max(FLOOR);
    }
}

/// `None` quando o GIF tem um quadro só: aí ele segue o caminho normal e
/// vira WebP, que é bem menor.
fn animated_gif(
    raw: &[u8],
    crop: Option<Crop>,
    max_width: u32,
    max_height: u32,
    max_bytes: usize,
    orientation: image::metadata::Orientation,
) -> Result<Option<Prepared>, String> {
    let Some(decoded) = animation_frames(raw)? else {
        return Ok(None);
    };
    let mut frames = Vec::new();
    let mut pixels = 0u64;
    for frame in decoded {
        let frame = frame.map_err(|error| error.to_string())?;
        pixels += u64::from(frame.buffer().width()) * u64::from(frame.buffer().height());
        if pixels > MAX_ANIMATION_PIXELS || frames.len() >= 1000 {
            return Err("Choose a shorter animation or a lower resolution".into());
        }
        frames.push(frame);
    }
    if frames.len() <= 1 {
        return Ok(None);
    }
    let original = frames
        .first()
        .map(|frame| (frame.buffer().width(), frame.buffer().height()))
        .unwrap_or((0, 0));
    // O recorte vale para todos os quadros: a animação inteira é recortada.
    let frames: Vec<(DynamicImage, image::Delay)> = frames
        .into_iter()
        .map(|frame| {
            let delay = frame.delay();
            let mut image = DynamicImage::ImageRgba8(frame.into_buffer());
            image.apply_orientation(orientation);
            let image = match crop {
                Some(crop) => {
                    let (x, y, w, h) = crop.pixels(image.width(), image.height());
                    image.crop_imm(x, y, w, h)
                }
                None => image,
            };
            (image, delay)
        })
        .collect();

    let (mut width, mut height) = (max_width, max_height);
    loop {
        let mut out = Vec::new();
        let mut first_size = (0, 0);
        {
            let mut encoder = GifEncoder::new(&mut out);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .map_err(|error| error.to_string())?;
            for (image, delay) in &frames {
                let small = scale_to(image, width, height).to_rgba8();
                if first_size == (0, 0) {
                    first_size = (small.width(), small.height());
                }
                encoder
                    .encode_frame(Frame::from_parts(small, 0, 0, *delay))
                    .map_err(|error| error.to_string())?;
            }
        }
        if out.len() <= max_bytes {
            return Ok(Some(Prepared {
                blob: encode64(&out),
                format: "GIF",
                width: first_size.0,
                height: first_size.1,
                bytes: out.len(),
                shrunk: first_size != original,
            }));
        }
        if width.max(height) <= FLOOR {
            return Err(format!(
                "o GIF animado não coube em {} KB nem a {FLOOR} px",
                max_bytes / 1024
            ));
        }
        width = (width * 3 / 4).max(FLOOR);
        height = (height * 3 / 4).max(FLOOR);
    }
}

/// Decode animation before any static-image conversion can discard its frames.
pub fn animation_frames(raw: &[u8]) -> Result<Option<image::Frames<'_>>, String> {
    let frames = match image::guess_format(raw).map_err(|error| error.to_string())? {
        ImageFormat::Gif => GifDecoder::new(Cursor::new(raw))
            .map_err(|error| error.to_string())?
            .into_frames(),
        ImageFormat::WebP => {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(raw))
                .map_err(|error| error.to_string())?;
            if !decoder.has_animation() {
                return Ok(None);
            }
            decoder.into_frames()
        }
        ImageFormat::Png => {
            let decoder = image::codecs::png::PngDecoder::new(Cursor::new(raw))
                .map_err(|error| error.to_string())?;
            if !decoder.is_apng().map_err(|error| error.to_string())? {
                return Ok(None);
            }
            decoder
                .apng()
                .map_err(|error| error.to_string())?
                .into_frames()
        }
        _ => return Ok(None),
    };
    Ok(Some(frames))
}

/// Reduz até caber na caixa, mantendo a proporção. Imagem menor que isso é
/// deixada em paz: ampliar só faria peso e borrão.
fn scale_to(image: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    if image.width() <= width && image.height() <= height {
        return image.clone();
    }
    image.resize(width, height, image::imageops::FilterType::Lanczos3)
}

fn webp(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let rgba = image.to_rgba8();
    let mut out = Vec::new();
    WebPEncoder::new_lossless(&mut out)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
    Ok(out)
}

fn jpeg(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let rgb = image.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| error.to_string())?;
    Ok(out)
}

fn encode64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_sources_and_invalid_crops_are_rejected_before_decode() {
        let path = temp("oversized.png");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();
        assert!(inspect_source(&path).unwrap_err().contains("32 MB"));
        assert!(fit(&path, 512, 256 * 1024).unwrap_err().contains("32 MB"));
        assert!(
            fit_cropped(
                &path,
                Some(Crop {
                    x: f32::NAN,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0
                }),
                512,
                512,
                1024
            )
            .unwrap_err()
            .contains("crop")
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn exif_orientation_matches_preview_dimensions_and_export() {
        let path = temp("oriented.jpg");
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
        let exif = vec![
            0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0,
            0,
        ];
        encoder.set_exif_metadata(exif).unwrap();
        encoder
            .encode_image(&DynamicImage::new_rgb8(12, 8))
            .unwrap();
        std::fs::write(&path, bytes).unwrap();
        let info = inspect_source(&path).unwrap();
        assert_eq!((info.width, info.height), (8, 12));
        let prepared = fit(&path, 512, 256 * 1024).unwrap();
        assert_eq!((prepared.width, prepared.height), (8, 12));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn webp_and_apng_keep_all_frames_when_prepared_and_cropped() {
        for (name, bytes) in [
            (
                "animated.webp",
                include_bytes!("../../../tests/fixtures/animated-sticker.webp").as_slice(),
            ),
            (
                "animated.png",
                include_bytes!("../../../tests/fixtures/animated-sticker.png").as_slice(),
            ),
        ] {
            let path = temp(name);
            std::fs::write(&path, bytes).unwrap();
            let prepared = fit_cropped(
                &path,
                Some(Crop {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                }),
                512,
                512,
                256 << 10,
            )
            .unwrap();
            use base64::Engine as _;
            let encoded = base64::engine::general_purpose::STANDARD
                .decode(prepared.blob)
                .unwrap();
            let frames = animation_frames(&encoded)
                .unwrap()
                .unwrap()
                .collect_frames()
                .unwrap();
            assert_eq!(frames.len(), 2);
            assert_eq!(frames[0].buffer().dimensions(), (4, 6));
            assert_ne!(
                frames[0].buffer().get_pixel(0, 0),
                frames[1].buffer().get_pixel(0, 0)
            );
            assert_eq!(frames[0].delay().numer_denom_ms(), (80, 1));
            assert_eq!(frames[1].delay().numer_denom_ms(), (120, 1));
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("papo-prepare-test");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    /// Uma imagem grande e ruidosa é o pior caso: sem perdas, ruído não
    /// comprime. Ela tem de sair encolhida, mas tem de sair.
    #[test]
    fn imagem_grande_encolhe_ate_caber() {
        let mut image = image::RgbaImage::new(1400, 1000);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let noise = ((x * 7 + y * 13) % 251) as u8;
            // Um pouco de transparência mantém o caminho sem perdas: é ele
            // que este teste quer ver encolher (foto opaca vira JPEG).
            *pixel = image::Rgba([noise, noise.wrapping_mul(3), noise.wrapping_add(90), 250]);
        }
        let path = temp("grande.png");
        image.save(&path).unwrap();

        let prepared = fit(&path, 512, 256 * 1024).expect("deveria caber");
        assert_eq!(prepared.format, "WEBP");
        assert!(prepared.bytes <= 256 * 1024, "{} bytes", prepared.bytes);
        assert!(prepared.width <= 512 && prepared.height <= 512);
        assert!(prepared.shrunk);
    }

    /// Imagem que já cabe não é mexida no tamanho.
    #[test]
    fn imagem_pequena_mantem_o_tamanho() {
        let image = image::RgbaImage::from_pixel(64, 48, image::Rgba([10, 200, 120, 255]));
        let path = temp("pequena.png");
        image.save(&path).unwrap();

        let prepared = fit(&path, 512, 256 * 1024).unwrap();
        assert_eq!((prepared.width, prepared.height), (64, 48));
        assert!(!prepared.shrunk);
    }

    /// O recorte do editor sai na proporção pedida: um banner 3:1 de uma
    /// foto 4:3 qualquer.
    #[test]
    fn recorte_do_banner_sai_em_tres_por_um() {
        let image = image::RgbaImage::from_pixel(4000, 3000, image::Rgba([30, 90, 160, 255]));
        let path = temp("foto.png");
        image.save(&path).unwrap();

        // Faixa de largura total com 1/3 da altura em pixels da largura.
        let crop = Crop {
            x: 0.0,
            y: 0.25,
            width: 1.0,
            height: 4000.0 / 3.0 / 3000.0,
        };
        let prepared = fit_cropped(&path, Some(crop), 1536, 512, 2 * 1024 * 1024).unwrap();
        assert_eq!((prepared.width, prepared.height), (1536, 512));
    }

    /// Foto opaca e ruidosa que não cabe sem perdas vira JPEG em vez de
    /// encolher até borrar.
    #[test]
    fn foto_opaca_grande_vira_jpeg() {
        // Ruído de verdade (xorshift): 1536 × 512 × 3 bytes não comprimem
        // sem perdas abaixo de 2 MB.
        let mut state = 0x9E37_79B9_u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        };
        let mut image = image::RgbImage::new(1536, 512);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgb([next(), next(), next()]);
        }
        let path = temp("ruido.png");
        image.save(&path).unwrap();

        let prepared = fit_cropped(&path, None, 1536, 512, 2 * 1024 * 1024).unwrap();
        assert_eq!(prepared.format, "JPEG");
        assert_eq!((prepared.width, prepared.height), (1536, 512));
    }

    /// O recorte nunca sai da imagem, mesmo com frações fora do lugar.
    #[test]
    fn recorte_fica_dentro_da_imagem() {
        let crop = Crop {
            x: 0.9,
            y: -0.2,
            width: 0.5,
            height: 2.0,
        };
        let (x, y, w, h) = crop.pixels(100, 50);
        assert!(x + w <= 100 && y + h <= 50, "{x} {y} {w} {h}");
    }

    /// GIF animado continua GIF e continua animado: virar imagem parada
    /// perderia justamente o que faz dele um GIF.
    #[test]
    fn gif_animado_continua_animado() {
        let path = temp("animado.gif");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = GifEncoder::new(file);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for step in 0..4u8 {
                let buffer = image::RgbaImage::from_pixel(
                    600,
                    600,
                    image::Rgba([step.wrapping_mul(60), 40, 200, 255]),
                );
                encoder
                    .encode_frame(Frame::from_parts(
                        buffer,
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(100, 1),
                    ))
                    .unwrap();
            }
        }

        let prepared = fit(&path, 512, 256 * 1024).expect("deveria caber");
        assert_eq!(prepared.format, "GIF");
        assert!(prepared.bytes <= 256 * 1024);

        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&prepared.blob)
            .unwrap();
        let frames = GifDecoder::new(Cursor::new(bytes))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(frames.len(), 4, "a animação tem de sobreviver");
        assert!(frames[0].buffer().width() <= 512);
    }
}

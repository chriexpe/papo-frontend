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

use std::io::Cursor;
use std::path::Path;

use image::codecs::gif::{GifDecoder, GifEncoder};
use image::codecs::webp::WebPEncoder;
use image::{AnimationDecoder, DynamicImage, Frame, ImageEncoder, ImageFormat};

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

/// Encolhe e converte até caber em `max_side` pixels e `max_bytes` bytes.
pub fn fit(path: &Path, max_side: u32, max_bytes: usize) -> Result<Prepared, String> {
    let raw = std::fs::read(path).map_err(|error| error.to_string())?;
    let format = image::guess_format(&raw).map_err(|_| "formato não reconhecido".to_owned())?;

    if format == ImageFormat::Gif {
        if let Some(prepared) = animated_gif(&raw, max_side, max_bytes)? {
            return Ok(prepared);
        }
    }

    let image = image::load_from_memory(&raw).map_err(|error| error.to_string())?;
    let original = (image.width(), image.height());
    let mut side = max_side;
    loop {
        let scaled = scale(&image, side);
        let encoded = webp(&scaled)?;
        if encoded.len() <= max_bytes || side <= FLOOR {
            if encoded.len() > max_bytes {
                return Err(format!(
                    "não coube em {} KB nem a {FLOOR} px",
                    max_bytes / 1024
                ));
            }
            return Ok(Prepared {
                blob: encode64(&encoded),
                format: "WEBP",
                width: scaled.width(),
                height: scaled.height(),
                bytes: encoded.len(),
                shrunk: (scaled.width(), scaled.height()) != original,
            });
        }
        side = (side * 3 / 4).max(FLOOR);
    }
}

/// `None` quando o GIF tem um quadro só: aí ele segue o caminho normal e
/// vira WebP, que é bem menor.
fn animated_gif(raw: &[u8], max_side: u32, max_bytes: usize) -> Result<Option<Prepared>, String> {
    let decoder = GifDecoder::new(Cursor::new(raw)).map_err(|error| error.to_string())?;
    let frames: Vec<Frame> = decoder
        .into_frames()
        .collect_frames()
        .map_err(|error| error.to_string())?;
    if frames.len() <= 1 {
        return Ok(None);
    }
    let original = frames
        .first()
        .map(|frame| (frame.buffer().width(), frame.buffer().height()))
        .unwrap_or((0, 0));

    let mut side = max_side;
    loop {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .map_err(|error| error.to_string())?;
            for frame in &frames {
                let delay = frame.delay();
                let small = scale(
                    &DynamicImage::ImageRgba8(frame.buffer().clone()),
                    side,
                )
                .to_rgba8();
                encoder
                    .encode_frame(Frame::from_parts(small, 0, 0, delay))
                    .map_err(|error| error.to_string())?;
            }
        }
        if out.len() <= max_bytes {
            let first = scale(
                &DynamicImage::ImageRgba8(frames[0].buffer().clone()),
                side,
            );
            return Ok(Some(Prepared {
                blob: encode64(&out),
                format: "GIF",
                width: first.width(),
                height: first.height(),
                bytes: out.len(),
                shrunk: (first.width(), first.height()) != original,
            }));
        }
        if side <= FLOOR {
            return Err(format!(
                "o GIF animado não coube em {} KB nem a {FLOOR} px",
                max_bytes / 1024
            ));
        }
        side = (side * 3 / 4).max(FLOOR);
    }
}

/// Reduz o lado maior para `side`, mantendo a proporção. Imagem menor que
/// isso é deixada em paz: ampliar só faria peso e borrão.
fn scale(image: &DynamicImage, side: u32) -> DynamicImage {
    if image.width() <= side && image.height() <= side {
        return image.clone();
    }
    image.resize(side, side, image::imageops::FilterType::Lanczos3)
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

fn encode64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            *pixel = image::Rgba([noise, noise.wrapping_mul(3), noise.wrapping_add(90), 255]);
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

    /// GIF animado continua GIF e continua animado: virar imagem parada
    /// perderia justamente o que faz dele um GIF.
    #[test]
    fn gif_animado_continua_animado() {
        let path = temp("animado.gif");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = GifEncoder::new(file);
            encoder.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
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

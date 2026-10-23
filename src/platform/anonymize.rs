//! "Anonimizar" um anexo antes de enviar: tira o que pode identificar quem
//! mandou e esconde o nome original atrás de um nome sem sentido.
//!
//! Imagens são decodificadas e gravadas de novo, o que descarta EXIF (GPS,
//! modelo da câmera, data), XMP, perfis e comentários — nada além dos pixels
//! sobrevive. A orientação do EXIF é aplicada antes, para a foto não girar.
//! Qualquer outro tipo só ganha o nome novo; o conteúdo segue como está.

use std::hash::{BuildHasher, Hasher, RandomState};
#[cfg(test)]
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::ImageFormat;
#[cfg(test)]
use image::DynamicImage;

use papo_core::api::client::Upload;

/// Resultado de anonimizar um anexo.
pub struct Anonymized {
    pub upload: Upload,
    /// Os metadados internos foram realmente descartados (só imagens).
    pub scrubbed: bool,
}

/// Nome sem relação com o original: 16 hex e a extensão (sem acentos nem
/// símbolos), que o servidor e quem recebe usam para reconhecer o tipo.
pub fn gibberish_name(original: &str) -> String {
    let extension: String = Path::new(original)
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    let mut name = String::with_capacity(24);
    for _ in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        );
        name.push_str(&format!("{:08x}", hasher.finish() as u32));
    }
    if !extension.is_empty() {
        name.push('.');
        name.push_str(&extension);
    }
    name
}

/// Rewrite pixels and animation timing, omitting identifying metadata.
pub fn scrub_image(bytes: &[u8]) -> Option<(Vec<u8>, ImageFormat)> {
    papo_core::media_prepare::strip_image_metadata(bytes).ok()
}

/// Anonimiza um anexo, gravando o resultado em `dir`. O original nunca é
/// tocado. Em caso de erro devolve o motivo.
pub fn anonymize(upload: &Upload, dir: &Path) -> Result<Anonymized, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let bytes_if_image = if upload.mime.starts_with("image/") {
        let bytes = std::fs::read(&upload.path).map_err(|error| error.to_string())?;
        Some(
            scrub_image(&bytes)
                .ok_or_else(|| "Cannot remove metadata from this image".to_owned())?,
        )
    } else {
        None
    };
    let mut name = gibberish_name(&upload.name);
    let mut mime = upload.mime.clone();
    if let Some((_, format)) = &bytes_if_image {
        let (extension, content_type) = match format {
            ImageFormat::Gif => ("gif", "image/gif"),
            ImageFormat::Png => ("png", "image/png"),
            ImageFormat::WebP => ("webp", "image/webp"),
            ImageFormat::Jpeg => ("jpg", "image/jpeg"),
            _ => unreachable!(),
        };
        name = Path::new(&name)
            .with_extension(extension)
            .to_string_lossy()
            .into_owned();
        mime = content_type.to_owned();
    }
    let target: PathBuf = dir.join(&name);

    let scrubbed = bytes_if_image.is_some();
    match bytes_if_image {
        Some((bytes, _)) => std::fs::write(&target, &bytes).map_err(|e| e.to_string())?,
        None => {
            std::fs::copy(&upload.path, &target).map_err(|e| e.to_string())?;
        }
    }
    let size = std::fs::metadata(&target).map_or(upload.size, |meta| meta.len());
    Ok(Anonymized {
        upload: Upload {
            path: target,
            name,
            mime,
            size,
            spoiler: upload.spoiler,
            anonymize: false,
        },
        scrubbed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymizing_animated_images_preserves_pixels_and_timing() {
        for bytes in [
            include_bytes!("../../tests/fixtures/animated-sticker.webp").as_slice(),
            include_bytes!("../../tests/fixtures/animated-sticker.png").as_slice(),
        ] {
            let (encoded, format) = scrub_image(bytes).unwrap();
            assert_eq!(format, ImageFormat::Gif);
            let frames = crate::media::prepare::animation_frames(&encoded)
                .unwrap()
                .unwrap()
                .collect_frames()
                .unwrap();
            assert_eq!(frames.len(), 2);
            assert_ne!(
                frames[0].buffer().get_pixel(0, 0),
                frames[1].buffer().get_pixel(0, 0)
            );
            assert_eq!(frames[0].delay().numer_denom_ms(), (80, 1));
            assert_eq!(frames[1].delay().numer_denom_ms(), (120, 1));
        }
    }

    fn png_with_text_chunk() -> Vec<u8> {
        let img = DynamicImage::new_rgb8(4, 3);
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        let mut bytes = out.into_inner();
        // tEXt "Author\0chris" antes do IEND.
        let payload = b"Author\0chris";
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(payload);
        let mut crc_data = b"tEXt".to_vec();
        crc_data.extend_from_slice(payload);
        chunk.extend_from_slice(&crc32(&crc_data).to_be_bytes());
        let iend = bytes.len() - 12;
        bytes.splice(iend..iend, chunk);
        bytes
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn names_are_unrelated_and_keep_the_extension() {
        let a = gibberish_name("Férias com a Maria (GPS).JPG");
        let b = gibberish_name("Férias com a Maria (GPS).JPG");
        assert!(a.ends_with(".jpg"));
        assert_eq!(a.len(), 16 + 4);
        assert_ne!(a, b);
        assert!(!a.contains("Maria"));
        assert!(!gibberish_name("noext").contains('.'));
    }

    #[test]
    fn scrubbing_drops_ancillary_chunks() {
        let original = png_with_text_chunk();
        assert!(original.windows(6).any(|w| w == b"Author"));
        let (clean, format) = scrub_image(&original).unwrap();
        assert_eq!(format, ImageFormat::Png);
        assert!(!clean.windows(6).any(|w| w == b"Author"));
        let decoded = image::load_from_memory(&clean).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (4, 3));
    }

    #[test]
    fn non_images_and_gifs_are_left_alone() {
        assert!(scrub_image(b"%PDF-1.7 hello").is_none());
        assert!(scrub_image(b"GIF89a\x01\x00\x01\x00").is_none());
    }
}

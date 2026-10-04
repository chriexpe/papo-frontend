//! Emoji em imagem, não em glifo.
//!
//! O egui desenha fontes em tons de cinza: não conhece as camadas de cor do
//! COLR nem os bitmaps do CBDT, então um emoji sai como contorno.
//!
//! No Android a plataforma é a autoridade: Canvas/Minikin rasteriza a
//! sequência usando o fallback emoji real do aparelho/OEM e devolve os pixels
//! prontos. Nos desktops continuamos rasterizando a fonte emoji do sistema
//! com swash. Em ambos os casos o resultado vira uma textura egui em cache.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureOptions};
#[cfg(not(target_os = "android"))]
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
#[cfg(not(target_os = "android"))]
use swash::shape::ShapeContext;
#[cfg(not(target_os = "android"))]
use swash::FontRef;

/// Tamanho de rasterização: os emojis aparecem entre 16 e 30 pixels, então
/// uma matriz de 72 cobre todos com folga e só precisa ser feita uma vez.
const RASTER_PX: f32 = 72.0;

/// Caminhos comuns da fonte colorida nos desktops; evita varrer o sistema
/// inteiro. Android não entra aqui: lá a própria plataforma desenha o emoji.
#[cfg(not(target_os = "android"))]
const CANDIDATES: [&str; 6] = [
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto-color-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/TTF/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/joypixels/JoyPixels.ttf",
];

pub struct EmojiRaster {
    /// Bytes da fonte e o índice da face dentro dela (desktop).
    #[cfg(not(target_os = "android"))]
    font: Option<(Vec<u8>, u32)>,
    #[cfg(not(target_os = "android"))]
    looked_up: bool,
    #[cfg(not(target_os = "android"))]
    shape: ShapeContext,
    #[cfg(not(target_os = "android"))]
    scale: ScaleContext,
    /// `None` guardado significa "essa fonte não tem esse emoji".
    cache: HashMap<String, Option<TextureHandle>>,
}

impl Default for EmojiRaster {
    fn default() -> Self {
        Self::new()
    }
}

impl EmojiRaster {
    pub fn new() -> Self {
        Self {
            #[cfg(not(target_os = "android"))]
            font: None,
            #[cfg(not(target_os = "android"))]
            looked_up: false,
            #[cfg(not(target_os = "android"))]
            shape: ShapeContext::new(),
            #[cfg(not(target_os = "android"))]
            scale: ScaleContext::new(),
            cache: HashMap::new(),
        }
    }

    /// Textura do emoji, rasterizada na primeira vez que for pedida.
    pub fn texture(&mut self, ctx: &egui::Context, emoji: &str) -> Option<&TextureHandle> {
        if !self.cache.contains_key(emoji) {
            let rendered = self.raster(emoji).map(|image| {
                ctx.load_texture(format!("emoji:{emoji}"), image, TextureOptions::LINEAR)
            });
            self.cache.insert(emoji.to_owned(), rendered);
        }
        self.cache.get(emoji).and_then(Option::as_ref)
    }

    #[cfg(not(target_os = "android"))]
    fn ensure_font(&mut self) {
        if self.looked_up {
            return;
        }
        self.looked_up = true;

        for path in CANDIDATES {
            if let Ok(data) = std::fs::read(path)
                && FontRef::from_index(&data, 0).is_some()
            {
                log::debug!("emoji colorido: {path}");
                self.font = Some((data, 0));
                return;
            }
        }

        // Nada nos lugares de sempre: pergunta ao fontconfig.
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let face = db.faces().find(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.to_lowercase().contains("emoji"))
        });
        let Some(face) = face else {
            log::info!("sem fonte de emoji colorida: os emojis ficam em traço");
            return;
        };
        let index = face.index;
        if let fontdb::Source::File(path) = &face.source
            && let Ok(data) = std::fs::read(path)
        {
            self.font = Some((data, index));
        }
    }

    #[cfg(target_os = "android")]
    fn raster(&mut self, emoji: &str) -> Option<ColorImage> {
        // Não procure nem abra NotoColorEmoji.ttf aqui. O desenho pertence ao
        // Android: Canvas/Minikin resolve a fonte/fallback emoji instalada no
        // aparelho (Google, OEM ou outra substituição do sistema).
        let png = crate::platform::jvm::call_activity_bytes_with_text_int(
            "renderSystemEmoji",
            emoji,
            RASTER_PX as i32,
        )?;
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .ok()?
            .to_rgba8();
        let size = [decoded.width() as usize, decoded.height() as usize];
        if size[0] == 0 || size[1] == 0 {
            return None;
        }
        let pixels = decoded
            .pixels()
            .map(|pixel| {
                let [r, g, b, a] = pixel.0;
                egui::Color32::from_rgba_unmultiplied(r, g, b, a)
            })
            .collect();
        Some(ColorImage {
            size,
            source_size: egui::Vec2::new(size[0] as f32, size[1] as f32),
            pixels,
        })
    }

    #[cfg(not(target_os = "android"))]
    fn raster(&mut self, emoji: &str) -> Option<ColorImage> {
        self.ensure_font();
        let (data, index) = self.font.as_ref()?;
        let font = FontRef::from_index(data, *index as usize)?;

        // Emoji quase nunca é um caractere só: bandeiras, tons de pele e
        // sequências com ZWJ viram um glifo só depois da substituição.
        let mut glyph = None;
        let mut shaper = self.shape.builder(font).size(RASTER_PX).build();
        shaper.add_str(emoji);
        shaper.shape_with(|cluster| {
            for candidate in cluster.glyphs {
                if glyph.is_none() && candidate.id != 0 {
                    glyph = Some(candidate.id);
                }
            }
        });
        let glyph = glyph?;

        let mut scaler = self.scale.builder(font).size(RASTER_PX).hint(false).build();
        let image = Render::new(&[
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::ColorOutline(0),
            Source::Outline,
        ])
        .render(&mut scaler, glyph)?;

        let width = image.placement.width as usize;
        let height = image.placement.height as usize;
        if width == 0 || height == 0 {
            return None;
        }

        let pixels = match image.content {
            swash::scale::image::Content::Color => image
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|[r, g, b, a]| egui::Color32::from_rgba_unmultiplied(*r, *g, *b, *a))
                .collect(),
            // Sem cor na fonte: a máscara vira um emoji branco, que o
            // chamador tinge com a cor do texto.
            _ => image
                .data
                .iter()
                .map(|alpha| egui::Color32::from_white_alpha(*alpha))
                .collect(),
        };

        Some(ColorImage {
            size: [width, height],
            source_size: egui::Vec2::new(width as f32, height as f32),
            pixels,
        })
    }
}

/// Faixas de emoji de presença gráfica — o suficiente para decidir se vale
/// olhar caractere por caractere.
pub fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF   // pictogramas, faces, bandeiras e extensões
        | 0x2600..=0x27BF   // símbolos diversos e dingbats
        | 0x2B00..=0x2BFF
        | 0x2190..=0x21FF   // setas com variante emoji
        | 0xFE0F            // seletor de variação
        | 0x20E3            // teclas combinadas
    )
}

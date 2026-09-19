//! Tokens de design e aplicação do estilo no egui.
//!
//! A linguagem visual segue as HIG da Apple adaptadas para desktop: duas
//! camadas (conteúdo opaco embaixo, controles translúcidos em cima), escala
//! tipográfica com corpo de 13 pt, grade de 4 pt e uma única cor de destaque.

#![allow(dead_code)] // o sistema de design existe inteiro, as telas chegam depois

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle};

// ---------------------------------------------------------------------------
// Aparência
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn is_dark(self) -> bool {
        matches!(self, Appearance::Dark)
    }
}

// ---------------------------------------------------------------------------
// Grade e raios — grade de 4 pt
// ---------------------------------------------------------------------------

pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
    pub const XXL: f32 = 20.0;
    pub const XXXL: f32 = 24.0;
}

pub mod radius {
    /// Controles pequenos (botões de ícone, chips).
    pub const CONTROL: u8 = 6;
    /// Campos de texto e linhas selecionáveis.
    pub const FIELD: u8 = 8;
    /// Cartões e balões.
    pub const CARD: u8 = 10;
    /// Popovers, folhas e janelas internas.
    pub const SHEET: u8 = 12;
}

/// Alvo de clique padrão no desktop (HIG: 28×28 pt, mínimo 20×20 pt).
pub const HIT_TARGET: f32 = 28.0;

// ---------------------------------------------------------------------------
// Escala tipográfica (pt = pontos do egui)
// ---------------------------------------------------------------------------

/// Família de exibição (Inter Display) para títulos grandes.
pub fn display_family() -> FontFamily {
    FontFamily::Name("display".into())
}

/// Escala aplicada a toda a rampa tipográfica. 1.0 = corpo de 13 pt.
static FONT_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1_000);

pub fn font_scale() -> f32 {
    FONT_SCALE.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0
}

/// Ajusta a rampa para o tamanho de fonte do sistema (corpo em pixels).
pub fn set_body_size(body_px: f32) {
    let scale = (body_px / 13.0).clamp(0.75, 2.0);
    FONT_SCALE.store((scale * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed);
}

pub mod text {
    use super::{display_family, font_scale, FontFamily, FontId};

    fn sized(size: f32, family: FontFamily) -> FontId {
        FontId::new((size * font_scale()).round(), family)
    }

    pub fn title1() -> FontId {
        sized(22.0, display_family())
    }
    pub fn title2() -> FontId {
        sized(17.0, display_family())
    }
    pub fn title3() -> FontId {
        sized(15.0, FontFamily::Name("medium".into()))
    }
    /// Rótulo com ênfase: nomes de autor, itens selecionados.
    pub fn headline() -> FontId {
        sized(13.0, FontFamily::Name("medium".into()))
    }
    /// Corpo da interface (HIG desktop: 13 pt).
    pub fn body() -> FontId {
        sized(13.0, FontFamily::Proportional)
    }
    /// Corpo das mensagens — conteúdo lido por longos períodos, 1 pt maior.
    pub fn message() -> FontId {
        sized(14.0, FontFamily::Proportional)
    }
    pub fn callout() -> FontId {
        sized(12.0, FontFamily::Proportional)
    }
    pub fn subheadline() -> FontId {
        sized(11.0, FontFamily::Name("medium".into()))
    }
    /// Cabeçalhos de seção em versalete (mínimo HIG desktop: 10 pt).
    pub fn caption() -> FontId {
        sized(10.0, FontFamily::Name("medium".into()))
    }
    pub fn footnote() -> FontId {
        sized(10.0, FontFamily::Proportional)
    }
    /// Glifos do Phosphor, na família exclusiva de ícones.
    pub fn icon(size: f32) -> FontId {
        FontId::new((size * font_scale()).round(), FontFamily::Name("icons".into()))
    }
    pub fn mono() -> FontId {
        sized(12.5, FontFamily::Monospace)
    }
}

// ---------------------------------------------------------------------------
// Cores
// ---------------------------------------------------------------------------

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Paleta resolvida para uma aparência.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub appearance: Appearance,

    /// Camada de conteúdo: opaca, onde ficam mensagens e mídia.
    pub content_bg: Color32,
    /// Fundo das superfícies elevadas (popovers, menus, diálogos).
    pub elevated_bg: Color32,
    /// Tinta do vidro que flutua sobre o conteúdo da própria janela: clara no
    /// escuro, clara no claro — o desfoque é que dá o corpo.
    pub glass_over_content: Color32,
    /// Fundo opaco equivalente, usado quando a translucidez está desligada.
    pub glass_opaque: Color32,
    /// Brilho de um pixel no topo do vidro.
    pub glass_highlight: Color32,

    pub label: Color32,
    pub label_secondary: Color32,
    pub label_tertiary: Color32,

    pub separator: Color32,
    /// Preenchimento de hover/seleção sutil sobre vidro.
    pub fill_soft: Color32,
    pub fill_medium: Color32,

    pub accent: Color32,
    pub accent_label: Color32,
    pub online: Color32,
    pub away: Color32,
    pub busy: Color32,
    pub danger: Color32,
    pub mention: Color32,
}

impl Tokens {
    pub fn new(appearance: Appearance, accent: Option<Color32>) -> Self {
        let mut tokens = match appearance {
            Appearance::Dark => Self {
                appearance,
                content_bg: rgb(0x16, 0x16, 0x18),
                elevated_bg: rgb(0x2C, 0x2C, 0x2E),
                glass_over_content: rgba(0xFF, 0xFF, 0xFF, 0x14),
                glass_opaque: rgb(0x1F, 0x1F, 0x21),
                glass_highlight: rgba(0xFF, 0xFF, 0xFF, 0x14),
                label: rgb(0xFF, 0xFF, 0xFF),
                label_secondary: rgba(0xEB, 0xEB, 0xF5, 0x99),
                label_tertiary: rgba(0xEB, 0xEB, 0xF5, 0x4D),
                separator: rgba(0xFF, 0xFF, 0xFF, 0x1A),
                fill_soft: rgba(0xFF, 0xFF, 0xFF, 0x0F),
                fill_medium: rgba(0xFF, 0xFF, 0xFF, 0x1F),
                accent: rgb(0x0A, 0x84, 0xFF),
                accent_label: rgb(0xFF, 0xFF, 0xFF),
                online: rgb(0x30, 0xD1, 0x58),
                away: rgb(0xFF, 0xD6, 0x0A),
                busy: rgb(0xFF, 0x45, 0x3A),
                danger: rgb(0xFF, 0x45, 0x3A),
                mention: rgba(0xFF, 0xD6, 0x0A, 0x24),
            },
            Appearance::Light => Self {
                appearance,
                content_bg: rgb(0xFF, 0xFF, 0xFF),
                elevated_bg: rgb(0xFF, 0xFF, 0xFF),
                glass_over_content: rgba(0xFF, 0xFF, 0xFF, 0x8C),
                glass_opaque: rgb(0xF4, 0xF4, 0xF6),
                glass_highlight: rgba(0xFF, 0xFF, 0xFF, 0x80),
                label: rgb(0x00, 0x00, 0x00),
                label_secondary: rgba(0x3C, 0x3C, 0x43, 0x99),
                label_tertiary: rgba(0x3C, 0x3C, 0x43, 0x4D),
                separator: rgba(0x3C, 0x3C, 0x43, 0x33),
                fill_soft: rgba(0x78, 0x78, 0x80, 0x1F),
                fill_medium: rgba(0x78, 0x78, 0x80, 0x33),
                accent: rgb(0x00, 0x7A, 0xFF),
                accent_label: rgb(0xFF, 0xFF, 0xFF),
                online: rgb(0x34, 0xC7, 0x59),
                away: rgb(0xFF, 0xCC, 0x00),
                busy: rgb(0xFF, 0x3B, 0x30),
                danger: rgb(0xFF, 0x3B, 0x30),
                mention: rgba(0xFF, 0xCC, 0x00, 0x24),
            },
        };
        if let Some(accent) = accent {
            tokens.accent = accent;
            // Texto sobre o destaque: branco ou preto, o que tiver mais contraste.
            let l = 0.2126 * accent.r() as f32
                + 0.7152 * accent.g() as f32
                + 0.0722 * accent.b() as f32;
            tokens.accent_label = if l > 150.0 {
                Color32::from_rgb(0, 0, 0)
            } else {
                Color32::from_rgb(255, 255, 255)
            };
        }
        tokens
    }

    /// Tinta de uma pastilha que flutua sobre o conteúdo.
    pub fn pill_fill(&self, translucent: bool) -> Color32 {
        if translucent {
            self.glass_over_content
        } else {
            self.glass_opaque
        }
    }
}

// ---------------------------------------------------------------------------
// Fontes
// ---------------------------------------------------------------------------

pub fn install_fonts(ctx: &egui::Context, system_font: Option<&crate::platform::desktop::SystemFont>) {
    use std::sync::Arc;

    let mut fonts = egui::FontDefinitions::default();

    fn add(fonts: &mut egui::FontDefinitions, name: &str, bytes: &'static [u8]) {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(egui::FontData::from_static(bytes)));
    }
    add(&mut fonts, "inter", include_bytes!("../../assets/fonts/Inter-Regular.ttf"));
    add(&mut fonts, "inter-medium", include_bytes!("../../assets/fonts/Inter-Medium.ttf"));
    add(&mut fonts, "inter-semibold", include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"));
    add(
        &mut fonts,
        "inter-display",
        include_bytes!("../../assets/fonts/InterDisplay-SemiBold.ttf"),
    );

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());

    fonts.families.insert(
        FontFamily::Name("medium".into()),
        vec!["inter-medium".to_owned(), "inter".to_owned()],
    );
    fonts.families.insert(
        FontFamily::Name("semibold".into()),
        vec!["inter-semibold".to_owned(), "inter".to_owned()],
    );
    fonts.families.insert(
        FontFamily::Name("display".into()),
        vec!["inter-display".to_owned(), "inter-semibold".to_owned()],
    );

    // Fonte de interface do sistema na frente da Inter, quando existir.
    if let Some(system_font) = system_font {
        if let Some(faces) = load_system_faces(&system_font.family) {
            for (slot, data) in faces {
                let name = format!("system-{slot}");
                fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(data)));
                let family = match slot {
                    "regular" => FontFamily::Proportional,
                    other => FontFamily::Name(other.into()),
                };
                fonts.families.entry(family).or_default().insert(0, name);
            }
            set_body_size(system_font.body_px());
        }
    }

    add(
        &mut fonts,
        "noto-emoji",
        include_bytes!("../../assets/fonts/NotoEmoji-Regular.ttf"),
    );
    for family in [
        FontFamily::Proportional,
        FontFamily::Name("medium".into()),
        FontFamily::Name("semibold".into()),
        FontFamily::Name("display".into()),
    ] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("noto-emoji".to_owned());
    }

    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Os ícones ficam numa família só deles: a fonte de ícones que o egui já
    // traz usa a mesma faixa de uso privado e venceria o Phosphor.
    let phosphor = fonts
        .families
        .get(&FontFamily::Proportional)
        .and_then(|names| names.iter().find(|n| n.contains("phosphor")).cloned());
    if let Some(phosphor) = phosphor {
        fonts
            .families
            .insert(FontFamily::Name("icons".into()), vec![phosphor]);
    }

    ctx.set_fonts(fonts);
}

/// Procura no fontconfig os pesos da família pedida.
fn load_system_faces(family: &str) -> Option<Vec<(&'static str, Vec<u8>)>> {
    use fontdb::{Database, Family, Query, Weight};

    let mut db = Database::new();
    db.load_system_fonts();

    let mut faces = Vec::new();
    for (slot, weight) in [
        ("regular", Weight::NORMAL),
        ("medium", Weight::MEDIUM),
        ("semibold", Weight::SEMIBOLD),
        ("display", Weight::SEMIBOLD),
    ] {
        let query = Query {
            families: &[Family::Name(family)],
            weight,
            ..Default::default()
        };
        let Some(id) = db.query(&query) else { continue };
        let data = db.with_face_data(id, |data, _index| data.to_vec());
        if let Some(data) = data {
            faces.push((slot, data));
        }
    }

    (!faces.is_empty()).then_some(faces)
}

// ---------------------------------------------------------------------------
// Estilo
// ---------------------------------------------------------------------------

/// `motion` é o fator de animação do sistema: 0 desliga as transições.
pub fn apply(ctx: &egui::Context, t: &Tokens, translucent: bool, motion: f32) {
    let mut style = (*ctx.global_style()).clone();

    style.text_styles = [
        (TextStyle::Heading, text::title2()),
        (TextStyle::Body, text::body()),
        (TextStyle::Monospace, text::mono()),
        (TextStyle::Button, text::headline()),
        (TextStyle::Small, text::footnote()),
    ]
    .into();

    let v = &mut style.visuals;
    v.dark_mode = t.appearance.is_dark();
    v.override_text_color = Some(t.label);
    v.panel_fill = t.content_bg;
    v.window_fill = t.elevated_bg;
    v.extreme_bg_color = t.fill_soft;
    v.faint_bg_color = t.fill_soft;
    v.code_bg_color = t.fill_soft;
    v.hyperlink_color = t.accent;
    v.selection.bg_fill = t.accent.linear_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, t.label);
    v.window_stroke = Stroke::new(1.0, t.separator);
    v.window_corner_radius = CornerRadius::same(radius::SHEET);
    v.menu_corner_radius = CornerRadius::same(radius::SHEET);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if t.appearance.is_dark() { 120 } else { 40 }),
    };
    v.window_shadow = v.popup_shadow;
    v.image_loading_spinners = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = Color32::TRANSPARENT;
    w.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.label_secondary);
    w.noninteractive.corner_radius = CornerRadius::same(radius::CONTROL);

    w.inactive.bg_fill = t.fill_soft;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, t.label_secondary);
    w.inactive.corner_radius = CornerRadius::same(radius::CONTROL);
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = t.fill_medium;
    w.hovered.weak_bg_fill = t.fill_soft;
    w.hovered.bg_stroke = Stroke::NONE;
    w.hovered.fg_stroke = Stroke::new(1.0, t.label);
    w.hovered.corner_radius = CornerRadius::same(radius::CONTROL);
    w.hovered.expansion = 0.0;

    w.active.bg_fill = t.accent;
    w.active.weak_bg_fill = t.fill_medium;
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0, t.accent_label);
    w.active.corner_radius = CornerRadius::same(radius::CONTROL);
    w.active.expansion = 0.0;

    w.open.bg_fill = t.fill_medium;
    w.open.weak_bg_fill = t.fill_soft;
    w.open.bg_stroke = Stroke::NONE;
    w.open.fg_stroke = Stroke::new(1.0, t.label);
    w.open.corner_radius = CornerRadius::same(radius::CONTROL);

    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(space::MD, space::SM);
    s.button_padding = egui::vec2(space::MD, space::SM);
    s.menu_margin = Margin::same(space::XS as i8);
    s.indent = space::LG;
    s.interact_size = egui::vec2(HIT_TARGET, HIT_TARGET);
    s.scroll.bar_width = 8.0;
    s.scroll.floating = true;
    s.scroll.bar_inner_margin = 2.0;

    style.animation_time = 0.12 * motion.clamp(0.0, 4.0);
    style.visuals.striped = false;

    ctx.set_global_style(style);

    // A janela só fica transparente quando o vidro está ligado; caso
    // contrário a cor de fundo cobre tudo.
    let _ = translucent;
}

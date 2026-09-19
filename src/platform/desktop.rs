//! Integração com o ambiente de trabalho: cor de destaque e aparência.
//!
//! No KDE Plasma as duas informações vêm do `kdeglobals`. Em outros ambientes
//! caímos no padrão da própria aplicação.

use egui::Color32;

/// Cor de destaque do sistema, quando o ambiente expõe uma.
pub fn system_accent() -> Option<Color32> {
    #[cfg(target_os = "linux")]
    {
        // `[General] AccentColor` existe quando o usuário escolhe um destaque
        // personalizado; senão o esquema de cores manda em `[Colors:Selection]`.
        kde_color(&["General"], "AccentColor")
            .or_else(|| kde_color(&["Colors:Selection"], "BackgroundNormal"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Aparência preferida do sistema, deduzida do brilho do fundo de janela.
pub fn system_prefers_dark() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let bg = kde_color(&["Colors:Window"], "BackgroundNormal")?;
        let l = 0.2126 * bg.r() as f32 + 0.7152 * bg.g() as f32 + 0.0722 * bg.b() as f32;
        Some(l < 128.0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn kde_color(sections: &[&str], key: &str) -> Option<Color32> {
    for path in kdeglobals_paths() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(value) = ini_lookup(&text, sections, key) {
            if let Some(color) = parse_rgb_triplet(&value) {
                return Some(color);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn kdeglobals_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(dirs) = directories::BaseDirs::new() {
        let config = dirs.config_dir().to_path_buf();
        paths.push(config.join("kdeglobals"));
        paths.push(config.join("kdedefaults/kdeglobals"));
    }
    paths
}

/// Leitor mínimo de INI: devolve o valor da primeira seção que contiver a chave.
#[cfg(target_os = "linux")]
fn ini_lookup(text: &str, sections: &[&str], key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_owned();
            continue;
        }
        if !sections.iter().any(|s| *s == current) {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_owned());
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn parse_rgb_triplet(value: &str) -> Option<Color32> {
    let mut parts = value.split(',').map(|p| p.trim().parse::<u8>());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(r)), Some(Ok(g)), Some(Ok(b))) => Some(Color32::from_rgb(r, g, b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn le_cor_da_secao_certa() {
        let ini = "[Colors:Window]\nBackgroundNormal=54,54,54\n\n[Colors:Selection]\nBackgroundNormal=49,91,239\n";
        assert_eq!(
            ini_lookup(ini, &["Colors:Selection"], "BackgroundNormal").as_deref(),
            Some("49,91,239")
        );
        assert_eq!(
            parse_rgb_triplet("49,91,239"),
            Some(Color32::from_rgb(49, 91, 239))
        );
    }
}

/// Fonte de interface configurada no sistema.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemFont {
    pub family: String,
    /// Tamanho em pontos tipográficos (Qt/GTK usam pt, não px).
    pub size_pt: f32,
}

impl SystemFont {
    /// Tamanho do corpo em pixels lógicos: pt × 96/72.
    pub fn body_px(&self) -> f32 {
        self.size_pt * 4.0 / 3.0
    }
}

/// Fonte de interface do sistema (`[General] font=Noto Sans,10,-1,...`).
pub fn system_ui_font() -> Option<SystemFont> {
    #[cfg(target_os = "linux")]
    {
        for path in kdeglobals_paths() {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(value) = ini_lookup(&text, &["General"], "font") {
                if let Some(font) = parse_qt_font(&value) {
                    return Some(font);
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn parse_qt_font(value: &str) -> Option<SystemFont> {
    let mut parts = value.split(',');
    let family = parts.next()?.trim().to_owned();
    let size_pt: f32 = parts.next()?.trim().parse().ok()?;
    if family.is_empty() || !(4.0..=32.0).contains(&size_pt) {
        return None;
    }
    Some(SystemFont { family, size_pt })
}

/// Fator de duração das animações do Plasma: 0 significa "sem animação".
pub fn animation_factor() -> Option<f32> {
    #[cfg(target_os = "linux")]
    {
        for path in kdeglobals_paths() {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(value) = ini_lookup(&text, &["KDE"], "AnimationDurationFactor") {
                if let Ok(factor) = value.parse::<f32>() {
                    return Some(factor.clamp(0.0, 4.0));
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

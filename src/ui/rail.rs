//! Trilho de servidores: a coluna estreita de ícones à esquerda de tudo.
//!
//! Um Papo é um servidor só, então "vários servidores" quer dizer vários
//! backends, cada um com conta, sessão e conversa próprias. O trilho é o que
//! mostra todos de uma vez: o ativo, quem tem conversa nova e quantas menções
//! esperam em cada um.

use egui::{Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, Vec2};

use crate::i18n::Strings;

use super::theme::{radius, space, text, Tokens};
use super::widgets::sidebar_frame;

/// Largura da coluna inteira, ícone mais as folgas dos dois lados.
pub const RAIL_WIDTH: f32 = 60.0;
const ICON: f32 = 44.0;
const GAP: f32 = space::MD;

/// Um servidor, do jeito que o trilho precisa enxergá-lo.
pub struct Entry {
    pub label: String,
    pub address: String,
    pub mentions: u32,
    pub unread: bool,
    /// Ainda sem sessão: o servidor aparece apagado, esperando login.
    pub signed_in: bool,
    pub online: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RailAction {
    Select(usize),
    Add,
    Remove(usize),
}

/// Desenha o trilho e devolve o que o usuário pediu nele.
pub fn draw(
    root: &mut egui::Ui,
    entries: &[Entry],
    active: usize,
    t: &Tokens,
    s: &Strings,
) -> Option<RailAction> {
    let mut action = None;

    egui::Panel::left("servers")
        .exact_size(RAIL_WIDTH)
        .resizable(false)
        .frame(sidebar_frame(t).inner_margin(egui::Margin::symmetric(
            ((RAIL_WIDTH - ICON) / 2.0) as i8,
            space::MD as i8,
        )))
        .show(root, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for (index, entry) in entries.iter().enumerate() {
                if let Some(chosen) = tile(ui, entry, index == active, t, s) {
                    action = Some(match chosen {
                        Tile::Click => RailAction::Select(index),
                        Tile::Remove => RailAction::Remove(index),
                    });
                }
                ui.add_space(GAP);
            }
            if add_button(ui, t, s) {
                action = Some(RailAction::Add);
            }
        });

    action
}

enum Tile {
    Click,
    Remove,
}

fn tile(ui: &mut egui::Ui, entry: &Entry, active: bool, t: &Tokens, s: &Strings) -> Option<Tile> {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ICON), Sense::click());
    let painter = ui.painter();
    let hovered = response.hovered();

    // Quadrado arredondado que vira quase círculo quando está parado, como
    // no resto da interface: o canto abre ao passar o mouse e ao selecionar.
    let corner = if active || hovered { radius::SHEET } else { 16 };
    let fill = if active {
        t.accent
    } else if hovered {
        t.fill_medium
    } else {
        t.fill_soft
    };
    painter.rect_filled(rect, CornerRadius::same(corner), fill);
    if !entry.signed_in {
        // Sem sessão: o contorno tracejado diz que falta entrar, sem gritar.
        painter.rect_stroke(
            rect,
            CornerRadius::same(corner),
            Stroke::new(1.0, t.separator),
            egui::StrokeKind::Inside,
        );
    }

    let label_color = if active {
        t.accent_label
    } else if entry.signed_in {
        t.label
    } else {
        t.label_tertiary
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        initials(&entry.label),
        text::headline(),
        label_color,
    );

    // Marcador à esquerda: barra comprida no ativo, pontinho no não lido.
    let marker_height = if active {
        24.0
    } else if entry.unread || entry.mentions > 0 {
        8.0
    } else {
        0.0
    };
    if marker_height > 0.0 {
        let marker = Rect::from_center_size(
            egui::pos2(rect.left() - space::SM, rect.center().y),
            Vec2::new(3.0, marker_height),
        );
        painter.rect_filled(marker, CornerRadius::same(2), t.label);
    }

    // Menções viram número; conversa nova sem menção fica só no marcador.
    if entry.mentions > 0 {
        badge(ui, rect, entry.mentions, t);
    }

    // Servidor fora do ar: um anel apagado no canto, sem texto.
    if entry.signed_in && !entry.online {
        ui.painter().circle_stroke(
            rect.left_bottom() + Vec2::new(6.0, -6.0),
            4.0,
            Stroke::new(1.5, t.label_tertiary),
        );
    }

    let response = response.on_hover_ui(|ui| {
        ui.label(RichText::new(&entry.label).font(text::body()).color(t.label));
        ui.label(
            RichText::new(&entry.address)
                .font(text::footnote())
                .color(t.label_tertiary),
        );
    });

    let mut chosen = response.clicked().then_some(Tile::Click);
    response.context_menu(|ui| {
        if ui
            .button(s.remove_server)
            .on_hover_text(s.remove_server_hint)
            .clicked()
        {
            chosen = Some(Tile::Remove);
            ui.close();
        }
    });
    chosen
}

/// Contador de menções, colado no canto de baixo do ícone.
fn badge(ui: &egui::Ui, icon: Rect, count: u32, t: &Tokens) {
    let text_value = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let font = text::caption();
    let galley = ui.painter().layout_no_wrap(
        text_value,
        font,
        Color32::WHITE,
    );
    let width = (galley.size().x + space::MD).max(16.0);
    let rect = Rect::from_center_size(
        icon.right_bottom() - Vec2::splat(2.0),
        Vec2::new(width, 16.0),
    );
    // O anel na cor do fundo separa o contador do ícone sem uma borda dura.
    ui.painter()
        .rect_filled(rect.expand(2.0), CornerRadius::same(10), t.content_bg);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), t.danger);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        Color32::WHITE,
    );
}

fn add_button(ui: &mut egui::Ui, t: &Tokens, s: &Strings) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ICON), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(if hovered { radius::SHEET } else { 16 }),
        if hovered { t.fill_medium } else { t.fill_soft },
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "+",
        text::title3(),
        if hovered { t.label } else { t.label_secondary },
    );
    response.on_hover_text(s.add_server).clicked()
}

/// Até duas letras: a inicial de cada uma das duas primeiras palavras, ou as
/// duas primeiras letras quando o nome é uma palavra só.
fn initials(label: &str) -> String {
    let words: Vec<&str> = label.split_whitespace().take(2).collect();
    match words.as_slice() {
        [] => "?".to_owned(),
        [one] => one.chars().take(2).collect::<String>().to_uppercase(),
        [first, second] => {
            let mut out = String::new();
            out.extend(first.chars().take(1));
            out.extend(second.chars().take(1));
            out.to_uppercase()
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn uma_palavra_vira_duas_letras() {
        assert_eq!(initials("Papo"), "PA");
    }

    #[test]
    fn duas_palavras_viram_uma_letra_cada() {
        assert_eq!(initials("Casa da Mãe"), "CD");
    }

    #[test]
    fn nome_vazio_vira_interrogacao() {
        assert_eq!(initials("   "), "?");
    }
}

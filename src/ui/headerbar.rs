//! Barra de título própria, para as áreas de trabalho sem menu global.
//!
//! No Plasma o menu do aplicativo vai para o painel e o compositor desenha a
//! barra de título: não há nada a fazer aqui. Fora dele — GNOME à frente —
//! não existe menu global nenhum, e o costume é o aplicativo desenhar a
//! própria barra, com o título no meio e um botão de hambúrguer guardando o
//! menu. Sem isso, ajustes, idioma, sair da conta e encerrar ficariam sem
//! caminho nenhum na janela.

use egui::{Align2, CornerRadius, Rect, RichText, Sense, Stroke, Vec2};

use crate::platform::menu::{MenuCommand, MenuKind, MenuModel, MenuNode};

use super::theme::{radius, space, text, Tokens};

pub const HEADER_HEIGHT: f32 = 40.0;
const BUTTON: f32 = 28.0;

/// O que o quadro anterior deixou aberto.
#[derive(Default)]
pub struct HeaderState {
    pub menu_open: bool,
}

/// Desenha a barra e devolve os comandos que o usuário acionou nela.
pub fn draw(
    root: &mut egui::Ui,
    state: &mut HeaderState,
    model: &MenuModel,
    title: &str,
    t: &Tokens,
) -> Vec<MenuCommand> {
    let mut commands = Vec::new();

    egui::Panel::top("headerbar")
        .exact_size(HEADER_HEIGHT)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(t.elevated_bg)
                .stroke(Stroke::new(1.0, t.separator)),
        )
        .show(root, |ui| {
            let rect = ui.max_rect();

            // Arrastar pela barra move a janela; dois cliques maximizam. É o
            // que o compositor faria pela gente se desenhasse a barra.
            let drag = ui.interact(rect, ui.id().with("drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }

            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                title,
                text::headline(),
                t.label,
            );

            // Botões da direita para a esquerda: fechar, maximizar, minimizar
            // e, separado deles, o hambúrguer.
            let mut x = rect.right() - space::MD;
            for (icon, command) in [
                (egui_phosphor::regular::X, Window::Close),
                (egui_phosphor::regular::SQUARE, Window::Maximize),
                (egui_phosphor::regular::MINUS, Window::Minimize),
            ] {
                x -= BUTTON;
                let button = Rect::from_center_size(
                    egui::pos2(x + BUTTON / 2.0, rect.center().y),
                    Vec2::splat(BUTTON),
                );
                if window_button(ui, button, icon, command == Window::Close, t) {
                    match command {
                        // Fechar passa pelo mesmo caminho do X do sistema:
                        // com "continuar em segundo plano" ligado, recolhe
                        // para a bandeja em vez de encerrar.
                        Window::Close => commands.push(MenuCommand::CloseWindow),
                        Window::Maximize => {
                            let maximized =
                                ui.input(|input| input.viewport().maximized.unwrap_or(false));
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        Window::Minimize => ui
                            .ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                    }
                }
                x -= space::XXS;
            }

            x -= space::MD + BUTTON;
            let hamburger = Rect::from_center_size(
                egui::pos2(x + BUTTON / 2.0, rect.center().y),
                Vec2::splat(BUTTON),
            );
            if window_button(ui, hamburger, egui_phosphor::regular::LIST, false, t) {
                state.menu_open = !state.menu_open;
            }

            if state.menu_open {
                menu_popup(ui, state, model, hamburger, t, &mut commands);
            }
        });

    commands
}

/// Bordas invisíveis que redimensionam a janela.
///
/// Sem a decoração do sistema some também o redimensionar por arrastar a
/// borda, e a janela ficaria presa no tamanho em que abriu. Estas faixas
/// ficam por cima de tudo, nas oito direções.
pub fn resize_handles(ctx: &egui::Context) {
    use egui::viewport::ResizeDirection as Dir;

    const EDGE: f32 = 6.0;
    let screen = ctx.viewport_rect();
    if screen.width() < EDGE * 4.0 || screen.height() < EDGE * 4.0 {
        return;
    }

    // Os cantos vêm antes das laterais: onde os dois se encostam, quem
    // responde tem de ser o canto.
    let corner = EDGE * 3.0;
    let regions = [
        (
            Rect::from_min_size(screen.min, Vec2::splat(corner)),
            Dir::NorthWest,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(
                egui::pos2(screen.right() - corner, screen.top()),
                Vec2::splat(corner),
            ),
            Dir::NorthEast,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(
                egui::pos2(screen.left(), screen.bottom() - corner),
                Vec2::splat(corner),
            ),
            Dir::SouthWest,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(
                egui::pos2(screen.right() - corner, screen.bottom() - corner),
                Vec2::splat(corner),
            ),
            Dir::SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(screen.min, Vec2::new(screen.width(), EDGE)),
            Dir::North,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_size(
                egui::pos2(screen.left(), screen.bottom() - EDGE),
                Vec2::new(screen.width(), EDGE),
            ),
            Dir::South,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_size(screen.min, Vec2::new(EDGE, screen.height())),
            Dir::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(
                egui::pos2(screen.right() - EDGE, screen.top()),
                Vec2::new(EDGE, screen.height()),
            ),
            Dir::East,
            egui::CursorIcon::ResizeHorizontal,
        ),
    ];

    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("resize"));
    let ui = egui::Ui::new(
        ctx.clone(),
        egui::Id::new("resize-handles"),
        egui::UiBuilder::new().layer_id(layer).max_rect(screen),
    );
    for (index, (rect, direction, cursor)) in regions.into_iter().enumerate() {
        let response = ui.interact(
            rect,
            egui::Id::new(("resize", index)),
            Sense::click_and_drag(),
        );
        if response.hovered() || response.is_pointer_button_down_on() {
            ctx.set_cursor_icon(cursor);
        }
        if response.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Window {
    Close,
    Maximize,
    Minimize,
}

fn window_button(ui: &egui::Ui, rect: Rect, icon: &str, danger: bool, t: &Tokens) -> bool {
    let response = ui.interact(rect, ui.id().with(icon), Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(radius::FIELD),
            if danger { t.danger } else { t.fill_medium },
        );
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        text::icon(14.0),
        match (hovered, danger) {
            (true, true) => egui::Color32::WHITE,
            (true, false) => t.label,
            (false, _) => t.label_secondary,
        },
    );
    response.clicked()
}

/// O menu do aplicativo, achatado.
///
/// A árvore do menu global tem submenus porque uma barra de menus precisa
/// deles. Num hambúrguer o costume é uma lista só, com os títulos virando
/// legenda de seção — que é também o que deixa tudo visível de uma vez.
fn menu_popup(
    ui: &mut egui::Ui,
    state: &mut HeaderState,
    model: &MenuModel,
    anchor: Rect,
    t: &Tokens,
    commands: &mut Vec<MenuCommand>,
) {
    let response = egui::Area::new(ui.id().with("menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(anchor.right() - 260.0, anchor.bottom() + space::XS))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(t.elevated_bg)
                .corner_radius(CornerRadius::same(radius::SHEET))
                .stroke(Stroke::new(1.0, t.separator))
                .inner_margin(egui::Margin::same(space::SM as i8))
                .shadow(ui.ctx().global_style().visuals.window_shadow)
                .show(ui, |ui| {
                    ui.set_width(248.0);
                    for root in &model.roots {
                        section(ui, root, t, commands);
                    }
                });
        })
        .response;

    // Clicar fora fecha, como qualquer menu.
    let clicked_outside = ui.input(|input| input.pointer.any_click())
        && !response.rect.contains(
            ui.ctx()
                .pointer_latest_pos()
                .unwrap_or(egui::Pos2::ZERO),
        )
        && !anchor.contains(ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO));
    if clicked_outside
        || !commands.is_empty()
        || ui.input(|input| input.key_pressed(egui::Key::Escape))
    {
        state.menu_open = false;
    }
}

fn section(ui: &mut egui::Ui, node: &MenuNode, t: &Tokens, commands: &mut Vec<MenuCommand>) {
    match node.kind {
        MenuKind::Submenu => {
            ui.add_space(space::XS);
            ui.label(
                RichText::new(node.label.to_uppercase())
                    .font(text::caption())
                    .color(t.label_tertiary),
            );
            ui.add_space(space::XXS);
            for child in &node.children {
                section(ui, child, t, commands);
            }
            ui.add_space(space::XS);
        }
        MenuKind::Separator => {
            ui.add_space(space::XS);
        }
        _ => {
            if let Some(command) = node.command {
                if entry(ui, node, t) {
                    commands.push(command);
                }
            }
        }
    }
}

fn entry(ui: &mut egui::Ui, node: &MenuNode, t: &Tokens) -> bool {
    let height = 26.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let hovered = response.hovered() && node.enabled;
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
    }

    let color = if node.enabled {
        t.label
    } else {
        t.label_tertiary
    };
    ui.painter().text(
        egui::pos2(rect.left() + space::MD, rect.center().y),
        Align2::LEFT_CENTER,
        &node.label,
        text::body(),
        color,
    );

    // Marcado é marcado: o mesmo sinal que o menu do painel mostraria.
    let mark = match node.kind {
        MenuKind::Checkbox { checked: true } => Some(egui_phosphor::regular::CHECK),
        MenuKind::Radio { selected: true } => Some(egui_phosphor::regular::DOT_OUTLINE),
        _ => None,
    };
    if let Some(mark) = mark {
        ui.painter().text(
            egui::pos2(rect.right() - space::MD, rect.center().y),
            Align2::RIGHT_CENTER,
            mark,
            text::icon(13.0),
            t.accent,
        );
    }

    node.enabled && response.clicked()
}

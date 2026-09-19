//! Telas de entrada: sessão e primeiro uso da instância.

use egui::{Align, CornerRadius, Layout, Rect, RichText, Sense, Stroke, TextEdit, UiBuilder, Vec2};

use crate::i18n::Strings;
use crate::state::Store;

use super::theme::{radius, space, text, Tokens};

#[derive(Clone, Debug, Default)]
pub struct AuthForm {
    pub server_url: String,
    pub username: String,
    pub password: String,
    /// Nome do servidor, no primeiro uso da instância.
    pub server_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthAction {
    None,
    SignIn,
    Register,
    CreateServer,
}

/// Cartão centrado de login/cadastro.
pub fn sign_in(
    ui: &mut egui::Ui,
    form: &mut AuthForm,
    store: &Store,
    t: &Tokens,
    s: &Strings,
) -> AuthAction {
    let mut action = AuthAction::None;

    card(ui, t, 400.0, 404.0, |ui| {
        ui.label(RichText::new("Papo").font(text::title1()).color(t.label));
        ui.add_space(space::XS);
        ui.label(
            RichText::new(&form.server_url)
                .font(text::footnote())
                .color(t.label_tertiary),
        );
        ui.add_space(space::XXL);

        field(ui, t, s.server, &mut form.server_url, false);
        ui.add_space(space::LG);
        field(ui, t, s.username, &mut form.username, false);
        ui.add_space(space::LG);
        let submitted = field(ui, t, s.password, &mut form.password, true);

        if let Some(error) = &store.error {
            ui.add_space(space::LG);
            ui.label(
                RichText::new(error)
                    .font(text::footnote())
                    .color(t.danger),
            );
        }

        ui.add_space(space::XXL);
        let ready = !form.username.trim().is_empty() && !form.password.is_empty();
        if primary_button(ui, t, s.sign_in, ready && !store.busy) || (submitted && ready) {
            action = AuthAction::SignIn;
        }
        ui.add_space(space::MD);
        if link_button(ui, t, s.sign_up, ready && !store.busy) {
            action = AuthAction::Register;
        }
    });

    action
}

/// Primeiro uso: a instância ainda não tem servidor.
pub fn create_server(
    ui: &mut egui::Ui,
    form: &mut AuthForm,
    store: &Store,
    t: &Tokens,
    s: &Strings,
) -> AuthAction {
    let mut action = AuthAction::None;

    card(ui, t, 400.0, 252.0, |ui| {
        ui.label(
            RichText::new(s.empty_channel_title)
                .font(text::title2())
                .color(t.label),
        );
        ui.add_space(space::XS);
        ui.label(
            RichText::new(s.server)
                .font(text::footnote())
                .color(t.label_tertiary),
        );
        ui.add_space(space::XL);

        let submitted = field(ui, t, s.server, &mut form.server_name, false);
        if let Some(error) = &store.error {
            ui.add_space(space::LG);
            ui.label(RichText::new(error).font(text::footnote()).color(t.danger));
        }

        ui.add_space(space::XXL);
        let ready = !form.server_name.trim().is_empty();
        if primary_button(ui, t, s.menu_new_channel, ready && !store.busy) || (submitted && ready) {
            action = AuthAction::CreateServer;
        }
    });

    action
}

/// Enquanto a sessão guardada é verificada.
pub fn starting(ui: &mut egui::Ui, t: &Tokens, s: &Strings) {
    let area = ui.max_rect();
    ui.painter().text(
        area.center(),
        egui::Align2::CENTER_CENTER,
        s.connecting,
        text::body(),
        t.label_tertiary,
    );
}

// ---------------------------------------------------------------------------

fn card(ui: &mut egui::Ui, t: &Tokens, width: f32, height: f32, contents: impl FnOnce(&mut egui::Ui)) {
    let area = ui.max_rect();
    let rect = Rect::from_center_size(area.center(), Vec2::new(width, height));
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::SHEET + 4),
        t.glass_opaque,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink(space::XXXL))
            .layout(Layout::top_down(Align::Min)),
        contents,
    );
}

/// Campo de texto rotulado. Devolve `true` quando o usuário aperta Enter.
fn field(ui: &mut egui::Ui, t: &Tokens, label: &str, value: &mut String, secret: bool) -> bool {
    ui.label(
        RichText::new(label.to_uppercase())
            .font(text::caption())
            .color(t.label_tertiary),
    );
    ui.add_space(space::XS);

    let height = 32.0;
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), height));
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::FIELD),
        t.fill_soft,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    let response = ui.put(
        rect.shrink2(Vec2::new(space::MD, space::XXS)),
        TextEdit::singleline(value)
            .password(secret)
            .frame(egui::Frame::NONE)
            .font(text::body())
            .vertical_align(Align::Center)
            .desired_width(f32::INFINITY),
    );
    ui.advance_cursor_after_rect(rect);

    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

fn primary_button(ui: &mut egui::Ui, t: &Tokens, label: &str, enabled: bool) -> bool {
    let height = 34.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let fill = if !enabled {
        t.fill_soft
    } else if response.hovered() {
        t.accent.gamma_multiply(0.85)
    } else {
        t.accent
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::FIELD), fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        text::headline(),
        if enabled { t.accent_label } else { t.label_tertiary },
    );
    enabled && response.clicked()
}

fn link_button(ui: &mut egui::Ui, t: &Tokens, label: &str, enabled: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        text::callout(),
        if response.hovered() && enabled {
            t.accent
        } else {
            t.label_secondary
        },
    );
    enabled && response.clicked()
}

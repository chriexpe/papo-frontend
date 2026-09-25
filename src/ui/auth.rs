//! Telas de entrada: sessão e primeiro uso da instância.

use egui::{Align2, CornerRadius, Rect, RichText, Sense, Stroke, Vec2};

use crate::i18n::Strings;
use crate::state::Store;

use super::theme::{radius, space, text, Tokens};

#[derive(Clone, Debug, Default)]
pub struct AuthForm {
    pub server_url: String,
    pub username: String,
    pub password: String,
    /// Senha do servidor, pedida só quando o servidor é fechado.
    pub server_password: String,
    /// Nome do servidor, no primeiro uso da instância.
    pub server_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthAction {
    None,
    SignIn,
    Register,
    CreateServer,
    /// Mandar a senha do servidor de um servidor fechado.
    UnlockServer,
}

/// Resultado de um cartão de autenticação. O retângulo serve para quem abre
/// o cartão como modal saber se o clique caiu fora dele.
#[derive(Clone, Copy, Debug)]
pub struct AuthResponse {
    pub action: AuthAction,
    pub rect: Rect,
}

/// Cartão centrado de login/cadastro.
pub fn sign_in(
    ui: &mut egui::Ui,
    form: &mut AuthForm,
    store: &Store,
    t: &Tokens,
    s: &Strings,
) -> AuthResponse {
    let mut action = AuthAction::None;
    // Um servidor fechado recusa login e cadastro antes da senha do
    // servidor. Não adianta pedir usuário e senha ainda: o cartão mostra
    // só o portão, e volta ao normal assim que ele abre.
    let locked = store.locked;
    let unmet = if form.password.is_empty() {
        Vec::new()
    } else {
        password_rules(&form.password, s)
    };

    let rect = card(ui, t, "auth-sign-in-card", 400.0, |ui| {
        ui.label(RichText::new("Papo").font(text::title1()).color(t.label));
        ui.add_space(space::XS);
        ui.label(
            RichText::new(&form.server_url)
                .font(text::footnote())
                .color(t.label_tertiary),
        );
        ui.add_space(space::XXL);

        field(ui, t, s.server_address, &mut form.server_url, false);
        ui.add_space(space::LG);

        if locked {
            let submitted = field(ui, t, s.server_password, &mut form.server_password, true);
            ui.add_space(space::SM);
            ui.label(
                RichText::new(s.server_password_hint)
                    .font(text::footnote())
                    .color(t.label_tertiary),
            );
            problem(ui, t, store, s);

            ui.add_space(space::XXL);
            let ready = !form.server_password.is_empty();
            if primary_button(ui, t, s.unlock_server, ready && !store.busy) || (submitted && ready)
            {
                action = AuthAction::UnlockServer;
            }
            return;
        }

        field(ui, t, s.username, &mut form.username, false);
        ui.add_space(space::LG);
        let submitted = field(ui, t, s.password, &mut form.password, true);

        // As regras só aparecem quando a senha digitada ainda não passa;
        // quem só está entrando numa conta antiga nunca as vê.
        for rule in &unmet {
            ui.add_space(space::XXS);
            ui.label(
                RichText::new(format!("· {rule}"))
                    .font(text::footnote())
                    .color(t.label_tertiary),
            );
        }

        problem(ui, t, store, s);

        ui.add_space(space::XXL);
        let ready = !form.username.trim().is_empty() && !form.password.is_empty();
        if primary_button(ui, t, s.sign_in, ready && !store.busy) || (submitted && ready) {
            action = AuthAction::SignIn;
        }
        ui.add_space(space::MD);
        // As regras acima já avisam de senha fraca; travar o botão por causa
        // delas só fazia parecer que criar conta não estava implementado.
        if link_button(ui, t, s.sign_up, ready && !store.busy) {
            action = AuthAction::Register;
        }
    });

    AuthResponse { action, rect }
}

/// Erro do formulário e aviso do servidor, nesta ordem.
fn problem(ui: &mut egui::Ui, t: &Tokens, store: &Store, s: &Strings) {
    if let Some(error) = &store.error {
        ui.add_space(space::LG);
        ui.label(RichText::new(error).font(text::footnote()).color(t.danger));
    }
    if store.notice.is_some() {
        ui.add_space(space::LG);
        ui.label(
            RichText::new(s.connection_violation)
                .font(text::footnote())
                .color(t.away),
        );
    }
}

/// Regras de senha que a senha digitada ainda não cumpre. É a mesma política
/// do backend, repetida aqui só para o aviso sair na hora em vez de voltar
/// como erro depois do envio.
fn password_rules(password: &str, s: &Strings) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if password.chars().count() < 8 {
        missing.push(s.password_rule_length);
    }
    if !password.chars().any(char::is_uppercase) {
        missing.push(s.password_rule_uppercase);
    }
    if !password.chars().any(|c| !c.is_alphanumeric()) {
        missing.push(s.password_rule_special);
    }
    missing
}

/// Primeiro uso: a instância ainda não tem servidor.
pub fn create_server(
    ui: &mut egui::Ui,
    form: &mut AuthForm,
    store: &Store,
    t: &Tokens,
    s: &Strings,
) -> AuthResponse {
    let mut action = AuthAction::None;

    let rect = card(ui, t, "auth-create-server-card", 400.0, |ui| {
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

    AuthResponse { action, rect }
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

fn card(
    ui: &mut egui::Ui,
    t: &Tokens,
    id: &'static str,
    width: f32,
    contents: impl FnOnce(&mut egui::Ui),
) -> Rect {
    let area = ui.max_rect();
    let inner_width = (width - 2.0 * space::XXXL).max(1.0);

    // A altura nasce do conteúdo. Antes ela era estimada à mão e qualquer
    // linha extra de validação/erro podia escapar pela borda do cartão.
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Foreground)
        .fixed_pos(area.center())
        .pivot(Align2::CENTER_CENTER)
        .constrain_to(area)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::same(space::XXXL as i8))
                .corner_radius(CornerRadius::same(radius::SHEET + 4))
                .fill(t.glass_opaque)
                .stroke(Stroke::new(1.0, t.separator))
                .show(ui, |ui| {
                    ui.set_width(inner_width);
                    contents(ui);
                });
        })
        .response
        .rect
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

    #[cfg(target_os = "android")]
    let submitted = {
        let edit_id = ui.id().with(("auth-field", label));
        let key = format!("auth:{edit_id:?}");
        crate::platform::native_field::show(
            ui.ctx(),
            &key,
            value,
            rect.shrink2(Vec2::new(space::MD, space::XXS)),
            "",
            if secret {
                crate::platform::native_field::Mode::Password
            } else {
                crate::platform::native_field::Mode::Text
            },
            0,
            false,
            t.label,
            t.label_tertiary,
            text::body().size,
        )
        .submit
    };

    #[cfg(not(target_os = "android"))]
    let submitted = {
        let edit_id = ui.id().with(("auth-field", label));
        let _ = crate::platform::ime::prepare_text_edit(ui.ctx(), edit_id, value);
        let response = ui.put(
            rect.shrink2(Vec2::new(space::MD, space::XXS)),
            egui::TextEdit::singleline(value)
                .id(edit_id)
                .password(secret)
                .frame(egui::Frame::NONE)
                .font(text::body())
                .vertical_align(egui::Align::Center)
                .desired_width(f32::INFINITY),
        );
        let _ = crate::platform::ime::sync_text_edit(
            ui.ctx(),
            edit_id,
            value,
            response.has_focus(),
            if secret {
                crate::platform::ime::Kind::Password
            } else {
                crate::platform::ime::Kind::Text
            },
        );
        response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
    };
    ui.advance_cursor_after_rect(rect);

    submitted
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
    // Desligado tem que parecer desligado: antes ficava igual ao ligado e
    // o clique sumia sem explicação.
    let color = match (enabled, response.hovered()) {
        (false, _) => t.label_tertiary,
        (true, true) => t.accent,
        (true, false) => t.label_secondary,
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        text::callout(),
        color,
    );
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    enabled && response.clicked()
}

#[cfg(test)]
mod tests {
    use super::password_rules;
    use crate::i18n::Lang;

    #[test]
    fn senha_forte_nao_quebra_regra() {
        assert!(password_rules("Segredo!1", Lang::PtBr.strings()).is_empty());
    }

    #[test]
    fn senha_curta_e_minuscula_quebra_as_tres() {
        assert_eq!(password_rules("abc", Lang::PtBr.strings()).len(), 3);
    }

    #[test]
    fn maiuscula_acentuada_conta_como_maiuscula() {
        assert!(password_rules("Ácido-forte", Lang::PtBr.strings()).is_empty());
    }
}

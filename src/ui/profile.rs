//! Cartão de perfil: quem é a pessoa, o que ela está fazendo, que cargos tem.
//!
//! Um cartão só, aberto de três lugares — a pastilha da conta, a lista de
//! pessoas e o autor de uma mensagem. O que muda entre eles é a âncora:
//!
//! - o seu, no estilo padrão, é a própria pastilha da conta "descomprimida":
//!   mesma largura, mesma borda de baixo, crescendo para cima por cima dela;
//! - os dos outros (e o seu, no estilo flutuante) têm 300 pt e abrem ao lado
//!   de quem foi clicado, sem sair da janela;
//! - no celular é uma folha que sobe do pé da tela.
//!
//! É uma superfície elevada e sólida, como Ajustes: texto demais para vidro.
//! As seções se separam por fio, sem cartões dentro do cartão.

use chrono::{DateTime, Local, Utc};
use egui::{Align2, Color32, CornerRadius, Id, Rect, RichText, Sense, Stroke, UiBuilder, Vec2};
use egui_phosphor::regular as icon;

use crate::i18n::Strings;
use crate::platform::menu::MenuCommand;
use crate::state::{Activity, ActivityKind, Member, Presence, Store};

use super::shell::{ChatAction, UiState, presence_color};
use super::theme::{Tokens, radius, space, text};
use super::widgets::{photo, round_photo};

/// Largura do cartão dos outros (e do seu, no estilo flutuante).
pub const CARD_WIDTH: f32 = 300.0;
/// O cartão que se edita em Ajustes é um pouco mais largo que o dos outros.
const EDIT_CARD_WIDTH: f32 = 340.0;
/// Respiro entre o cartão e o que ele acompanha, e entre ele e a janela.
const GAP: f32 = space::MD;
const MARGIN: f32 = space::LG;
/// O banner é sempre 3:1 — o mesmo recorte que o editor de imagem pede.
const BANNER_RATIO: f32 = 3.0;
/// Quanto da foto invade o banner.
const AVATAR_OVERLAP: f32 = 0.55;
const OPEN_SECONDS: f64 = 0.16;
/// Linhas da descrição antes do "Mostrar tudo".
const ABOUT_ROWS: usize = 4;
/// O cartão no celular nunca passa disto da altura da tela.
const SHEET_MAX: f32 = 0.86;
const SHEET_MAX_WIDTH: f32 = 520.0;

/// Como o seu próprio cartão abre. Ajuste de Aparência.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SelfCardStyle {
    /// A pastilha da conta cresce e vira o cartão, na largura dela.
    #[default]
    Pill,
    /// O cartão largo dos outros, acima da pastilha.
    Floating,
}

/// De onde o cartão nasce.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// A pastilha da conta, que cresce e vira o cartão.
    Pill(Rect),
    /// Acima da pastilha da conta, com a largura dos outros cartões.
    AbovePill(Rect),
    /// Ao lado do que foi clicado: a linha da lista, a foto na conversa.
    Beside(Rect),
}

#[derive(Clone, Debug)]
pub struct ProfileCard {
    pub user_id: String,
    pub anchor: Anchor,
    opened: f64,
    about_expanded: bool,
    /// Alturas medidas no quadro anterior. Sem elas o primeiro quadro sai
    /// invisível — a animação de abertura esconde essa medição.
    body_height: Option<f32>,
    footer_height: f32,
}

/// Abre o cartão de alguém (ou troca o que estava aberto) e pede a ficha
/// fresca. O lote de perfis já trouxe o bastante para desenhar na hora.
pub fn open(state: &mut UiState, user_id: &str, anchor: Anchor, now: f64) {
    if let Some(card) = &state.profile
        && card.user_id == user_id
    {
        // Clicar de novo em quem já está aberto fecha, como os painéis.
        state.profile = None;
        return;
    }
    state.profile = Some(ProfileCard {
        user_id: user_id.to_owned(),
        anchor,
        opened: now,
        about_expanded: false,
        body_height: None,
        footer_height: 0.0,
    });
    state
        .actions
        .push(ChatAction::LoadProfile(user_id.to_owned()));
}

/// Retângulo do cartão na janela. `height` é o que o conteúdo quer; o que
/// volta já cabe na tela (o corpo rola quando não cabe).
pub fn place(anchor: Anchor, height: f32, screen: Rect) -> Rect {
    let inner = screen.shrink(MARGIN);
    match anchor {
        Anchor::Pill(pill) => {
            // A pastilha é a base: o cartão tem a largura dela e cresce
            // para cima a partir da borda de baixo.
            let top = (pill.max.y - height).max(screen.min.y + GAP);
            Rect::from_min_max(
                egui::pos2(pill.min.x, top),
                egui::pos2(pill.max.x, pill.max.y),
            )
        }
        Anchor::AbovePill(pill) => {
            let bottom = pill.min.y - GAP;
            let top = (bottom - height).max(inner.min.y);
            let left = pill.min.x.min(inner.max.x - CARD_WIDTH).max(inner.min.x);
            Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + CARD_WIDTH, bottom))
        }
        Anchor::Beside(target) => {
            // À direita quando cabe; senão à esquerda (a lista de pessoas
            // fica na borda direita da janela).
            let right = target.max.x + GAP;
            let left = if right + CARD_WIDTH <= inner.max.x {
                right
            } else {
                (target.min.x - GAP - CARD_WIDTH).max(inner.min.x)
            };
            let height = height.min(inner.height());
            let top = target.min.y.min(inner.max.y - height).max(inner.min.y);
            Rect::from_min_size(egui::pos2(left, top), Vec2::new(CARD_WIDTH, height))
        }
    }
}

/// Retângulo da folha no celular: presa ao pé, centrada quando a tela é
/// larga.
pub(crate) fn place_sheet(height: f32, screen: Rect) -> Rect {
    let width = screen.width().min(SHEET_MAX_WIDTH);
    let height = height.min(screen.height() * SHEET_MAX);
    Rect::from_min_size(
        egui::pos2(screen.center().x - width / 2.0, screen.max.y - height),
        Vec2::new(width, height),
    )
}

/// Desenha o cartão aberto, se houver. Mora na camada de cima; um bloqueio
/// logo abaixo dele recebe qualquer clique fora, fecha o cartão e não deixa
/// o clique chegar à conversa.
/// `back` é o voltar do sistema neste quadro, já roteado pelo app: quem
/// decide se o voltar é nosso é um lugar só, para que um cartão fechado não
/// desligue o voltar de outro que está aberto.
pub fn draw(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, s: &Strings, back: bool) {
    let Some(mut card) = state.profile.take() else {
        return;
    };
    let ctx = ui.ctx().clone();
    let Some(member) = store.member(&card.user_id).cloned() else {
        // A pessoa saiu da lista (banida, servidor trocado): nada a mostrar.
        return;
    };

    let screen = ctx.content_rect();
    let compact = state.compact;
    let now = ctx.input(|input| input.time);
    let motion = ui.style().animation_time > 0.0;
    let progress = if motion {
        (((now - card.opened) / OPEN_SECONDS) as f32).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let eased = 1.0 - (1.0 - progress).powi(3);
    if progress < 1.0 || card.body_height.is_none() {
        ctx.request_repaint();
    }

    let narrow = !compact && matches!(card.anchor, Anchor::Pill(_));
    let wanted = card.body_height.unwrap_or(screen.height()) + card.footer_height;
    let rect = if compact {
        place_sheet(wanted, screen)
    } else {
        place(card.anchor, wanted, screen)
    };

    let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("perfil-cartao"));
    // The card is a real blocking surface: keep it above foreground
    // sidebars/drawers throughout its lifetime, including dismissal frames.
    ctx.move_to_top(layer);
    let mut close = back || ctx.input(|input| input.key_pressed(egui::Key::Escape));
    let mut outcome = Outcome::default();
    // Uma Area de verdade, do tamanho da tela: é assim que o egui sabe que
    // esta camada está por cima — a roda do mouse rola o cartão, não a
    // conversa atrás dele.
    egui::Area::new(layer.id)
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(&ctx, |ui| {
            ui.set_min_size(screen.size());

            // Bloqueio: a tela inteira, registrada antes do cartão. No egui ganha o
            // último widget sob o ponteiro, então dentro do cartão quem responde é
            // ele; fora, é isto — que fecha e não deixa o clique chegar à conversa.
            // No celular ele escurece a conversa, como uma folha do sistema.
            let blocker = ui.interact(screen, Id::new("perfil-bloqueio"), Sense::click_and_drag());
            if compact {
                // A camada sobe junto com a folha; o véu sobra por cima para não
                // deixar uma faixa clara durante a animação.
                ui.painter().rect_filled(
                    screen.expand2(Vec2::new(0.0, screen.height() * 0.25)),
                    CornerRadius::ZERO,
                    Color32::from_black_alpha((90.0 * eased) as u8),
                );
            }
            close |= blocker.clicked();

            // Abertura: sobe alguns pontos e aparece. No estilo da pastilha ele
            // cresce do pé, que é onde a pastilha estava.
            let lift = if compact { rect.height() * 0.25 } else { 6.0 };
            ctx.set_transform_layer(
                layer,
                egui::emath::TSTransform::from_translation(Vec2::new(0.0, (1.0 - eased) * lift)),
            );
            ui.multiply_opacity(if card.body_height.is_some() {
                eased
            } else {
                0.0
            });

            let corners = if compact {
                CornerRadius {
                    nw: radius::SHEET,
                    ne: radius::SHEET,
                    sw: 0,
                    se: 0,
                }
            } else {
                CornerRadius::same(radius::SHEET)
            };
            ui.painter()
                .add(ui.visuals().window_shadow.as_shape(rect, corners));
            ui.painter().rect(
                rect,
                corners,
                t.elevated_bg,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            );
            // O cartão também é um alvo: um clique dentro dele não é "fora".
            ui.interact(rect, Id::new("perfil-superficie"), Sense::click());

            let inset = if narrow { space::LG } else { space::XL };
            let body_max = (rect.height() - card.footer_height).max(0.0);
            let body_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), body_max));

            let body = ui.scope_builder(UiBuilder::new().max_rect(body_rect), |ui| {
                ui.set_clip_rect(body_rect);
                egui::ScrollArea::vertical()
                    .id_salt(("perfil-corpo", &card.user_id))
                    .max_height(body_max)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        body_contents(
                            ui,
                            store,
                            state,
                            t,
                            s,
                            &member,
                            &mut card,
                            rect,
                            corners,
                            narrow,
                            inset,
                            compact,
                            &mut outcome,
                        );
                    })
            });
            let content = body.inner.content_size.y;
            if card.body_height != Some(content) {
                card.body_height = Some(content);
                ctx.request_repaint();
            }

            // Ações fixas no pé: nunca rolam para fora.
            let footer_top = rect.min.y + body_max.min(content);
            let footer_rect = Rect::from_min_max(egui::pos2(rect.min.x, footer_top), rect.max);
            ui.painter().line_segment(
                [
                    egui::pos2(rect.min.x, footer_top),
                    egui::pos2(rect.max.x, footer_top),
                ],
                Stroke::new(1.0, t.separator),
            );
            let footer = ui.scope_builder(
                UiBuilder::new()
                    .max_rect(footer_rect.shrink(space::MD))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(0.0, space::XXS);
                    footer_contents(
                        ui,
                        store,
                        state,
                        t,
                        s,
                        &member,
                        narrow,
                        compact,
                        rect,
                        &mut outcome,
                    );
                },
            );
            let footer_height = footer.response.rect.height() + space::MD * 2.0;
            if (footer_height - card.footer_height).abs() > 0.5 {
                card.footer_height = footer_height;
                ctx.request_repaint();
            }
        });

    if let Some(expand) = outcome.toggle_about {
        card.about_expanded = expand;
    }
    if outcome.close {
        close = true;
    }
    if !close {
        state.profile = Some(card);
    }
}

/// O que foi clicado na prévia do cabeçalho, nos Ajustes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewHit {
    None,
    Banner,
    Avatar,
}

/// O que o cartão editável deixa mudar no próprio cartão.
pub struct CardDraft<'a> {
    pub nickname: &'a mut String,
    pub status: &'a mut String,
    pub description: &'a mut String,
}

/// Respiro de um campo escondido: só aparece no destaque, o texto fica no
/// alinhamento do cartão.
const FIELD_PAD: Vec2 = Vec2::new(space::SM, space::XS);

/// Altura que o texto de um campo ocupa (sem o respiro), entre `rows.0` e
/// `rows.1` linhas. Vazio mede a dica, que ocupa o lugar do texto.
fn field_height(
    ui: &egui::Ui,
    key: &str,
    font: &egui::FontId,
    value: &str,
    hint: &str,
    width: f32,
    rows: (usize, usize),
) -> (f32, usize) {
    // Campo de várias linhas: tamanho A numa linha, B quando quebra.
    let multiline = rows.1 > 1;
    let font = if multiline { super::theme::adaptive_font(ui, font, value, width) } else { font.clone() };
    // No Android quem mede é o campo nativo (outra fonte, outra quebra): a
    // caixa segue as linhas dele.
    #[cfg(target_os = "android")]
    if multiline
        && !value.is_empty()
        && let Some((native_lines, line_h)) = crate::platform::native_field::measured(&format!("perfil:{key}"))
    {
        let lines = native_lines.clamp(rows.0, rows.1);
        return (lines as f32 * line_h, lines);
    }
    #[cfg(not(target_os = "android"))]
    let _ = key;
    let shown = if value.is_empty() { hint } else { value };
    let lines = ui
        .painter()
        .layout(shown.to_owned(), font.clone(), Color32::WHITE, width.max(24.0))
        .rows
        .len()
        .clamp(rows.0, rows.1);
    let row_h = ui.fonts_mut(|fonts| fonts.row_height(&font));
    (lines as f32 * row_h, lines)
}

/// Campo "escondido": parece o conteúdo do cartão e só mostra que é um campo
/// quando o ponteiro passa ou ele ganha o foco. Vazio, mostra a dica em
/// itálico onde o texto ficaria. Devolve se está com o foco.
///
/// `text_area` é onde o texto fica; o destaque sai um pouco para fora dele.
#[allow(clippy::too_many_arguments)]
fn hidden_field(
    ui: &mut egui::Ui,
    t: &Tokens,
    key: &str,
    text_area: Rect,
    value: &mut String,
    hint: &str,
    font: &egui::FontId,
    color: Color32,
    limit: usize,
    lines: usize,
    multiline: bool,
) -> bool {
    let outer = text_area.expand2(FIELD_PAD);
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let hovered = ui.rect_contains_pointer(outer);
    // O mesmo tamanho A/B que `field_height` usou para medir.
    let shown_font = if multiline {
        super::theme::adaptive_font(ui, font, value, text_area.width())
    } else {
        font.clone()
    };

    #[cfg(not(target_os = "android"))]
    let focused = {
        let id = ui.id().with(("campo-escondido", key));
        let _ = crate::platform::ime::prepare_text_edit(ui.ctx(), id, value);
        let editor = if multiline {
            egui::TextEdit::multiline(value).desired_rows(lines)
        } else {
            egui::TextEdit::singleline(value)
        };
        let editor = editor
            .id(id)
            .char_limit(limit)
            .font(shown_font.clone())
            .text_color(color)
            .hint_text(
                RichText::new(hint)
                    .font(shown_font.clone())
                    .italics()
                    .color(t.label_tertiary),
            )
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .desired_width(text_area.width());
        let response = if multiline {
            // Passando do limite de linhas o texto rola dentro do campo (roda
            // do mouse ou arrasto), em vez de vazar para fora dele.
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_area));
            egui::ScrollArea::vertical()
                .max_height(text_area.height())
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(&mut child, |ui| ui.add(editor))
                .inner
        } else {
            ui.put(text_area, editor)
        };
        let _ = crate::platform::ime::sync_text_edit(
            ui.ctx(),
            id,
            value,
            response.has_focus(),
            if multiline {
                crate::platform::ime::Kind::Multiline
            } else {
                crate::platform::ime::Kind::Text
            },
        );
        response.has_focus()
    };
    #[cfg(target_os = "android")]
    let focused = {
        let _ = lines;
        crate::platform::native_field::show(
            ui.ctx(),
            &format!("perfil:{key}"),
            value,
            text_area,
            hint,
            if multiline {
                crate::platform::native_field::Mode::Multiline
            } else {
                crate::platform::native_field::Mode::Text
            },
            limit,
            false,
            color,
            t.label_tertiary,
            font.size,
        )
        .focused
    };

    let (fill, stroke) = if focused {
        (t.fill_soft, Stroke::new(1.5, t.accent))
    } else if hovered || cfg!(target_os = "android") {
        // No toque não há hover: o fundo suave fica sempre, para o campo
        // se reconhecer como campo.
        (t.fill_soft, Stroke::NONE)
    } else {
        (Color32::TRANSPARENT, Stroke::NONE)
    };
    ui.painter().set(
        backdrop,
        egui::epaint::RectShape::new(
            outer,
            CornerRadius::same(radius::CONTROL),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        ),
    );
    focused
}

/// Como `inline_field`, mas do tamanho do texto: o campo (e o realce) abraça
/// o que está escrito, ou a dica quando vazio, até o limite do espaço.
pub fn inline_field_fit(
    ui: &mut egui::Ui,
    t: &Tokens,
    key: &str,
    value: &mut String,
    hint: &str,
    font: egui::FontId,
    color: Color32,
    limit: usize,
) -> bool {
    let probe = if value.is_empty() { hint } else { value.as_str() };
    let text_w = ui.painter().layout_no_wrap(probe.to_owned(), font.clone(), color).size().x;
    let avail = ui.available_width();
    // Folga para o cursor e para a letra seguinte não quebrar a linha.
    let width = (text_w + 14.0).min(avail).max(24.0_f32.min(avail));
    let (height, lines) = field_height(ui, key, &font, value, hint, width, (1, 1));
    let (area, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    hidden_field(ui, t, key, area, value, hint, &font, color, limit, lines, false)
}

/// Mede, reserva e desenha um campo escondido na largura que sobrou.
#[allow(clippy::too_many_arguments)]
pub fn inline_field(
    ui: &mut egui::Ui,
    t: &Tokens,
    key: &str,
    value: &mut String,
    hint: &str,
    font: egui::FontId,
    color: Color32,
    limit: usize,
    rows: (usize, usize),
) -> bool {
    let width = ui.available_width();
    let (height, lines) = field_height(ui, key, &font, value, hint, width, rows);
    let (area, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    hidden_field(ui, t, key, area, value, hint, &font, color, limit, lines, rows.1 > 1)
}

/// O seu cartão, nos Ajustes, como ele fica para os outros — e cada parte é o
/// próprio campo: o recado no balão ao lado da foto, o apelido onde o nome
/// aparece, a bio na seção "Sobre". Nada de formulário: vazio mostra uma dica
/// em itálico no lugar do texto, e o campo só se revela ao passar o ponteiro
/// ou ao tocar. O banner e a foto continuam clicáveis para trocar a imagem.
pub fn edit_card(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    store: &Store,
    media: &mut crate::media::MediaStore,
    draft: CardDraft<'_>,
) -> PreviewHit {
    let ctx = ui.ctx().clone();
    let width = EDIT_CARD_WIDTH.min(ui.available_width());
    let scale = width / EDIT_CARD_WIDTH;
    let banner_h = (width / BANNER_RATIO).round();
    let avatar_size = 80.0 * scale;
    let overlap = (avatar_size * AVATAR_OVERLAP).round();
    let inset = space::XL * scale;
    let me = store.member(&store.me);
    let tint = me.and_then(|member| member.role_color).map(rgb).unwrap_or(t.accent);
    let corners = CornerRadius::same(radius::SHEET);
    let top_corners = CornerRadius {
        nw: corners.nw,
        ne: corners.ne,
        sw: 0,
        se: 0,
    };
    let mut hit = PreviewHit::None;

    ui.scope(|ui| {
        ui.set_width(width);
        let start = ui.cursor().min;
        let surface = ui.painter().add(egui::Shape::Noop);

        // --- Banner, foto e balão do recado ---------------------------------
        // O balão fica ao lado da foto, como no cartão dos outros; a altura
        // dele vem do texto de agora.
        let status_font = text::callout();
        let bubble_left = inset + avatar_size + space::LG;
        let bubble_width = (width - inset.min(space::MD + 2.0) - bubble_left).max(60.0);
        let text_width = bubble_width - space::MD * 2.0 - 4.0;
        let (status_h, status_lines) = field_height(
            ui,
            "recado",
            &status_font,
            draft.status,
            s.profile_status_hint,
            text_width,
            (1, 3),
        );
        let bubble_h = status_h + space::SM * 2.0 + 2.0;
        let below = (avatar_size - overlap).max(space::SM + bubble_h);

        let (slot, _) =
            ui.allocate_exact_size(Vec2::new(width, banner_h + below), Sense::hover());
        let banner = Rect::from_min_size(slot.min, Vec2::new(width, banner_h));
        let avatar = Rect::from_min_size(
            egui::pos2(slot.min.x + inset, banner.max.y - overlap),
            Vec2::splat(avatar_size),
        );

        // A foto se registra depois do banner: onde os dois se sobrepõem,
        // ganha ela.
        let banner_response = ui.interact(banner, Id::new("previa-banner"), Sense::click());
        let avatar_response = ui.interact(avatar, Id::new("previa-foto"), Sense::click());
        let avatar_hovered = avatar_response.hovered();
        let banner_hovered = banner_response.hovered() && !avatar_hovered;

        let picture = store
            .profiles
            .get(&store.me)
            .and_then(|details| details.banner.as_deref())
            .and_then(|sha| media.banner(sha))
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| (handle.id(), handle.size_vec2()));
        let painter = ui.painter();
        match picture {
            Some((texture, size)) => photo(
                painter,
                banner,
                texture,
                cover_uv(size, banner.size()),
                top_corners,
                Color32::WHITE,
            ),
            None => {
                // Sem banner: a faixa da cor de destaque que vai ao rosa,
                // em tiras (o painter não tem gradiente com cantos).
                const STRIPS: usize = 34;
                let from = t.accent;
                let to = blend(t.accent, Color32::from_rgb(0xFF, 0x4F, 0x8B), 0.65);
                let strip_w = banner.width() / STRIPS as f32;
                for i in 0..STRIPS {
                    let amount = i as f32 / (STRIPS - 1) as f32;
                    let x0 = banner.min.x + strip_w * i as f32;
                    let rect = Rect::from_min_max(egui::pos2(x0, banner.min.y), egui::pos2(x0 + strip_w + 0.5, banner.max.y));
                    let corners = CornerRadius {
                        nw: if i == 0 { top_corners.nw } else { 0 },
                        ne: if i == STRIPS - 1 { top_corners.ne } else { 0 },
                        sw: 0,
                        se: 0,
                    };
                    painter.rect_filled(rect, corners, blend(from, to, amount));
                }
            }
        }
        if banner_hovered {
            painter.rect_filled(banner, top_corners, Color32::from_black_alpha(90));
            painter.text(
                egui::pos2(banner.max.x - space::MD, banner.min.y + space::MD),
                Align2::RIGHT_TOP,
                s.change_banner,
                text::callout(),
                Color32::WHITE,
            );
        }

        painter.circle_filled(avatar.center(), avatar_size / 2.0 + 4.0, t.card);
        let face = media
            .avatar(&store.me, store.avatars.get(&store.me).map(String::as_str))
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| handle.id());
        let painter = ui.painter();
        match face {
            Some(texture) => round_photo(painter, avatar, texture, Color32::WHITE),
            None => {
                painter.circle_filled(
                    avatar.center(),
                    avatar_size / 2.0,
                    blend(t.card, tint, 0.30),
                );
                painter.text(
                    avatar.center(),
                    Align2::CENTER_CENTER,
                    me.map(|member| member.initials()).unwrap_or_default(),
                    egui::FontId::new(
                        (avatar_size * 0.32).round(),
                        egui::FontFamily::Name("semibold".into()),
                    ),
                    tint,
                );
            }
        }
        if avatar_hovered {
            painter.circle_filled(avatar.center(), avatar_size / 2.0, Color32::from_black_alpha(110));
            painter.text(
                avatar.center(),
                Align2::CENTER_CENTER,
                icon::CAMERA,
                text::icon(avatar_size * 0.3),
                Color32::WHITE,
            );
        }
        if avatar_hovered || banner_hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if avatar_response.clicked() {
            hit = PreviewHit::Avatar;
        } else if banner_response.clicked() {
            hit = PreviewHit::Banner;
        }

        // Balão de pensamento, com o recado dentro: o mesmo do cartão.
        let bubble = Rect::from_min_size(
            egui::pos2(slot.min.x + bubble_left, banner.max.y + space::SM),
            Vec2::new(bubble_width, bubble_h),
        );
        let painter = ui.painter();
        painter.rect_filled(bubble, CornerRadius::same(radius::CARD), t.fill_medium);
        painter.circle_filled(
            egui::pos2(bubble.min.x - 3.0, bubble.min.y + 8.0),
            4.5,
            t.fill_medium,
        );
        painter.circle_filled(
            egui::pos2(bubble.min.x - 9.0, bubble.min.y + 3.0),
            2.5,
            t.fill_medium,
        );
        let status_area = Rect::from_min_size(
            bubble.min + Vec2::new(space::MD + 2.0, space::SM + 1.0),
            Vec2::new(text_width, status_h),
        );
        hidden_field(
            ui,
            t,
            "recado",
            status_area,
            draft.status,
            s.profile_status_hint,
            &status_font,
            t.label,
            64,
            status_lines,
            true,
        );

        // --- Nome --------------------------------------------------------------
        ui.add_space(space::MD);
        padded(ui, inset, |ui| {
            inline_field(
                ui,
                t,
                "apelido",
                draft.nickname,
                s.profile_name_hint,
                text::title2(),
                t.label,
                32,
                (1, 1),
            );
            ui.add_space(1.0);
            if let Some(member) = me {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("@{}", member.username))
                            .font(text::callout())
                            .color(t.label_secondary),
                    )
                    .truncate(),
                );
            }
        });
        ui.add_space(space::LG);

        // --- Cargos (só leitura) ------------------------------------------------
        let roles: Vec<(String, Color32)> = store
            .profiles
            .get(&store.me)
            .map(|details| {
                details
                    .roles
                    .iter()
                    .map(|role| {
                        let color = role
                            .color
                            .as_deref()
                            .and_then(papo_core::api::models::parse_hex_color)
                            .map(rgb)
                            .unwrap_or(t.label_tertiary);
                        (role.name.clone(), color)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !roles.is_empty() {
            section(ui, t, inset, |ui| {
                caption(ui, t, s.profile_roles);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(space::SM);
                    for (name, color) in &roles {
                        role_chip(ui, t, name, *color);
                    }
                });
            });
        }

        // --- Sobre --------------------------------------------------------------
        section(ui, t, inset, |ui| {
            caption(ui, t, s.profile_about);
            let focused = inline_field(
                ui,
                t,
                "sobre",
                draft.description,
                s.profile_about_hint,
                text::body(),
                t.label,
                512,
                (1, 8),
            );
            let used = draft.description.chars().count();
            if focused && used > 0 {
                ui.add_space(space::XS);
                ui.label(
                    RichText::new(format!("{used}/512"))
                        .font(text::footnote())
                        .color(t.label_tertiary),
                );
            }
        });

        // A superfície do cartão, por trás de tudo, do tamanho que deu.
        let end = ui.min_rect().max.y;
        ui.painter().set(
            surface,
            egui::epaint::RectShape::new(
                Rect::from_min_max(start, egui::pos2(start.x + width, end)),
                corners,
                t.card,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            ),
        );
    });
    hit
}

/// O que as partes do cartão pediram neste quadro.
#[derive(Default)]
struct Outcome {
    close: bool,
    toggle_about: Option<bool>,
}

#[allow(clippy::too_many_arguments)]
fn body_contents(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    member: &Member,
    card: &mut ProfileCard,
    rect: Rect,
    corners: CornerRadius,
    narrow: bool,
    inset: f32,
    compact: bool,
    outcome: &mut Outcome,
) {
    let ctx = ui.ctx().clone();
    let width = rect.width();
    let details = store.profiles.get(&member.id);
    let is_me = member.id == store.me;
    let tint = member.role_color.map(rgb).unwrap_or(t.accent);

    // --- Banner ------------------------------------------------------------
    let banner_h = (width / BANNER_RATIO).round();
    let (banner, _) = ui.allocate_exact_size(Vec2::new(width, banner_h), Sense::hover());
    let banner_corners = CornerRadius {
        nw: corners.nw,
        ne: corners.ne,
        sw: 0,
        se: 0,
    };
    let picture = details
        .and_then(|details| details.banner.as_deref())
        .and_then(|sha| state.media.banner(sha))
        .and_then(|texture| texture.frame(&ctx))
        .map(|handle| (handle.id(), handle.size_vec2()));
    match picture {
        Some((texture, size)) => photo(
            ui.painter(),
            banner,
            texture,
            cover_uv(size, banner.size()),
            banner_corners,
            Color32::WHITE,
        ),
        // Sem banner: a cor do cargo, bem baixa, sobre a superfície. Nada
        // de gradiente — o cartão não é uma vitrine.
        None => {
            ui.painter()
                .rect_filled(banner, banner_corners, blend(t.card, tint, 0.38));
        }
    }
    if compact {
        // Alça da folha.
        let grab = Rect::from_center_size(
            egui::pos2(banner.center().x, banner.min.y + 9.0),
            Vec2::new(36.0, 5.0),
        );
        ui.painter()
            .rect_filled(grab, CornerRadius::same(3), Color32::from_white_alpha(130));
    }
    banner_buttons(ui, store, state, s, member, banner, is_me, narrow, outcome);

    // --- Foto e recado -----------------------------------------------------
    let avatar_size = if narrow { 64.0 } else { 80.0 };
    let ring = 4.0;
    let overlap = (avatar_size * AVATAR_OVERLAP).round();
    let avatar_rect = Rect::from_min_size(
        egui::pos2(rect.min.x + inset, banner.max.y - overlap),
        Vec2::splat(avatar_size),
    );
    let status = member
        .status_message
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let bubble_left = avatar_rect.max.x + space::LG;
    let bubble_width = rect.max.x - inset.min(space::MD + 2.0) - bubble_left;
    let bubble = status.filter(|_| bubble_width > 60.0).map(|status| {
        let rows = if narrow { 3 } else { 2 };
        let galley = clamp_rows(
            ui,
            status,
            text::callout(),
            t.label,
            bubble_width - space::LG * 2.0 + 4.0,
            rows,
        );
        let size = galley.size() + Vec2::new(space::MD * 2.0 + 4.0, space::SM * 2.0 + 2.0);
        (
            galley,
            Rect::from_min_size(egui::pos2(bubble_left, banner.max.y + space::SM), size),
        )
    });
    let head_bottom = avatar_rect
        .max
        .y
        .max(bubble.as_ref().map(|(_, rect)| rect.max.y).unwrap_or(0.0));
    ui.allocate_exact_size(Vec2::new(width, head_bottom - banner.max.y), Sense::hover());

    let painter = ui.painter();
    painter.circle_filled(
        avatar_rect.center(),
        avatar_size / 2.0 + ring,
        t.card,
    );
    let face = state
        .media
        .avatar(
            &member.id,
            store.avatars.get(&member.id).map(String::as_str),
        )
        .and_then(|texture| texture.frame(&ctx))
        .map(|handle| handle.id());
    match face {
        Some(texture) => round_photo(painter, avatar_rect, texture, Color32::WHITE),
        None => {
            painter.circle_filled(
                avatar_rect.center(),
                avatar_size / 2.0,
                blend(t.card, tint, 0.30),
            );
            painter.text(
                avatar_rect.center(),
                Align2::CENTER_CENTER,
                member.initials(),
                egui::FontId::new(
                    (avatar_size * 0.32).round(),
                    egui::FontFamily::Name("semibold".into()),
                ),
                tint,
            );
        }
    }
    let dot_r = if narrow { 6.0 } else { 7.0 };
    let dot = avatar_rect.center() + Vec2::splat(avatar_size / 2.0 * 0.72);
    painter.circle_filled(dot, dot_r + ring, t.card);
    painter.circle_filled(dot, dot_r, presence_color(t, member.presence));

    if let Some((galley, bubble)) = bubble {
        // Balão de pensamento: duas bolinhas levam até a foto.
        painter.rect_filled(bubble, CornerRadius::same(radius::CARD), t.fill_medium);
        painter.circle_filled(
            egui::pos2(bubble.min.x - 3.0, bubble.min.y + 8.0),
            4.5,
            t.fill_medium,
        );
        painter.circle_filled(
            egui::pos2(bubble.min.x - 9.0, bubble.min.y + 3.0),
            2.5,
            t.fill_medium,
        );
        painter.galley(
            bubble.min + Vec2::new(space::MD + 2.0, space::SM + 1.0),
            galley,
            t.label,
        );
    }

    // --- Nome ----------------------------------------------------------------
    ui.add_space(space::MD);
    padded(ui, inset, |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(&member.name)
                    .font(text::title2())
                    .color(t.label),
            )
            .truncate(),
        );
        ui.add_space(1.0);
        ui.add(
            egui::Label::new(
                RichText::new(format!("@{}", member.username))
                    .font(text::callout())
                    .color(t.label_secondary),
            )
            .truncate(),
        );
    });
    ui.add_space(space::LG);

    // --- Atividade -----------------------------------------------------------
    if let Some(activity) = store.activities.get(&member.id) {
        section(ui, t, inset, |ui| {
            activity_block(ui, t, s, activity, narrow)
        });
    }

    // --- Cargos --------------------------------------------------------------
    let roles: Vec<(String, Color32)> = details
        .map(|details| {
            details
                .roles
                .iter()
                .map(|role| {
                    let color = role
                        .color
                        .as_deref()
                        .and_then(papo_core::api::models::parse_hex_color)
                        .map(rgb)
                        .unwrap_or(t.label_tertiary);
                    (role.name.clone(), color)
                })
                .collect()
        })
        .unwrap_or_default();
    if !roles.is_empty() {
        section(ui, t, inset, |ui| {
            caption(ui, t, s.profile_roles);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(space::SM);
                for (name, color) in &roles {
                    role_chip(ui, t, name, *color);
                }
            });
        });
    }

    // --- Sobre ---------------------------------------------------------------
    if let Some(about) = details.and_then(|details| details.description.as_deref()) {
        section(ui, t, inset, |ui| {
            caption(ui, t, s.profile_about);
            let wrap = ui.available_width();
            let full = ui
                .painter()
                .layout(about.to_owned(), text::body(), t.label, wrap);
            let long = full.rows.len() > ABOUT_ROWS;
            let galley = if long && !card.about_expanded {
                clamp_rows(ui, about, text::body(), t.label, wrap, ABOUT_ROWS)
            } else {
                full
            };
            let (slot, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            ui.painter().galley(slot.min, galley, t.label);
            if long {
                ui.add_space(space::XS);
                let label = if card.about_expanded {
                    s.profile_show_less
                } else {
                    s.profile_show_all
                };
                let link = ui.add(
                    egui::Label::new(RichText::new(label).font(text::callout()).color(t.accent))
                        .sense(Sense::click()),
                );
                if link.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if link.clicked() {
                    outcome.toggle_about = Some(!card.about_expanded);
                }
            }
        });
    }

    // --- Desde quando --------------------------------------------------------
    if let Some(since) = details.and_then(|details| details.created_at) {
        padded(ui, inset, |ui| {
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    s.profile_member_since,
                    member_since(since)
                ))
                .font(text::footnote())
                .color(t.label_tertiary),
            );
        });
        ui.add_space(space::LG);
    }
}

/// Os botões de vidro no canto do banner. Pequenos e translúcidos: é o único
/// lugar do cartão onde vidro cabe, porque são controles, não texto.
#[allow(clippy::too_many_arguments)]
fn banner_buttons(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    s: &Strings,
    member: &Member,
    banner: Rect,
    is_me: bool,
    narrow: bool,
    outcome: &mut Outcome,
) {
    let size = 28.0;
    let mut right = banner.max.x - space::MD;
    let mut slot = |ui: &mut egui::Ui, glyph: &str, tip: &str, enabled: bool, id: &str| {
        let rect = Rect::from_min_size(
            egui::pos2(right - size, banner.min.y + space::MD),
            Vec2::splat(size),
        );
        right -= size + space::XS;
        let response = ui.interact(rect, Id::new(("perfil-banner", id)), Sense::click());
        let hovered = response.hovered() && enabled;
        ui.painter().rect(
            rect,
            CornerRadius::same(radius::FIELD),
            Color32::from_black_alpha(if hovered { 110 } else { 72 }),
            Stroke::new(1.0, Color32::from_white_alpha(30)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            text::icon(14.0),
            Color32::from_white_alpha(if enabled { 235 } else { 120 }),
        );
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let response = response.on_hover_text(tip);
        (response.clicked() && enabled, response)
    };

    if is_me {
        if narrow {
            // A engrenagem da pastilha continua no mesmo canto.
            if slot(ui, icon::GEAR_SIX, s.menu_preferences, true, "ajustes").0 {
                state.pending.push(MenuCommand::Preferences);
                outcome.close = true;
            }
        } else if slot(ui, icon::PENCIL_SIMPLE, s.profile_edit, true, "editar").0 {
            state.actions.push(ChatAction::EditProfile);
            outcome.close = true;
        }
        return;
    }

    let (_, more) = slot(ui, icon::DOTS_THREE, s.profile_more, true, "mais");
    egui::Popup::menu(&more).frame(crate::ui::widgets::menu_frame(ui.ctx())).show(|ui| {
        if ui.button(s.profile_copy_id).clicked() {
            crate::platform::copy::text(ui.ctx(), member.id.clone());
            state.error = Some((s.profile_id_copied.to_owned(), ui.input(|input| input.time)));
            ui.close();
        }
        ui.separator();
        let blocked = store.is_user_blocked(&member.id);
        let block_label = if blocked { s.unblock_user } else { s.block_user };
        if ui.button(block_label).clicked() {
            state.actions.push(ChatAction::SetUserBlocked {
                user_id: member.id.clone(),
                blocked: !blocked,
            });
            outcome.close = true;
            ui.close();
        }
        ui.separator();
        let (label, banned) = if member.banned {
            (s.unban_user, false)
        } else {
            (s.ban_user, true)
        };
        if ui.button(label).clicked() {
            state.actions.push(ChatAction::BanUser {
                user_id: member.id.clone(),
                banned,
            });
            ui.close();
        }
        if ui.button(s.reset_user).clicked() {
            state.actions.push(ChatAction::ResetUser(member.id.clone()));
            ui.close();
        }
    });
    let blocked = store.is_user_blocked(&member.id);
    if slot(
        ui,
        icon::CHAT_CIRCLE,
        if blocked { s.blocked_user } else { s.profile_dm_soon },
        !blocked,
        "conversa",
    )
    .0
    {
        state.actions.push(ChatAction::OpenDirectMessage(member.id.clone()));
        outcome.close = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn footer_contents(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    member: &Member,
    narrow: bool,
    compact: bool,
    card: Rect,
    outcome: &mut Outcome,
) {
    let row_h: f32 = if compact { 44.0 } else { 34.0 };
    if member.id != store.me {
        let blocked = store.is_user_blocked(&member.id);
        let label = if blocked {
            s.blocked_user.to_owned()
        } else {
            format!("{} @{}", s.profile_message_to, member.username)
        };
        if action_row(
            ui,
            t,
            Some(icon::CHAT_CIRCLE),
            None,
            &label,
            !blocked,
            row_h.max(36.0),
        )
        .on_hover_text(if blocked { s.blocked_user } else { s.profile_dm_soon })
        .clicked()
        {
            state.actions.push(ChatAction::OpenDirectMessage(member.id.clone()));
            outcome.close = true;
        }
        return;
    }

    let presences = [
        (Presence::Online, s.presence_online, None),
        (Presence::Away, s.presence_away, Some("away")),
        (Presence::Busy, s.presence_busy, Some("busy")),
    ];
    if narrow {
        // Na largura da pastilha o segmentado não cabe: vira uma linha que
        // abre a escolha.
        let current = presences
            .iter()
            .find(|(presence, _, _)| *presence == member.presence)
            .map(|(_, label, _)| *label)
            .unwrap_or(s.presence_online);
        let row = action_row(
            ui,
            t,
            None,
            Some(presence_color(t, member.presence)),
            current,
            true,
            row_h,
        );
        // Sai do lado do cartão (desktop) ou por cima da linha (celular).
        let value = Rect::from_x_y_ranges(row.rect.center().x..=row.rect.max.x, row.rect.y_range());
        super::widgets::dropdown(&row, card, value, compact).show(|ui| {
            for (presence, label, status) in presences {
                let text = RichText::new(format!("●  {label}")).color(presence_color(t, presence));
                if ui.button(text).clicked() {
                    state
                        .actions
                        .push(ChatAction::SetPresence(status.map(str::to_owned)));
                    ui.close();
                }
            }
        });
    } else {
        let mut chosen = member.presence;
        if presence_segments(ui, t, &mut chosen, &presences) {
            let status = presences
                .iter()
                .find(|(presence, _, _)| *presence == chosen)
                .and_then(|(_, _, status)| *status);
            state
                .actions
                .push(ChatAction::SetPresence(status.map(str::to_owned)));
        }
        ui.add_space(space::XS);
    }
    if action_row(
        ui,
        t,
        Some(icon::PENCIL_SIMPLE),
        None,
        s.profile_edit,
        true,
        row_h,
    )
    .clicked()
    {
        state.actions.push(ChatAction::EditProfile);
        outcome.close = true;
    }
    if action_row(
        ui,
        t,
        Some(icon::COPY),
        None,
        s.profile_copy_id,
        false,
        row_h,
    )
    .clicked()
    {
        crate::platform::copy::text(ui.ctx(), member.id.clone());
        state.error = Some((s.profile_id_copied.to_owned(), ui.input(|input| input.time)));
    }
}

/// Linha de ação do pé: ícone (ou ponto de cor), rótulo e, se leva a outro
/// lugar, a seta.
fn action_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    glyph: Option<&str>,
    dot: Option<Color32>,
    label: &str,
    chevron: bool,
    height: f32,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::FIELD), t.fill_soft);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let lead = egui::pos2(rect.min.x + space::MD + 7.0, rect.center().y);
    if let Some(glyph) = glyph {
        ui.painter().text(
            lead,
            Align2::CENTER_CENTER,
            glyph,
            text::icon(15.0),
            t.label_secondary,
        );
    }
    if let Some(dot) = dot {
        ui.painter().circle_filled(lead, 4.0, dot);
    }
    ui.painter().text(
        egui::pos2(rect.min.x + space::MD + 22.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        text::body(),
        t.label,
    );
    if chevron {
        ui.painter().text(
            egui::pos2(rect.max.x - space::MD, rect.center().y),
            Align2::RIGHT_CENTER,
            icon::CARET_RIGHT,
            text::icon(12.0),
            t.label_tertiary,
        );
    }
    response
}

/// Segmentado de presença com o ponto de cor de cada uma. Devolve `true`
/// quando a escolha muda.
fn presence_segments(
    ui: &mut egui::Ui,
    t: &Tokens,
    current: &mut Presence,
    options: &[(Presence, &str, Option<&str>)],
) -> bool {
    let height = 26.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
    let width = rect.width() / options.len() as f32;
    let mut changed = false;
    for (index, (presence, label, _)) in options.iter().enumerate() {
        let slot = Rect::from_min_size(
            egui::pos2(rect.min.x + width * index as f32, rect.min.y),
            Vec2::new(width, height),
        );
        let response = ui.interact(slot, ui.id().with(("presenca", index)), Sense::click());
        let selected = *current == *presence;
        if selected {
            ui.painter().rect(
                slot.shrink(2.0),
                CornerRadius::same(radius::CONTROL - 1),
                t.card,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let galley = ui
            .painter()
            .layout_no_wrap((*label).to_owned(), text::callout(), t.label);
        let total = galley.size().x + 12.0;
        let left = slot.center().x - total / 2.0;
        ui.painter().circle_filled(
            egui::pos2(left + 3.5, slot.center().y),
            3.5,
            presence_color(t, *presence),
        );
        ui.painter().galley(
            egui::pos2(left + 12.0, slot.center().y - galley.size().y / 2.0),
            galley,
            if selected { t.label } else { t.label_secondary },
        );
        if response.clicked() && !selected {
            *current = *presence;
            changed = true;
        }
    }
    changed
}

pub(crate) fn activity_block(ui: &mut egui::Ui, t: &Tokens, s: &Strings, activity: &Activity, narrow: bool) {
    let (verb, glyph, base) = match activity.kind {
        ActivityKind::Listening => (
            s.activity_listening,
            icon::MUSIC_NOTES,
            Color32::from_rgb(0x1F, 0x6F, 0x5C),
        ),
        ActivityKind::Playing => (
            s.activity_playing,
            icon::GAME_CONTROLLER,
            Color32::from_rgb(0x7A, 0x2A, 0x1E),
        ),
        ActivityKind::Working => (
            s.activity_working,
            icon::CODE,
            Color32::from_rgb(0x2B, 0x3A, 0x67),
        ),
    };
    // Ouvindo · Spotify; nos outros tipos o nome do app é o título.
    let listening = activity.kind == ActivityKind::Listening;
    caption(
        ui,
        t,
        &if listening {
            format!("{verb} · {}", activity.name)
        } else {
            verb.to_owned()
        },
    );
    let art = if narrow { 44.0 } else { 56.0 };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::LG;
        let (slot, _) = ui.allocate_exact_size(Vec2::splat(art), Sense::hover());
        match activity.image.as_deref().and_then(|path| activity_art(ui.ctx(), path)) {
            Some(texture) => {
                egui::Image::new((texture.id(), slot.size()))
                    .corner_radius(CornerRadius::same(radius::FIELD))
                    .paint_at(ui, slot);
            }
            None => {
                ui.painter()
                    .rect_filled(slot, CornerRadius::same(radius::FIELD), base);
                ui.painter().text(
                    slot.center(),
                    Align2::CENTER_CENTER,
                    glyph,
                    text::icon(art * 0.45),
                    Color32::WHITE,
                );
            }
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let (title, first, second) = if listening {
                (
                    activity.details.as_deref().unwrap_or(&activity.name),
                    activity.state.as_deref(),
                    None,
                )
            } else {
                (
                    activity.name.as_str(),
                    activity.details.as_deref(),
                    activity.state.as_deref(),
                )
            };
            ui.add(
                egui::Label::new(RichText::new(title).font(text::headline()).color(t.label))
                    .truncate(),
            );
            for line in [first, second].into_iter().flatten() {
                ui.add(
                    egui::Label::new(
                        RichText::new(line)
                            .font(text::callout())
                            .color(t.label_secondary),
                    )
                    .truncate(),
                );
            }
            let now = Utc::now();
            match (activity.started_at, activity.ends_at) {
                (Some(start), Some(end)) if end > start => {
                    progress_bar(ui, t, start, end, now);
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs(1));
                }
                (Some(start), _) => {
                    ui.label(
                        RichText::new(format!("{} {}", s.activity_for, elapsed(now - start)))
                            .font(text::callout())
                            .color(t.label_tertiary),
                    );
                }
                _ => {}
            }
        });
    });
}

/// Arte da atividade guardada em disco. Decodifica uma vez por arquivo e
/// versão (a data de modificação muda quando a pessoa troca a imagem) e
/// guarda a textura na memória do egui; uma falha também fica guardada,
/// para não reler o arquivo a cada quadro.
pub(crate) fn activity_art(ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
    use base64::Engine as _;

    let data_uri = path.strip_prefix("data:image/");
    let modified = data_uri
        .is_none()
        .then(|| std::fs::metadata(path).and_then(|meta| meta.modified()).ok())
        .flatten();
    let id = egui::Id::new(("activity-art", path, modified));
    if let Some(cached) = ctx.data(|data| data.get_temp::<Option<egui::TextureHandle>>(id)) {
        return cached;
    }

    let decoded = if data_uri.is_some() {
        path.split_once(";base64,")
            .and_then(|(_, encoded)| {
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()
            })
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
    } else {
        image::open(path).ok()
    };

    let texture = decoded.map(|decoded| {
        let rgba = decoded.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        ctx.load_texture(
            format!("activity-art:{id:?}"),
            egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
            egui::TextureOptions::LINEAR,
        )
    });
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

fn progress_bar(
    ui: &mut egui::Ui,
    t: &Tokens,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: DateTime<Utc>,
) {
    let total = (end - start).num_seconds().max(1);
    let done = (now - start).num_seconds().clamp(0, total);
    let clock = |seconds: i64| format!("{}:{:02}", seconds / 60, seconds % 60);
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::MD;
        let font = text::footnote();
        ui.label(
            RichText::new(clock(done))
                .font(font.clone())
                .color(t.label_tertiary),
        );
        let end_label = ui
            .painter()
            .layout_no_wrap(clock(total), font.clone(), t.label_tertiary);
        let width = (ui.available_width() - end_label.size().x - space::MD).max(20.0);
        let (bar, _) = ui.allocate_exact_size(Vec2::new(width, 10.0), Sense::hover());
        let track = Rect::from_center_size(bar.center(), Vec2::new(bar.width(), 3.0));
        ui.painter()
            .rect_filled(track, CornerRadius::same(2), t.fill_medium);
        let mut filled = track;
        filled.set_width(track.width() * done as f32 / total as f32);
        ui.painter()
            .rect_filled(filled, CornerRadius::same(2), t.label_secondary);
        ui.label(
            RichText::new(clock(total))
                .font(font)
                .color(t.label_tertiary),
        );
    });
}

fn role_chip(ui: &mut egui::Ui, t: &Tokens, name: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_owned(), text::callout(), t.label);
    let size = Vec2::new(galley.size().x + space::MD * 2.0 + 14.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
    ui.painter().circle_filled(
        egui::pos2(rect.min.x + space::MD + 4.0, rect.center().y),
        4.0,
        color,
    );
    ui.painter().galley(
        egui::pos2(
            rect.min.x + space::MD + 14.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        t.label,
    );
}

/// Uma seção: o fio em cima começa no recuo do texto e vai até a borda,
/// como nos grupos dos Ajustes.
fn section(ui: &mut egui::Ui, t: &Tokens, inset: f32, contents: impl FnOnce(&mut egui::Ui)) {
    let top = ui.cursor().min.y;
    let left = ui.max_rect().min.x;
    let right = ui.max_rect().max.x;
    ui.painter().line_segment(
        [egui::pos2(left + inset, top), egui::pos2(right, top)],
        Stroke::new(1.0, t.separator),
    );
    ui.add_space(space::LG);
    padded(ui, inset, contents);
    ui.add_space(space::LG);
}

fn padded(ui: &mut egui::Ui, inset: f32, contents: impl FnOnce(&mut egui::Ui)) {
    let width = ui.max_rect().width() - inset * 2.0;
    ui.horizontal(|ui| {
        ui.add_space(inset);
        ui.vertical(|ui| {
            ui.set_width(width);
            contents(ui);
        });
    });
}

fn caption(ui: &mut egui::Ui, t: &Tokens, label: &str) {
    ui.label(
        RichText::new(label.to_uppercase())
            .font(text::caption())
            .color(t.label_tertiary),
    );
    ui.add_space(space::MD);
}

/// Texto quebrado em até `rows` linhas, com reticências na última.
fn clamp_rows(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    wrap: f32,
    rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, wrap);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = false;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

/// Recorte que cobre `target` sem distorcer a imagem.
fn cover_uv(source: Vec2, target: Vec2) -> Rect {
    let source_ratio = source.x / source.y.max(1.0);
    let target_ratio = target.x / target.y.max(1.0);
    if source_ratio > target_ratio {
        let visible = target_ratio / source_ratio;
        let start = (1.0 - visible) / 2.0;
        Rect::from_min_max(egui::pos2(start, 0.0), egui::pos2(start + visible, 1.0))
    } else {
        let visible = source_ratio / target_ratio;
        let start = (1.0 - visible) / 2.0;
        Rect::from_min_max(egui::pos2(0.0, start), egui::pos2(1.0, start + visible))
    }
}

fn blend(base: Color32, over: Color32, amount: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(
        mix(base.r(), over.r()),
        mix(base.g(), over.g()),
        mix(base.b(), over.b()),
    )
}

fn rgb([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn member_since(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%d/%m/%Y").to_string()
}

/// "1 h 12 min", "38 min", "2 d".
fn elapsed(duration: chrono::Duration) -> String {
    let minutes = duration.num_minutes().max(1);
    let (days, hours, minutes) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes} min"),
        (0, _) if minutes == 0 => format!("{hours} h"),
        (0, _) => format!("{hours} h {minutes} min"),
        _ => format!("{days} d"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(1200.0, 800.0))
    }

    #[test]
    fn pill_card_keeps_the_pill_width_and_bottom_edge() {
        let pill = Rect::from_min_size(egui::pos2(72.0, 746.0), Vec2::new(216.0, 46.0));
        let rect = place(Anchor::Pill(pill), 420.0, screen());
        assert_eq!(rect.min.x, pill.min.x);
        assert_eq!(rect.max.x, pill.max.x);
        assert_eq!(rect.max.y, pill.max.y);
        assert_eq!(rect.height(), 420.0);
    }

    #[test]
    fn tall_pill_card_stops_at_the_window_top() {
        let pill = Rect::from_min_size(egui::pos2(72.0, 746.0), Vec2::new(216.0, 46.0));
        let rect = place(Anchor::Pill(pill), 5000.0, screen());
        assert!(rect.min.y >= 0.0);
        assert_eq!(rect.max.y, pill.max.y);
    }

    #[test]
    fn floating_card_sits_above_the_pill() {
        let pill = Rect::from_min_size(egui::pos2(72.0, 746.0), Vec2::new(216.0, 46.0));
        let rect = place(Anchor::AbovePill(pill), 400.0, screen());
        assert_eq!(rect.width(), CARD_WIDTH);
        assert_eq!(rect.min.x, pill.min.x);
        assert!(rect.max.y < pill.min.y);
    }

    #[test]
    fn member_row_on_the_right_edge_opens_to_the_left() {
        let row = Rect::from_min_size(egui::pos2(1016.0, 60.0), Vec2::new(172.0, 32.0));
        let rect = place(Anchor::Beside(row), 400.0, screen());
        assert!(rect.max.x <= row.min.x);
        assert_eq!(rect.min.y, row.min.y);
    }

    #[test]
    fn chat_avatar_opens_to_the_right_and_stays_inside_the_window() {
        let face = Rect::from_min_size(egui::pos2(320.0, 700.0), Vec2::splat(36.0));
        let rect = place(Anchor::Beside(face), 400.0, screen());
        assert!(rect.min.x >= face.max.x);
        assert!(rect.max.y <= screen().max.y - MARGIN);
    }

    #[test]
    fn sheet_is_pinned_to_the_bottom_and_capped() {
        let phone = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(390.0, 800.0));
        let rect = place_sheet(5000.0, phone);
        assert_eq!(rect.max.y, phone.max.y);
        assert!(rect.height() <= phone.height() * SHEET_MAX + 0.5);
        assert_eq!(rect.width(), 390.0);
    }

    #[test]
    fn banner_cover_crop_keeps_the_aspect() {
        let uv = cover_uv(Vec2::new(1000.0, 1000.0), Vec2::new(300.0, 100.0));
        assert!((uv.width() - 1.0).abs() < 1e-6);
        assert!((uv.height() - 1.0 / 3.0).abs() < 1e-3);
    }

    #[test]
    fn elapsed_reads_like_a_person() {
        assert_eq!(elapsed(chrono::Duration::minutes(38)), "38 min");
        assert_eq!(elapsed(chrono::Duration::minutes(72)), "1 h 12 min");
        assert_eq!(elapsed(chrono::Duration::minutes(120)), "2 h");
    }

    /// Quadro do cartão editável sem janela: devolve o que foi pintado.
    struct CardHarness {
        ctx: egui::Context,
        t: Tokens,
        store: Store,
        media: crate::media::MediaStore,
        nickname: String,
        status: String,
        description: String,
    }

    impl CardHarness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::ui::theme::install_fonts(&ctx, None);
            let t = Tokens::new(crate::ui::theme::Appearance::Dark, None);
            crate::ui::theme::apply(&ctx, &t, false, 1.0);
            let mut harness = Self {
                ctx,
                t,
                store: Store::default(),
                media: crate::media::MediaStore::new(None),
                nickname: String::new(),
                status: String::new(),
                description: String::new(),
            };
            // As fontes só valem a partir do quadro seguinte.
            harness.frame(vec![]);
            harness
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(500.0, 1600.0))),
                events,
                ..Default::default()
            };
            let Self {
                ctx,
                t,
                store,
                media,
                nickname,
                status,
                description,
            } = self;
            let s = crate::i18n::Lang::En.strings();
            let mut output = ctx.run_ui(input, |ui| {
                ui.scope_builder(
                    UiBuilder::new().max_rect(Rect::from_min_size(
                        egui::pos2(20.0, 20.0),
                        Vec2::new(460.0, 1500.0),
                    )),
                    |ui| {
                        edit_card(
                            ui,
                            t,
                            s,
                            store,
                            media,
                            CardDraft {
                                nickname,
                                status,
                                description,
                            },
                        );
                    },
                );
            });
            output.textures_delta.clear();
            std::mem::take(&mut output.shapes)
        }

        fn settle(&mut self) -> Vec<egui::epaint::ClippedShape> {
            self.frame(vec![]);
            self.frame(vec![])
        }
    }

    fn text_rect(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<Rect> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                Some(text.visual_bounding_rect())
            }
            _ => None,
        })
    }

    fn click(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![
            vec![egui::Event::PointerMoved(at)],
            vec![button(true)],
            vec![button(false)],
        ]
    }

    #[test]
    fn empty_card_shows_placeholders_where_the_text_goes() {
        let mut harness = CardHarness::new();
        let shapes = harness.settle();
        for hint in [
            "Leave a note…",
            "What should we call you",
            "Tell us about yourself…",
        ] {
            assert!(text_rect(&shapes, hint).is_some(), "falta a dica {hint:?}");
        }
        // Nada fora da largura do cartão.
        let card = Rect::from_min_size(egui::pos2(20.0, 20.0), Vec2::new(EDIT_CARD_WIDTH, 2000.0));
        for clipped in &shapes {
            let bounds = clipped.shape.visual_bounding_rect();
            if bounds.is_finite() {
                assert!(
                    bounds.min.x >= card.min.x - 6.0 && bounds.max.x <= card.max.x + 6.0,
                    "pintou fora do cartão: {bounds:?}"
                );
            }
        }
    }

    #[test]
    fn recado_bubble_sits_beside_the_avatar_like_the_real_card() {
        let mut harness = CardHarness::new();
        let shapes = harness.settle();
        let hint = text_rect(&shapes, "Leave a note…").expect("dica do recado");
        // À direita da foto (80 pt + recuo) e logo abaixo do banner (100 pt).
        assert!(hint.min.x > 20.0 + 16.0 + 80.0, "{hint:?}");
        assert!(hint.min.y > 20.0 + 100.0 && hint.min.y < 20.0 + 100.0 + 40.0, "{hint:?}");
    }

    #[test]
    fn clicking_a_placeholder_and_typing_fills_that_field() {
        let mut harness = CardHarness::new();
        let shapes = harness.settle();
        let about = text_rect(&shapes, "Tell us about yourself…").expect("dica da bio");
        for events in click(about.center()) {
            harness.frame(events);
        }
        harness.frame(vec![egui::Event::Text("oi, tudo bem".into())]);
        assert_eq!(harness.description, "oi, tudo bem");
        assert!(harness.status.is_empty() && harness.nickname.is_empty());

        let status = text_rect(&harness.settle(), "Leave a note…").expect("dica do recado");
        for events in click(status.center()) {
            harness.frame(events);
        }
        harness.frame(vec![egui::Event::Text("no ar".into())]);
        assert_eq!(harness.status, "no ar");
        // O que foi digitado aparece no lugar da dica.
        let shapes = harness.settle();
        assert!(text_rect(&shapes, "no ar").is_some());
        assert!(text_rect(&shapes, "Leave a note…").is_none());
    }

    #[test]
    fn multiline_text_switches_between_size_a_and_b() {
        let ctx = egui::Context::default();
        let a = egui::FontId::proportional(14.0);
        let mut sizes = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for text in ["", "oi", "uma frase bem mais comprida que não cabe em cento e vinte pontos", "a\nb", "oi"] {
                sizes.push(super::super::theme::adaptive_font(ui, &a, text, 120.0).size);
            }
        });
        output.textures_delta.clear();
        let b = (14.0 * super::super::theme::WRAP_SCALE).round();
        assert_eq!(sizes, vec![14.0, 14.0, b, b, 14.0]);
    }

    #[test]
    fn bio_uses_size_a_on_one_line_and_size_b_once_it_wraps() {
        fn bio_font_size(shapes: &[egui::epaint::ClippedShape], needle: &str) -> f32 {
            shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                        text.galley.rows.first().map(|row| row.height())
                    }
                    _ => None,
                })
                .expect("texto da bio")
        }
        let mut harness = CardHarness::new();
        let shapes = harness.settle();
        let about = text_rect(&shapes, "Tell us about yourself…").expect("dica da bio");
        for events in click(about.center()) {
            harness.frame(events);
        }
        harness.frame(vec![egui::Event::Text("oi".into())]);
        let one_line = bio_font_size(&harness.settle(), "oi");
        harness.frame(vec![egui::Event::Text(
            " e agora um texto bem mais comprido, que com certeza não cabe numa linha só do cartão".into(),
        )]);
        let wrapped = bio_font_size(&harness.settle(), "oi e agora");
        assert!(wrapped < one_line, "{wrapped} !< {one_line}");
    }
}

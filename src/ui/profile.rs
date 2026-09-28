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

/// Prévia do cabeçalho do seu cartão (banner e foto), no tamanho real do
/// cartão de 300 pt — ou menor, se a tela não tiver isso. Clicar no banner
/// ou na foto troca cada um: é o próprio objeto que se edita.
pub fn header_preview(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    store: &Store,
    media: &mut crate::media::MediaStore,
) -> PreviewHit {
    let ctx = ui.ctx().clone();
    let width = CARD_WIDTH.min(ui.available_width());
    let banner_h = (width / BANNER_RATIO).round();
    let avatar_size = 80.0 * width / CARD_WIDTH;
    let overlap = (avatar_size * AVATAR_OVERLAP).round();
    let inset = space::XL * width / CARD_WIDTH;
    let (slot, _) = ui.allocate_exact_size(
        Vec2::new(width, banner_h + avatar_size - overlap),
        Sense::hover(),
    );
    let banner = Rect::from_min_size(slot.min, Vec2::new(width, banner_h));
    let avatar = Rect::from_min_size(
        egui::pos2(slot.min.x + inset, banner.max.y - overlap),
        Vec2::splat(avatar_size),
    );
    let me = store.member(&store.me);
    let tint = me.and_then(|member| member.role_color).map(rgb).unwrap_or(t.accent);
    let corners = CornerRadius::same(radius::CARD);

    // A foto se registra depois do banner: onde os dois se sobrepõem, ganha
    // ela.
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
            corners,
            Color32::WHITE,
        ),
        None => {
            painter.rect_filled(banner, corners, blend(t.elevated_bg, tint, 0.38));
        }
    }
    if banner_hovered {
        painter.rect_filled(banner, corners, Color32::from_black_alpha(90));
        painter.text(
            egui::pos2(banner.max.x - space::MD, banner.min.y + space::MD),
            Align2::RIGHT_TOP,
            s.change_banner,
            text::callout(),
            Color32::WHITE,
        );
    }

    painter.circle_filled(avatar.center(), avatar_size / 2.0 + 4.0, t.elevated_bg);
    let face = media
        .avatar(&store.me, store.avatars.get(&store.me).map(String::as_str))
        .and_then(|texture| texture.frame(&ctx))
        .map(|handle| handle.id());
    let painter = ui.painter();
    match face {
        Some(texture) => round_photo(painter, avatar, texture, Color32::WHITE),
        None => {
            painter.circle_filled(avatar.center(), avatar_size / 2.0, blend(t.elevated_bg, tint, 0.30));
            painter.text(
                avatar.center(),
                Align2::CENTER_CENTER,
                me.map(|member| member.initials()).unwrap_or_default(),
                egui::FontId::new((avatar_size * 0.32).round(), egui::FontFamily::Name("semibold".into())),
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
        PreviewHit::Avatar
    } else if banner_response.clicked() {
        PreviewHit::Banner
    } else {
        PreviewHit::None
    }
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
                .rect_filled(banner, banner_corners, blend(t.elevated_bg, tint, 0.38));
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
    banner_buttons(ui, state, s, member, banner, is_me, narrow, outcome);

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
        t.elevated_bg,
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
                blend(t.elevated_bg, tint, 0.30),
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
    painter.circle_filled(dot, dot_r + ring, t.elevated_bg);
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
    egui::Popup::menu(&more).show(|ui| {
        if ui.button(s.profile_copy_id).clicked() {
            ui.ctx().copy_text(member.id.clone());
            state.error = Some((s.profile_id_copied.to_owned(), ui.input(|input| input.time)));
            ui.close();
        }
        ui.separator();
        if ui.button(s.ban_user).clicked() {
            state.actions.push(ChatAction::BanUser {
                user_id: member.id.clone(),
                banned: true,
            });
            ui.close();
        }
        if ui.button(s.unban_user).clicked() {
            state.actions.push(ChatAction::BanUser {
                user_id: member.id.clone(),
                banned: false,
            });
            ui.close();
        }
        if ui.button(s.reset_user).clicked() {
            state.actions.push(ChatAction::ResetUser(member.id.clone()));
            ui.close();
        }
    });
    // Conversa privada: o atalho existe, mas o backend ainda não tem DM.
    slot(ui, icon::CHAT_CIRCLE, s.profile_dm_soon, false, "conversa");
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
        // Mensagem direta ainda não existe no servidor: o campo mostra onde
        // ela vai morar, e diz que ainda não chegou.
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), row_h.max(36.0)),
            Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CARD), t.fill_soft);
        ui.painter().text(
            egui::pos2(rect.min.x + space::MD + 7.0, rect.center().y),
            Align2::CENTER_CENTER,
            icon::CHAT_CIRCLE,
            text::icon(14.0),
            t.label_tertiary,
        );
        let soon = ui.painter().layout_no_wrap(
            s.profile_soon.to_uppercase(),
            text::caption(),
            t.label_tertiary,
        );
        let soon_x = rect.max.x - space::MD - soon.size().x;
        ui.painter().galley(
            egui::pos2(soon_x, rect.center().y - soon.size().y / 2.0),
            soon,
            t.label_tertiary,
        );
        let label_left = rect.min.x + space::MD + 20.0;
        ui.painter()
            .with_clip_rect(Rect::from_x_y_ranges(
                label_left..=soon_x - space::SM,
                rect.y_range(),
            ))
            .text(
                egui::pos2(label_left, rect.center().y),
                Align2::LEFT_CENTER,
                format!("{} @{}", s.profile_message_to, member.username),
                text::body(),
                t.label_tertiary,
            );
        response.on_hover_text(s.profile_dm_soon);
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
        ui.ctx().copy_text(member.id.clone());
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
                t.elevated_bg,
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
    let modified = std::fs::metadata(path).and_then(|meta| meta.modified()).ok();
    let id = egui::Id::new(("activity-art", path, modified));
    if let Some(cached) = ctx.data(|data| data.get_temp::<Option<egui::TextureHandle>>(id)) {
        return cached;
    }
    let texture = image::open(path)
        .map_err(|error| log::warn!("arte da atividade não abriu ({path}): {error}"))
        .ok()
        .map(|decoded| {
            let rgba = decoded.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            ctx.load_texture(
                format!("activity-art:{path}"),
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
}

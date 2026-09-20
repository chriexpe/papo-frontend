//! Janela principal: três colunas (canais · conversa · membros).
//!
//! As barras funcionais (laterais, topo e caixa de mensagem) formam a camada
//! translúcida; a lista de mensagens é a camada de conteúdo, sempre opaca.

use chrono::{Datelike, Local};
use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Rect, RichText, Sense, Stroke,
    TextEdit, UiBuilder, Vec2,
};
use egui_phosphor::regular as icon;

use crate::api::client::Upload;
use crate::i18n::Strings;
use crate::media::MediaStore;
use crate::platform::menu::MenuCommand;
use crate::state::{ChannelKind, Emoji, Message, Presence, Store};

use super::attachments::{self, MediaAction};
use super::emoji;
use super::glass::SharedGlass;
use super::theme::{radius, space, text, Tokens, HIT_TARGET};
use super::viewer::{self, Viewer, ViewerAction};
use super::widgets::{avatar, icon_button, scroll_edge_fade, section_caption, sidebar_frame};

pub const SIDEBAR_WIDTH: f32 = 232.0;
pub const MEMBERS_WIDTH: f32 = 196.0;
const SIDEBAR_HEADER_HEIGHT: f32 = 56.0;
const ACCOUNT_PILL_HEIGHT: f32 = 40.0;
/// Altura das pastilhas flutuantes e respiro entre elas e a borda.
const PILL_HEIGHT: f32 = 36.0;
const PILL_MARGIN: f32 = 12.0;
/// Raio das pastilhas flutuantes — o mesmo canto do realce interno.
const PILL_RADIUS: f32 = 12.0;
const ACTIONS_PILL_WIDTH: f32 = HIT_TARGET * 3.0 + space::XXS * 2.0 + space::XS * 2.0;
const GROUP_GAP_MINUTES: i64 = 5;
/// Folga do realce da linha, igual em cima e embaixo.
const ROW_PADDING: f32 = 4.0;
/// Quanto tempo a descrição do canal fica visível antes de recolher.
const TOPIC_HOLD: f64 = 4.0;
const TOPIC_SLIDE: f64 = 0.45;
/// Altura da faixa de anexos à espera de envio, dentro da caixa de texto.
const COMPOSER_ATTACH_H: f32 = 62.0;
const COMPOSER_REPLY_H: f32 = 26.0;
/// Altura da linha onde se digita, sem as faixas de cima.
const COMPOSER_LINE_H: f32 = 44.0;
const TOAST_SECONDS: f64 = 6.0;

/// O que a conversa pede para a camada de cima fazer.
#[derive(Debug, Clone)]
pub enum ChatAction {
    Send {
        content: String,
        reply_to: Option<String>,
        attachments: Vec<Upload>,
    },
    Edit {
        message_id: String,
        content: String,
    },
    Delete(String),
    React {
        message_id: String,
        emoji: Emoji,
        add: bool,
    },
    Pin {
        message_id: String,
        pin: bool,
    },
    Download {
        id: String,
        name: String,
    },
    /// Abre o seletor de arquivos do sistema.
    PickFiles,
    /// O mesmo seletor, filtrado em imagens animadas.
    PickGif,
    /// Abre o que já está no cache com o aplicativo padrão.
    OpenExternally(std::path::PathBuf),
    /// Abre o diálogo de criar canal.
    NewChannel,
    /// Abre o diálogo de editar um canal existente.
    EditChannel(String),
    /// Apaga o canal, depois da confirmação.
    DeleteChannel(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    /// Reagir a uma mensagem.
    Emoji,
    Menu,
    /// Inserir emoji no que está sendo escrito.
    ComposerEmoji,
    /// Só os emojis do servidor, que é o que faz as vezes de figurinha.
    ComposerSticker,
}

/// Popup ancorado a uma mensagem (seletor de emoji ou menu de contexto).
#[derive(Clone, Debug)]
pub struct Popup {
    pub kind: PopupKind,
    pub message_id: String,
    pub anchor: Rect,
    /// O menu de contexto abre no ponteiro; o seletor, embaixo do botão.
    pub at_pointer: bool,
    /// Instante da abertura: o clique que abriu não pode fechar.
    pub opened: f64,
}

/// Estado que pertence à interface, não ao servidor.
/// O pedaço da interface que pertence a um servidor.
///
/// Só um servidor aparece na janela por vez, mas todos ficam conectados. Ao
/// trocar, estes campos trocam de lugar com os da [`UiState`] em vez de serem
/// zerados: o rascunho por escrever, o anexo já escolhido e a mídia já
/// baixada continuam esperando quando você volta.
pub struct Stash {
    pub media: MediaStore,
    pub composer: String,
    pub attachments: Vec<Upload>,
    pub replying: Option<String>,
    pub editing: Option<(String, String)>,
    pub viewer: Option<Viewer>,
    pub popup: Option<Popup>,
    pub last_channel: String,
    pub topic_since: Option<f64>,
}

impl Stash {
    pub fn new(media: MediaStore) -> Self {
        Self {
            media,
            composer: String::new(),
            attachments: Vec::new(),
            replying: None,
            editing: None,
            viewer: None,
            popup: None,
            last_channel: String::new(),
            topic_since: None,
        }
    }

    /// Troca este guardado com o que está na tela.
    pub fn swap(&mut self, ui: &mut UiState) {
        std::mem::swap(&mut self.media, &mut ui.media);
        std::mem::swap(&mut self.composer, &mut ui.composer);
        std::mem::swap(&mut self.attachments, &mut ui.attachments);
        std::mem::swap(&mut self.replying, &mut ui.replying);
        std::mem::swap(&mut self.editing, &mut ui.editing);
        std::mem::swap(&mut self.viewer, &mut ui.viewer);
        std::mem::swap(&mut self.popup, &mut ui.popup);
        std::mem::swap(&mut self.last_channel, &mut ui.last_channel);
        std::mem::swap(&mut self.topic_since, &mut ui.topic_since);
    }
}

pub struct UiState {
    pub composer: String,
    pub show_members: bool,
    pub translucent: bool,
    pub pending: Vec<MenuCommand>,
    /// Renderizador do vidro fosco; ausente quando o backend não é o glow.
    pub glass: Option<SharedGlass>,
    /// O texto mudou neste quadro (dispara o evento de digitação).
    pub typed: bool,
    /// Mídia baixada, decodificada e tocando.
    pub media: MediaStore,
    /// Arquivos escolhidos, ainda não enviados.
    pub attachments: Vec<Upload>,
    /// Mensagem sendo respondida.
    pub replying: Option<String>,
    /// Mensagem sendo editada, com o texto em edição.
    pub editing: Option<(String, String)>,
    pub actions: Vec<ChatAction>,
    pub viewer: Option<Viewer>,
    pub popup: Option<Popup>,
    pub emoji_query: String,
    pub emoji_group: usize,
    /// Canal desenhado no quadro anterior, para saber quando ele trocou.
    pub last_channel: String,
    /// Início da aparição da descrição do canal.
    pub topic_since: Option<f64>,
    /// A descrição aparece ao abrir o canal (ajuste do usuário).
    pub reveal_topic: bool,
    /// Botão de gravar recado na caixa de texto (ajuste do usuário).
    pub show_record: bool,
    /// Gravação em curso.
    pub recorder: Option<crate::media::player::Recorder>,
    /// Recado curto de erro da própria interface, com o instante em que
    /// apareceu.
    pub error: Option<(String, f64)>,
    /// A lista mudou de altura no quadro anterior. O egui só reencosta a
    /// rolagem no fim do quadro, então o seguinte sairia com a posição velha:
    /// ele é refeito antes de chegar à tela.
    pub relayout: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            composer: String::new(),
            show_members: true,
            translucent: true,
            pending: Vec::new(),
            glass: None,
            typed: false,
            media: MediaStore::new(None),
            attachments: Vec::new(),
            replying: None,
            editing: None,
            actions: Vec::new(),
            viewer: None,
            popup: None,
            emoji_query: String::new(),
            emoji_group: 0,
            last_channel: String::new(),
            topic_since: None,
            reveal_topic: true,
            show_record: true,
            recorder: None,
            error: None,
            relayout: false,
        }
    }
}

impl UiState {
    fn close_popup(&mut self) {
        self.popup = None;
        self.emoji_query.clear();
    }
}

pub fn draw(ui: &mut egui::Ui, store: &mut Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    // Mídia que acabou de chegar muda a altura das mensagens.
    if state.media.pump(ui.ctx()) {
        state.relayout = true;
    }

    // O erro que veio do servidor vira o mesmo aviso passageiro dos erros da
    // interface. Sem isto ele ficava só no `Store`, onde a tela de conversa
    // nunca o lia: um 403 ao criar canal, ou um envio recusado, sumiam sem
    // deixar rastro — e o que se via era um botão que não faz nada.
    if let Some(message) = store.error.take() {
        state.error = Some((message, ui.input(|input| input.time)));
    }

    // Trocou de canal: a descrição reaparece e a mídia que estava tocando
    // para, porque ela já saiu da tela.
    if state.last_channel != store.selected_channel {
        state.last_channel = store.selected_channel.clone();
        state.topic_since = Some(ui.input(|input| input.time));
        state.media.pause_all();
        state.media.saved = None;
        state.editing = None;
        state.replying = None;
        state.close_popup();
    }

    // A altura da lista mudou no quadro anterior (uma reação a mais, por
    // exemplo): a rolagem ainda está no lugar antigo e a conversa daria um
    // pulo. Refazer o quadro antes de mostrá-lo resolve na origem.
    if std::mem::take(&mut state.relayout) {
        ui.ctx().request_discard("a lista mudou de altura");
    }

    channels_sidebar(ui, store, state, t, s);
    if state.show_members {
        members_sidebar(ui, store, t, s);
    }
    conversation(ui, store, state, t, s);
    overlays(ui, store, state, t, s);
}

/// Desenha o fundo embaçado de uma barra: o que já foi pintado por baixo
/// dela entra borrado, e a tinta translúcida vem por cima.
fn glass_backdrop(ui: &egui::Ui, state: &UiState, rect: Rect, corner: f32) {
    if !state.translucent {
        return;
    }
    let Some(glass) = state.glass.clone() else {
        return;
    };
    let callback = eframe::egui_glow::CallbackFn::new(move |info, painter| {
        let viewport = info.viewport_in_pixels();
        if let Ok(mut glass) = glass.lock() {
            glass.render(
                painter.gl(),
                (
                    viewport.left_px,
                    viewport.top_px,
                    viewport.width_px,
                    viewport.height_px,
                ),
                info.screen_size_px[1] as i32,
                corner * info.pixels_per_point,
            );
        }
    });
    ui.painter().add(egui::PaintCallback {
        rect,
        callback: std::sync::Arc::new(callback),
    });
}
// ---------------------------------------------------------------------------
// Coluna esquerda — canais
// ---------------------------------------------------------------------------

fn channels_sidebar(
    root: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
) {
    egui::Panel::left("channels")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(sidebar_frame(t))
        .show(root, |ui| {
            let full = ui.max_rect();

            // Cabeçalho do servidor.
            ui.scope_builder(
                UiBuilder::new().max_rect(Rect::from_min_size(
                    full.min,
                    Vec2::new(full.width(), SIDEBAR_HEADER_HEIGHT),
                )),
                |ui| {
                    ui.add_space(space::LG);
                    ui.horizontal(|ui| {
                        ui.add_space(space::XL);
                        ui.vertical(|ui| {
                            let (name, description) = match &store.server {
                                Some(server) => {
                                    (server.name.clone(), server.description.clone())
                                }
                                None => ("Papo".to_owned(), None),
                            };
                            ui.label(RichText::new(name).font(text::title3()).color(t.label));
                            if let Some(description) = description {
                                ui.label(
                                    RichText::new(description)
                                        .font(text::footnote())
                                        .color(t.label_tertiary),
                                );
                            }
                        });
                        // Criar canal fica no cabeçalho da coluna que lista
                        // os canais, que é onde se procura por ele.
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(space::MD);
                            if icon_button(ui, t, icon::PLUS, s.new_channel_title).clicked() {
                                state.actions.push(ChatAction::NewChannel);
                            }
                        });
                    });
                },
            );
            let header_bottom = full.min.y + SIDEBAR_HEADER_HEIGHT;
            ui.painter().line_segment(
                [
                    egui::pos2(full.min.x, header_bottom),
                    egui::pos2(full.max.x, header_bottom),
                ],
                Stroke::new(1.0, t.separator),
            );

            // A lista ocupa toda a altura restante: os canais passam por
            // baixo da pastilha flutuante da conta.
            let list = Rect::from_min_max(egui::pos2(full.min.x, header_bottom), full.max);

            ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(space::XS);
                        let indent = space::LG;
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            ui.vertical(|ui| {
                                section_caption(ui, t, s.text_channels);
                                for channel in store
                                    .channels
                                    .clone()
                                    .iter()
                                    .filter(|c| c.kind == ChannelKind::Text)
                                {
                                    let row = channel_row(
                                        ui,
                                        t,
                                        icon::HASH,
                                        &channel.name,
                                        store.selected_channel == channel.id,
                                        channel.unread,
                                        channel.mentions,
                                        SIDEBAR_WIDTH - indent * 2.0,
                                    );
                                    if row.clicked() {
                                        store.selected_channel = channel.id.clone();
                                    }
                                    channel_menu(&row, channel, state, s);
                                }
                                // Sem canal nenhum a lista fica muda; este é o
                                // único caminho para o primeiro canal.
                                if store.channels.is_empty()
                                    && add_channel_row(ui, t, s, SIDEBAR_WIDTH - indent * 2.0)
                                {
                                    state.actions.push(ChatAction::NewChannel);
                                }

                                section_caption(ui, t, s.voice_channels);
                                for channel in store
                                    .channels
                                    .clone()
                                    .iter()
                                    .filter(|c| c.kind == ChannelKind::Voice)
                                {
                                    let row = channel_row(
                                        ui,
                                        t,
                                        icon::SPEAKER_HIGH,
                                        &channel.name,
                                        false,
                                        false,
                                        0,
                                        SIDEBAR_WIDTH - indent * 2.0,
                                    );
                                    channel_menu(&row, channel, state, s);
                                }
                                // Espaço para a pastilha da conta não cobrir o
                                // último canal quando a lista chega ao fim.
                                ui.add_space(ACCOUNT_PILL_HEIGHT + space::XXL);
                            });
                        });
                    });
            });

            account_pill(ui, store, state, t, s, full);
        });
}

/// Pastilha flutuante da conta, no rodapé da barra lateral.
fn account_pill(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    sidebar: Rect,
) {
    let me = store.member(&store.me).cloned().unwrap_or(crate::state::Member {
        id: store.me.clone(),
        name: store.my_name.clone(),
        presence: crate::state::Presence::Online,
        role_color: None,
        roles: Vec::new(),
    });
    if me.name.is_empty() {
        return;
    }

    let label = ui
        .painter()
        .layout_no_wrap(me.name.clone(), text::headline(), t.label);
    let avatar_size = 26.0;
    let width = space::SM + avatar_size + space::MD + label.size().x + space::LG;
    let rect = Rect::from_min_size(
        egui::pos2(
            sidebar.min.x + space::LG,
            sidebar.max.y - ACCOUNT_PILL_HEIGHT - space::LG,
        ),
        Vec2::new(width.min(sidebar.width() - space::LG * 2.0), ACCOUNT_PILL_HEIGHT),
    );

    pill_surface(ui, state, t, rect);
    let response = ui.interact(rect, Id::new("account-pill"), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(PILL_RADIUS as u8),
            t.fill_soft,
        );
    }

    let painter = ui.painter();
    let avatar_center = egui::pos2(rect.min.x + space::SM + avatar_size / 2.0, rect.center().y);
    let tint = me.role_color.unwrap_or(t.accent);
    painter.circle_filled(avatar_center, avatar_size / 2.0, tint.gamma_multiply(0.30));
    painter.text(
        avatar_center,
        egui::Align2::CENTER_CENTER,
        me.initials(),
        egui::FontId::new(10.0, egui::FontFamily::Name("semibold".into())),
        tint,
    );
    let dot = avatar_center + Vec2::splat(avatar_size / 2.0 * 0.72);
    painter.circle_filled(dot, 5.0, t.glass_opaque);
    painter.circle_filled(dot, 3.5, presence_color(t, me.presence));

    painter.galley(
        egui::pos2(
            avatar_center.x + avatar_size / 2.0 + space::MD,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        t.label,
    );

    let _ = s;
}

#[allow(clippy::too_many_arguments)]
fn channel_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    glyph: &str,
    name: &str,
    selected: bool,
    unread: bool,
    mentions: u32,
    width: f32,
) -> egui::Response {
    let height = 30.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());

    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
    }

    let label_color = if selected || unread {
        t.label
    } else {
        t.label_secondary
    };
    let font = if unread && !selected {
        text::headline()
    } else {
        text::body()
    };

    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.min.x + space::MD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        glyph,
        text::icon(14.0),
        if selected { t.accent } else { t.label_tertiary },
    );
    painter.text(
        egui::pos2(rect.min.x + space::MD + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        font,
        label_color,
    );

    if mentions > 0 {
        let pill_width = 20.0_f32.max(12.0 + mentions.to_string().len() as f32 * 7.0);
        let pill = Rect::from_center_size(
            egui::pos2(rect.max.x - space::MD - pill_width / 2.0, rect.center().y),
            Vec2::new(pill_width, 16.0),
        );
        painter.rect_filled(pill, CornerRadius::same(8), t.accent);
        painter.text(
            pill.center(),
            egui::Align2::CENTER_CENTER,
            mentions.to_string(),
            text::caption(),
            t.accent_label,
        );
    } else if unread {
        painter.circle_filled(
            egui::pos2(rect.min.x - space::SM * 0.5, rect.center().y),
            3.0,
            t.label,
        );
    }

    response
}

/// Menu do botão direito de um canal: renomear e excluir.
fn channel_menu(
    response: &egui::Response,
    channel: &crate::state::Channel,
    state: &mut UiState,
    s: &Strings,
) {
    response.context_menu(|ui| {
        if ui.button(s.rename_channel).clicked() {
            state.actions.push(ChatAction::EditChannel(channel.id.clone()));
            ui.close();
        }
        if ui
            .button(s.delete_channel)
            .on_hover_text(s.delete_channel_confirm)
            .clicked()
        {
            state
                .actions
                .push(ChatAction::DeleteChannel(channel.id.clone()));
            ui.close();
        }
    });
}

/// Linha «criar canal», no lugar onde estariam os canais.
fn add_channel_row(ui: &mut egui::Ui, t: &Tokens, s: &Strings, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.min.x + space::MD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon::PLUS,
        text::icon(14.0),
        t.label_tertiary,
    );
    painter.text(
        egui::pos2(rect.min.x + space::MD + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        s.create_channel,
        text::body(),
        t.label_secondary,
    );
    response.clicked()
}

// ---------------------------------------------------------------------------
// Coluna direita — membros
// ---------------------------------------------------------------------------

fn members_sidebar(root: &mut egui::Ui, store: &Store, t: &Tokens, s: &Strings) {
    egui::Panel::right("members")
        .exact_size(MEMBERS_WIDTH)
        .resizable(false)
        .frame(sidebar_frame(t))
        .show(root, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(space::LG);
                        ui.vertical(|ui| {
                            for (presence, label) in [
                                (Presence::Online, s.online),
                                (Presence::Away, s.away),
                                (Presence::Busy, s.busy),
                                (Presence::Offline, s.offline),
                            ] {
                                let people: Vec<_> = store.members_by_presence(presence).collect();
                                if people.is_empty() {
                                    continue;
                                }
                                section_caption(
                                    ui,
                                    t,
                                    &format!("{label} — {}", people.len()),
                                );
                                for member in people {
                                    member_row(
                                        ui,
                                        t,
                                        &member.initials(),
                                        &member.name,
                                        presence_color(t, member.presence),
                                        member.role_color,
                                        presence == Presence::Offline,
                                        MEMBERS_WIDTH - space::LG * 2.0,
                                    );
                                }
                            }
                            ui.add_space(space::LG);
                        });
                    });
                });
        });
}

#[allow(clippy::too_many_arguments)]
fn member_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    initials: &str,
    name: &str,
    dot: Color32,
    role_color: Option<Color32>,
    dimmed: bool,
    width: f32,
) {
    let height = 32.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
    }

    let alpha = if dimmed { 0.45 } else { 1.0 };
    let avatar_rect = Rect::from_center_size(
        egui::pos2(rect.min.x + space::SM + 11.0, rect.center().y),
        Vec2::splat(22.0),
    );
    let tint = role_color.unwrap_or(t.accent).gamma_multiply(alpha);
    let painter = ui.painter();
    painter.circle_filled(avatar_rect.center(), 11.0, tint.gamma_multiply(0.30));
    painter.text(
        avatar_rect.center(),
        egui::Align2::CENTER_CENTER,
        initials,
        egui::FontId::new(9.0, egui::FontFamily::Name("semibold".into())),
        tint,
    );
    painter.circle_filled(avatar_rect.right_bottom() - Vec2::splat(1.0), 4.5, t.glass_opaque);
    painter.circle_filled(avatar_rect.right_bottom() - Vec2::splat(1.0), 3.0, dot.gamma_multiply(alpha));
    painter.text(
        egui::pos2(avatar_rect.max.x + space::MD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        text::body(),
        if dimmed { t.label_tertiary } else { t.label_secondary },
    );
}


/// Fundo de uma pastilha: vidro fosco, tinta translúcida e fio de contorno.
fn pill_surface(ui: &egui::Ui, state: &UiState, t: &Tokens, rect: Rect) {
    let radius = PILL_RADIUS;
    glass_backdrop(ui, state, rect, radius);
    ui.painter().rect(
        rect,
        CornerRadius::same(radius as u8),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    // Brilho de um pixel no topo, como nas barras da Apple.
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + radius * 0.6, rect.min.y + 0.5),
            egui::pos2(rect.max.x - radius * 0.6, rect.min.y + 0.5),
        ],
        Stroke::new(1.0, t.glass_highlight),
    );
}

/// Aviso de conexão, centralizado no alto: só aparece quando o socket não
/// está de pé.
fn connection_pill(
    ui: &egui::Ui,
    store: &Store,
    state: &UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let (label, color) = match store.connection {
        crate::api::ws::Connection::Online => return,
        crate::api::ws::Connection::Connecting => (s.reconnecting, t.away),
        crate::api::ws::Connection::Offline => (s.offline_banner, t.danger),
    };

    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::footnote(), t.label_secondary);
    let height = 26.0;
    let width = galley.size().x + space::XXL + space::LG;
    let rect = Rect::from_center_size(
        egui::pos2(area.center().x, area.min.y + PILL_MARGIN + height / 2.0),
        Vec2::new(width, height),
    );

    glass_backdrop(ui, state, rect, height / 2.0);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same((height / 2.0) as u8),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(
        egui::pos2(rect.min.x + space::LG, rect.center().y),
        3.5,
        color,
    );
    painter.galley(
        egui::pos2(
            rect.min.x + space::LG + space::LG,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        t.label_secondary,
    );
}

/// Pastilha pequena de "fulano está digitando", acima da caixa de texto.
fn typing_pill(
    ui: &egui::Ui,
    store: &Store,
    state: &UiState,
    t: &Tokens,
    s: &Strings,
    composer: Rect,
) {
    let names = store.typing_names();
    if names.is_empty() {
        return;
    }
    let verb = if names.len() == 1 {
        s.typing_one
    } else {
        s.typing_many
    };
    let label = ui.painter().layout_no_wrap(
        format!("{} {verb}", names.join(", ")),
        text::footnote(),
        t.label_secondary,
    );

    let height = 22.0;
    let rect = Rect::from_min_size(
        egui::pos2(composer.min.x, composer.min.y - height - space::XS),
        Vec2::new(label.size().x + space::LG * 2.0, height),
    );
    glass_backdrop(ui, state, rect, height / 2.0);
    ui.painter().rect(
        rect,
        CornerRadius::same((height / 2.0) as u8),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(rect.min.x + space::LG, rect.center().y - label.size().y / 2.0),
        label,
        t.label_secondary,
    );
}
// ---------------------------------------------------------------------------
// Centro — conversa
// ---------------------------------------------------------------------------

fn conversation(
    root: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
) {
    let frame = Frame::new().fill(t.content_bg);
    egui::CentralPanel::default().frame(frame).show(root, |ui| {
        let full = ui.max_rect();
        let composer_height = composer_height(state);
        let top_inset = PILL_MARGIN * 2.0 + PILL_HEIGHT;
        let bottom_inset = PILL_MARGIN * 2.0 + composer_height;

        // Camada de conteúdo: ocupa a janela inteira e corre por baixo das
        // pastilhas.
        ui.scope_builder(UiBuilder::new().max_rect(full), |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add_space(top_inset);
                    message_list(ui, store, state, t, s, full);
                    ui.add_space(bottom_inset);
                });
        });

        // O conteúdo se dissolve onde encontra a camada flutuante, em vez de
        // ser cortado por ela.
        scroll_edge_fade(
            ui,
            Rect::from_min_size(full.min, Vec2::new(full.width(), top_inset)),
            t.content_bg,
            true,
        );
        scroll_edge_fade(
            ui,
            Rect::from_min_size(
                egui::pos2(full.min.x, full.max.y - bottom_inset),
                Vec2::new(full.width(), bottom_inset),
            ),
            t.content_bg,
            false,
        );

        // Camada funcional: tudo flutua.
        connection_pill(ui, store, state, t, s, full);
        channel_pill(ui, store, state, t, full);
        actions_pill(ui, state, t, s, full);
        composer(ui, store, state, t, s, full, composer_height);
    });
}

/// Pastilha de identidade do canal, no alto à esquerda.
///
/// A descrição entra junto com o canal, fica alguns segundos e escorrega na
/// direção do nome até sumir — a pastilha encolhe junto.
fn channel_pill(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, area: Rect) {
    let Some(channel) = store.channel(&store.selected_channel).cloned() else {
        return;
    };

    let painter = ui.painter();
    let glyph = painter.layout_no_wrap(icon::HASH.to_owned(), text::icon(15.0), t.label_tertiary);
    let name = painter.layout_no_wrap(channel.name.clone(), text::title3(), t.label);
    let topic = channel.topic.as_ref().filter(|topic| !topic.is_empty()).map(|topic| {
        painter.layout_no_wrap(topic.clone(), text::callout(), t.label_tertiary)
    });

    // Quanto da descrição ainda está na tela: 1 inteira, 0 recolhida.
    let reveal = match (&topic, state.reveal_topic, state.topic_since) {
        (None, _, _) => 0.0,
        (Some(_), false, _) => 0.0,
        (Some(_), true, None) => 0.0,
        (Some(_), true, Some(since)) => {
            let elapsed = ui.input(|input| input.time) - since;
            if elapsed < TOPIC_HOLD {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(
                    (TOPIC_HOLD - elapsed).max(0.01),
                ));
                1.0
            } else if elapsed < TOPIC_HOLD + TOPIC_SLIDE {
                ui.ctx().request_repaint();
                let progress = ((elapsed - TOPIC_HOLD) / TOPIC_SLIDE) as f32;
                // Desacelerando no fim, como todo movimento da casa.
                1.0 - (1.0 - (1.0 - progress).powi(3))
            } else {
                state.topic_since = None;
                0.0
            }
        }
    };

    let base_width = space::LG + glyph.size().x + space::SM + name.size().x + space::LG;
    let topic_width = topic
        .as_ref()
        .map(|topic| space::LG + 1.0 + space::LG + topic.size().x)
        .unwrap_or(0.0);
    let limit = area.width() - PILL_MARGIN * 2.0 - ACTIONS_PILL_WIDTH - space::MD;
    let width = (base_width + topic_width * reveal).min(limit);

    let rect = Rect::from_min_size(
        area.min + Vec2::splat(PILL_MARGIN),
        Vec2::new(width, PILL_HEIGHT),
    );
    pill_surface(ui, state, t, rect);

    let mid = rect.center().y;
    let mut x = rect.min.x + space::LG;
    let painter = ui.painter();
    painter.galley(egui::pos2(x, mid - glyph.size().y / 2.0), glyph.clone(), t.label_tertiary);
    x += glyph.size().x + space::SM;
    painter.galley(egui::pos2(x, mid - name.size().y / 2.0), name.clone(), t.label);
    x += name.size().x + space::LG;

    // A descrição escorrega no mesmo passo em que a pastilha encolhe: a
    // borda direita fica colada na pastilha e o que passa do nome é cortado,
    // então ela some exatamente ali, sem aparecer do outro lado.
    if let (Some(topic), true) = (topic, reveal > 0.001) {
        let slide = (1.0 - reveal) * topic_width;
        let start = x - slide;
        let alpha = (reveal * 1.8).clamp(0.0, 1.0);
        let clip = Rect::from_min_max(
            egui::pos2(x, rect.min.y),
            egui::pos2(rect.max.x - space::SM, rect.max.y),
        );
        let painter = painter.with_clip_rect(clip);
        painter.line_segment(
            [egui::pos2(start, mid - 7.0), egui::pos2(start, mid + 7.0)],
            Stroke::new(1.0, t.separator.gamma_multiply(alpha)),
        );
        painter.galley(
            egui::pos2(start + space::LG, mid - topic.size().y / 2.0),
            topic,
            t.label_tertiary.gamma_multiply(alpha),
        );
    }
}

/// Pastilha de ações do canal, no alto à direita.
fn actions_pill(ui: &mut egui::Ui, state: &mut UiState, t: &Tokens, s: &Strings, area: Rect) {
    let rect = Rect::from_min_size(
        egui::pos2(
            area.max.x - PILL_MARGIN - ACTIONS_PILL_WIDTH,
            area.min.y + PILL_MARGIN,
        ),
        Vec2::new(ACTIONS_PILL_WIDTH, PILL_HEIGHT),
    );
    pill_surface(ui, state, t, rect);

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(space::XS, space::XS)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = space::XXS;
            if icon_button(ui, t, icon::MAGNIFYING_GLASS, s.search).clicked() {
                state.pending.push(MenuCommand::Search);
            }
            let _ = icon_button(ui, t, icon::PUSH_PIN, s.pinned);
            if icon_button(ui, t, icon::USERS, s.members).clicked() {
                state.pending.push(MenuCommand::ToggleMembers);
            }
        },
    );
}

// ---------------------------------------------------------------------------
// Lista de mensagens
// ---------------------------------------------------------------------------

fn message_list(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let messages: Vec<Message> = store.messages_in(&store.selected_channel).cloned().collect();
    if messages.is_empty() {
        empty_state(ui, store, state, t, s);
        return;
    }

    let width = area.width();
    let gutter = space::XL;
    let avatar_size = 36.0;
    let text_indent = gutter + avatar_size + space::LG;
    let text_width = width - text_indent - space::XL;
    let rows = egui::Rangef::new(area.min.x + space::MD, area.max.x - space::MD);
    // Com um popup aberto, só a mensagem dona dele fica em destaque: o resto
    // da lista não deve reagir ao ponteiro que está a caminho do menu.
    let interactive = state.viewer.is_none();
    let focused_message = state
        .popup
        .as_ref()
        .map(|popup| popup.message_id.clone());

    let mut last_author: Option<String> = None;
    let mut last_at: Option<chrono::DateTime<Local>> = None;
    let mut last_day: Option<u32> = None;

    for message in &messages {
        let day = message.at.day();
        if last_day != Some(day) {
            day_divider(ui, t, s, message.at, width);
            last_day = Some(day);
            last_author = None;
        }

        let grouped = last_author.as_deref() == Some(message.author_id.as_str())
            && last_at.is_some_and(|prev| {
                (message.at - prev).num_minutes() < GROUP_GAP_MINUTES
            });

        // O realce da linha é pintado depois, quando já sabemos a altura.
        // O respiro entre grupos fica fora da linha: dentro dela, ele jogaria
        // o texto para baixo e o realce ficaria torto.
        if !grouped {
            ui.add_space(space::LG);
        }
        let backdrop = ui.painter().add(egui::Shape::Noop);

        let author = store.member(&message.author_id);
        let inner = ui.scope(|ui| {
            ui.horizontal_top(|ui| {
                ui.add_space(gutter);
                if grouped {
                    ui.allocate_exact_size(Vec2::new(avatar_size, 0.0), Sense::hover());
                    ui.add_space(space::LG);
                } else {
                    let initials = author.map(|a| a.initials()).unwrap_or_else(|| "?".into());
                    avatar(ui, t, &initials, avatar_size, author.and_then(|a| a.role_color));
                    ui.add_space(space::LG);
                }
                ui.vertical(|ui| {
                    ui.set_max_width(text_width);
                    if !grouped {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(author.map(|a| a.name.as_str()).unwrap_or("?"))
                                    .font(text::headline())
                                    .color(author.and_then(|a| a.role_color).unwrap_or(t.label)),
                            );
                            ui.add_space(space::XS);
                            ui.label(
                                RichText::new(message.at.format("%H:%M").to_string())
                                    .font(text::footnote())
                                    .color(t.label_tertiary),
                            );
                            if message.pinned {
                                ui.add_space(space::XS);
                                ui.label(
                                    RichText::new(icon::PUSH_PIN)
                                        .font(text::icon(11.0))
                                        .color(t.label_tertiary),
                                );
                            }
                        });
                        ui.add_space(space::XXS);
                    }
                    if let Some(reply_to) = &message.reply_to {
                        reply_quote(ui, store, t, s, reply_to, text_width);
                    }
                    message_body(ui, store, state, t, s, message, text_width);
                });
            });
        });

        let row = Rect::from_x_y_ranges(rows, inner.response.rect.y_range());
        // Mensagem que cita você fica marcada, com ou sem o ponteiro em cima.
        let mentions_me = store.mentions_me(message);
        let hovered = match &focused_message {
            Some(id) => id == &message.id,
            None => interactive && ui.rect_contains_pointer(row),
        };
        if mentions_me {
            let band = row.expand2(Vec2::new(0.0, ROW_PADDING));
            let wash = t.mention.gamma_multiply(if hovered { 0.16 } else { 0.10 });
            ui.painter().set(
                backdrop,
                egui::epaint::RectShape::filled(
                    band,
                    CornerRadius::same(radius::CARD),
                    wash,
                ),
            );
            // Fio na borda esquerda, como um marcador de página.
            ui.painter().rect_filled(
                Rect::from_min_size(band.min, Vec2::new(2.5, band.height())),
                CornerRadius::same(1),
                t.mention.gamma_multiply(0.9),
            );
        } else if hovered {
            ui.painter().set(
                backdrop,
                egui::epaint::RectShape::filled(
                    row.expand2(Vec2::new(0.0, ROW_PADDING)),
                    CornerRadius::same(radius::CARD),
                    t.fill_soft,
                ),
            );
        }
        if hovered {
            if focused_message.is_none() {
                hover_pill(ui, state, t, s, message, row, store);
            }

            let secondary =
                focused_message.is_none() && ui.input(|input| input.pointer.secondary_clicked());
            if secondary {
                let at = ui.ctx().pointer_latest_pos().unwrap_or(row.center());
                state.popup = Some(Popup {
                    kind: PopupKind::Menu,
                    message_id: message.id.clone(),
                    anchor: Rect::from_min_size(at, Vec2::ZERO),
                    at_pointer: true,
                    opened: ui.input(|input| input.time),
                });
            }
        }

        last_author = Some(message.author_id.clone());
        last_at = Some(message.at);
    }
}

/// Citação da mensagem respondida, acima do corpo.
fn reply_quote(
    ui: &mut egui::Ui,
    store: &Store,
    t: &Tokens,
    s: &Strings,
    reply_to: &str,
    width: f32,
) {
    let (name, preview) = match store.message(reply_to) {
        Some(target) => (
            store
                .member(&target.author_id)
                .map(|member| member.name.clone())
                .unwrap_or_else(|| "?".into()),
            if target.content.is_empty() && !target.attachments.is_empty() {
                format!("{} {}", icon::PAPERCLIP, target.attachments[0].name())
            } else {
                attachments::elide(&target.content, 80)
            },
        ),
        // Sem chave estrangeira no banco: a original pode ter sumido.
        None => (String::new(), s.reply_missing.to_owned()),
    };

    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.min.x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon::ARROW_BEND_UP_LEFT,
        text::icon(11.0),
        t.label_tertiary,
    );
    let mut x = rect.min.x + 16.0;
    if !name.is_empty() {
        let galley = painter.layout_no_wrap(name, text::caption(), t.label_secondary);
        painter.galley(
            egui::pos2(x, rect.center().y - galley.size().y / 2.0),
            galley.clone(),
            t.label_secondary,
        );
        x += galley.size().x + space::SM;
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        preview,
        text::footnote(),
        t.label_tertiary,
    );
    ui.add_space(space::XXS);
}

fn message_body(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    message: &Message,
    width: f32,
) {
    // Em edição, o corpo vira uma caixa de texto no lugar exato do texto.
    if let Some((id, buffer)) = &mut state.editing {
        if id == &message.id {
            let mut buffer_copy = buffer.clone();
            let response = ui.add(
                TextEdit::multiline(&mut buffer_copy)
                    .font(text::message())
                    .desired_width(width)
                    .desired_rows(1)
                    .margin(Margin::symmetric(space::MD as i8, space::SM as i8)),
            );
            *buffer = buffer_copy;
            response.request_focus();

            let (save, cancel) = ui.input(|input| {
                (
                    input.key_pressed(egui::Key::Enter) && !input.modifiers.shift,
                    input.key_pressed(egui::Key::Escape),
                )
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(s.edit_hint)
                        .font(text::footnote())
                        .color(t.label_tertiary),
                );
            });
            if cancel {
                state.editing = None;
            } else if save {
                if let Some((id, content)) = state.editing.take() {
                    let content = content.trim().to_owned();
                    if content.is_empty() {
                        state.actions.push(ChatAction::Delete(id));
                    } else {
                        state.actions.push(ChatAction::Edit {
                            message_id: id,
                            content,
                        });
                    }
                }
            }
            return;
        }
    }

    if !message.content.is_empty() {
        let color = if message.pending {
            t.label_secondary
        } else {
            t.label
        };
        let tokens = emoji::tokenize(&message.content, &store.emojis);
        let plain = tokens
            .iter()
            .all(|token| matches!(token, emoji::Token::Text(_)));

        if plain {
            // Sem emoji, uma passada só de texto — é o caminho rápido.
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &message.content,
                0.0,
                egui::TextFormat {
                    font_id: text::message(),
                    color,
                    ..Default::default()
                },
            );
            if message.edited {
                job.append(
                    &format!("  ({})", s.edited),
                    0.0,
                    egui::TextFormat {
                        font_id: text::footnote(),
                        color: t.label_tertiary,
                        ..Default::default()
                    },
                );
            }
            job.wrap.max_width = width;
            ui.label(job);
        } else {
            rich_body(ui, t, s, store, state, &tokens, color, message.edited, width);
        }
    }

    if !message.attachments.is_empty() {
        if let Some(action) = attachments::draw(
            ui,
            t,
            s,
            &mut state.media,
            &message.id,
            &message.attachments,
            width,
        ) {
            match action {
                MediaAction::Open { message_id, index } => {
                    state.viewer = Some(Viewer::new(message_id, index));
                    state.media.pause_all();
                }
                MediaAction::Download { id, name } => {
                    state.actions.push(ChatAction::Download { id, name })
                }
                MediaAction::Reveal(id) => state.media.reveal(&id),
            }
        }
    }

    if !message.reactions.is_empty() {
        ui.add_space(space::XS);
        let mut toggled = None;
        let mut open_picker = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            for reaction in &message.reactions {
                if reaction_chip(ui, t, store, &mut state.media, reaction) {
                    toggled = Some((reaction.emoji.clone(), !reaction.mine));
                }
            }
            // Atalho para reagir com mais um emoji.
            let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0, 22.0), Sense::click());
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::SMILEY_STICKER,
                text::icon(13.0),
                t.label_tertiary,
            );
            if response.clicked() {
                open_picker = Some(rect);
            }
        });
        if let Some((emoji, add)) = toggled {
            state.relayout = true;
            state.actions.push(ChatAction::React {
                message_id: message.id.clone(),
                emoji,
                add,
            });
        }
        if let Some(anchor) = open_picker {
            state.popup = Some(Popup {
                kind: PopupKind::Emoji,
                message_id: message.id.clone(),
                anchor,
                at_pointer: false,
                opened: ui.input(|input| input.time),
            });
        }
    }
}

/// Texto entremeado de emoji: cada emoji vira imagem, o resto é palavra
/// solta para o egui quebrar a linha onde precisar.
#[allow(clippy::too_many_arguments)]
fn rich_body(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    store: &Store,
    state: &mut UiState,
    tokens: &[emoji::Token],
    color: Color32,
    edited: bool,
    width: f32,
) {
    // Mensagem só de emoji aparece grande, como manda o costume.
    let size = if emoji::jumbo(tokens) { 34.0 } else { 18.0 };
    let font = text::message();

    ui.set_max_width(width);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(0.0, space::XXS);
        for token in tokens {
            match token {
                emoji::Token::Text(text) => {
                    for word in text.split_inclusive(' ') {
                        if word.trim().is_empty() && word != " " {
                            continue;
                        }
                        ui.label(RichText::new(word).font(font.clone()).color(color));
                    }
                }
                emoji::Token::Unicode(glyph) => {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
                    emoji::draw_unicode(ui, t, &mut state.media, glyph, rect);
                }
                emoji::Token::Custom(id) => {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
                    emoji::draw_reaction(
                        ui,
                        t,
                        &mut state.media,
                        store,
                        &Emoji::Custom(id.clone()),
                        rect,
                    );
                    if let Some(custom) = store.emojis.iter().find(|emoji| &emoji.id == id) {
                        response.on_hover_text(format!(":{}:", custom.name));
                    }
                }
            }
        }
        if edited {
            ui.label(
                RichText::new(format!("  ({})", s.edited))
                    .font(text::footnote())
                    .color(t.label_tertiary),
            );
        }
    });
}

fn reaction_chip(
    ui: &mut egui::Ui,
    t: &Tokens,
    store: &Store,
    media: &mut MediaStore,
    reaction: &crate::state::Reaction,
) -> bool {
    let count = reaction.count.to_string();
    let glyph = emoji::reaction_width(&reaction.emoji);
    let galley = ui
        .painter()
        .layout_no_wrap(count.clone(), text::caption(), t.label);
    let size = Vec2::new(glyph + galley.size().x + space::LG + space::XS, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    let (fill, stroke, label) = if reaction.mine {
        (t.accent.gamma_multiply(0.20), t.accent, t.label)
    } else if response.hovered() {
        (t.fill_medium, t.separator, t.label)
    } else {
        (t.fill_soft, Color32::TRANSPARENT, t.label_secondary)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::CONTROL),
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );

    let glyph_rect = Rect::from_center_size(
        egui::pos2(rect.min.x + space::SM + glyph / 2.0, rect.center().y),
        Vec2::splat(glyph),
    );
    emoji::draw_reaction(ui, t, media, store, &reaction.emoji, glyph_rect);
    ui.painter().galley(
        egui::pos2(
            glyph_rect.max.x + space::XS,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        label,
    );
    response.clicked()
}

/// Pastilha de ações que aparece no alto da linha ao passar o mouse.
fn hover_pill(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    message: &Message,
    row: Rect,
    store: &Store,
) {
    if message.pending {
        return;
    }
    let mine = message.mine(&store.me);
    let buttons: Vec<(&str, &str)> = if mine {
        vec![
            (icon::SMILEY_STICKER, s.react),
            (icon::ARROW_BEND_UP_LEFT, s.reply),
            (icon::PENCIL_SIMPLE, s.edit),
            (icon::DOTS_THREE, s.more),
        ]
    } else {
        vec![
            (icon::SMILEY_STICKER, s.react),
            (icon::ARROW_BEND_UP_LEFT, s.reply),
            (icon::DOTS_THREE, s.more),
        ]
    };

    let height = 30.0;
    let button = 26.0;
    let width = button * buttons.len() as f32 + space::XS * 2.0;
    let rect = Rect::from_min_size(
        egui::pos2(
            row.max.x - width - space::MD,
            row.min.y - ROW_PADDING - height / 2.0,
        ),
        Vec2::new(width, height),
    );

    glass_backdrop(ui, state, rect, radius::CARD as f32);
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::CARD),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    for (index, (glyph, tooltip)) in buttons.iter().enumerate() {
        let slot = Rect::from_min_size(
            egui::pos2(rect.min.x + space::XS + index as f32 * button, rect.min.y + 2.0),
            Vec2::new(button, height - 4.0),
        );
        let response = ui.interact(
            slot,
            Id::new(("hover", &message.id, index)),
            Sense::click(),
        );
        if response.hovered() {
            ui.painter()
                .rect_filled(slot, CornerRadius::same(radius::CONTROL), t.fill_medium);
        }
        ui.painter().text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            *glyph,
            text::icon(14.0),
            if response.hovered() {
                t.label
            } else {
                t.label_secondary
            },
        );
        let response = response.on_hover_text(*tooltip);
        if response.clicked() {
            let opened = ui.input(|input| input.time);
            match *glyph {
                icon::SMILEY_STICKER => {
                    state.popup = Some(Popup {
                        kind: PopupKind::Emoji,
                        message_id: message.id.clone(),
                        anchor: slot,
                        at_pointer: false,
                        opened,
                    })
                }
                icon::ARROW_BEND_UP_LEFT => state.replying = Some(message.id.clone()),
                icon::PENCIL_SIMPLE => {
                    state.editing = Some((message.id.clone(), message.content.clone()))
                }
                _ => {
                    state.popup = Some(Popup {
                        kind: PopupKind::Menu,
                        message_id: message.id.clone(),
                        anchor: slot,
                        at_pointer: false,
                        opened,
                    })
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Camada de cima: popups, visualizador e aviso de download
// ---------------------------------------------------------------------------

fn overlays(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("papo-overlays"));
    let screen = ui.ctx().viewport_rect();
    let mut top = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(screen));

    saved_toast(&mut top, state, t, s);

    if let Some(popup) = state.popup.clone() {
        match popup.kind {
            PopupKind::Emoji | PopupKind::ComposerEmoji | PopupKind::ComposerSticker => {
                emoji_popup(&mut top, store, state, t, s, &popup)
            }
            PopupKind::Menu => context_menu(&mut top, store, state, t, s, &popup),
        }
    }

    if let Some(mut viewer) = state.viewer.take() {
        let attachments = store
            .message(&viewer.message_id)
            .map(|message| message.attachments.clone())
            .unwrap_or_default();
        match viewer::draw(ui, t, s, &mut state.media, &mut viewer, &attachments) {
            Some(ViewerAction::Close) => state.media.pause_all(),
            Some(ViewerAction::Download { id, name }) => {
                state.actions.push(ChatAction::Download { id, name });
                state.viewer = Some(viewer);
            }
            None => state.viewer = Some(viewer),
        }
    }
}

fn emoji_popup(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    popup: &Popup,
) {
    let custom_only = popup.kind == PopupKind::ComposerSticker;
    let size = Vec2::new(316.0, if custom_only { 300.0 } else { 380.0 });
    let rect = emoji::popup_area(ui, popup.anchor, size);
    let mut chosen = None;
    let mut query = std::mem::take(&mut state.emoji_query);
    let mut group = state.emoji_group;

    emoji::popup_frame(ui, t, rect, |ui| {
        chosen = emoji::picker(
            ui,
            t,
            s,
            store,
            &mut state.media,
            &mut query,
            &mut group,
            custom_only,
        );
    });
    state.emoji_query = query;
    state.emoji_group = group;

    // No compositor o emoji entra no texto; o do servidor entra como
    // `:apelido:`, que a mensagem desenha como imagem.
    if matches!(
        popup.kind,
        PopupKind::ComposerEmoji | PopupKind::ComposerSticker
    ) {
        if let Some(emoji) = chosen {
            let insert = match &emoji {
                Emoji::Unicode(glyph) => glyph.clone(),
                Emoji::Custom(id) => store
                    .emojis
                    .iter()
                    .find(|custom| &custom.id == id)
                    .map(|custom| format!(":{}:", custom.name))
                    .unwrap_or_default(),
            };
            if !insert.is_empty() {
                if !state.composer.is_empty() && !state.composer.ends_with(' ') {
                    state.composer.push(' ');
                }
                state.composer.push_str(&insert);
                state.composer.push(' ');
            }
            state.close_popup();
            return;
        }
        dismiss_on_outside_click(ui, state, rect);
        return;
    }

    if let Some(emoji) = chosen {
        let mine = store
            .message(&popup.message_id)
            .and_then(|message| {
                message
                    .reactions
                    .iter()
                    .find(|reaction| reaction.emoji == emoji)
            })
            .map(|reaction| reaction.mine)
            .unwrap_or(false);
        state.relayout = true;
        state.actions.push(ChatAction::React {
            message_id: popup.message_id.clone(),
            emoji,
            add: !mine,
        });
        state.close_popup();
        return;
    }

    dismiss_on_outside_click(ui, state, rect);
}

fn context_menu(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    popup: &Popup,
) {
    let Some(message) = store.message(&popup.message_id).cloned() else {
        state.close_popup();
        return;
    };
    let mine = message.mine(&store.me);

    let mut items: Vec<(&str, &str, MessageCommand)> = vec![
        (icon::SMILEY_STICKER, s.add_reaction, MessageCommand::React),
        (icon::ARROW_BEND_UP_LEFT, s.reply, MessageCommand::Reply),
    ];
    if mine {
        items.push((icon::PENCIL_SIMPLE, s.edit, MessageCommand::Edit));
    }
    items.push((icon::COPY, s.copy_text, MessageCommand::Copy));
    items.push((
        icon::PUSH_PIN,
        if message.pinned { s.unpin } else { s.pin },
        MessageCommand::Pin,
    ));
    if !message.attachments.is_empty() {
        items.push((
            icon::DOWNLOAD_SIMPLE,
            s.save_attachment,
            MessageCommand::Download,
        ));
    }
    if mine {
        items.push((icon::TRASH, s.delete, MessageCommand::Delete));
    }

    let row = 30.0;
    let size = Vec2::new(212.0, row * items.len() as f32 + space::SM * 2.0);
    let anchor = if popup.at_pointer {
        Rect::from_min_size(popup.anchor.min - Vec2::new(0.0, 4.0), Vec2::ZERO)
    } else {
        popup.anchor
    };
    let rect = emoji::popup_area(ui, anchor, size);

    ui.painter().rect(
        rect,
        CornerRadius::same(radius::SHEET),
        t.elevated_bg,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    let mut chosen = None;
    for (index, (glyph, label, command)) in items.iter().enumerate() {
        let slot = Rect::from_min_size(
            egui::pos2(rect.min.x + space::XS, rect.min.y + space::SM + index as f32 * row),
            Vec2::new(rect.width() - space::XS * 2.0, row),
        );
        let response = ui.interact(slot, Id::new(("menu", index)), Sense::click());
        let destructive = matches!(command, MessageCommand::Delete);
        if response.hovered() {
            ui.painter().rect_filled(
                slot,
                CornerRadius::same(radius::CONTROL),
                if destructive {
                    t.danger.gamma_multiply(0.18)
                } else {
                    t.fill_soft
                },
            );
        }
        let color = if destructive { t.danger } else { t.label };
        ui.painter().text(
            egui::pos2(slot.min.x + space::MD, slot.center().y),
            egui::Align2::LEFT_CENTER,
            *glyph,
            text::icon(14.0),
            color,
        );
        ui.painter().text(
            egui::pos2(slot.min.x + space::MD + 22.0, slot.center().y),
            egui::Align2::LEFT_CENTER,
            *label,
            text::body(),
            color,
        );
        if response.clicked() {
            chosen = Some(*command);
        }
    }

    if let Some(command) = chosen {
        let anchor = rect;
        match command {
            MessageCommand::React => {
                state.popup = Some(Popup {
                    kind: PopupKind::Emoji,
                    message_id: message.id.clone(),
                    anchor: Rect::from_min_size(anchor.min, Vec2::ZERO),
                    at_pointer: false,
                    opened: ui.input(|input| input.time),
                });
                return;
            }
            MessageCommand::Reply => state.replying = Some(message.id.clone()),
            MessageCommand::Edit => {
                state.editing = Some((message.id.clone(), message.content.clone()))
            }
            MessageCommand::Copy => {
                ui.ctx().copy_text(message.content.clone());
            }
            MessageCommand::Pin => state.actions.push(ChatAction::Pin {
                message_id: message.id.clone(),
                pin: !message.pinned,
            }),
            MessageCommand::Download => {
                for attachment in &message.attachments {
                    state.actions.push(ChatAction::Download {
                        id: attachment.id.clone(),
                        name: attachment.name().to_owned(),
                    });
                }
            }
            MessageCommand::Delete => state.actions.push(ChatAction::Delete(message.id.clone())),
        }
        state.close_popup();
        return;
    }

    dismiss_on_outside_click(ui, state, rect);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageCommand {
    React,
    Reply,
    Edit,
    Copy,
    Pin,
    Download,
    Delete,
}

/// Clique fora ou Esc fecha o popup — menos o clique que acabou de abri-lo.
fn dismiss_on_outside_click(ui: &egui::Ui, state: &mut UiState, rect: Rect) {
    let (now, clicked, escape, pointer) = ui.ctx().input(|input| {
        (
            input.time,
            input.pointer.any_click(),
            input.key_pressed(egui::Key::Escape),
            input.pointer.interact_pos(),
        )
    });
    let just_opened = state
        .popup
        .as_ref()
        .map(|popup| now - popup.opened < 0.001)
        .unwrap_or(false);
    let anchor = state.popup.as_ref().map(|popup| popup.anchor);
    let outside = pointer
        .map(|pos| {
            !rect.contains(pos) && !anchor.map(|anchor| anchor.contains(pos)).unwrap_or(false)
        })
        .unwrap_or(false);
    if escape || (clicked && outside && !just_opened) {
        state.close_popup();
    }
}

/// Aviso discreto depois de salvar um anexo.
fn saved_toast(ui: &mut egui::Ui, state: &mut UiState, t: &Tokens, s: &Strings) {
    // Erro da interface usa o mesmo lugar, só que sem botão.
    if let Some((message, at)) = state.error.clone() {
        let now = ui.ctx().input(|input| input.time);
        if now - at > TOAST_SECONDS {
            state.error = None;
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
            toast_frame(ui, state, t, |ui, rect| {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{} {message}", icon::WARNING),
                    text::body(),
                    t.danger,
                );
            });
            return;
        }
    }

    let Some((name, path, at)) = state.media.saved.clone() else {
        return;
    };
    let (now, scrolled) = ui.ctx().input(|input| {
        (
            input.time,
            input.smooth_scroll_delta.length_sq() > 0.01,
        )
    });
    // Sai de cena ao passar o tempo ou assim que o usuário mexe na conversa.
    if now - at > TOAST_SECONDS || scrolled {
        state.media.saved = None;
        return;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(250));

    let rect = toast_rect(ui);

    toast_surface(ui, state, t, rect);
    ui.painter().text(
        egui::pos2(rect.min.x + space::XL, rect.center().y - 9.0),
        egui::Align2::LEFT_CENTER,
        format!("{} {}", icon::CHECK_CIRCLE, attachments::elide(&name, 28)),
        text::body(),
        t.label,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + space::XL, rect.center().y + 10.0),
        egui::Align2::LEFT_CENTER,
        attachments::elide(&path.display().to_string(), 44),
        text::footnote(),
        t.label_tertiary,
    );

    let open = Rect::from_min_size(
        egui::pos2(rect.max.x - space::XL - 64.0, rect.center().y - 12.0),
        Vec2::new(64.0, 24.0),
    );
    let response = ui.interact(open, Id::new("toast-open"), Sense::click());
    ui.painter().rect_filled(
        open,
        CornerRadius::same(radius::CONTROL),
        if response.hovered() {
            t.fill_medium
        } else {
            t.fill_soft
        },
    );
    ui.painter().text(
        open.center(),
        egui::Align2::CENTER_CENTER,
        s.open,
        text::caption(),
        t.label,
    );
    if response.clicked() {
        state.actions.push(ChatAction::OpenExternally(path));
        state.media.saved = None;
    }
}

/// Lugar do aviso flutuante: centralizado, acima da caixa de texto.
fn toast_rect(ui: &egui::Ui) -> Rect {
    let screen = ui.ctx().viewport_rect();
    let width = 320.0;
    let height = 58.0;
    Rect::from_min_size(
        egui::pos2(
            screen.center().x - width / 2.0,
            screen.max.y - height - space::XXXL * 2.0,
        ),
        Vec2::new(width, height),
    )
}

/// Mesmo vidro das pastilhas: o aviso pertence à camada flutuante.
fn toast_surface(ui: &egui::Ui, state: &UiState, t: &Tokens, rect: Rect) {
    glass_backdrop(ui, state, rect, radius::SHEET as f32);
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::SHEET),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + radius::SHEET as f32 * 0.6, rect.min.y + 0.5),
            egui::pos2(rect.max.x - radius::SHEET as f32 * 0.6, rect.min.y + 0.5),
        ],
        Stroke::new(1.0, t.glass_highlight),
    );
}

fn toast_frame(
    ui: &mut egui::Ui,
    state: &UiState,
    t: &Tokens,
    build: impl FnOnce(&mut egui::Ui, Rect),
) {
    let rect = toast_rect(ui);
    toast_surface(ui, state, t, rect);
    build(ui, rect);
}

// ---------------------------------------------------------------------------
// Divisores e vazios
// ---------------------------------------------------------------------------

fn day_divider(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    at: chrono::DateTime<Local>,
    width: f32,
) {
    let today = Local::now().date_naive();
    let date = at.date_naive();
    let label = if date == today {
        s.today.to_owned()
    } else if today.signed_duration_since(date).num_days() == 1 {
        s.yesterday.to_owned()
    } else {
        at.format("%d/%m/%Y").to_string()
    };

    ui.add_space(space::XXL);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width - space::XL * 2.0, 16.0), Sense::hover());
    let galley = ui
        .painter()
        .layout_no_wrap(label, text::caption(), t.label_tertiary);
    let text_width = galley.size().x;
    let mid = rect.center();
    let painter = ui.painter();
    painter.line_segment(
        [
            egui::pos2(rect.min.x + space::XL, mid.y),
            egui::pos2(mid.x - text_width / 2.0 - space::LG, mid.y),
        ],
        Stroke::new(1.0, t.separator),
    );
    painter.line_segment(
        [
            egui::pos2(mid.x + text_width / 2.0 + space::LG, mid.y),
            egui::pos2(rect.max.x - space::XL, mid.y),
        ],
        Stroke::new(1.0, t.separator),
    );
    painter.galley(
        egui::pos2(mid.x - text_width / 2.0, mid.y - galley.size().y / 2.0),
        galley,
        t.label_tertiary,
    );
    ui.add_space(space::XS);
}

/// Conversa vazia. Um servidor sem canal nenhum é um caso diferente de um
/// canal sem mensagens: ali não há o que dizer até existir um canal, então a
/// tela oferece a saída em vez de convidar a falar sozinho.
fn empty_state(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    let bare = store.channels.is_empty();
    ui.add_space(space::XXXL * 2.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(if bare {
                icon::HASH
            } else {
                icon::CHATS_CIRCLE
            })
            .font(text::icon(44.0))
            .color(t.label_tertiary),
        );
        ui.add_space(space::LG);
        ui.label(
            RichText::new(if bare {
                s.no_channels_title
            } else {
                s.empty_channel_title
            })
            .font(text::title2())
            .color(t.label),
        );
        ui.add_space(space::XS);
        ui.label(
            RichText::new(if bare {
                s.no_channels_body
            } else {
                s.empty_channel_body
            })
            .font(text::body())
            .color(t.label_secondary),
        );
        if bare {
            ui.add_space(space::XL);
            if ui.button(s.create_channel).clicked() {
                state.actions.push(ChatAction::NewChannel);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Caixa de mensagem
// ---------------------------------------------------------------------------

/// Tira o texto da caixa e o coloca na fila de envio.
fn submit(state: &mut UiState) {
    let content = state.composer.trim().to_owned();
    if content.is_empty() && state.attachments.is_empty() {
        return;
    }
    state.composer.clear();
    state.actions.push(ChatAction::Send {
        content,
        reply_to: state.replying.take(),
        attachments: std::mem::take(&mut state.attachments),
    });
}

fn composer_height(state: &UiState) -> f32 {
    let lines = state.composer.lines().count().clamp(1, 8) as f32;
    let mut height = COMPOSER_LINE_H + (lines - 1.0) * 18.0;
    if state.replying.is_some() {
        height += COMPOSER_REPLY_H;
    }
    if !state.attachments.is_empty() {
        height += COMPOSER_ATTACH_H;
    }
    height
}

/// Pastilha redonda com um ícone, solta ao lado da caixa de texto.
fn side_pill(
    ui: &mut egui::Ui,
    state: &UiState,
    t: &Tokens,
    rect: Rect,
    glyph: &str,
    tooltip: &str,
    tag: &str,
    active: bool,
) -> egui::Response {
    let response = ui.interact(rect, Id::new(("side-pill", tag)), Sense::click());
    glass_backdrop(ui, state, rect, PILL_RADIUS);
    let fill = if active {
        t.danger.gamma_multiply(0.85)
    } else {
        t.pill_fill(state.translucent)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(PILL_RADIUS as u8),
        fill,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + PILL_RADIUS * 0.6, rect.min.y + 0.5),
            egui::pos2(rect.max.x - PILL_RADIUS * 0.6, rect.min.y + 0.5),
        ],
        Stroke::new(1.0, t.glass_highlight),
    );
    if response.hovered() && !active {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(PILL_RADIUS as u8), t.fill_soft);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(16.0),
        if active {
            t.accent_label
        } else if response.hovered() {
            t.label
        } else {
            t.label_secondary
        },
    );
    response.on_hover_text(tooltip)
}

/// Botãozinho dentro da caixa de texto (emoji, figurinha, gif, enviar).
fn inline_button(
    ui: &mut egui::Ui,
    t: &Tokens,
    rect: Rect,
    glyph: &str,
    tooltip: &str,
    tag: &str,
) -> egui::Response {
    let response = ui.interact(rect, Id::new(("composer", tag)), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(15.0),
        if response.hovered() {
            t.label
        } else {
            t.label_secondary
        },
    );
    response.on_hover_text(tooltip)
}

fn composer(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
    height: f32,
) {
    // Anexar e gravar ficam de fora, cada um na sua pastilha; o resto mora
    // dentro da caixa de texto.
    let side = PILL_HEIGHT;
    let recording = state.recorder.is_some();
    let with_record = state.show_record || recording;
    let left_count = 1 + usize::from(with_record);
    let left_width = left_count as f32 * side + (left_count as f32 - 1.0) * space::SM + space::MD;

    let rect = Rect::from_min_max(
        egui::pos2(
            area.min.x + PILL_MARGIN + left_width,
            area.max.y - PILL_MARGIN - height,
        ),
        egui::pos2(area.max.x - PILL_MARGIN, area.max.y - PILL_MARGIN),
    );

    typing_pill(ui, store, state, t, s, rect);
    pill_surface(ui, state, t, rect);

    // As pastilhas laterais acompanham a última linha da caixa.
    let line_mid = rect.max.y - COMPOSER_LINE_H / 2.0;
    let attach_rect = Rect::from_center_size(
        egui::pos2(area.min.x + PILL_MARGIN + side / 2.0, line_mid),
        Vec2::splat(side),
    );
    if side_pill(ui, state, t, attach_rect, icon::PAPERCLIP, s.attach, "attach", false).clicked() {
        state.actions.push(ChatAction::PickFiles);
    }
    if with_record {
        let record_rect = Rect::from_center_size(
            egui::pos2(attach_rect.center().x + side + space::SM, line_mid),
            Vec2::splat(side),
        );
        let label = if recording { s.record_stop } else { s.record };
        let glyph = if recording {
            icon::STOP_CIRCLE
        } else {
            icon::MICROPHONE
        };
        if side_pill(ui, state, t, record_rect, glyph, label, "record", recording).clicked() {
            match state.recorder.take() {
                // Parar vira anexo na hora.
                Some(recorder) => {
                    if let Some(path) = recorder.finish() {
                        state
                            .attachments
                            .push(crate::platform::files::describe(&path));
                    }
                }
                None => {
                    state.recorder = crate::media::player::Recorder::start(
                        &crate::media::cache_root().join("recordings"),
                    );
                    if state.recorder.is_none() {
                        let now = ui.input(|input| input.time);
                        state.error = Some((s.record_failed.to_owned(), now));
                    }
                }
            }
        }
    }

    let mut cursor = rect.min.y;

    // Faixa de resposta.
    if let Some(reply_to) = state.replying.clone() {
        let band = Rect::from_min_size(
            egui::pos2(rect.min.x, cursor),
            Vec2::new(rect.width(), COMPOSER_REPLY_H),
        );
        let name = store
            .message(&reply_to)
            .and_then(|message| store.member(&message.author_id))
            .map(|member| member.name.clone())
            .unwrap_or_else(|| s.reply_missing.to_owned());
        ui.painter().text(
            egui::pos2(band.min.x + space::LG, band.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{} {} {name}", icon::ARROW_BEND_UP_LEFT, s.replying_to),
            text::footnote(),
            t.label_secondary,
        );
        let close = Rect::from_center_size(
            egui::pos2(band.max.x - space::LG - 8.0, band.center().y),
            Vec2::splat(20.0),
        );
        let response = ui.interact(close, Id::new("reply-close"), Sense::click());
        ui.painter().text(
            close.center(),
            egui::Align2::CENTER_CENTER,
            icon::X,
            text::icon(12.0),
            if response.hovered() {
                t.label
            } else {
                t.label_tertiary
            },
        );
        if response.clicked() {
            state.replying = None;
        }
        cursor = band.max.y;
    }

    // Faixa dos anexos escolhidos.
    if !state.attachments.is_empty() {
        let band = Rect::from_min_size(
            egui::pos2(rect.min.x, cursor),
            Vec2::new(rect.width(), COMPOSER_ATTACH_H),
        );
        let mut remove = None;
        let mut x = band.min.x + space::LG;
        for (index, upload) in state.attachments.iter().enumerate() {
            let chip = Rect::from_min_size(
                egui::pos2(x, band.min.y + space::XS),
                Vec2::new(168.0, COMPOSER_ATTACH_H - space::MD),
            );
            if chip.max.x > band.max.x - space::LG {
                break;
            }
            ui.painter().rect(
                chip,
                CornerRadius::same(radius::CARD),
                t.fill_soft,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            );
            let glyph = match crate::api::models::Kind::guess(&upload.mime, &upload.name) {
                crate::api::models::Kind::Image => icon::IMAGE,
                crate::api::models::Kind::Video => icon::FILM_STRIP,
                crate::api::models::Kind::Audio => icon::MICROPHONE,
                crate::api::models::Kind::Other => icon::FILE,
            };
            ui.painter().text(
                egui::pos2(chip.min.x + space::LG, chip.center().y),
                egui::Align2::CENTER_CENTER,
                glyph,
                text::icon(16.0),
                t.label_secondary,
            );
            ui.painter().text(
                egui::pos2(chip.min.x + space::XXXL, chip.center().y - 7.0),
                egui::Align2::LEFT_CENTER,
                attachments::elide(&upload.name, 16),
                text::caption(),
                t.label,
            );
            ui.painter().text(
                egui::pos2(chip.min.x + space::XXXL, chip.center().y + 8.0),
                egui::Align2::LEFT_CENTER,
                attachments::size_label(upload.size as i64),
                text::footnote(),
                t.label_tertiary,
            );
            let close = Rect::from_center_size(
                egui::pos2(chip.max.x - space::MD, chip.min.y + space::MD),
                Vec2::splat(18.0),
            );
            let response = ui.interact(close, Id::new(("chip", index)), Sense::click());
            ui.painter().text(
                close.center(),
                egui::Align2::CENTER_CENTER,
                icon::X_CIRCLE,
                text::icon(13.0),
                if response.hovered() {
                    t.label
                } else {
                    t.label_tertiary
                },
            );
            if response.clicked() {
                remove = Some(index);
            }
            x = chip.max.x + space::SM;
        }
        if let Some(index) = remove {
            state.attachments.remove(index);
        }
        cursor = band.max.y;
    }

    let line = Rect::from_min_max(egui::pos2(rect.min.x, cursor), rect.max);

    // Gravando: a caixa vira o painel da gravação, sem lugar para digitar.
    if recording {
        let elapsed = state
            .recorder
            .as_ref()
            .map(|recorder| recorder.elapsed())
            .unwrap_or(0.0);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));

        let pulse = 0.55 + 0.45 * ((ui.input(|input| input.time) * 3.0).sin() as f32).abs();
        ui.painter().circle_filled(
            egui::pos2(line.min.x + space::XXL, line.center().y),
            5.0,
            t.danger.gamma_multiply(pulse),
        );
        ui.painter().text(
            egui::pos2(line.min.x + space::XXL + space::LG, line.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{} · {}", s.recording, attachments::clock(elapsed)),
            text::body(),
            t.label,
        );

        let cancel = Rect::from_center_size(
            egui::pos2(line.max.x - space::XXL, line.center().y),
            Vec2::splat(HIT_TARGET),
        );
        if inline_button(ui, t, cancel, icon::TRASH, s.record_cancel, "record-cancel").clicked() {
            if let Some(recorder) = state.recorder.take() {
                recorder.cancel();
            }
        }
        return;
    }

    let channel_name = store
        .channel(&store.selected_channel)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    // Botões do lado direito, de fora para dentro: enviar, gif, figurinha,
    // emoji.
    let ready = !state.composer.trim().is_empty() || !state.attachments.is_empty();
    let mid = line.center().y;
    let send_rect = Rect::from_center_size(
        egui::pos2(line.max.x - space::MD - HIT_TARGET / 2.0, mid),
        Vec2::splat(HIT_TARGET),
    );
    let gif_rect = Rect::from_center_size(
        egui::pos2(send_rect.center().x - HIT_TARGET - space::XXS, mid),
        Vec2::splat(HIT_TARGET),
    );
    let sticker_rect = Rect::from_center_size(
        egui::pos2(gif_rect.center().x - HIT_TARGET - space::XXS, mid),
        Vec2::splat(HIT_TARGET),
    );
    let emoji_rect = Rect::from_center_size(
        egui::pos2(sticker_rect.center().x - HIT_TARGET - space::XXS, mid),
        Vec2::splat(HIT_TARGET),
    );

    let opened = ui.input(|input| input.time);
    if inline_button(ui, t, emoji_rect, icon::SMILEY, s.emoji, "emoji").clicked() {
        state.popup = Some(Popup {
            kind: PopupKind::ComposerEmoji,
            message_id: String::new(),
            anchor: emoji_rect,
            at_pointer: false,
            opened,
        });
    }
    if inline_button(ui, t, sticker_rect, icon::STICKER, s.sticker, "sticker").clicked() {
        state.popup = Some(Popup {
            kind: PopupKind::ComposerSticker,
            message_id: String::new(),
            anchor: sticker_rect,
            at_pointer: false,
            opened,
        });
    }
    if inline_button(ui, t, gif_rect, icon::GIF, s.gif, "gif").clicked() {
        state.actions.push(ChatAction::PickGif);
    }

    let send = ui.interact(send_rect, Id::new("composer-send"), Sense::click());
    ui.painter().circle_filled(
        send_rect.center(),
        13.0,
        if ready { t.accent } else { Color32::TRANSPARENT },
    );
    ui.painter().text(
        send_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon::PAPER_PLANE_TILT,
        text::icon(14.0),
        if ready { t.accent_label } else { t.label_tertiary },
    );
    if send.clicked() && ready {
        submit(state);
    }

    // O campo de texto ocupa o que sobrou entre a borda e os botões.
    let field = Rect::from_min_max(
        egui::pos2(line.min.x + space::MD, line.min.y + space::XS),
        egui::pos2(emoji_rect.min.x - space::XXS, line.max.y - space::XS),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(field)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let hint = format!("{} #{}…", s.composer_hint, channel_name);
            let response = ui.add(
                TextEdit::multiline(&mut state.composer)
                    .hint_text(RichText::new(hint).color(t.label_tertiary))
                    .frame(Frame::NONE)
                    .font(text::message())
                    .desired_rows(1)
                    .vertical_align(Align::Center)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(space::XS as i8, space::SM as i8)),
            );
            state.typed = response.changed();
            // Enter envia; Shift+Enter quebra linha.
            let enter = response.has_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
            if enter {
                submit(state);
            }
        },
    );
}

// ---------------------------------------------------------------------------

pub fn presence_color(t: &Tokens, presence: Presence) -> Color32 {
    match presence {
        Presence::Online => t.online,
        Presence::Away => t.away,
        Presence::Busy => t.busy,
        Presence::Offline => t.label_tertiary,
    }
}

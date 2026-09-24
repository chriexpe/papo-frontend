//! Ajustes, numa folha só.
//!
//! Duas pastilhas gêmeas na coluna da esquerda abrem a mesma folha: a de
//! baixo é você (conta, aparência, avisos, arquivos, idioma, sessões), a de
//! cima é o servidor (identidade, canais, cargos, figurinhas, auditoria).
//!
//! A folha **não** usa o vidro das pastilhas. O guia do material é explícito:
//! superfícies grandes ficam mais opacas para o texto continuar legível, e
//! vidro sobre vidro desmancha a hierarquia que o vidro existe para criar.
//! Então a pastilha é fosca e a folha é sólida — e é essa diferença que
//! separa "o controle que flutua" de "a tela onde se trabalha".

use egui::{Align, CornerRadius, Layout, Rect, RichText, Sense, Stroke, UiBuilder, Vec2};

use crate::i18n::Strings;
use crate::api::net::{ReconcileJobState, RuntimeDiagnostics};
use crate::state::{Store, StoreDiagnostics};

pub struct WorkspaceDiagnostics {
    pub label: String,
    pub server_key: String,
    pub runtime: RuntimeDiagnostics,
    pub store: StoreDiagnostics,
    pub cache_enabled: bool,
    pub cache: papo_core::cache::CacheStatsSnapshot,
    pub notification: papo_core::notification::NotificationDiagnostics,
}

use super::theme::{radius, space, text, Tokens};

/// Qual das duas pastilhas abriu a folha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    App,
    Server,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppPane {
    Account,
    Appearance,
    Alerts,
    Files,
    Language,
    Sessions,
    Diagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerPane {
    General,
    Channels,
    Roles,
    Emojis,
    Audit,
}

impl AppPane {
    const ALL: [Self; 7] = [
        Self::Account,
        Self::Appearance,
        Self::Alerts,
        Self::Files,
        Self::Language,
        Self::Sessions,
        Self::Diagnostics,
    ];

    fn title(self, s: &Strings) -> &'static str {
        match self {
            Self::Account => s.pane_account,
            Self::Appearance => s.appearance,
            Self::Alerts => s.pane_alerts,
            Self::Files => s.pane_files,
            Self::Language => s.language,
            Self::Sessions => s.sessions,
            Self::Diagnostics => "Diagnostics",
        }
    }
}

impl ServerPane {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Channels,
        Self::Roles,
        Self::Emojis,
        Self::Audit,
    ];

    /// Nome curto, para a coluna. O título longo continua valendo dentro
    /// do painel, na legenda da seção.
    fn title(self, s: &Strings) -> &'static str {
        match self {
            Self::General => s.pane_identity,
            Self::Channels => s.pane_channels,
            Self::Roles => s.roles,
            Self::Emojis => s.pane_emojis,
            Self::Audit => s.pane_audit,
        }
    }
}

/// Estado da folha. O painel aberto é lembrado por superfície: quem ajusta
/// uma coisa costuma voltar para ajustar a vizinha.
pub struct SettingsState {
    pub open: Option<Surface>,
    pub app_pane: AppPane,
    pub server_pane: ServerPane,
    /// Rascunhos dos campos de texto, para não reescrever o estado a cada
    /// tecla digitada.
    pub draft: Draft,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: None,
            app_pane: AppPane::Account,
            server_pane: ServerPane::General,
            draft: Draft::default(),
        }
    }
}

/// Figurinha já escolhida e encolhida, à espera de um nome.
#[derive(Clone, Debug)]
pub struct PendingSticker {
    pub blob: String,
    pub format: String,
    /// Precisou ser reduzida para caber no limite do servidor.
    pub shrunk: bool,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ChannelDraft {
    pub id: Option<String>,
    pub name: String,
    pub topic: String,
    /// `text`, `voice` ou `category`, como o contrato espera.
    pub kind: String,
}

impl ChannelDraft {
    pub fn create() -> Self {
        Self {
            id: None,
            name: String::new(),
            topic: String::new(),
            kind: "text".to_owned(),
        }
    }

    pub fn edit(channel: &crate::state::Channel) -> Self {
        use crate::state::ChannelKind;
        Self {
            id: Some(channel.id.clone()),
            name: channel.name.clone(),
            topic: channel.topic.clone().unwrap_or_default(),
            kind: match channel.kind {
                ChannelKind::Voice => "voice".to_owned(),
                ChannelKind::Category => "category".to_owned(),
                ChannelKind::Text => "text".to_owned(),
            },
        }
    }
}

#[derive(Default)]
pub struct Draft {
    pub loaded_account: bool,
    pub nickname: String,
    pub status_message: String,
    pub description: String,
    pub password: String,
    pub loaded_server: bool,
    pub server_name: String,
    pub server_public: bool,
    pub server_password: String,
    /// Criação/edição de canal acontece dentro da própria folha.
    pub channel: Option<ChannelDraft>,
    /// Canal a apagar: id, nome esperado e o que foi digitado. A confirmação
    /// é por cópia do nome — apagar um canal leva junto tudo que foi dito
    /// nele, e um clique só é barato demais para isso.
    pub deleting: Option<(String, String, String)>,
    pub pending_sticker: Option<PendingSticker>,
    pub loaded_audit: bool,
}

impl SettingsState {
    pub fn open_new_channel(&mut self) {
        self.open = Some(Surface::Server);
        self.server_pane = ServerPane::Channels;
        self.draft.channel = Some(ChannelDraft::create());
        self.draft.deleting = None;
    }

    pub fn open_edit_channel(&mut self, channel: &crate::state::Channel) {
        self.open = Some(Surface::Server);
        self.server_pane = ServerPane::Channels;
        self.draft.channel = Some(ChannelDraft::edit(channel));
        self.draft.deleting = None;
    }

    pub fn open_delete_channel(&mut self, channel: &crate::state::Channel) {
        self.open = Some(Surface::Server);
        self.server_pane = ServerPane::Channels;
        self.draft.channel = None;
        self.draft.deleting = Some((
            channel.id.clone(),
            channel.name.clone(),
            String::new(),
        ));
    }

    /// Abre a folha na superfície pedida, ou fecha se ela já era a aberta.
    pub fn toggle(&mut self, surface: Surface) {
        if self.open == Some(surface) {
            self.open = None;
            return;
        }
        self.open = Some(surface);
        match surface {
            Surface::App => self.draft.loaded_account = false,
            Surface::Server => {
                self.draft.loaded_server = false;
                self.draft.loaded_audit = false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Primitivas da folha
//
// É aqui que a tela deixa de ser uma pilha de caixas de seleção. Uma linha
// tem rótulo à esquerda, explicação embaixo dele quando precisa, e o controle
// encostado à direita; linhas vizinhas moram num cartão de cantos redondos
// separadas por fios que não encostam na borda. É o idioma da tela de ajustes
// do sistema, e ele carrega hierarquia sem precisar de mais cor.
// ---------------------------------------------------------------------------

const ROW_HEIGHT: f32 = 38.0;
const ROW_INSET: f32 = space::LG;
/// Largura reservada ao controle, encostado à direita da linha.
const CONTROL_COLUMN: f32 = 190.0;
/// Linhas que mostram figurinha: a arte e a altura que ela pede.
const STICKER_SIDE: f32 = 28.0;
const STICKER_ROW_H: f32 = 44.0;
const PREVIEW_SIDE: f32 = 72.0;
const PREVIEW_H: f32 = 88.0;
/// Alvo de clique no desktop: 28×28 é o padrão do guia, 20×20 o mínimo.
const SWITCH_W: f32 = 38.0;
const SWITCH_H: f32 = 22.0;

/// Título de seção acima de um cartão.
fn section(ui: &mut egui::Ui, t: &Tokens, label: &str) {
    ui.add_space(space::LG);
    ui.horizontal(|ui| {
        // Mesmo recuo das linhas: o título tem de nascer na mesma coluna
        // que os rótulos que ele cobre.
        ui.add_space(ROW_INSET);
        ui.label(
            RichText::new(label.to_uppercase())
                .font(text::caption())
                .color(t.label_tertiary),
        );
    });
    ui.add_space(space::SM);
}

/// Agrupa linhas. Sem fundo próprio: um bloco mais claro dentro da folha
/// vira uma segunda janela, e duas superfícies empilhadas foi exatamente o
/// que ficou pesado. O que separa as linhas é o fio entre elas, que começa
/// onde o rótulo começa — assim a lista tem estrutura sem ter caixa.
fn group(ui: &mut egui::Ui, t: &Tokens, contents: impl FnOnce(&mut Rows)) {
    let mut rows = Rows {
        ui,
        t: *t,
        separators: Vec::new(),
        first: true,
    };
    contents(&mut rows);
    let separators = std::mem::take(&mut rows.separators);
    let ui = rows.ui;

    let rect = ui.min_rect();
    for y in separators {
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x + ROW_INSET, y),
                egui::pos2(rect.max.x, y),
            ],
            Stroke::new(1.0, t.separator),
        );
    }
}

/// Construtor de linhas dentro de um cartão.
pub struct Rows<'u> {
    ui: &'u mut egui::Ui,
    t: Tokens,
    separators: Vec<f32>,
    first: bool,
}

impl Rows<'_> {
    /// Reserva a faixa de uma linha e devolve a área útil dela.
    fn band(&mut self, height: f32) -> (Rect, Rect) {
        let width = self.ui.available_width();
        let (rect, _) = self
            .ui
            .allocate_exact_size(Vec2::new(width, height), Sense::hover());
        if !self.first {
            self.separators.push(rect.min.y);
        }
        self.first = false;
        let inner = Rect::from_min_max(
            egui::pos2(rect.min.x + ROW_INSET, rect.min.y),
            egui::pos2(rect.max.x - space::MD, rect.max.y),
        );
        (rect, inner)
    }

    /// Rótulo + apoio + controle. Em espaço largo mantém duas colunas;
    /// quando o próprio painel fica estreito, empilha o controle embaixo.
    fn row(
        &mut self,
        label: &str,
        hint: Option<&str>,
        control: impl FnOnce(&mut egui::Ui, &Tokens),
    ) {
        let t = self.t;
        let full = (self.ui.available_width() - ROW_INSET - space::MD).max(120.0);
        let stacked = full < 430.0;
        let control_width = if stacked {
            full
        } else {
            CONTROL_COLUMN.min(full * 0.48)
        };
        let text_width = if stacked {
            full
        } else {
            (full - control_width - space::LG).max(120.0)
        };

        let label_galley = self.ui.painter().layout(
            label.to_owned(),
            text::body(),
            t.label,
            text_width,
        );
        let hint_galley = hint.map(|hint| {
            self.ui.painter().layout(
                hint.to_owned(),
                text::footnote(),
                t.label_tertiary,
                text_width,
            )
        });
        let text_height = label_galley.size().y
            + hint_galley
                .as_ref()
                .map(|galley| space::XXS + galley.size().y)
                .unwrap_or(0.0);
        let height = if stacked {
            (space::MD + text_height + space::SM + 30.0 + space::MD).max(ROW_HEIGHT)
        } else {
            (text_height + space::MD * 2.0).max(ROW_HEIGHT)
        };
        let (_, inner) = self.band(height);

        let text_top = if stacked {
            inner.min.y + space::MD
        } else {
            inner.center().y - text_height / 2.0
        };
        self.ui.painter().galley(
            egui::pos2(inner.min.x, text_top),
            label_galley,
            t.label,
        );
        if let Some(galley) = hint_galley {
            self.ui.painter().galley(
                egui::pos2(inner.min.x, text_top + text_height - galley.size().y),
                galley,
                t.label_tertiary,
            );
        }

        let control_rect = if stacked {
            Rect::from_min_max(
                egui::pos2(inner.min.x, inner.max.y - 30.0 - space::MD),
                egui::pos2(inner.max.x, inner.max.y - space::MD),
            )
        } else {
            Rect::from_min_max(
                egui::pos2(inner.max.x - control_width, inner.min.y),
                inner.max,
            )
        };
        self.ui.scope_builder(
            UiBuilder::new()
                .max_rect(control_rect)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| control(ui, &t),
        );
    }

    /// Linha inteira clicável, para navegar ou disparar uma ação.
    fn action(&mut self, label: &str, hint: Option<&str>, danger: bool) -> bool {
        let width = (self.ui.available_width() - ROW_INSET - space::MD).max(120.0);
        let label_galley = self
            .ui
            .painter()
            .layout(label.to_owned(), text::body(), self.t.label, width);
        let hint_galley = hint.map(|hint| {
            self.ui.painter().layout(
                hint.to_owned(),
                text::footnote(),
                self.t.label_tertiary,
                width,
            )
        });
        let block = label_galley.size().y
            + hint_galley
                .as_ref()
                .map(|galley| space::XXS + galley.size().y)
                .unwrap_or(0.0);
        let (rect, inner) = self.band((block + space::MD * 2.0).max(ROW_HEIGHT));
        let t = self.t;
        let response = self
            .ui
            .interact(rect, self.ui.id().with(label).with(hint), Sense::click());
        if response.hovered() {
            self.ui
                .painter()
                .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
            self.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let tint = if danger { t.danger } else { t.accent };
        let top = inner.center().y - block / 2.0;
        self.ui
            .painter()
            .galley(egui::pos2(inner.min.x, top), label_galley, tint);
        if let Some(galley) = hint_galley {
            self.ui.painter().galley(
                egui::pos2(inner.min.x, top + block - galley.size().y),
                galley,
                t.label_tertiary,
            );
        }
        response.clicked()
    }

    /// Linha de escolha numa lista: o rótulo fica normal e a marca vai à
    /// direita. É o idioma de "escolha um", diferente da linha de ação, que
    /// é colorida porque faz alguma coisa.
    fn choice(&mut self, label: &str, selected: bool) -> bool {
        let (rect, inner) = self.band(ROW_HEIGHT);
        let t = self.t;
        let response = self
            .ui
            .interact(rect, self.ui.id().with("escolha").with(label), Sense::click());
        if response.hovered() && !selected {
            self.ui
                .painter()
                .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
            self.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let painter = self.ui.painter();
        painter.text(
            egui::pos2(inner.min.x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            text::body(),
            t.label,
        );
        if selected {
            painter.text(
                egui::pos2(inner.max.x, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                egui_phosphor::regular::CHECK,
                text::icon(13.0),
                t.accent,
            );
        }
        response.clicked()
    }

    /// Linha com a figurinha desenhada à esquerda do rótulo. Uma lista de
    /// figurinhas sem as figurinhas é uma lista de nomes.
    fn preview_row(
        &mut self,
        label: &str,
        media: &mut crate::media::MediaStore,
        id: &str,
        blob: Option<&str>,
        control: impl FnOnce(&mut egui::Ui, &Tokens),
    ) {
        let (rect, inner) = self.band(STICKER_ROW_H);
        let t = self.t;
        let ctx = self.ui.ctx().clone();
        let texture = media
            .emoji(id, blob)
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| handle.id());
        let art = Rect::from_center_size(
            egui::pos2(inner.min.x + STICKER_SIDE / 2.0, rect.center().y),
            Vec2::splat(STICKER_SIDE),
        );
        draw_sticker(self.ui, &t, art, texture);
        self.ui.painter().text(
            egui::pos2(art.max.x + space::MD, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            text::body(),
            t.label,
        );
        let control_rect =
            Rect::from_min_max(egui::pos2(inner.max.x - CONTROL_COLUMN, inner.min.y), inner.max);
        self.ui.scope_builder(
            UiBuilder::new()
                .max_rect(control_rect)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| control(ui, &t),
        );
    }

    /// Figurinha grande, para conferir antes de dar nome a ela.
    fn preview(
        &mut self,
        label: &str,
        note: Option<&str>,
        media: &mut crate::media::MediaStore,
        id: &str,
        blob: Option<&str>,
    ) {
        let (rect, inner) = self.band(PREVIEW_H);
        let t = self.t;
        let ctx = self.ui.ctx().clone();
        let texture = media
            .emoji(id, blob)
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| handle.id());
        let painter = self.ui.painter();
        let middle = rect.center().y;
        match note {
            None => painter.text(
                egui::pos2(inner.min.x, middle),
                egui::Align2::LEFT_CENTER,
                label,
                text::body(),
                t.label,
            ),
            Some(note) => {
                painter.text(
                    egui::pos2(inner.min.x, middle - 9.0),
                    egui::Align2::LEFT_CENTER,
                    label,
                    text::body(),
                    t.label,
                );
                painter.text(
                    egui::pos2(inner.min.x, middle + 9.0),
                    egui::Align2::LEFT_CENTER,
                    note,
                    text::footnote(),
                    t.label_tertiary,
                )
            }
        };
        let art = Rect::from_center_size(
            egui::pos2(inner.max.x - PREVIEW_SIDE / 2.0, middle),
            Vec2::splat(PREVIEW_SIDE),
        );
        draw_sticker(self.ui, &t, art, texture);
    }

    /// Linha com um campo de texto ocupando a direita.
    fn field(&mut self, label: &str, value: &mut String, limit: usize, secret: bool) -> bool {
        let mut submitted = false;
        self.row(label, None, |ui, _t| {
            let width = ui.available_width();

            #[cfg(target_os = "android")]
            {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::hover());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(radius::FIELD),
                    _t.fill_soft,
                    Stroke::new(1.0, _t.separator),
                    egui::StrokeKind::Inside,
                );
                let edit_id = ui.id().with(("settings-field", label));
                let key = format!("settings:{edit_id:?}");
                submitted = crate::platform::native_field::show(
                    ui.ctx(),
                    &key,
                    value,
                    rect.shrink2(Vec2::new(space::MD, space::XS)),
                    "",
                    if secret {
                        crate::platform::native_field::Mode::Password
                    } else {
                        crate::platform::native_field::Mode::Text
                    },
                    limit,
                    false,
                    _t.label,
                    _t.label_tertiary,
                    text::body().size,
                )
                .submit;
            }

            #[cfg(not(target_os = "android"))]
            {
                let edit_id = ui.id().with(("settings-field", label));
                let _ = crate::platform::ime::prepare_text_edit(ui.ctx(), edit_id, value);
                let response = ui.add_sized(
                    Vec2::new(width, 26.0),
                    egui::TextEdit::singleline(value)
                        .id(edit_id)
                        .char_limit(limit)
                        .password(secret)
                        .font(text::body())
                        .margin(egui::Margin::symmetric(space::MD as i8, space::XS as i8)),
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
                submitted =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            }
        });
        submitted
    }
}

/// Botão de ícone para a linha. Mover para cima e para baixo em palavras
/// ocupavam metade da linha e encostavam no nome do canal; a seta diz o
/// mesmo em um quadrado.
fn row_icon(ui: &mut egui::Ui, t: &Tokens, glyph: &str, tip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(13.0),
        t.label_secondary,
    );
    response.on_hover_text(tip).clicked()
}

/// Faixa dos botões de ação, logo abaixo de uma lista.
///
/// Precisa de altura própria: um `with_layout` solto herda toda a altura
/// que sobrou no painel e centra o botão no meio dela — era o que deixava
/// "Salvar" boiando longe do que ele salva.
fn actions_row(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 30.0),
        Layout::right_to_left(Align::Center),
        contents,
    );
}

/// Desenha a figurinha, ou o lugar dela enquanto não chega.
fn draw_sticker(ui: &egui::Ui, t: &Tokens, rect: Rect, texture: Option<egui::TextureId>) {
    match texture {
        Some(texture) => {
            let mut mesh = egui::Mesh::with_texture(texture);
            mesh.add_rect_with_uv(
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            ui.painter().add(egui::Shape::mesh(mesh));
        }
        None => {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
        }
    }
}

/// Interruptor. Num painel de ajustes que aplica na hora, o interruptor diz
/// a verdade e a caixa de seleção mente: caixa pede um "OK" que não existe.
fn switch(ui: &mut egui::Ui, t: &Tokens, on: &mut bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(SWITCH_W, SWITCH_H), Sense::click());
    if response.clicked() {
        *on = !*on;
    }
    let track = if *on {
        t.accent
    } else {
        t.fill_medium
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same((SWITCH_H / 2.0) as u8), track);
    let travel = rect.width() - SWITCH_H;
    let knob = egui::pos2(
        rect.min.x + SWITCH_H / 2.0 + if *on { travel } else { 0.0 },
        rect.center().y,
    );
    ui.painter()
        .circle_filled(knob, SWITCH_H / 2.0 - 3.0, egui::Color32::WHITE);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// Controle segmentado: as opções lado a lado, uma acesa.
fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    t: &Tokens,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    let height = 26.0;
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), text::callout(), t.label)
                .size()
                .x
                + space::LG
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);

    let mut x = rect.min.x;
    for ((value, label), width) in options.iter().zip(widths) {
        let slot = Rect::from_min_size(egui::pos2(x, rect.min.y), Vec2::new(width, height));
        x += width;
        let selected = *current == *value;
        let response = ui.interact(slot, ui.id().with(*label), Sense::click());
        if selected {
            ui.painter().rect_filled(
                slot.shrink(2.0),
                CornerRadius::same(radius::CONTROL),
                t.glass_opaque,
            );
        }
        ui.painter().text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            text::callout(),
            if selected { t.label } else { t.label_secondary },
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() && !selected {
            *current = *value;
            changed = true;
        }
    }
    changed
}

/// Botão à direita de uma linha.
///
/// O guia do material é claro sobre o orçamento de cor: colorir o fundo de
/// vários controles ao mesmo tempo tira o sentido de cada um. Então o
/// normal é neutro, `Emphasis::Primary` fica para a ação principal do
/// painel — uma por painel — e `Danger` para o que não tem volta.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Emphasis {
    Quiet,
    Primary,
    Danger,
}

fn row_button(ui: &mut egui::Ui, t: &Tokens, label: &str, emphasis: Emphasis) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::callout(), t.label);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + space::LG, 26.0),
        Sense::click(),
    );
    let (fill, ink) = match emphasis {
        Emphasis::Quiet => (
            if response.hovered() {
                t.fill_medium
            } else {
                t.fill_soft
            },
            t.label,
        ),
        Emphasis::Primary => (
            if response.hovered() {
                t.accent.gamma_multiply(0.85)
            } else {
                t.accent
            },
            t.accent_label,
        ),
        Emphasis::Danger => (
            t.danger
                .gamma_multiply(if response.hovered() { 0.24 } else { 0.14 }),
            t.danger,
        ),
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL), fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        text::callout(),
        ink,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

// ---------------------------------------------------------------------------
// A folha
// ---------------------------------------------------------------------------

const SHEET_W: f32 = 620.0;
const SHEET_H: f32 = 440.0;
const RAIL_W: f32 = 148.0;
/// Altura de um item da coluna de painéis.
const RAIL_ENTRY_H: f32 = 32.0;
const HEADER_H: f32 = 52.0;

/// O que a folha pediu. O `app` traduz em comandos de rede; os ajustes que
/// são só locais ela mesma já escreveu em `Settings`.
#[derive(Clone, Debug)]
pub enum SettingsAction {
    Admin(super::admin::AdminAction),
    Role(super::roles::RoleAction),
    Chat(super::shell::ChatAction),
    Menu(crate::platform::menu::MenuCommand),
    /// Abre o seletor de pasta do sistema para os downloads.
    PickDownloadFolder,
}

/// Tudo que a folha precisa do resto do programa.
pub struct Context<'a> {
    pub store: &'a Store,
    /// As figurinhas viram textura pelo mesmo caminho da conversa.
    pub media: &'a mut crate::media::MediaStore,
    pub roles: &'a mut super::roles::RolesState,
    pub lang: &'a mut crate::i18n::Lang,
    pub theme: &'a mut crate::ui::theme::ThemePref,
    pub translucency: &'a mut bool,
    pub notifications: &'a mut bool,
    pub reply_notifications: &'a mut bool,
    pub close_to_tray: &'a mut bool,
    pub autostart: &'a mut bool,
    pub badge: &'a mut bool,
    pub topic_reveal: &'a mut bool,
    pub record_button: &'a mut bool,
    pub ask_download: &'a mut bool,
    pub download_dir: Option<String>,
    pub diagnostics: &'a [WorkspaceDiagnostics],
}

/// Desenha a folha, ancorada na pastilha que a abriu.
pub fn sheet(
    ctx: &egui::Context,
    state: &mut SettingsState,
    data: &mut Context<'_>,
    anchor: Rect,
    screen: Rect,
    t: &Tokens,
    s: &Strings,
) -> Vec<SettingsAction> {
    let mut actions = Vec::new();
    let Some(surface) = state.open else {
        return actions;
    };

    // Sobe da pastilha de baixo, desce da de cima; e nunca passa da janela.
    // A altura é fixa, como a largura: uma folha que encolhe e cresce a cada
    // painel faz a coluna da esquerda dançar debaixo do ponteiro, e o guia
    // pede justamente o contrário — uma tela de ajustes estável, para que se
    // aprenda onde as coisas ficam.
    let width = SHEET_W.min((screen.width() - space::MD * 2.0).max(280.0));
    let height = SHEET_H.min((screen.height() - space::MD * 2.0).max(260.0));
    let top = if anchor.center().y > screen.center().y {
        (anchor.min.y - space::SM - height).max(screen.min.y + space::MD)
    } else {
        (anchor.max.y + space::SM).min(screen.max.y - space::MD - height)
    };
    let left = anchor
        .min
        .x
        .min(screen.max.x - space::MD - width)
        .max(screen.min.x + space::MD);
    let rect = Rect::from_min_size(egui::pos2(left, top), Vec2::new(width, height));

    egui::Area::new(egui::Id::new("folha-de-ajustes"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            // Superfície sólida: a folha é grande e cheia de texto, e é aí
            // que o vidro atrapalha em vez de ajudar.
            // A sombra vai embaixo. `add` empilha por cima do que já foi
            // pintado, então reservar o lugar dela antes do retângulo é o
            // que a mantém atrás — do jeito anterior ela cobria a folha
            // inteira com um véu, e era isso que parecia vidro sujo.
            let shadow = ctx.global_style().visuals.window_shadow;
            ui.painter()
                .add(shadow.as_shape(rect, radius::SHEET as f32 + 4.0));
            ui.painter().rect(
                rect,
                CornerRadius::same(radius::SHEET + 4),
                t.elevated_bg,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            );

            let title = match surface {
                Surface::App => state.app_pane.title(s),
                Surface::Server => state.server_pane.title(s),
            };
            header(ui, state, t, s, rect, surface, title);
            let body = Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y + HEADER_H),
                rect.max,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(body.min.x + space::MD, body.min.y),
                    egui::pos2(body.max.x - space::MD, body.min.y),
                ],
                Stroke::new(1.0, t.separator),
            );

            let rail_rect =
                Rect::from_min_size(body.min, Vec2::new(RAIL_W, body.height()));
            rail(ui, state, t, s, rail_rect, surface);
            ui.painter().line_segment(
                [
                    egui::pos2(rail_rect.max.x, body.min.y + space::SM),
                    egui::pos2(rail_rect.max.x, body.max.y - space::SM),
                ],
                Stroke::new(1.0, t.separator),
            );

            let pane_rect = Rect::from_min_max(
                egui::pos2(rail_rect.max.x, body.min.y),
                body.max,
            );
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(pane_rect.shrink2(Vec2::new(space::XL, space::SM)))
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("corpo-do-painel")
                        .auto_shrink([false, false])
                        .show(ui, |ui| match surface {
                            Surface::App => {
                                app_pane(ui, state, data, t, s, &mut actions);
                            }
                            Surface::Server => {
                                server_pane(ui, state, data, t, s, &mut actions);
                            }
                        });
                },
            );
        });

    // Ajustes também se comportam como uma folha/popover ancorada na
    // pastilha que a abriu. Clique fora fecha; a própria pastilha fica de
    // fora desta regra porque ela já possui o comportamento de toggle.
    let outside = ctx.input(|input| {
        input.pointer.any_click()
            && input.pointer.interact_pos().is_some_and(|position| {
                !rect.contains(position) && !anchor.contains(position)
            })
    });
    if outside {
        state.open = None;
    }

    actions
}

/// Cabeçalho: quem ou o quê está sendo ajustado, o nome do painel e o X.
fn header(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    t: &Tokens,
    s: &Strings,
    rect: Rect,
    surface: Surface,
    title: &str,
) {
    let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H));
    let painter = ui.painter();
    painter.text(
        egui::pos2(head.min.x + space::XL, head.center().y),
        egui::Align2::LEFT_CENTER,
        match surface {
            Surface::App => s.settings,
            Surface::Server => s.server_settings,
        },
        text::title3(),
        t.label,
    );
    // O guia pede que o título acompanhe o painel aberto; o nome do painel
    // vem depois do da folha, em tom secundário.
    let width = painter
        .layout_no_wrap(
            match surface {
                Surface::App => s.settings.to_owned(),
                Surface::Server => s.server_settings.to_owned(),
            },
            text::title3(),
            t.label,
        )
        .size()
        .x;
    painter.text(
        egui::pos2(head.min.x + space::XL + width + space::MD, head.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        text::body(),
        t.label_tertiary,
    );

    // Margens iguais dos dois lados do canto: com o centro a 16 do bordo,
    // o botão de 28 encostava a 2 px da direita e ficava a 12 do topo — o
    // desencontro é que fazia parecer que ele estava abaixo do canto.
    const CLOSE: f32 = 28.0;
    let inset = (HEADER_H - CLOSE) / 2.0;
    let close = Rect::from_center_size(
        egui::pos2(head.max.x - inset - CLOSE / 2.0, head.min.y + inset + CLOSE / 2.0),
        Vec2::splat(CLOSE),
    );
    let response = ui.interact(close, ui.id().with("fechar-ajustes"), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(close, CornerRadius::same(radius::FIELD), t.fill_soft);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(
        close.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::X,
        text::icon(14.0),
        t.label_secondary,
    );
    if response.clicked() || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.open = None;
    }
}

/// Coluna dos painéis. Fica sempre visível e sempre marca onde você está.
fn rail(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    t: &Tokens,
    s: &Strings,
    rect: Rect,
    surface: Surface,
) {
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(space::SM, space::MD)))
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            // O `top_down` põe um respiro entre widgets por conta própria;
            // com ele, seis painéis passavam da borda de baixo da folha.
            ui.spacing_mut().item_spacing.y = 0.0;
            let entry = |ui: &mut egui::Ui, label: &str, active: bool| -> bool {
                let (slot, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), RAIL_ENTRY_H),
                    Sense::click(),
                );
                if active {
                    ui.painter().rect_filled(
                        slot,
                        CornerRadius::same(radius::CONTROL),
                        t.accent.gamma_multiply(0.16),
                    );
                } else if response.hovered() {
                    ui.painter().rect_filled(
                        slot,
                        CornerRadius::same(radius::CONTROL),
                        t.fill_soft,
                    );
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                ui.painter().text(
                    egui::pos2(slot.min.x + space::MD, slot.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    if active { text::headline() } else { text::body() },
                    if active { t.accent } else { t.label_secondary },
                );
                response.clicked()
            };

            match surface {
                Surface::App => {
                    for pane in AppPane::ALL {
                        if entry(ui, pane.title(s), state.app_pane == pane) {
                            state.app_pane = pane;
                        }
                    }
                }
                Surface::Server => {
                    for pane in ServerPane::ALL {
                        if entry(ui, pane.title(s), state.server_pane == pane) {
                            state.server_pane = pane;
                        }
                    }
                }
            }
        },
    );
}

// ---------------------------------------------------------------------------
// Painéis do usuário
// ---------------------------------------------------------------------------

fn app_pane(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    data: &mut Context<'_>,
    t: &Tokens,
    s: &Strings,
    actions: &mut Vec<SettingsAction>,
) {
    use super::admin::AdminAction;

    match state.app_pane {
        AppPane::Account => {
            if !state.draft.loaded_account {
                state.draft.loaded_account = true;
                state.draft.nickname = data.store.my_name.clone();
                state.draft.password.clear();
                actions.push(SettingsAction::Admin(AdminAction::LoadDevices));
            }
            let draft = &mut state.draft;
            // O clique sai de dentro do cartão, e `actions` já está
            // emprestado ali; a bandeira atravessa esse empréstimo.
            let mut pick_avatar = false;

            section(ui, t, s.profile);
            group(ui, t, |rows| {
                rows.field(s.nickname, &mut draft.nickname, 32, false);
                rows.field(s.status_message, &mut draft.status_message, 64, false);
                rows.field(s.description, &mut draft.description, 512, false);
                rows.row(s.avatar, None, |ui, t| {
                    if row_button(ui, t, s.change_avatar, Emphasis::Quiet) {
                        pick_avatar = true;
                    }
                });
            });
            ui.add_space(space::SM);
            actions_row(ui, |ui| {
                if row_button(ui, t, s.save, Emphasis::Primary) {
                    actions.push(SettingsAction::Admin(AdminAction::SaveProfile(Box::new(
                        crate::api::models::UpdateUserRequest {
                            nickname: draft.nickname.trim().to_owned(),
                            status: draft.status_message.trim().to_owned(),
                            description: draft.description.trim().to_owned(),
                            typing: None,
                        },
                    ))));
                }
            });
            if pick_avatar {
                actions.push(SettingsAction::Admin(AdminAction::PickAvatar));
            }

            section(ui, t, s.presence);
            group(ui, t, |rows| {
                let mine = data
                    .store
                    .member(&data.store.me)
                    .map(|member| member.presence);
                rows.row(s.presence, None, |ui, t| {
                    let mut chosen = mine.unwrap_or(crate::state::Presence::Online);
                    if segmented(
                        ui,
                        t,
                        &mut chosen,
                        &[
                            (crate::state::Presence::Busy, s.presence_busy),
                            (crate::state::Presence::Away, s.presence_away),
                            (crate::state::Presence::Online, s.presence_online),
                        ],
                    ) {
                        actions.push(SettingsAction::Admin(AdminAction::SetPresence(
                            match chosen {
                                crate::state::Presence::Busy => Some("busy".to_owned()),
                                crate::state::Presence::Away => Some("away".to_owned()),
                                _ => None,
                            },
                        )));
                    }
                });
            });

            section(ui, t, s.new_password);
            group(ui, t, |rows| {
                let sent = rows.field(s.new_password, &mut draft.password, 64, true);
                let ready = draft.password.chars().count() >= 8;
                rows.row(s.change_password, Some(s.password_rule_length), |ui, t| {
                    if (row_button(ui, t, s.change_password, Emphasis::Primary) || sent) && ready {
                        actions.push(SettingsAction::Admin(AdminAction::ChangePassword(
                            draft.password.clone(),
                        )));
                        draft.password.clear();
                    }
                });
            });

            section(ui, t, s.sign_out);
            group(ui, t, |rows| {
                if rows.action(s.sign_out, Some(s.sign_out_hint), true) {
                    actions.push(SettingsAction::Menu(
                        crate::platform::menu::MenuCommand::SignOut,
                    ));
                }
            });
        }

        AppPane::Appearance => {
            section(ui, t, s.appearance);
            group(ui, t, |rows| {
                rows.row(s.theme, None, |ui, t| {
                    segmented(
                        ui,
                        t,
                        data.theme,
                        &[
                            (crate::ui::theme::ThemePref::Dark, s.theme_dark),
                            (crate::ui::theme::ThemePref::Light, s.theme_light),
                            (crate::ui::theme::ThemePref::System, s.theme_system),
                        ],
                    );
                });
                rows.row(s.translucency, Some(s.translucency_hint), |ui, t| {
                    switch(ui, t, data.translucency);
                });
                rows.row(s.topic_reveal, Some(s.topic_reveal_hint), |ui, t| {
                    switch(ui, t, data.topic_reveal);
                });
                rows.row(s.record_button, Some(s.record_button_hint), |ui, t| {
                    switch(ui, t, data.record_button);
                });
            });
        }

        AppPane::Alerts => {
            section(ui, t, s.menu_notifications);
            group(ui, t, |rows| {
                rows.row(s.menu_notifications, Some(s.notifications_hint), |ui, t| {
                    switch(ui, t, data.notifications);
                });
                rows.row(
                    s.reply_notifications_default,
                    Some(s.reply_notifications_default_hint),
                    |ui, t| {
                        switch(ui, t, data.reply_notifications);
                    },
                );
                rows.row(s.badge, Some(s.badge_hint), |ui, t| {
                    switch(ui, t, data.badge);
                });
                rows.row(s.menu_close_to_tray, Some(s.close_to_tray_hint), |ui, t| {
                    switch(ui, t, data.close_to_tray);
                });
                rows.row(s.start_at_login, Some(s.start_at_login_hint), |ui, t| {
                    switch(ui, t, data.autostart);
                });
            });
        }

        AppPane::Files => {
            section(ui, t, s.downloads);
            group(ui, t, |rows| {
                rows.row(s.downloads, Some(s.downloads_hint), |ui, t| {
                    segmented(
                        ui,
                        t,
                        data.ask_download,
                        &[(true, s.download_ask), (false, s.download_folder)],
                    );
                });
                if let Some(dir) = data.download_dir.clone() {
                    rows.row(s.download_folder, Some(&dir), |ui, t| {
                        if row_button(ui, t, s.download_choose, Emphasis::Quiet) {
                            actions.push(SettingsAction::PickDownloadFolder);
                        }
                    });
                }
            });
        }

        AppPane::Language => {
            section(ui, t, s.language);
            group(ui, t, |rows| {
                for lang in crate::i18n::Lang::ALL {
                    if rows.choice(lang.endonym(), *data.lang == lang) {
                        *data.lang = lang;
                    }
                }
            });
        }

        AppPane::Diagnostics => {
            section(ui, t, "Runtime diagnostics");
            for workspace in data.diagnostics {
                let title = if workspace.label.is_empty() {
                    workspace.server_key.as_str()
                } else {
                    workspace.label.as_str()
                };
                section(ui, t, title);
                group(ui, t, |rows| {
                    let runtime = &workspace.runtime;
                    let connection = format!("{:?}", runtime.connection);
                    rows.row("Server key", Some(&workspace.server_key), |_, _| {});
                    rows.row("Connection", Some(&connection), |_, _| {});
                    let network = format!(
                        "{:?} · epoch {}",
                        runtime.network.availability,
                        runtime.network.epoch
                    );
                    rows.row("Network", Some(&network), |_, _| {});
                    rows.row(
                        "Generation",
                        Some(&runtime.sync_generation.to_string()),
                        |_, _| {},
                    );
                    let cache = if workspace.cache_enabled {
                        format!(
                            "{} restored · {} written · {} dropped · {} failures",
                            workspace.cache.restores,
                            workspace.cache.written,
                            workspace.cache.dropped,
                            workspace.cache.write_failures,
                        )
                    } else {
                        "unavailable".to_owned()
                    };
                    rows.row("Cache", Some(&cache), |_, _| {});
                    let notification = format!(
                        "{} rows · {} delivered · {} suppressed · {} duplicates · {} failures",
                        workspace.notification.ledger_rows,
                        workspace.notification.delivered,
                        workspace.notification.suppressed,
                        workspace.notification.duplicate_claims,
                        workspace.notification.claim_failures,
                    );
                    rows.row("Notification ledger", Some(&notification), |_, _| {});
                    let outgoing = format!(
                        "{} queued · {} sending · {} uncertain · {} failed{}",
                        workspace.store.outgoing_queued,
                        workspace.store.outgoing_sending,
                        workspace.store.outgoing_unknown,
                        workspace.store.outgoing_failed,
                        workspace
                            .store
                            .outgoing_oldest_age_ms
                            .map(|age| format!(" · oldest {}s", age / 1000))
                            .unwrap_or_default(),
                    );
                    rows.row("Outgoing", Some(&outgoing), |_, _| {});
                    let scheduler = format!(
                        "{} running / {} queued / limit {}",
                        runtime.scheduler.running,
                        runtime.scheduler.queued,
                        runtime.scheduler.capacity
                    );
                    rows.row("Scheduler", Some(&scheduler), |_, _| {});
                    for job in &runtime.scheduler.jobs {
                        let state = match job.state {
                            ReconcileJobState::Queued => "queued",
                            ReconcileJobState::Running => "running",
                        };
                        let detail = format!(
                            "{} · {} · gen {}{}{}",
                            state,
                            job.priority,
                            job.generation,
                            job.request_id
                                .map(|id| format!(" · request {id}"))
                                .unwrap_or_default(),
                            if job.retry_blocked { " · retry gated" } else { "" }
                        );
                        rows.row(&job.key, Some(&detail), |_, _| {});
                    }
                });

                if workspace.store.timelines.is_empty() {
                    group(ui, t, |rows| {
                        rows.row("Timelines", Some("no cached timeline state"), |_, _| {});
                    });
                    continue;
                }

                group(ui, t, |rows| {
                    for timeline in &workspace.store.timelines {
                        let mut detail = format!("{:?}", timeline.status);
                        if Some(timeline.channel_id.as_str())
                            == (!workspace.store.selected_channel.is_empty())
                                .then_some(workspace.store.selected_channel.as_str())
                        {
                            detail.push_str(" · selected");
                        }
                        if let Some(generation) = timeline.fresh_generation {
                            detail.push_str(&format!(" · fresh gen {generation}"));
                        }
                        if let Some(refresh) = &timeline.active_refresh {
                            detail.push_str(&format!(
                                " · request {} gen {} · barrier {}",
                                refresh.request_id,
                                refresh.generation,
                                refresh.barrier_revision
                            ));
                        }
                        if timeline.journal_revision > 0 || timeline.journal_entries > 0 {
                            detail.push_str(&format!(
                                " · journal rev {} / {} entries",
                                timeline.journal_revision,
                                timeline.journal_entries
                            ));
                        }
                        rows.row(&timeline.channel_id, Some(&detail), |_, _| {});
                    }
                });
            }
        }

        AppPane::Sessions => {
            section(ui, t, s.sessions);
            group(ui, t, |rows| {
                if data.store.devices.is_empty() {
                    rows.row(s.connecting, None, |_, _| {});
                }
                for device in &data.store.devices {
                    let when = device
                        .created_at
                        .map(|at| {
                            at.with_timezone(&chrono::Local)
                                .format("%d/%m/%Y %H:%M")
                                .to_string()
                        })
                        .unwrap_or_else(|| device.id.clone());
                    let expires = device
                        .expires_at
                        .map(|at| {
                            at.with_timezone(&chrono::Local)
                                .format("%d/%m %H:%M")
                                .to_string()
                        })
                        .unwrap_or_default();
                    rows.row(&when, Some(&expires), |ui, t| {
                        if row_button(ui, t, s.drop_session, Emphasis::Danger) {
                            actions.push(SettingsAction::Admin(AdminAction::DropConnection(
                                device.id.clone(),
                            )));
                        }
                    });
                }
            });
            if !data.store.devices.is_empty() {
                ui.add_space(space::SM);
                actions_row(ui, |ui| {
                    if row_button(ui, t, s.drop_all_sessions, Emphasis::Danger) {
                        actions.push(SettingsAction::Admin(AdminAction::DropConnection(
                            "ALL".to_owned(),
                        )));
                    }
                });
            }

            section(ui, t, s.menu_about);
            group(ui, t, |rows| {
                rows.row(s.version, Some(s.about_body), |ui, t| {
                    ui.label(
                        RichText::new(env!("CARGO_PKG_VERSION"))
                            .font(text::body())
                            .color(t.label_secondary),
                    );
                });
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Painéis do servidor
// ---------------------------------------------------------------------------

fn server_pane(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    data: &mut Context<'_>,
    t: &Tokens,
    s: &Strings,
    actions: &mut Vec<SettingsAction>,
) {
    use super::admin::AdminAction;
    use super::roles::RoleAction;
    use super::shell::ChatAction;

    match state.server_pane {
        ServerPane::General => {
            if !state.draft.loaded_server {
                state.draft.loaded_server = true;
                state.draft.server_name = data
                    .store
                    .server
                    .as_ref()
                    .map(|server| server.name.clone())
                    .unwrap_or_default();
                state.draft.server_public = true;
                state.draft.server_password.clear();
            }
            let draft = &mut state.draft;

            section(ui, t, s.pane_identity);
            group(ui, t, |rows| {
                rows.field(s.server_name, &mut draft.server_name, 64, false);
                rows.row(s.server_public, Some(s.server_public_hint), |ui, t| {
                    switch(ui, t, &mut draft.server_public);
                });
                if !draft.server_public {
                    rows.field(s.server_password, &mut draft.server_password, 64, true);
                }
            });

            ui.add_space(space::SM);
            let ready = !draft.server_name.trim().is_empty()
                && (draft.server_public || draft.server_password.chars().count() >= 8);
            actions_row(ui, |ui| {
                if ready && row_button(ui, t, s.save, Emphasis::Primary) {
                    actions.push(SettingsAction::Admin(AdminAction::SaveServer(Box::new(
                        crate::api::models::UpdateServerRequest {
                            name: draft.server_name.trim().to_owned(),
                            public: Some(draft.server_public),
                            password: (!draft.server_password.is_empty())
                                .then(|| draft.server_password.clone()),
                            ..Default::default()
                        },
                    ))));
                }
            });
        }

        ServerPane::Channels => {
            section(ui, t, s.text_channels);
            let channels = data.store.channels.clone();
            let draft = &mut state.draft;
            let mut close_editor = false;
            let mut cancel_delete = false;
            let mut confirmed_delete: Option<String> = None;

            if let Some(editor) = draft.channel.as_mut() {
                section(
                    ui,
                    t,
                    if editor.id.is_some() {
                        s.edit_channel_title
                    } else {
                        s.create_channel
                    },
                );
                group(ui, t, |rows| {
                    rows.field(s.channel_name, &mut editor.name, 32, false);

                    rows.row(s.channel_kind, None, |ui, t| {
                        if editor.id.is_some() {
                            let kind = match editor.kind.as_str() {
                                "voice" => s.channel_kind_voice,
                                "category" => s.channel_kind_category,
                                _ => s.channel_kind_text,
                            };
                            ui.label(
                                RichText::new(kind)
                                    .font(text::body())
                                    .color(t.label_secondary),
                            );
                        } else {
                            for (value, label) in [
                                ("text", s.channel_kind_text),
                                ("voice", s.channel_kind_voice),
                                ("category", s.channel_kind_category),
                            ] {
                                let selected = editor.kind == value;
                                if ui.add(egui::Button::selectable(selected, label)).clicked() {
                                    editor.kind = value.to_owned();
                                }
                            }
                        }
                    });

                    if editor.kind != "category" {
                        rows.field(s.channel_topic, &mut editor.topic, 512, false);
                    }
                });

                ui.add_space(space::SM);
                actions_row(ui, |ui| {
                    if row_button(ui, t, s.cancel, Emphasis::Quiet) {
                        close_editor = true;
                    }
                    let ready = !editor.name.trim().is_empty();
                    if ready
                        && row_button(
                            ui,
                            t,
                            if editor.id.is_some() {
                                s.save
                            } else {
                                s.create_channel
                            },
                            Emphasis::Primary,
                        )
                    {
                        let name = editor.name.trim().to_owned();
                        let topic = (editor.kind != "category")
                            .then(|| editor.topic.trim().to_owned());
                        match editor.id.clone() {
                            Some(channel_id) => {
                                actions.push(SettingsAction::Chat(ChatAction::UpdateChannel {
                                    channel_id,
                                    name,
                                    topic,
                                }));
                            }
                            None => {
                                actions.push(SettingsAction::Chat(ChatAction::CreateChannel {
                                    name,
                                    kind: editor.kind.clone(),
                                    topic: topic.filter(|topic| !topic.is_empty()),
                                }));
                            }
                        }
                        close_editor = true;
                    }
                });
                ui.add_space(space::LG);
            }

            group(ui, t, |rows| {
                if channels.is_empty() {
                    rows.row(s.no_channels_yet, None, |_, _| {});
                }
                for (index, channel) in channels.iter().enumerate() {
                    // Apagar pede o nome digitado: é o que separa "quis" de
                    // "esbarrou".
                    if let Some((id, expected, typed)) = draft.deleting.as_mut()
                        && id == &channel.id
                    {
                        rows.row(&channel.name, Some(s.delete_type_name), |ui, t| {
                            if row_button(ui, t, s.cancel, Emphasis::Quiet) {
                                cancel_delete = true;
                            }
                        });
                        let sent = rows.field(s.channel_name, typed, 32, false);
                        let matches = typed.trim() == expected.as_str();
                        rows.row("", None, |ui, t| {
                            if (row_button(ui, t, s.delete_forever, Emphasis::Danger) || sent)
                                && matches
                            {
                                confirmed_delete = Some(channel.id.clone());
                            }
                        });
                        continue;
                    }

                    let kind = match channel.kind {
                        crate::state::ChannelKind::Voice => s.channel_kind_voice,
                        crate::state::ChannelKind::Category => s.channel_kind_category,
                        crate::state::ChannelKind::Text => s.channel_kind_text,
                    };
                    rows.row(&channel.name, Some(kind), |ui, t| {
                        if row_button(ui, t, s.delete, Emphasis::Danger) {
                            draft.channel = None;
                            draft.deleting = Some((
                                channel.id.clone(),
                                channel.name.clone(),
                                String::new(),
                            ));
                        }
                        if row_button(ui, t, s.edit, Emphasis::Quiet) {
                            draft.deleting = None;
                            draft.channel = Some(ChannelDraft::edit(channel));
                        }
                        if index + 1 < channels.len()
                            && row_icon(ui, t, egui_phosphor::regular::ARROW_DOWN, s.move_down)
                        {
                            actions.push(SettingsAction::Chat(ChatAction::MoveChannel {
                                channel_id: channel.id.clone(),
                                old_position: channel.position,
                                new_position: channel.position + 1,
                            }));
                        }
                        if index > 0
                            && row_icon(ui, t, egui_phosphor::regular::ARROW_UP, s.move_up)
                        {
                            actions.push(SettingsAction::Chat(ChatAction::MoveChannel {
                                channel_id: channel.id.clone(),
                                old_position: channel.position,
                                new_position: channel.position - 1,
                            }));
                        }
                    });
                }
            });

            if close_editor {
                draft.channel = None;
            }
            if cancel_delete {
                draft.deleting = None;
            }
            if let Some(channel_id) = confirmed_delete {
                actions.push(SettingsAction::Chat(ChatAction::DeleteChannel(channel_id)));
                draft.deleting = None;
            }

            ui.add_space(space::SM);
            actions_row(ui, |ui| {
                if row_button(ui, t, s.create_channel, Emphasis::Primary) {
                    draft.deleting = None;
                    draft.channel = Some(ChannelDraft::create());
                }
            });
        }

        ServerPane::Roles => {
            let roles = data.store.roles.clone();
            section(ui, t, s.roles);
            group(ui, t, |rows| {
                if roles.is_empty() {
                    rows.row(s.no_roles, None, |_, _| {});
                }
                for role in &roles {
                    let selected = data
                        .roles
                        .draft
                        .as_ref()
                        .and_then(|draft| draft.id.as_deref())
                        == Some(role.id.as_str());
                    if rows.choice(&role.name, selected) {
                        data.roles.draft = Some(super::roles::Draft {
                            id: Some(role.id.clone()),
                            name: role.name.clone(),
                            color: role.color.clone().unwrap_or_default(),
                            permissions: role.permissions,
                        });
                    }
                }
            });
            ui.add_space(space::SM);
            actions_row(ui, |ui| {
                if row_button(ui, t, s.new_role, Emphasis::Quiet) {
                    data.roles.draft = Some(super::roles::Draft::default());
                }
            });

            let Some(draft) = data.roles.draft.as_mut() else {
                return;
            };
            section(ui, t, s.role_permissions);
            group(ui, t, |rows| {
                rows.field(s.role_name, &mut draft.name, 32, false);
                rows.row(s.role_color, None, |ui, _| {
                    let mut picked = crate::api::models::parse_hex_color(&draft.color)
                        .unwrap_or([128, 128, 128]);
                    if ui.color_edit_button_srgb(&mut picked).changed() {
                        draft.color =
                            format!("#{:02X}{:02X}{:02X}", picked[0], picked[1], picked[2]);
                    }
                });
                for (label, value) in [
                    (s.perm_send_attachment, &mut draft.permissions.send_attachment),
                    (s.perm_manage_channels, &mut draft.permissions.manage_channels),
                    (s.perm_pin_message, &mut draft.permissions.pin_message),
                    (s.perm_everyone_message, &mut draft.permissions.everyone_message),
                    (s.perm_ban_members, &mut draft.permissions.ban_members),
                    (s.perm_manage_roles, &mut draft.permissions.manage_roles),
                    (s.perm_manage_server, &mut draft.permissions.manage_server),
                ] {
                    rows.row(label, None, |ui, t| {
                        switch(ui, t, value);
                    });
                }
            });

            section(ui, t, s.role_members);
            if draft.id.is_none() {
                ui.label(
                    RichText::new(s.role_members_hint)
                        .font(text::footnote())
                        .color(t.label_tertiary),
                );
            }
            if let Some(role_id) = draft.id.clone() {
                group(ui, t, |rows| {
                    for member in &data.store.members {
                        let has = member.roles.iter().any(|id| id == &role_id);
                        if rows.choice(&member.name, has) {
                            actions.push(SettingsAction::Role(if has {
                                RoleAction::Unassign {
                                    user_id: member.id.clone(),
                                    role_id: role_id.clone(),
                                }
                            } else {
                                RoleAction::Assign {
                                    user_id: member.id.clone(),
                                    role_id: role_id.clone(),
                                }
                            }));
                        }
                    }
                });
            }

            ui.add_space(space::SM);
            let color = (!draft.color.trim().is_empty()).then(|| draft.color.trim().to_owned());
            actions_row(ui, |ui| {
                if !draft.name.trim().is_empty() && row_button(ui, t, s.save, Emphasis::Primary) {
                    actions.push(SettingsAction::Role(match draft.id.clone() {
                        Some(role_id) => RoleAction::Update {
                            role_id,
                            name: draft.name.trim().to_owned(),
                            color,
                            permissions: draft.permissions,
                        },
                        None => RoleAction::Create {
                            name: draft.name.trim().to_owned(),
                            color,
                            permissions: draft.permissions,
                        },
                    }));
                }
                if let Some(role_id) = draft.id.clone()
                    && row_button(ui, t, s.delete_role, Emphasis::Danger)
                {
                    actions.push(SettingsAction::Role(RoleAction::Delete(role_id)));
                }
            });
        }

        ServerPane::Emojis => {
            // Escolher o arquivo vem primeiro: dá para ver o que se está
            // nomeando, em vez de nomear no escuro e só então abrir o
            // seletor.
            if let Some(pending) = state.draft.pending_sticker.as_mut() {
                section(ui, t, s.sticker_new);
                let ready = !pending.name.trim().is_empty();
                let mut submit = false;
                let mut discard = false;
                group(ui, t, |rows| {
                    rows.preview(
                        s.sticker_preview,
                        pending.shrunk.then_some(s.sticker_shrunk),
                        data.media,
                        "pendente",
                        Some(pending.blob.as_str()),
                    );
                    submit = rows.field(s.emoji_name, &mut pending.name, 32, false) && ready;
                });
                ui.add_space(space::SM);
                actions_row(ui, |ui| {
                    if row_button(ui, t, s.save, Emphasis::Primary) && ready {
                        submit = true;
                    }
                    if row_button(ui, t, s.cancel, Emphasis::Quiet) {
                        discard = true;
                    }
                });
                if submit {
                    actions.push(SettingsAction::Admin(AdminAction::CreateSticker {
                        name: pending.name.trim().to_owned(),
                        blob: pending.blob.clone(),
                        format: pending.format.clone(),
                    }));
                    discard = true;
                }
                if discard {
                    state.draft.pending_sticker = None;
                }
            } else {
                section(ui, t, s.sticker_new);
                group(ui, t, |rows| {
                    if rows.action(s.add_emoji, Some(s.sticker_hint), false) {
                        actions.push(SettingsAction::Admin(AdminAction::PickSticker));
                    }
                });
            }

            section(ui, t, s.server_emojis);
            let stickers: Vec<(String, String, Option<String>)> = data
                .store
                .emojis
                .iter()
                .map(|emoji| (emoji.id.clone(), emoji.name.clone(), emoji.blob.clone()))
                .collect();
            group(ui, t, |rows| {
                if stickers.is_empty() {
                    rows.row(s.no_stickers, None, |_, _| {});
                }
                for (id, name, blob) in &stickers {
                    rows.preview_row(
                        &format!(":{name}:"),
                        data.media,
                        id,
                        blob.as_deref(),
                        |ui, t| {
                            if row_button(ui, t, s.delete, Emphasis::Danger) {
                                actions.push(SettingsAction::Admin(AdminAction::DeleteEmoji(
                                    id.clone(),
                                )));
                            }
                        },
                    );
                }
            });
            // O backend não tem como renomear: `/emojis` só aceita criar e
            // apagar. Dizer isso é melhor do que oferecer um campo que não
            // salvaria, ou apagar e recriar por baixo — o id mudaria e as
            // reações já feitas com ela iriam junto.
            ui.add_space(space::SM);
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                ui.label(
                    RichText::new(s.sticker_rename_note)
                        .font(text::footnote())
                        .color(t.label_tertiary),
                );
            });
        }

        ServerPane::Audit => {
            // O registro é pedido ao abrir o painel, não ao abrir a folha:
            // quem só queria renomear o servidor não precisa esperá-lo.
            if !state.draft.loaded_audit {
                state.draft.loaded_audit = true;
                actions.push(SettingsAction::Admin(AdminAction::LoadAuditLogs));
            }
            section(ui, t, s.audit_log);
            group(ui, t, |rows| {
                if data.store.audit_logs.is_empty() {
                    rows.row(s.connecting, None, |_, _| {});
                }
                for entry in data.store.audit_logs.iter().take(60) {
                    let when = entry
                        .created_at
                        .map(|at| {
                            at.with_timezone(&chrono::Local)
                                .format("%d/%m %H:%M")
                                .to_string()
                        })
                        .unwrap_or_default();
                    rows.row(
                        &entry.action,
                        Some(&format!("{when} · {}", entry.actor_username)),
                        |_, _| {},
                    );
                }
            });
        }
    }
}

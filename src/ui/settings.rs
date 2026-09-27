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

/// Avisos de um canal para esta conta. O cartão do servidor escolhe entre
/// eles; o servidor guarda `all`, `only_mentions` ou `off`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelNotifyMode {
    All,
    Mentions,
    Off,
}

impl ChannelNotifyMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "all" => Self::All,
            "off" => Self::Off,
            _ => Self::Mentions,
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "only_mentions",
            Self::Off => "off",
        }
    }
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

    /// Ícone do painel: o mesmo na coluna do desktop e na lista do celular.
    fn glyph(self) -> &'static str {
        use egui_phosphor::regular as icon;
        match self {
            Self::Account => icon::USER,
            Self::Appearance => icon::PALETTE,
            Self::Alerts => icon::BELL,
            Self::Files => icon::FOLDER,
            Self::Language => icon::GLOBE,
            Self::Sessions => icon::DEVICES,
            Self::Diagnostics => icon::PULSE,
        }
    }

    fn title(self, s: &Strings) -> &'static str {
        match self {
            Self::Account => s.pane_account,
            Self::Appearance => s.appearance,
            Self::Alerts => s.pane_alerts,
            Self::Files => s.pane_files,
            Self::Language => s.language,
            Self::Sessions => s.sessions,
            Self::Diagnostics => s.pane_diagnostics,
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

    fn glyph(self) -> &'static str {
        use egui_phosphor::regular as icon;
        match self {
            Self::General => icon::IDENTIFICATION_CARD,
            Self::Channels => icon::LIST_BULLETS,
            Self::Roles => icon::SHIELD,
            Self::Emojis => icon::SMILEY,
            Self::Audit => icon::CLOCK_COUNTER_CLOCKWISE,
        }
    }

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
    /// Há um modal por cima da folha (o editor de recorte): clique e Esc
    /// são dele, e não podem fechar a folha que ficou embaixo.
    pub modal_above: bool,
    /// A folha acabou de abrir por um clique fora dela (o lápis do cartão
    /// de perfil): esse mesmo clique não pode contar como "clique fora".
    pub opened_by_click: bool,
    /// No celular os painéis empilham: `false` é a lista, `true` é o painel
    /// aberto por cima dela.
    pub mobile_page: bool,
    /// Busca da lista do celular.
    pub mobile_query: String,
    /// Canal sendo arrastado na árvore de canais.
    pub drag_channel: Option<String>,
    /// A folha foi desenhada no formato do celular neste quadro (o voltar
    /// decide se sobe um nível ou fecha).
    pub compact: bool,
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
            modal_above: false,
            opened_by_click: false,
            mobile_page: false,
            mobile_query: String::new(),
            drag_channel: None,
            compact: false,
            app_pane: AppPane::Account,
            server_pane: ServerPane::General,
            draft: Draft::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChannelDraft {
    pub id: Option<String>,
    pub name: String,
    pub topic: String,
    /// `text`, `voice` ou `category`, como o contrato espera.
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct ChannelPermissionDraft {
    pub channel_id: String,
    pub role_id: String,
    pub role_name: String,
    pub permissions: crate::api::models::ChannelPermissions,
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
    /// Frase de digitação ("está digitando…" do seu jeito).
    pub typing: String,
    pub password: String,
    pub loaded_server: bool,
    pub server_name: String,
    pub server_public: bool,
    pub server_password: String,
    /// Criação/edição de canal acontece dentro da própria folha.
    pub channel: Option<ChannelDraft>,
    /// Override por cargo sendo editado no canal aberto.
    pub channel_permission: Option<ChannelPermissionDraft>,
    /// Canal a apagar: id, nome esperado e o que foi digitado. A confirmação
    /// é por cópia do nome — apagar um canal leva junto tudo que foi dito
    /// nele, e um clique só é barato demais para isso.
    pub deleting: Option<(String, String, String)>,
    pub loaded_audit: bool,
    pub audit: crate::ui::audit::Filter,
}

impl SettingsState {
    pub fn open_new_channel(&mut self) {
        self.open = Some(Surface::Server);
        self.server_pane = ServerPane::Channels;
        self.mobile_page = true;
        self.draft.channel = Some(ChannelDraft::create());
        self.draft.channel_permission = None;
        self.draft.deleting = None;
    }

    pub fn open_edit_channel(&mut self, channel: &crate::state::Channel) {
        self.open = Some(Surface::Server);
        self.server_pane = ServerPane::Channels;
        self.mobile_page = true;
        self.draft.channel = Some(ChannelDraft::edit(channel));
        self.draft.channel_permission = None;
        self.draft.deleting = None;
    }

    pub fn open_delete_channel(&mut self, channel: &crate::state::Channel) {
        self.open = Some(Surface::Server);
        self.server_pane = ServerPane::Channels;
        self.mobile_page = true;
        self.draft.channel = None;
        self.draft.channel_permission = None;
        self.draft.deleting = Some((
            channel.id.clone(),
            channel.name.clone(),
            String::new(),
        ));
    }

    /// Ajustes → Conta, com o formulário relido do estado: é para onde o
    /// "Editar perfil" do cartão leva.
    pub fn open_account(&mut self) {
        self.open = Some(Surface::App);
        self.app_pane = AppPane::Account;
        self.mobile_page = true;
        self.draft.loaded_account = false;
        self.opened_by_click = true;
    }

    /// Abre a primeira área administrativa que esta conta pode gerir.
    pub fn open_server_admin(&mut self, store: &Store) -> bool {
        let pane = if store.can_manage_server() {
            ServerPane::General
        } else if store.can_manage_channels() {
            ServerPane::Channels
        } else if store.can_manage_roles() {
            ServerPane::Roles
        } else {
            return false;
        };

        if self.open == Some(Surface::Server) {
            self.open = None;
            return false;
        }
        self.open = Some(Surface::Server);
        self.server_pane = pane;
        self.mobile_page = false;
        self.mobile_query.clear();
        self.draft.loaded_server = false;
        self.draft.loaded_audit = false;
        self.opened_by_click = true;
        true
    }

    /// Voltar do sistema com a folha aberta. No celular, com um painel
    /// aberto, volta para a lista; senão fecha.
    pub fn back(&mut self) {
        if self.compact && self.mobile_page {
            self.mobile_page = false;
        } else {
            self.open = None;
        }
    }

    /// Abre a folha na superfície pedida, ou fecha se ela já era a aberta.
    pub fn toggle(&mut self, surface: Surface) {
        if self.open == Some(surface) {
            self.open = None;
            return;
        }
        self.open = Some(surface);
        self.opened_by_click = true;
        self.mobile_page = false;
        self.mobile_query.clear();
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
/// Alvo de clique no desktop: 28×28 é o padrão do guia, 20×20 o mínimo.
const SWITCH_W: f32 = 38.0;
const SWITCH_H: f32 = 22.0;

/// Título de seção acima de um cartão.
/// Os painéis desenhados como peças (celular): cada linha é uma peça
/// arredondada dentro do grupo, em vez de linhas separadas por fio.
fn tiles(ui: &egui::Ui) -> bool {
    ui.ctx()
        .data(|data| data.get_temp::<bool>(egui::Id::new(TILES_KEY)))
        .unwrap_or(false)
}

const TILES_KEY: &str = "ajustes-em-pecas";
/// Espaço entre peças de um mesmo grupo, e o canto das pontas do grupo.
const TILE_GAP: f32 = 3.0;
const TILE_END: u8 = 16;
const TILE_MID: u8 = 4;
const TILE_INSET: f32 = space::LG + 2.0;

fn section(ui: &mut egui::Ui, t: &Tokens, label: &str) {
    let top = ui.cursor().min.y;
    let spot = Rect::from_min_size(egui::pos2(ui.max_rect().min.x, top), Vec2::new(ui.available_width(), 28.0));
    spotlight(ui, spot, label);
    if tiles(ui) {
        // Legenda de grupo no celular: na cor de destaque, como nos
        // ajustes do sistema, e em caixa normal.
        ui.add_space(space::LG);
        ui.horizontal(|ui| {
            ui.add_space(space::MD);
            ui.label(RichText::new(label).font(text::subheadline()).color(t.accent));
        });
        ui.add_space(space::XS);
        return;
    }
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
    let tiled = tiles(ui);
    // Em peças o espaço entre elas é o nosso (TILE_GAP), não o do egui.
    let spacing = ui.spacing().item_spacing.y;
    if tiled {
        ui.spacing_mut().item_spacing.y = 0.0;
    }
    let mut rows = Rows {
        ui,
        t: *t,
        separators: Vec::new(),
        first: true,
        lines: 1,
        tiled,
        pieces: Vec::new(),
        stack_next: false,
    };
    contents(&mut rows);
    let separators = std::mem::take(&mut rows.separators);
    let pieces = std::mem::take(&mut rows.pieces);
    let ui = rows.ui;

    if tiled {
        ui.spacing_mut().item_spacing.y = spacing;
        // Só agora se sabe qual peça é a última: os fundos foram reservados
        // antes do conteúdo e são preenchidos aqui, com os cantos certos.
        let last = pieces.len().saturating_sub(1);
        for (index, (slot, rect)) in pieces.into_iter().enumerate() {
            let top = if index == 0 { TILE_END } else { TILE_MID };
            let bottom = if index == last { TILE_END } else { TILE_MID };
            ui.painter().set(
                slot,
                egui::Shape::rect_filled(
                    rect,
                    CornerRadius { nw: top, ne: top, sw: bottom, se: bottom },
                    t.fill_soft,
                ),
            );
        }
        return;
    }

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
    /// Linhas visíveis do próximo campo; 1 é o campo de uma linha.
    lines: usize,
    /// Desenho em peças (celular).
    tiled: bool,
    /// Fundo reservado de cada peça e o retângulo dela.
    pieces: Vec<(egui::layers::ShapeIdx, Rect)>,
    /// A próxima linha empilha o controle embaixo do rótulo (campos).
    stack_next: bool,
}

impl Rows<'_> {
    /// Reserva a faixa de uma linha e devolve a área útil dela.
    fn band(&mut self, height: f32) -> (Rect, Rect) {
        let width = self.ui.available_width();
        if self.tiled {
            if !self.first {
                self.ui.add_space(TILE_GAP);
            }
            self.first = false;
            let height = height.max(52.0);
            let (rect, _) = self
                .ui
                .allocate_exact_size(Vec2::new(width, height), Sense::hover());
            let slot = self.ui.painter().add(egui::Shape::Noop);
            self.pieces.push((slot, rect));
            let inner = Rect::from_min_max(
                egui::pos2(rect.min.x + TILE_INSET, rect.min.y),
                egui::pos2(rect.max.x - TILE_INSET, rect.max.y),
            );
            return (rect, inner);
        }
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
        let inset = if self.tiled { TILE_INSET * 2.0 } else { ROW_INSET + space::MD };
        let full = (self.ui.available_width() - inset).max(120.0);
        // Em peças o controle fica à direita (chave, menu) e só os campos
        // descem para baixo do rótulo.
        let stacked = if self.tiled {
            std::mem::take(&mut self.stack_next) || full < 220.0
        } else {
            full < 430.0
        };
        let control_width = if stacked {
            full
        } else if self.tiled {
            (full * 0.42).min(200.0)
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
        let control_h = if self.lines > 1 { self.lines as f32 * 17.0 + space::SM * 2.0 } else { 30.0 };
        let height = if stacked {
            (space::MD + text_height + space::SM + control_h + space::MD).max(ROW_HEIGHT)
        } else {
            // Campo de várias linhas ao lado do rótulo: a linha cresce com ele.
            (text_height.max(if self.lines > 1 { control_h } else { 0.0 }) + space::MD * 2.0).max(ROW_HEIGHT)
        };
        let (band_rect, inner) = self.band(height);
        spotlight(self.ui, band_rect, label);

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
                egui::pos2(inner.min.x, inner.max.y - control_h - space::MD),
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
        // O controle mora dentro da faixa; o escopo dele deixaria o cursor
        // no pé do controle, acima do fim da faixa, e a linha seguinte
        // começaria por cima desta (no celular, as peças se sobrepunham).
        self.ui.advance_cursor_after_rect(band_rect);
    }

    /// Linha inteira clicável, para navegar ou disparar uma ação.
    fn action(&mut self, label: &str, hint: Option<&str>, danger: bool) -> bool {
        let inset = if self.tiled { TILE_INSET * 2.0 } else { ROW_INSET + space::MD };
        let width = (self.ui.available_width() - inset).max(120.0);
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
        spotlight(self.ui, rect, label);
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
        // O controle mora dentro da faixa; o escopo dele deixaria o cursor
        // no pé do controle, acima do fim da faixa, e a linha seguinte
        // começaria por cima desta (no celular, as peças se sobrepunham).
        self.ui.advance_cursor_after_rect(rect);
    }

    /// Linha com um campo de texto ocupando a direita.
    fn field(&mut self, label: &str, value: &mut String, limit: usize, secret: bool) -> bool {
        self.field_hinted(label, None, value, limit, secret)
    }

    /// Campo de várias linhas, para texto que quebra linha (a descrição).
    /// No Android o campo nativo segue de uma linha, por enquanto.
    fn field_lines(&mut self, label: &str, value: &mut String, limit: usize, lines: usize) {
        self.lines = lines;
        self.field_hinted(label, None, value, limit, false);
        self.lines = 1;
    }

    fn field_hinted(
        &mut self,
        label: &str,
        hint: Option<&str>,
        value: &mut String,
        limit: usize,
        secret: bool,
    ) -> bool {
        let mut submitted = false;
        #[cfg_attr(target_os = "android", allow(unused_variables))]
        let lines = self.lines;
        // Contador: sempre nos campos longos (recado, bio, frase) quando têm
        // texto; nos curtos, só perto do limite. Vai na linha de apoio.
        let used = value.chars().count();
        let counter = (!secret && used > 0 && (limit >= 64 || used * 5 >= limit * 4))
            .then(|| format!("{used}/{limit}"));
        let joined;
        let hint = match (hint, counter) {
            (Some(hint), Some(counter)) => {
                joined = format!("{hint} · {counter}");
                Some(joined.as_str())
            }
            (None, Some(counter)) => {
                joined = counter;
                Some(joined.as_str())
            }
            (hint, None) => hint,
        };
        // Campo em peça: largura inteira, embaixo do rótulo.
        if self.tiled {
            self.stack_next = true;
        }
        self.row(label, hint, |ui, _t| {
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
                let editor = if lines > 1 {
                    egui::TextEdit::multiline(value).desired_rows(lines)
                } else {
                    egui::TextEdit::singleline(value)
                };
                let height = if lines > 1 { lines as f32 * 17.0 + space::SM * 2.0 } else { 26.0 };
                let editor = editor
                    .id(edit_id)
                    .char_limit(limit)
                    .password(secret)
                    .font(text::body())
                    .margin(egui::Margin::symmetric(space::MD as i8, space::XS as i8));
                // Várias linhas: altura fixa e rolagem por dentro. Sem isso o
                // campo crescia com o texto (512 caracteres de bio) e cobria
                // o resto do painel.
                let response = if lines > 1 {
                    egui::ScrollArea::vertical()
                        .id_salt(edit_id.with("rolagem"))
                        .max_height(height)
                        .show(ui, |ui| ui.add_sized(Vec2::new(width, height), editor))
                        .inner
                } else {
                    ui.add_sized(Vec2::new(width, height), editor)
                };
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
/// Abas (Permissões / Membros / Aparência): sempre lado a lado, mesmo
/// com rótulos longos — navegar não é escolher um valor.
fn tabs<T: PartialEq + Copy>(ui: &mut egui::Ui, t: &Tokens, current: &mut T, options: &[(T, &str)]) -> bool {
    segmented_inner(ui, t, current, options, true)
}

fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    t: &Tokens,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    segmented_inner(ui, t, current, options, false)
}

fn segmented_inner<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    t: &Tokens,
    current: &mut T,
    options: &[(T, &str)],
    always: bool,
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
    // Sem espaço para as opções lado a lado (celular, painel estreito),
    // vira menu suspenso: o valor atual e a seta, e a lista abre por cima.
    // No celular (peças), escolha é sempre menu suspenso: o rótulo fica
    // com a linha, e o menu abre por cima do valor tocado.
    // Segmentado só para poucas opções curtas (Tema, presença); o resto é
    // menu suspenso, também no desktop — é o que não transborda.
    let short = options.len() <= 3 && options.iter().all(|(_, label)| label.chars().count() <= 10);
    if !always && (tiles(ui) || !short || total > ui.available_width() + 0.5) {
        return dropdown_choice(ui, t, current, options);
    }
    // O id vem do próprio controle, não do rótulo: numa lista de canais
    // cada linha tem os mesmos rótulos, e `ui.id().with(label)` repetia o
    // mesmo id em todas — o aviso vermelho de colisão do egui.
    let (rect, base) = ui.allocate_exact_size(Vec2::new(total, height), Sense::hover());
    let base = base.id;
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);

    let mut x = rect.min.x;
    for ((value, label), width) in options.iter().zip(widths) {
        let slot = Rect::from_min_size(egui::pos2(x, rect.min.y), Vec2::new(width, height));
        x += width;
        let selected = *current == *value;
        let response = ui.interact(slot, base.with(*label), Sense::click());
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

/// Escolha em menu suspenso: o valor atual com a seta; tocar abre as
/// opções por cima dele.
fn dropdown_choice<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    t: &Tokens,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let label = options
        .iter()
        .find(|(value, _)| *value == *current)
        .map(|(_, label)| *label)
        .unwrap_or("");
    let galley = ui.painter().layout_no_wrap(label.to_owned(), text::callout(), t.label);
    let width = (galley.size().x + space::MD * 2.0 + 14.0).min(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(radius::FIELD),
        if response.hovered() { t.fill_medium } else { t.fill_soft },
    );
    ui.painter()
        .with_clip_rect(rect.shrink2(Vec2::new(space::MD, 0.0)))
        .galley(egui::pos2(rect.min.x + space::MD, rect.center().y - galley.size().y / 2.0), galley, t.label);
    ui.painter().text(
        egui::pos2(rect.max.x - space::MD, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        egui_phosphor::regular::CARET_DOWN,
        text::icon(11.0),
        t.label_secondary,
    );
    let mut changed = false;
    crate::ui::widgets::dropdown(&response, rect, rect, true).show(|ui| {
        for (value, option) in options {
            if crate::ui::widgets::menu_option(ui, t, option, *value == *current) {
                if *value != *current {
                    *current = *value;
                    changed = true;
                }
                ui.close();
            }
        }
    });
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

const SHEET_W: f32 = 720.0;
/// A altura acompanha a janela (80%) entre estes limites: Conta ficou
/// grande demais para 440, e uma folha enorme numa tela alta vira janela.
const SHEET_H_MIN: f32 = 440.0;
const SHEET_H_MAX: f32 = 640.0;
const RAIL_W: f32 = 176.0;
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
    #[cfg(any(target_os = "windows", target_os = "android"))]
    CheckUpdates,
}

/// Tudo que a folha precisa do resto do programa.
pub struct Context<'a> {
    pub store: &'a Store,
    /// Categorias recolhidas, as mesmas da coluna de canais.
    pub collapsed: &'a mut std::collections::HashSet<String>,
    /// Endpoint local deste workspace; não é uma propriedade administrativa.
    pub server_url: &'a str,
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
    pub self_card: &'a mut crate::ui::profile::SelfCardStyle,
    pub webembed_offscreen: &'a mut crate::webembed::OffscreenBehavior,
    pub webembed_scope: &'a mut crate::webembed::FloatScope,
    pub ask_download: &'a mut bool,
    pub download_dir: Option<String>,
    pub diagnostics: &'a [WorkspaceDiagnostics],
    pub preview: papo_core::preview::PreviewStatsSnapshot,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    pub update_status: Option<&'a str>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    pub update_enabled: bool,
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
    // Os ajustes do servidor são só administração: sem nenhuma área
    // permitida (o cargo mudou com a folha aberta), a folha fecha.
    if surface == Surface::Server && !server_pane_allowed(data.store, state.server_pane) {
        match ServerPane::ALL
            .into_iter()
            .find(|pane| server_pane_allowed(data.store, *pane))
        {
            Some(pane) => state.server_pane = pane,
            None => {
                state.open = None;
                return actions;
            }
        }
    }

    // No celular a folha vira tela cheia com pilha de navegação: a lista
    // de áreas, e o painel por cima dela.
    state.compact = screen.width() < crate::ui::shell::COMPACT_BREAKPOINT;
    if state.compact {
        mobile(ctx, state, data, t, s, surface, screen, &mut actions);
        return actions;
    }

    // Sobe da pastilha de baixo, desce da de cima; e nunca passa da janela.
    // A altura é fixa, como a largura: uma folha que encolhe e cresce a cada
    // painel faz a coluna da esquerda dançar debaixo do ponteiro, e o guia
    // pede justamente o contrário — uma tela de ajustes estável, para que se
    // aprenda onde as coisas ficam.
    let width = SHEET_W.min((screen.width() - space::MD * 2.0).max(280.0));
    let height = (screen.height() * 0.8)
        .clamp(SHEET_H_MIN, SHEET_H_MAX)
        .min((screen.height() - space::MD * 2.0).max(260.0));
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
            rail(ui, state, data.store, t, s, rail_rect, surface);
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
                    // Um rolamento por painel: trocar de painel começa do
                    // topo (com a prévia), não onde o anterior parou.
                    egui::ScrollArea::vertical()
                        .id_salt(("corpo-do-painel", title))
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
    let just_opened = std::mem::take(&mut state.opened_by_click);
    let outside = !state.modal_above && !just_opened && ctx.input(|input| {
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

// ---------------------------------------------------------------------------
// Cargos
// ---------------------------------------------------------------------------

/// Cargos: a lista à esquerda (com quantos têm cada um) e o cargo aberto à
/// direita, em abas — Permissões, Membros, Aparência. No celular, empilhado.
fn roles_pane(ui: &mut egui::Ui, data: &mut Context<'_>, t: &Tokens, s: &Strings, actions: &mut Vec<SettingsAction>) {
    let wide = !tiles(ui) && ui.available_width() >= 460.0;
    if wide {
        ui.add_space(space::MD);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                role_list(ui, data, t, s);
            });
            ui.add_space(space::LG);
            ui.vertical(|ui| role_detail(ui, data, t, s, actions));
        });
    } else {
        role_list(ui, data, t, s);
        role_detail(ui, data, t, s, actions);
    }
}

fn role_list(ui: &mut egui::Ui, data: &mut Context<'_>, t: &Tokens, s: &Strings) {
    use super::roles::{Draft, Tab};
    let roles = data.store.roles.clone();
    section(ui, t, s.roles);
    if roles.is_empty() {
        ui.add(egui::Label::new(RichText::new(s.no_roles).font(text::footnote()).color(t.label_tertiary)).wrap());
    }
    let selected_id = data.roles.draft.as_ref().and_then(|draft| draft.id.clone());
    for role in &roles {
        let count = data.store.members.iter().filter(|m| m.roles.iter().any(|id| id == &role.id)).count();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
        let selected = selected_id.as_deref() == Some(role.id.as_str());
        if selected || response.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(radius::FIELD), if selected { t.fill_medium } else { t.fill_soft });
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let color = role
            .color
            .as_deref()
            .and_then(crate::api::models::parse_hex_color)
            .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
            .unwrap_or(t.label_tertiary);
        ui.painter().circle_filled(egui::pos2(rect.min.x + space::MD + 5.0, rect.center().y), 5.0, color);
        let count_text = count.to_string();
        let count_w = ui.painter().layout_no_wrap(count_text.clone(), text::footnote(), t.label_tertiary).size().x;
        ui.painter().text(egui::pos2(rect.max.x - space::MD, rect.center().y), egui::Align2::RIGHT_CENTER, count_text, text::footnote(), t.label_tertiary);
        let left = rect.min.x + space::MD + 18.0;
        crate::ui::widgets::text_fit(
            ui.painter(),
            egui::pos2(left, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &role.name,
            text::body(),
            t.label,
            rect.max.x - space::MD - count_w - space::SM - left,
        );
        if response.clicked() {
            data.roles.draft = Some(Draft {
                id: Some(role.id.clone()),
                name: role.name.clone(),
                color: role.color.clone().unwrap_or_default(),
                permissions: role.permissions,
            });
            data.roles.member_query.clear();
        }
    }
    ui.add_space(space::SM);
    if row_button(ui, t, s.new_role, Emphasis::Quiet) {
        // Cargo novo começa pelo nome e pela cor.
        data.roles.draft = Some(Draft::default());
        data.roles.tab = Tab::Look;
    }
}

fn role_detail(ui: &mut egui::Ui, data: &mut Context<'_>, t: &Tokens, s: &Strings, actions: &mut Vec<SettingsAction>) {
    use super::roles::{RoleAction, Tab};
    let Some(draft) = data.roles.draft.as_mut() else {
        ui.add_space(space::LG);
        ui.label(RichText::new(s.role_pick_hint).font(text::body()).color(t.label_tertiary));
        return;
    };
    let members = data
        .store
        .members
        .iter()
        .filter(|m| draft.id.as_deref().is_some_and(|id| m.roles.iter().any(|r| r == id)))
        .count();
    let members_label = format!("{} · {members}", s.role_tab_members);
    ui.add_space(space::SM);
    let mut tab = data.roles.tab;
    if tabs(
        ui,
        t,
        &mut tab,
        &[(Tab::Permissions, s.role_tab_permissions), (Tab::Members, &members_label), (Tab::Look, s.role_tab_look)],
    ) {
        data.roles.tab = tab;
    }

    match data.roles.tab {
        Tab::Permissions => {
            group(ui, t, |rows| {
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
        }
        Tab::Look => {
            group(ui, t, |rows| {
                rows.field(s.role_name, &mut draft.name, 32, false);
                rows.row(s.role_color, None, |ui, _| {
                    let mut picked = crate::api::models::parse_hex_color(&draft.color).unwrap_or([128, 128, 128]);
                    if ui.color_edit_button_srgb(&mut picked).changed() {
                        draft.color = format!("#{:02X}{:02X}{:02X}", picked[0], picked[1], picked[2]);
                    }
                });
            });
        }
        Tab::Members => {
            let Some(role_id) = draft.id.clone() else {
                ui.add_space(space::MD);
                ui.label(RichText::new(s.role_members_hint).font(text::footnote()).color(t.label_tertiary));
                return;
            };
            ui.add_space(space::MD);
            // Busca por apelido, @usuário ou ID.
            let (field, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
            ui.painter().rect_filled(field, CornerRadius::same(radius::FIELD), t.fill_soft);
            ui.painter().text(
                egui::pos2(field.min.x + space::MD + 6.0, field.center().y),
                egui::Align2::CENTER_CENTER,
                egui_phosphor::regular::MAGNIFYING_GLASS,
                text::icon(12.0),
                t.label_tertiary,
            );
            let inner = Rect::from_min_max(egui::pos2(field.min.x + space::MD + 18.0, field.min.y), egui::pos2(field.max.x - space::SM, field.max.y));
            let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
            child.add_sized(
                inner.size(),
                egui::TextEdit::singleline(&mut data.roles.member_query)
                    .hint_text(s.role_member_search)
                    .frame(egui::Frame::NONE)
                    .font(text::callout())
                    .vertical_align(Align::Center),
            );
            ui.add_space(space::SM);

            let query = data.roles.member_query.trim().trim_start_matches('@').to_lowercase();
            let role_colors: std::collections::HashMap<String, egui::Color32> = data
                .store
                .roles
                .iter()
                .filter_map(|role| {
                    let color = role.color.as_deref().and_then(crate::api::models::parse_hex_color)?;
                    Some((role.id.clone(), egui::Color32::from_rgb(color[0], color[1], color[2])))
                })
                .collect();
            let mut shown = 0;
            for member in &data.store.members {
                let matches = query.is_empty()
                    || member.name.to_lowercase().contains(&query)
                    || member.username.to_lowercase().contains(&query)
                    || member.id.to_lowercase().starts_with(&query);
                if !matches {
                    continue;
                }
                shown += 1;
                let has = member.roles.iter().any(|id| id == &role_id);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
                let face = Rect::from_center_size(egui::pos2(rect.min.x + space::MD + 13.0, rect.center().y), Vec2::splat(26.0));
                let tint = member.role_color.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)).unwrap_or(t.accent);
                ui.painter().circle_filled(face.center(), 13.0, tint.gamma_multiply(0.30));
                ui.painter().text(face.center(), egui::Align2::CENTER_CENTER, member.initials(), egui::FontId::new(10.0, egui::FontFamily::Name("semibold".into())), tint);
                // Chave à direita; pontinhos dos outros cargos antes dela.
                let toggle = Rect::from_min_size(egui::pos2(rect.max.x - space::SM - SWITCH_W, rect.center().y - SWITCH_H / 2.0), Vec2::new(SWITCH_W, SWITCH_H));
                let mut dots_x = toggle.min.x - space::MD;
                for other in member.roles.iter().filter(|id| *id != &role_id).take(4) {
                    if let Some(color) = role_colors.get(other) {
                        ui.painter().circle_filled(egui::pos2(dots_x - 4.0, rect.center().y), 4.0, *color);
                        dots_x -= 11.0;
                    }
                }
                let text_left = face.max.x + space::MD;
                let text_right = dots_x - space::SM;
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(text_left, rect.center().y - 7.0), egui::Align2::LEFT_CENTER, &member.name, text::body(), t.label, text_right - text_left);
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(text_left, rect.center().y + 8.0), egui::Align2::LEFT_CENTER, &format!("@{}", member.username), text::footnote(), t.label_tertiary, text_right - text_left);
                let mut on = has;
                let mut slot = ui.new_child(UiBuilder::new().max_rect(toggle));
                if switch(&mut slot, t, &mut on) {
                    actions.push(SettingsAction::Role(if has {
                        RoleAction::Unassign { user_id: member.id.clone(), role_id: role_id.clone() }
                    } else {
                        RoleAction::Assign { user_id: member.id.clone(), role_id: role_id.clone() }
                    }));
                }
            }
            if shown == 0 {
                ui.label(RichText::new(s.no_results).font(text::body()).color(t.label_secondary));
            }
            ui.add_space(space::SM);
            ui.add(egui::Label::new(RichText::new(s.role_members_toggle_hint).font(text::footnote()).color(t.label_tertiary)).wrap());
            return;
        }
    }

    // Salvar e apagar valem para as abas que editam o cargo.
    ui.add_space(space::SM);
    let color = (!draft.color.trim().is_empty()).then(|| draft.color.trim().to_owned());
    actions_row(ui, |ui| {
        if !draft.name.trim().is_empty() && row_button(ui, t, s.save, Emphasis::Primary) {
            actions.push(SettingsAction::Role(match draft.id.clone() {
                Some(role_id) => RoleAction::Update { role_id, name: draft.name.trim().to_owned(), color, permissions: draft.permissions },
                None => RoleAction::Create { name: draft.name.trim().to_owned(), color, permissions: draft.permissions },
            }));
        }
        if let Some(role_id) = draft.id.clone()
            && row_button(ui, t, s.delete_role, Emphasis::Danger)
        {
            actions.push(SettingsAction::Role(RoleAction::Delete(role_id)));
        }
    });
}

// ---------------------------------------------------------------------------
// Árvore de canais (arrastar e soltar)
// ---------------------------------------------------------------------------

enum TreeAction {
    Edit(String),
    Delete(String, String),
    Move {
        channel_id: String,
        old_position: i32,
        new_position: i32,
        parent_id: Option<String>,
    },
}

const TREE_ROW: f32 = 34.0;
const TREE_INDENT: f32 = 20.0;

/// Para onde um arrasto leva: antes de qual item visível (ou para o fim) e
/// em qual categoria. Função pura para dar para testar sem janela.
///
/// `items` é a lista visível: (id, posição, é categoria, categoria-mãe,
/// recolhida). `gap` é a fresta entre `items[gap-1]` e `items[gap]`.
fn tree_drop(
    items: &[(String, i32, bool, Option<String>, bool)],
    dragged: usize,
    mut gap: usize,
    total: i32,
) -> (i32, Option<String>) {
    let (_, old, is_category, _, _) = &items[dragged];
    // Categoria não entra em categoria: dentro da lista de outra, ela vai
    // para antes daquela categoria.
    if *is_category {
        while gap < items.len() && items[gap].3.is_some() && gap > 0 && items[gap - 1].3.is_some() {
            gap -= 1;
        }
        if gap < items.len() && items[gap].3.is_some() {
            // Primeiro filho de uma categoria: antes do cabeçalho dela.
            while gap > 0 && !items[gap - 1].2 {
                gap -= 1;
            }
            gap = gap.saturating_sub(1);
        }
    }
    let parent = if *is_category || gap == 0 {
        None
    } else {
        let above = &items[gap - 1];
        match (above.2, above.4) {
            // Logo abaixo de uma categoria aberta: entra nela.
            (true, false) => Some(above.0.clone()),
            (true, true) => None,
            _ => above.3.clone(),
        }
    };
    let new = match items.get(gap) {
        Some((_, before, ..)) if *old < *before => before - 1,
        Some((_, before, ..)) => *before,
        None => total,
    };
    (new.max(1), parent)
}

/// Categorias e canais com alça de arrastar; editar e apagar aparecem ao
/// passar o ponteiro. Soltar reordena (e troca de categoria) num pedido só.
fn channel_tree(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    store: &Store,
    collapsed: &mut std::collections::HashSet<String>,
    dragging: &mut Option<String>,
) -> Option<TreeAction> {
    use egui_phosphor::regular as icon;
    let layout = store.channel_layout();
    let total = store.channels.len() as i32;
    // Visíveis: os filhos de categoria recolhida somem.
    let visible: Vec<(&crate::state::Channel, Option<String>)> = layout
        .iter()
        .map(|(index, parent)| (&store.channels[*index], parent.clone()))
        .filter(|(_, parent)| parent.as_ref().is_none_or(|p| !collapsed.contains(p)))
        .collect();
    let items: Vec<(String, i32, bool, Option<String>, bool)> = visible
        .iter()
        .map(|(c, parent)| {
            let is_category = c.kind == crate::state::ChannelKind::Category;
            (c.id.clone(), c.position, is_category, parent.clone(), is_category && collapsed.contains(&c.id))
        })
        .collect();

    let mut action = None;
    let mut rects = Vec::with_capacity(visible.len());
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 2.0);
    for (index, (channel, parent)) in visible.iter().enumerate() {
        let is_category = items[index].2;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TREE_ROW), Sense::click());
        rects.push(rect);
        let being_dragged = dragging.as_deref() == Some(channel.id.as_str());
        let hovered = response.hovered() || being_dragged;
        if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(radius::FIELD), t.fill_soft);
        }
        let alpha = if being_dragged { 0.45 } else { 1.0 };
        let indent = if parent.is_some() { TREE_INDENT } else { 0.0 };

        // Alça: é por ela que se arrasta, para o clique no resto da linha
        // continuar sendo clique.
        let grip = Rect::from_min_size(egui::pos2(rect.min.x + indent, rect.min.y), Vec2::new(22.0, TREE_ROW));
        let grip_response = ui.interact(grip, egui::Id::new(("alca-canal", &channel.id)), Sense::drag());
        if grip_response.hovered() || grip_response.dragged() {
            ui.ctx().set_cursor_icon(if grip_response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        if grip_response.drag_started() {
            *dragging = Some(channel.id.clone());
        }
        ui.painter().text(
            grip.center(),
            egui::Align2::CENTER_CENTER,
            icon::DOTS_SIX_VERTICAL,
            text::icon(13.0),
            if hovered { t.label_secondary } else { t.label_tertiary }.gamma_multiply(alpha),
        );

        let mut x = grip.max.x + space::XS;
        if is_category {
            let folded = collapsed.contains(&channel.id);
            ui.painter().text(
                egui::pos2(x + 5.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                if folded { icon::CARET_RIGHT } else { icon::CARET_DOWN },
                text::icon(10.0),
                t.label_secondary,
            );
            if response.clicked() {
                if folded {
                    collapsed.remove(&channel.id);
                } else {
                    collapsed.insert(channel.id.clone());
                }
            }
            x += 16.0;
        } else {
            let glyph = if channel.kind == crate::state::ChannelKind::Voice { icon::SPEAKER_HIGH } else { icon::HASH };
            ui.painter().text(egui::pos2(x + 6.0, rect.center().y), egui::Align2::CENTER_CENTER, glyph, text::icon(13.0), t.label_tertiary.gamma_multiply(alpha));
            x += 18.0;
        }
        let tools_w = if hovered { 60.0 } else { 0.0 };
        let name = if is_category { channel.name.to_uppercase() } else { channel.name.clone() };
        let name_rect = crate::ui::widgets::text_fit(
            ui.painter(),
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &name,
            if is_category { text::subheadline() } else { text::body() },
            if is_category { t.label_secondary } else { t.label }.gamma_multiply(alpha),
            rect.max.x - tools_w - space::MD - x,
        );
        if let Some(topic) = channel.topic.as_deref().filter(|topic| !topic.is_empty() && !is_category) {
            let left = name_rect.max.x + space::SM;
            let room = rect.max.x - tools_w - space::MD - left;
            if room > 40.0 {
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, rect.center().y), egui::Align2::LEFT_CENTER, topic, text::footnote(), t.label_tertiary, room);
            }
        }

        // Editar e apagar só aparecem na linha sob o ponteiro.
        if hovered && dragging.is_none() {
            let mut right = rect.max.x - space::SM;
            for (glyph, danger, which) in [(icon::TRASH, true, 1), (icon::PENCIL_SIMPLE, false, 0)] {
                let tool = Rect::from_min_size(egui::pos2(right - 24.0, rect.center().y - 12.0), Vec2::splat(24.0));
                right -= 28.0;
                let tool_response = ui.interact(tool, egui::Id::new(("ferramenta-canal", &channel.id, which)), Sense::click());
                if tool_response.hovered() {
                    ui.painter().rect_filled(tool, CornerRadius::same(radius::CONTROL), t.fill_medium);
                }
                ui.painter().text(tool.center(), egui::Align2::CENTER_CENTER, glyph, text::icon(13.0), if danger { t.danger } else { t.label_secondary });
                if tool_response.clicked() {
                    action = Some(if which == 1 {
                        TreeAction::Delete(channel.id.clone(), channel.name.clone())
                    } else {
                        TreeAction::Edit(channel.id.clone())
                    });
                }
            }
        }
    }
    ui.spacing_mut().item_spacing.y = spacing;

    // Durante o arrasto: a fresta mais perto do ponteiro vira uma linha de
    // destaque; ao soltar, o pedido sai.
    if let Some(id) = dragging.clone() {
        let Some(dragged) = items.iter().position(|item| item.0 == id) else {
            *dragging = None;
            return action;
        };
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let released = ui.input(|input| !input.pointer.any_down());
        if let Some(pointer) = pointer {
            let gap = rects.iter().position(|rect| pointer.y < rect.center().y).unwrap_or(rects.len());
            let (_, parent) = tree_drop(&items, dragged, gap, total);
            let y = match rects.get(gap) {
                Some(rect) => rect.min.y - 1.0,
                None => rects.last().map(|rect| rect.max.y + 1.0).unwrap_or(0.0),
            };
            let left = rects.first().map(|rect| rect.min.x).unwrap_or(0.0) + if parent.is_some() { TREE_INDENT } else { 0.0 };
            let right = rects.first().map(|rect| rect.max.x).unwrap_or(0.0);
            ui.painter().line_segment([egui::pos2(left + space::SM, y), egui::pos2(right - space::SM, y)], Stroke::new(2.0, t.accent));
            ui.ctx().request_repaint();
            if released {
                let (new_position, parent) = tree_drop(&items, dragged, gap, total);
                let (_, old_position, _, old_parent, _) = &items[dragged];
                let parent_change = (parent != *old_parent).then(|| parent.clone().unwrap_or_default());
                if new_position != *old_position || parent_change.is_some() {
                    action = Some(TreeAction::Move {
                        channel_id: id,
                        old_position: *old_position,
                        new_position,
                        parent_id: parent_change,
                    });
                }
                *dragging = None;
            }
        } else if released {
            *dragging = None;
        }
    }
    let _ = s;
    action
}

// ---------------------------------------------------------------------------
// Busca por ajuste
// ---------------------------------------------------------------------------

/// Cada ajuste que a busca encontra, com o painel onde ele mora. O rótulo é
/// o mesmo da linha, que é como o destaque a reconhece ao chegar lá.
fn app_settings_index(s: &Strings) -> Vec<(AppPane, &'static str)> {
    vec![
        (AppPane::Account, s.nickname),
        (AppPane::Account, s.status_message),
        (AppPane::Account, s.description),
        (AppPane::Account, s.typing_phrase),
        (AppPane::Account, s.presence),
        (AppPane::Account, s.new_password),
        (AppPane::Account, s.change_password),
        (AppPane::Account, s.sign_out),
        (AppPane::Appearance, s.theme),
        (AppPane::Appearance, s.translucency),
        (AppPane::Appearance, s.topic_reveal),
        (AppPane::Appearance, s.record_button),
        (AppPane::Appearance, s.webembed_offscreen),
        (AppPane::Appearance, s.webembed_scope),
        (AppPane::Appearance, s.self_card),
        (AppPane::Alerts, s.menu_notifications),
        (AppPane::Alerts, s.reply_notifications_default),
        (AppPane::Alerts, s.badge),
        (AppPane::Alerts, s.menu_close_to_tray),
        (AppPane::Alerts, s.start_at_login),
        (AppPane::Files, s.downloads),
        (AppPane::Files, s.download_folder),
        (AppPane::Sessions, s.version),
    ]
}

fn server_settings_index(s: &Strings) -> Vec<(ServerPane, &'static str)> {
    vec![
        (ServerPane::General, s.server_name),
        (ServerPane::General, s.server_icon),
        (ServerPane::General, s.server_public),
        (ServerPane::General, s.server_password),
        (ServerPane::Channels, s.channel_name),
        (ServerPane::Channels, s.channel_topic),
        (ServerPane::Channels, s.channel_permissions),
        (ServerPane::Roles, s.role_permissions),
        (ServerPane::Roles, s.role_color),
        (ServerPane::Roles, s.role_members),
        (ServerPane::Emojis, s.add_emoji),
        (ServerPane::Emojis, s.server_emojis),
        (ServerPane::Audit, s.audit_log),
    ]
}

const FOCUS_KEY: &str = "ajustes-foco";
const FOCUS_SECONDS: f64 = 1.6;

/// Pede que a linha com este rótulo apareça e pisque ao abrir o painel.
fn focus_setting(ctx: &egui::Context, label: &str) {
    let now = ctx.input(|input| input.time);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(FOCUS_KEY), (label.to_owned(), now)));
}

/// Chamado por cada linha: se é a procurada, rola até ela e acende por um
/// instante, apagando aos poucos.
fn spotlight(ui: &egui::Ui, rect: Rect, label: &str) {
    let Some((wanted, since)) = ui.ctx().data(|data| data.get_temp::<(String, f64)>(egui::Id::new(FOCUS_KEY))) else {
        return;
    };
    if label.is_empty() || wanted != label {
        return;
    }
    let elapsed = ui.input(|input| input.time) - since;
    if elapsed > FOCUS_SECONDS {
        ui.ctx().data_mut(|data| data.remove::<(String, f64)>(egui::Id::new(FOCUS_KEY)));
        return;
    }
    if elapsed < 0.3 {
        ui.scroll_to_rect(rect, Some(Align::Center));
    }
    let fade = (1.0 - elapsed / FOCUS_SECONDS) as f32;
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(radius::FIELD),
        ui.visuals().selection.bg_fill.gamma_multiply(0.6 * fade),
    );
    ui.ctx().request_repaint();
}

// ---------------------------------------------------------------------------
// Prévias no topo dos painéis
// ---------------------------------------------------------------------------

/// Faixa de prévia no topo de um painel: fundo suave, cantos de cartão. O
/// conteúdo é desenhado por `draw` dentro da área útil.
fn hero(ui: &mut egui::Ui, t: &Tokens, height: f32, draw: impl FnOnce(&mut egui::Ui, Rect)) {
    ui.add_space(space::MD);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height + space::LG * 2.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(radius::SHEET), t.fill_soft);
    draw(ui, rect.shrink(space::LG));
}

/// Texto de apoio à direita da prévia, quando sobra espaço.
fn hint_beside(ui: &egui::Ui, t: &Tokens, area: Rect, used: Rect, hint: &str) {
    let left = used.max.x + space::XL;
    let width = area.max.x - left;
    if width < 140.0 {
        return;
    }
    let galley = ui.painter().layout(hint.to_owned(), text::footnote(), t.label_tertiary, width);
    let pos = egui::pos2(left, area.max.y - galley.size().y);
    ui.painter().galley(pos, galley, t.label_tertiary);
}

/// Um Papo em miniatura com as cores do momento: trilho, conversa e a
/// caixa de texto flutuando. Muda junto com o tema e a translucidez.
fn appearance_preview(ui: &egui::Ui, t: &Tokens, rect: Rect, translucent: bool) {
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(radius::CARD), t.content_bg, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
    let rail = Rect::from_min_size(rect.min, Vec2::new(40.0, rect.height()));
    painter.rect_filled(
        rail.shrink(1.0),
        CornerRadius { nw: radius::CARD - 1, sw: radius::CARD - 1, ne: 0, se: 0 },
        t.glass_opaque,
    );
    for index in 0..3 {
        let tile = Rect::from_min_size(rail.min + Vec2::new(6.0, 6.0 + index as f32 * 33.0), Vec2::splat(28.0));
        painter.rect_filled(tile, CornerRadius::same(radius::FIELD), if index == 0 { t.accent } else { t.fill_medium });
    }
    let content = Rect::from_min_max(egui::pos2(rail.max.x + 10.0, rect.min.y + 12.0), egui::pos2(rect.max.x - 10.0, rect.max.y - 10.0));
    for (row, fraction) in [0.6_f32, 0.85, 0.4].into_iter().enumerate() {
        let line = Rect::from_min_size(
            content.min + Vec2::new(0.0, row as f32 * 16.0),
            Vec2::new(content.width() * fraction, 8.0),
        );
        painter.rect_filled(line, CornerRadius::same(4), t.fill_medium);
    }
    let pill = Rect::from_min_max(egui::pos2(content.min.x, content.max.y - 22.0), content.max);
    painter.rect(pill, CornerRadius::same(radius::FIELD), t.pill_fill(translucent), Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
}

/// Um aviso de exemplo, como o sistema mostraria. Apagado quando os avisos
/// estão desligados.
fn notification_preview(ui: &egui::Ui, t: &Tokens, s: &Strings, rect: Rect, initials: &str, on: bool) {
    let painter = ui.painter();
    let alpha = if on { 1.0 } else { 0.4 };
    painter.add(ui.visuals().popup_shadow.as_shape(rect, CornerRadius::same(radius::SHEET)));
    painter.rect(rect, CornerRadius::same(radius::SHEET), t.elevated_bg, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
    let icon = Rect::from_min_size(rect.min + Vec2::new(space::LG, space::LG), Vec2::splat(32.0));
    painter.rect_filled(icon, CornerRadius::same(radius::FIELD), t.accent.gamma_multiply(alpha));
    painter.text(icon.center(), egui::Align2::CENTER_CENTER, initials, text::headline(), t.accent_label);
    let x = icon.max.x + space::MD;
    let clip = painter.with_clip_rect(rect.shrink(2.0));
    clip.text(egui::pos2(x, icon.min.y), egui::Align2::LEFT_TOP, s.preview_notify_title, text::headline(), t.label.gamma_multiply(alpha));
    clip.text(egui::pos2(x, icon.min.y + 17.0), egui::Align2::LEFT_TOP, s.preview_notify_body, text::callout(), t.label_secondary.gamma_multiply(alpha));
    clip.text(egui::pos2(rect.max.x - space::LG, icon.min.y), egui::Align2::RIGHT_TOP, s.preview_notify_meta, text::footnote(), t.label_tertiary);
}

/// Ícone do servidor num quadrado arredondado, ou as iniciais.
fn paint_server_icon(ui: &egui::Ui, t: &Tokens, rect: Rect, icon: Option<egui::TextureId>, initials: &str, corner: u8) {
    match icon {
        Some(texture) => crate::ui::widgets::photo(
            ui.painter(),
            rect,
            texture,
            crate::ui::widgets::FULL_UV,
            CornerRadius::same(corner),
            egui::Color32::WHITE,
        ),
        None => {
            ui.painter().rect_filled(rect, CornerRadius::same(corner), t.accent.gamma_multiply(0.30));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials, text::headline(), t.accent);
        }
    }
}

// ---------------------------------------------------------------------------
// Celular
// ---------------------------------------------------------------------------

const MOBILE_NAV: f32 = 56.0;
const MOBILE_SIDE: f32 = space::LG;

/// Ajustes no celular: tela cheia, lista de áreas com busca, e o painel
/// empurrado por cima. Os painéis são os mesmos do desktop, desenhados em
/// peças.
#[allow(clippy::too_many_arguments)]
fn mobile(
    ctx: &egui::Context,
    state: &mut SettingsState,
    data: &mut Context<'_>,
    t: &Tokens,
    s: &Strings,
    surface: Surface,
    screen: Rect,
    actions: &mut Vec<SettingsAction>,
) {
    egui::Area::new(egui::Id::new("ajustes-celular"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.painter().rect_filled(screen, CornerRadius::ZERO, t.content_bg);
            ui.interact(screen, egui::Id::new("ajustes-celular-fundo"), Sense::click_and_drag());

            // --- Barra: voltar + título à esquerda; X na lista -------------------
            let nav = Rect::from_min_size(screen.min, Vec2::new(screen.width(), MOBILE_NAV));
            let page = state.mobile_page;
            let title = match (page, surface) {
                (true, Surface::App) => state.app_pane.title(s),
                (true, Surface::Server) => state.server_pane.title(s),
                (false, Surface::App) => s.settings,
                (false, Surface::Server) => s.server_settings,
            };
            let mut left = nav.min.x + space::SM;
            if page {
                let back = Rect::from_min_size(egui::pos2(left, nav.center().y - 22.0), Vec2::splat(44.0));
                if nav_button(ui, t, back, egui_phosphor::regular::ARROW_LEFT, "voltar").on_hover_text(s.back).clicked() {
                    state.mobile_page = false;
                }
                left = back.max.x;
            } else {
                left += space::SM;
            }
            let close = Rect::from_min_size(egui::pos2(nav.max.x - space::SM - 44.0, nav.center().y - 22.0), Vec2::splat(44.0));
            if !page && nav_button(ui, t, close, egui_phosphor::regular::X, "fechar").clicked() {
                state.open = None;
            }
            ui.painter()
                .with_clip_rect(Rect::from_x_y_ranges(left..=close.min.x, nav.y_range()))
                .text(
                    egui::pos2(left + space::XS, nav.center().y),
                    egui::Align2::LEFT_CENTER,
                    title,
                    egui::FontId::new(20.0, crate::ui::theme::display_family()),
                    t.label,
                );

            // --- Corpo -----------------------------------------------------------
            let body = Rect::from_min_max(egui::pos2(screen.min.x, nav.max.y), screen.max);
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(body.shrink2(Vec2::new(MOBILE_SIDE, 0.0)))
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("ajustes-celular-corpo", page, state.app_pane.title(s), state.server_pane.title(s)))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(TILES_KEY), true));
                            if page {
                                match surface {
                                    Surface::App => app_pane(ui, state, data, t, s, actions),
                                    Surface::Server => server_pane(ui, state, data, t, s, actions),
                                }
                            } else {
                                match surface {
                                    Surface::App => app_list(ui, state, data, t, s),
                                    Surface::Server => server_list(ui, state, data, t, s),
                                }
                            }
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(TILES_KEY), false));
                            ui.add_space(space::XXXL);
                        });
                },
            );
        });
}

fn nav_button(ui: &mut egui::Ui, t: &Tokens, rect: Rect, glyph: &str, id: &str) -> egui::Response {
    let response = ui.interact(rect, egui::Id::new(("ajustes-celular-nav", id)), Sense::click());
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter().circle_filled(rect.center(), 20.0, t.fill_soft);
    }
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, text::icon(20.0), t.label);
    response
}

/// Uma área na lista: ícone num círculo, título e o resumo do que está
/// valendo.
struct Entry<'a> {
    glyph: &'a str,
    title: &'a str,
    summary: String,
}

/// Peça clicável da lista. `first`/`last` dão os cantos grandes nas pontas
/// do grupo.
/// Desenho próprio do círculo da esquerda (a sua foto, o ícone do servidor).
type Lead<'a> = &'a dyn Fn(&egui::Painter, Rect);

fn list_tile(ui: &mut egui::Ui, t: &Tokens, entry: &Entry<'_>, first: bool, last: bool, lead: Option<Lead<'_>>) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::click());
    let top = if first { TILE_END } else { TILE_MID };
    let bottom = if last { TILE_END } else { TILE_MID };
    let pressed = response.hovered() || response.is_pointer_button_down_on();
    ui.painter().rect_filled(
        rect,
        CornerRadius { nw: top, ne: top, sw: bottom, se: bottom },
        if pressed { t.fill_medium } else { t.fill_soft },
    );
    let icon = Rect::from_center_size(egui::pos2(rect.min.x + TILE_INSET + 20.0, rect.center().y), Vec2::splat(40.0));
    match lead {
        Some(draw) => draw(ui.painter(), icon),
        None => {
            ui.painter().circle_filled(icon.center(), 20.0, t.fill_medium);
            ui.painter().text(icon.center(), egui::Align2::CENTER_CENTER, entry.glyph, text::icon(18.0), t.label);
        }
    }
    let x = icon.max.x + space::LG;
    let clip = ui.painter().with_clip_rect(Rect::from_x_y_ranges(x..=rect.max.x - TILE_INSET, rect.y_range()));
    let has_summary = !entry.summary.is_empty();
    let width = rect.max.x - TILE_INSET - x;
    crate::ui::widgets::text_fit(
        &clip,
        egui::pos2(x, rect.center().y - if has_summary { 9.0 } else { 0.0 }),
        egui::Align2::LEFT_CENTER,
        entry.title,
        egui::FontId::new(15.0, egui::FontFamily::Proportional),
        t.label,
        width,
    );
    if has_summary {
        crate::ui::widgets::text_fit(
            &clip,
            egui::pos2(x, rect.center().y + 10.0),
            egui::Align2::LEFT_CENTER,
            &entry.summary,
            text::callout(),
            t.label_secondary,
            width,
        );
    }
    if pressed {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// Grupo de peças; devolve o índice da que foi tocada.
fn tile_group(ui: &mut egui::Ui, t: &Tokens, entries: &[Entry<'_>]) -> Option<usize> {
    let mut hit = None;
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            ui.add_space(TILE_GAP);
        }
        if list_tile(ui, t, entry, index == 0, index + 1 == entries.len(), None) {
            hit = Some(index);
        }
    }
    ui.spacing_mut().item_spacing.y = spacing;
    ui.add_space(space::LG);
    hit
}

/// Campo de busca da lista, em pílula.
fn search_field(ui: &mut egui::Ui, t: &Tokens, s: &Strings, query: &mut String) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(24), t.glass_opaque, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
    ui.painter().text(
        egui::pos2(rect.min.x + space::XL + 8.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::MAGNIFYING_GLASS,
        text::icon(17.0),
        t.accent,
    );
    let field = Rect::from_min_max(egui::pos2(rect.min.x + space::XL + 24.0, rect.min.y + 4.0), egui::pos2(rect.max.x - space::XL, rect.max.y - 4.0));
    #[cfg(target_os = "android")]
    {
        let _ = crate::platform::native_field::show(
            ui.ctx(),
            "ajustes-celular:busca",
            query,
            field,
            s.settings_search,
            crate::platform::native_field::Mode::Text,
            64,
            false,
            t.label,
            t.label_tertiary,
            15.0,
        );
    }
    #[cfg(not(target_os = "android"))]
    {
        let mut child = ui.new_child(UiBuilder::new().max_rect(field));
        child.add_sized(
            field.size(),
            egui::TextEdit::singleline(query)
                .hint_text(s.settings_search)
                .frame(egui::Frame::NONE)
                .font(egui::FontId::new(15.0, egui::FontFamily::Proportional))
                .vertical_align(Align::Center),
        );
    }
    ui.add_space(space::MD);
}

/// Entradas que casam com a busca, sem acento nem caixa.
fn matches(query: &str, entry: &Entry<'_>) -> bool {
    let fold = |text: &str| -> String {
        text.to_lowercase()
            .chars()
            .map(|c| match c {
                'á' | 'à' | 'â' | 'ã' => 'a',
                'é' | 'ê' => 'e',
                'í' => 'i',
                'ó' | 'ô' | 'õ' => 'o',
                'ú' => 'u',
                'ç' => 'c',
                other => other,
            })
            .collect()
    };
    let query = fold(query.trim());
    query.is_empty() || fold(entry.title).contains(&query) || fold(&entry.summary).contains(&query)
}

/// Lista filtrada pela busca; um grupo só, ou "nada encontrado".
fn filtered<T: Copy>(ui: &mut egui::Ui, t: &Tokens, s: &Strings, query: &str, all: &[(T, Entry<'_>)]) -> Option<T> {
    let shown: Vec<_> = all.iter().filter(|(_, entry)| matches(query, entry)).collect();
    if shown.is_empty() {
        ui.add_space(space::XL);
        ui.label(RichText::new(s.no_results).font(text::body()).color(t.label_secondary));
        return None;
    }
    let entries: Vec<Entry<'_>> = shown
        .iter()
        .map(|(_, entry)| Entry { glyph: entry.glyph, title: entry.title, summary: entry.summary.clone() })
        .collect();
    tile_group(ui, t, &entries).map(|index| shown[index].0)
}

fn footnote(ui: &mut egui::Ui, t: &Tokens, note: &str) {
    ui.horizontal(|ui| {
        ui.add_space(space::MD);
        ui.add(egui::Label::new(RichText::new(note).font(text::footnote()).color(t.label_tertiary)).wrap());
    });
}

/// Lista dos ajustes do usuário.
fn app_list(ui: &mut egui::Ui, state: &mut SettingsState, data: &mut Context<'_>, t: &Tokens, s: &Strings) {
    use egui_phosphor::regular as icon;
    search_field(ui, t, s, &mut state.mobile_query);

    let theme = match *data.theme {
        crate::ui::theme::ThemePref::System => s.theme_system,
        crate::ui::theme::ThemePref::Light => s.theme_light,
        crate::ui::theme::ThemePref::Dark => s.theme_dark,
    };
    let appearance = if *data.translucency { format!("{theme} · {}", s.translucency) } else { theme.to_owned() };
    let alerts = if *data.notifications {
        if *data.badge { format!("{} · {}", s.sum_on, s.sum_badge) } else { s.sum_on.to_owned() }
    } else {
        s.sum_off.to_owned()
    };
    // Só o nome da pasta: o caminho inteiro não cabe numa linha de resumo.
    let files = data
        .download_dir
        .as_deref()
        .map(|dir| {
            std::path::Path::new(dir)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| dir.to_owned())
        })
        .unwrap_or_else(|| s.download_ask.to_owned());
    let count = data.store.devices.len().max(1);
    let devices = format!("{count} {}", if count == 1 { s.sum_device } else { s.sum_devices });
    let rows: Vec<(AppPane, Entry<'_>)> = vec![
        (AppPane::Appearance, Entry { glyph: icon::PALETTE, title: AppPane::Appearance.title(s), summary: appearance }),
        (AppPane::Alerts, Entry { glyph: icon::BELL, title: AppPane::Alerts.title(s), summary: alerts }),
        (AppPane::Language, Entry { glyph: icon::GLOBE, title: AppPane::Language.title(s), summary: data.lang.endonym().to_owned() }),
        (AppPane::Files, Entry { glyph: icon::FOLDER, title: AppPane::Files.title(s), summary: files }),
        (AppPane::Sessions, Entry { glyph: icon::DEVICES, title: AppPane::Sessions.title(s), summary: devices }),
        (AppPane::Diagnostics, Entry { glyph: icon::PULSE, title: AppPane::Diagnostics.title(s), summary: s.sum_diagnostics.to_owned() }),
    ];

    let mut open = None;
    if !state.mobile_query.trim().is_empty() {
        // Painéis e ajustes soltos: o ajuste leva ao painel e acende.
        let mut all: Vec<((AppPane, Option<&'static str>), Entry<'_>)> =
            vec![((AppPane::Account, None), Entry { glyph: icon::USER, title: AppPane::Account.title(s), summary: s.sum_account.to_owned() })];
        all.extend(rows.into_iter().map(|(pane, entry)| ((pane, None), entry)));
        all.extend(app_settings_index(s).into_iter().map(|(pane, label)| {
            ((pane, Some(label)), Entry { glyph: pane.glyph(), title: label, summary: pane.title(s).to_owned() })
        }));
        if let Some((pane, label)) = filtered(ui, t, s, &state.mobile_query, &all) {
            if let Some(label) = label {
                focus_setting(ui.ctx(), label);
            }
            open = Some(pane);
        }
    } else {
        // Você no topo: a foto, o nome e o que fica em "Conta".
        let me = data.store.member(&data.store.me);
        let tint = me
            .and_then(|member| member.role_color)
            .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
            .unwrap_or(t.accent);
        let initials = me.map(|member| member.initials()).unwrap_or_default();
        let ctx = ui.ctx().clone();
        let face = data
            .media
            .avatar(&data.store.me, data.store.avatars.get(&data.store.me).map(String::as_str))
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| handle.id());
        let lead = |painter: &egui::Painter, rect: Rect| match face {
            Some(texture) => crate::ui::widgets::round_photo(painter, rect, texture, egui::Color32::WHITE),
            None => {
                painter.circle_filled(rect.center(), 20.0, tint.gamma_multiply(0.30));
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, &initials, egui::FontId::new(14.0, egui::FontFamily::Name("semibold".into())), tint);
            }
        };
        let me_entry = Entry { glyph: "", title: &data.store.my_name, summary: s.sum_account.to_owned() };
        if list_tile(ui, t, &me_entry, true, true, Some(&lead)) {
            open = Some(AppPane::Account);
        }
        ui.add_space(space::LG);
        let (device, rest) = rows.split_at(4);
        let groups: [&[(AppPane, Entry<'_>)]; 3] = [device, &rest[..1], &rest[1..]];
        for group in groups {
            let entries: Vec<Entry<'_>> = group.iter().map(|(_, e)| Entry { glyph: e.glyph, title: e.title, summary: e.summary.clone() }).collect();
            if let Some(index) = tile_group(ui, t, &entries) {
                open = Some(group[index].0);
            }
        }
        footnote(ui, t, s.sum_synced);
    }
    if let Some(pane) = open {
        state.app_pane = pane;
        state.mobile_page = true;
        if pane == AppPane::Account {
            state.draft.loaded_account = false;
        }
    }
}

/// Lista dos ajustes do servidor: só o que o cargo permite.
fn server_list(ui: &mut egui::Ui, state: &mut SettingsState, data: &mut Context<'_>, t: &Tokens, s: &Strings) {
    use egui_phosphor::regular as icon;
    search_field(ui, t, s, &mut state.mobile_query);
    let store = data.store;
    let all: Vec<(ServerPane, Entry<'_>)> = [
        (ServerPane::General, icon::IDENTIFICATION_CARD, s.sum_identity.to_owned()),
        (ServerPane::Channels, icon::LIST_BULLETS, format!("{} {}", store.channels.len(), s.sum_channels)),
        (ServerPane::Roles, icon::SHIELD, format!("{} {}", store.roles.len(), s.sum_roles)),
        (ServerPane::Emojis, icon::SMILEY, format!("{} {}", store.emojis.len(), s.sum_stickers)),
        (ServerPane::Audit, icon::CLOCK_COUNTER_CLOCKWISE, s.sum_audit.to_owned()),
    ]
    .into_iter()
    .filter(|(pane, _, _)| server_pane_allowed(store, *pane))
    .map(|(pane, glyph, summary)| (pane, Entry { glyph, title: pane.title(s), summary }))
    .collect();

    let mut open = None;
    if !state.mobile_query.trim().is_empty() {
        let mut found: Vec<((ServerPane, Option<&'static str>), Entry<'_>)> = all
            .iter()
            .map(|(pane, entry)| ((*pane, None), Entry { glyph: entry.glyph, title: entry.title, summary: entry.summary.clone() }))
            .collect();
        found.extend(
            server_settings_index(s)
                .into_iter()
                .filter(|(pane, _)| server_pane_allowed(store, *pane))
                .map(|(pane, label)| ((pane, Some(label)), Entry { glyph: pane.glyph(), title: label, summary: pane.title(s).to_owned() })),
        );
        if let Some((pane, label)) = filtered(ui, t, s, &state.mobile_query, &found) {
            if let Some(label) = label {
                focus_setting(ui.ctx(), label);
            }
            open = Some(pane);
        }
    } else {
        // O servidor no topo: ícone, nome, endereço e quantos são.
        let name = store.server.as_ref().map(|server| server.name.clone()).unwrap_or_default();
        let ctx = ui.ctx().clone();
        let texture = store
            .server
            .as_ref()
            .and_then(|server| server.icon.as_deref())
            .and_then(|blob| data.media.server_icon(blob))
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| handle.id());
        let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
        let lead = |painter: &egui::Painter, rect: Rect| match texture {
            Some(texture) => crate::ui::widgets::photo(painter, rect, texture, crate::ui::widgets::FULL_UV, CornerRadius::same(radius::CARD), egui::Color32::WHITE),
            None => {
                painter.rect_filled(rect, CornerRadius::same(radius::CARD), t.accent.gamma_multiply(0.30));
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, &initials, egui::FontId::new(14.0, egui::FontFamily::Name("semibold".into())), t.accent);
            }
        };
        let address = data.server_url.trim_start_matches("https://").trim_start_matches("http://");
        let head = Entry { glyph: "", title: &name, summary: format!("{address} · {} {}", store.members.len(), s.count_members) };
        list_tile(ui, t, &head, true, true, Some(&lead));
        ui.add_space(space::LG);
        let split = all.iter().position(|(pane, _)| matches!(pane, ServerPane::Emojis | ServerPane::Audit)).unwrap_or(all.len());
        let (admin, extra) = all.split_at(split);
        for group in [admin, extra] {
            if group.is_empty() {
                continue;
            }
            let entries: Vec<Entry<'_>> = group.iter().map(|(_, e)| Entry { glyph: e.glyph, title: e.title, summary: e.summary.clone() }).collect();
            if let Some(index) = tile_group(ui, t, &entries) {
                open = Some(group[index].0);
            }
        }
        footnote(ui, t, s.sum_admin_note);
    }
    if let Some(pane) = open {
        state.server_pane = pane;
        state.mobile_page = true;
        state.draft.loaded_server = false;
        state.draft.loaded_audit = false;
    }
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
    let escape = !state.modal_above && ui.input(|input| input.key_pressed(egui::Key::Escape));
    if response.clicked() || escape {
        state.open = None;
    }
}

/// Coluna dos painéis. Fica sempre visível e sempre marca onde você está.
fn rail(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    store: &Store,
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
            // Busca no topo da coluna: filtra os painéis pelo nome.
            {
                let (field, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
                ui.painter().rect_filled(field, CornerRadius::same(radius::FIELD), t.fill_soft);
                ui.painter().text(
                    egui::pos2(field.min.x + space::MD + 6.0, field.center().y),
                    egui::Align2::CENTER_CENTER,
                    egui_phosphor::regular::MAGNIFYING_GLASS,
                    text::icon(12.0),
                    t.label_tertiary,
                );
                let inner = Rect::from_min_max(egui::pos2(field.min.x + space::MD + 16.0, field.min.y), egui::pos2(field.max.x - space::SM, field.max.y));
                let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
                child.add_sized(
                    inner.size(),
                    egui::TextEdit::singleline(&mut state.mobile_query)
                        .hint_text(s.rail_search)
                        .frame(egui::Frame::NONE)
                        .font(text::callout())
                        .vertical_align(Align::Center),
                );
                ui.add_space(space::MD);
            }
            let query = state.mobile_query.trim().to_lowercase();
            let shown = |title: &str| query.is_empty() || title.to_lowercase().contains(&query);
            let entry = |ui: &mut egui::Ui, glyph: &str, label: &str, active: bool| -> bool {
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
                let ink = if active { t.accent } else { t.label_secondary };
                ui.painter().text(
                    egui::pos2(slot.min.x + space::MD + 7.0, slot.center().y),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    text::icon(14.0),
                    if active { t.accent } else { t.label_tertiary },
                );
                crate::ui::widgets::text_fit(
                    ui.painter(),
                    egui::pos2(slot.min.x + space::MD + 22.0, slot.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    if active { text::headline() } else { text::body() },
                    ink,
                    slot.max.x - space::SM - (slot.min.x + space::MD + 22.0),
                );
                response.clicked()
            };

            // A coluna rola: com a busca, a lista de ajustes pode passar da
            // altura da folha.
            egui::ScrollArea::vertical()
                .id_salt(("coluna-de-ajustes", surface == Surface::App))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                match surface {
                    Surface::App => {
                        for pane in AppPane::ALL {
                            if !shown(pane.title(s)) {
                                continue;
                            }
                            if entry(ui, pane.glyph(), pane.title(s), state.app_pane == pane) {
                                state.app_pane = pane;
                            }
                        }
                    }
                    Surface::Server => {
                        for pane in ServerPane::ALL {
                            if !server_pane_allowed(store, pane) || !shown(pane.title(s)) {
                                continue;
                            }
                            if entry(ui, pane.glyph(), pane.title(s), state.server_pane == pane) {
                                state.server_pane = pane;
                            }
                        }
                    }
                }

                // Com busca, os ajustes que casam aparecem embaixo dos painéis;
                // clicar abre o painel e acende a linha.
                if !query.is_empty() {
                    let hits: Vec<(Option<AppPane>, Option<ServerPane>, &'static str, &'static str)> = match surface {
                        Surface::App => app_settings_index(s)
                            .into_iter()
                            .filter(|(_, label)| shown(label))
                            .map(|(pane, label)| (Some(pane), None, pane.glyph(), label))
                            .collect(),
                        Surface::Server => server_settings_index(s)
                            .into_iter()
                            .filter(|(pane, label)| server_pane_allowed(store, *pane) && shown(label))
                            .map(|(pane, label)| (None, Some(pane), pane.glyph(), label))
                            .collect(),
                    };
                    if !hits.is_empty() {
                        ui.add_space(space::MD);
                        ui.label(RichText::new(s.settings.to_uppercase()).font(text::caption()).color(t.label_tertiary));
                        ui.add_space(space::XS);
                    }
                    for (app, server, glyph, label) in hits {
                        if entry(ui, glyph, label, false) {
                            if let Some(pane) = app {
                                state.app_pane = pane;
                            }
                            if let Some(pane) = server {
                                state.server_pane = pane;
                            }
                            focus_setting(ui.ctx(), label);
                        }
                    }
                }
            });
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
                // O formulário começa com o que está valendo. Antes recado e
                // descrição abriam vazios, e salvar o apelido apagava os dois.
                let me = data.store.member(&data.store.me);
                state.draft.nickname = data.store.my_name.clone();
                state.draft.status_message = me
                    .and_then(|member| member.status_message.clone())
                    .unwrap_or_default();
                state.draft.typing = me
                    .and_then(|member| member.typing_label.clone())
                    .unwrap_or_default();
                state.draft.description = data
                    .store
                    .profiles
                    .get(&data.store.me)
                    .and_then(|profile| profile.description.clone())
                    .unwrap_or_default();
                state.draft.password.clear();
                actions.push(SettingsAction::Admin(AdminAction::LoadDevices));
            }
            let draft = &mut state.draft;
            // O clique sai de dentro do cartão, e `actions` já está
            // emprestado ali; a bandeira atravessa esse empréstimo.
            let mut pick_avatar = false;
            let mut pick_banner = false;
            let mut remove_banner = false;
            let has_banner = data
                .store
                .profiles
                .get(&data.store.me)
                .is_some_and(|profile| profile.banner.is_some());

            section(ui, t, s.profile);
            // O cabeçalho do cartão como ele fica, e clicável.
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                match crate::ui::profile::header_preview(ui, t, s, data.store, data.media) {
                    crate::ui::profile::PreviewHit::Avatar => pick_avatar = true,
                    crate::ui::profile::PreviewHit::Banner => pick_banner = true,
                    crate::ui::profile::PreviewHit::None => {}
                }
            });
            // Os botões moram com a prévia: são dela, não linhas à parte.
            ui.add_space(space::SM);
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                ui.spacing_mut().item_spacing.x = space::SM;
                if row_button(ui, t, s.change_avatar, Emphasis::Quiet) {
                    pick_avatar = true;
                }
                if row_button(ui, t, s.change_banner, Emphasis::Quiet) {
                    pick_banner = true;
                }
                if has_banner && row_button(ui, t, s.remove_banner, Emphasis::Danger) {
                    remove_banner = true;
                }
            });
            ui.add_space(space::MD);
            group(ui, t, |rows| {
                rows.field(s.nickname, &mut draft.nickname, 32, false);
                rows.field(s.status_message, &mut draft.status_message, 64, false);
                rows.field_lines(s.description, &mut draft.description, 512, 3);
                rows.field_hinted(
                    s.typing_phrase,
                    Some(s.typing_phrase_hint),
                    &mut draft.typing,
                    64,
                    false,
                );
            });
            ui.add_space(space::SM);
            actions_row(ui, |ui| {
                if row_button(ui, t, s.save_profile, Emphasis::Primary) {
                    actions.push(SettingsAction::Admin(AdminAction::SaveProfile(Box::new(
                        crate::api::models::UpdateUserRequest {
                            nickname: draft.nickname.trim().to_owned(),
                            status: draft.status_message.trim().to_owned(),
                            description: draft.description.trim().to_owned(),
                            // Vazio grava vazio, e vazio volta ao texto padrão.
                            typing: Some(draft.typing.trim().to_owned()),
                        },
                    ))));
                }
            });
            if pick_avatar {
                actions.push(SettingsAction::Admin(AdminAction::PickAvatar));
            }
            if pick_banner {
                actions.push(SettingsAction::Admin(AdminAction::PickBanner));
            }
            if remove_banner {
                actions.push(SettingsAction::Admin(AdminAction::RemoveBanner));
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

            // Segurança: senha e sair, juntos — as duas coisas que não são
            // perfil e não mudam na hora como a presença.
            section(ui, t, s.security);
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
                if rows.action(s.sign_out, Some(s.sign_out_hint), true) {
                    actions.push(SettingsAction::Menu(
                        crate::platform::menu::MenuCommand::SignOut,
                    ));
                }
            });
        }

        AppPane::Appearance => {
            hero(ui, t, 150.0, |ui, area| {
                let window = Rect::from_min_size(area.min, Vec2::new(area.width().min(300.0), 150.0));
                appearance_preview(ui, t, window, *data.translucency);
                hint_beside(ui, t, area, window, s.preview_theme_hint);
            });
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
                rows.row(
                    s.webembed_offscreen,
                    Some(s.webembed_offscreen_hint),
                    |ui, t| {
                        segmented(
                            ui,
                            t,
                            data.webembed_offscreen,
                            &[
                                (crate::webembed::OffscreenBehavior::Stop, s.webembed_stop),
                                (crate::webembed::OffscreenBehavior::Float, s.webembed_float),
                            ],
                        );
                    },
                );
                rows.row(
                    s.webembed_scope,
                    Some(s.webembed_scope_hint),
                    |ui, t| {
                        segmented(
                            ui,
                            t,
                            data.webembed_scope,
                            &[
                                (crate::webembed::FloatScope::CurrentChannel, s.webembed_scope_channel),
                                (crate::webembed::FloatScope::Global, s.webembed_scope_global),
                            ],
                        );
                    },
                );
            });

            section(ui, t, s.profile);
            group(ui, t, |rows| {
                use crate::ui::profile::SelfCardStyle;
                let hint = match *data.self_card {
                    SelfCardStyle::Pill => s.self_card_pill_hint,
                    SelfCardStyle::Floating => s.self_card_floating_hint,
                };
                rows.row(s.self_card, Some(hint), |ui, t| {
                    segmented(
                        ui,
                        t,
                        data.self_card,
                        &[
                            (SelfCardStyle::Pill, s.self_card_pill),
                            (SelfCardStyle::Floating, s.self_card_floating),
                        ],
                    );
                });
            });
        }

        AppPane::Alerts => {
            hero(ui, t, 76.0, |ui, area| {
                let card = Rect::from_min_size(area.min + Vec2::new(0.0, 6.0), Vec2::new(area.width().min(300.0), 64.0));
                let initials: String = data
                    .store
                    .server
                    .as_ref()
                    .map(|server| server.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase())
                    .unwrap_or_else(|| "P".to_owned());
                notification_preview(ui, t, s, card, &initials, *data.notifications);
                hint_beside(ui, t, area, card, s.preview_notify_hint);
            });
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
            group(ui, t, |rows| {
                let preview = format!(
                    "{} cache-ready · {} cache-negative · {} queued · {} network · {} transient · {} stale-served",
                    data.preview.cache_ready,
                    data.preview.cache_negative,
                    data.preview.queued,
                    data.preview.network_resolves,
                    data.preview.transient_failures,
                    data.preview.stale_served,
                );
                rows.row("Preview coordinator", Some(&preview), |_, _| {});
            });
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
                #[cfg(any(target_os = "windows", target_os = "android"))]
                if data.update_enabled {
                    rows.row(
                        s.check_updates,
                        data.update_status,
                        |ui, t| {
                            if row_button(ui, t, s.check_updates, Emphasis::Quiet) {
                                actions.push(SettingsAction::CheckUpdates);
                            }
                        },
                    );
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Painéis do servidor
// ---------------------------------------------------------------------------

fn server_pane_allowed(store: &Store, pane: ServerPane) -> bool {
    match pane {
        ServerPane::General | ServerPane::Emojis | ServerPane::Audit => store.can_manage_server(),
        ServerPane::Channels => store.can_manage_channels(),
        ServerPane::Roles => store.can_manage_roles(),
    }
}

fn server_pane(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    data: &mut Context<'_>,
    t: &Tokens,
    s: &Strings,
    actions: &mut Vec<SettingsAction>,
) {
    use super::admin::AdminAction;
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

            let mut pick_icon = false;
            let ctx = ui.ctx().clone();
            let icon = data
                .store
                .server
                .as_ref()
                .and_then(|server| server.icon.as_deref())
                .and_then(|blob| data.media.server_icon(blob))
                .and_then(|texture| texture.frame(&ctx))
                .map(|handle| handle.id());
            let initials: String = draft
                .server_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();
            hero(ui, t, 52.0, |ui, area| {
                let tile = Rect::from_min_size(area.min + Vec2::new(0.0, 4.0), Vec2::splat(44.0));
                paint_server_icon(ui, t, tile, icon, &initials, radius::SHEET);
                let pill = Rect::from_min_size(egui::pos2(tile.max.x + space::LG, area.min.y + 3.0), Vec2::new(216.0_f32.min(area.width() - 60.0), 46.0));
                ui.painter().rect(pill, CornerRadius::same(12), t.glass_opaque, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
                let small = Rect::from_center_size(egui::pos2(pill.min.x + space::SM + 14.0, pill.center().y), Vec2::splat(28.0));
                paint_server_icon(ui, t, small, icon, &initials, radius::FIELD);
                let clip = ui.painter().with_clip_rect(pill.shrink(2.0));
                clip.text(egui::pos2(small.max.x + space::MD, pill.center().y - 7.0), egui::Align2::LEFT_CENTER, &draft.server_name, text::headline(), t.label);
                let owner = data.store.server.as_ref().and_then(|server| server.description.clone()).unwrap_or_default();
                clip.text(egui::pos2(small.max.x + space::MD, pill.center().y + 8.0), egui::Align2::LEFT_CENTER, owner, text::footnote(), t.label_tertiary);
                let wide = Rect::from_min_max(tile.min, pill.max);
                hint_beside(ui, t, area, wide, s.preview_identity_hint);
            });
            section(ui, t, s.pane_identity);
            group(ui, t, |rows| {
                rows.field(s.server_name, &mut draft.server_name, 64, false);
                rows.row(s.server_icon, None, |ui, t| {
                    if row_button(ui, t, s.change_server_icon, Emphasis::Quiet) {
                        pick_icon = true;
                    }
                    // O ícone como o trilho mostra; clicar nele também troca.
                    let (slot, response) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
                    let corners = CornerRadius::same(radius::FIELD);
                    match icon {
                        Some(icon) => crate::ui::widgets::photo(
                            ui.painter(),
                            slot,
                            icon,
                            crate::ui::widgets::FULL_UV,
                            corners,
                            egui::Color32::WHITE,
                        ),
                        None => {
                            ui.painter().rect_filled(slot, corners, t.fill_medium);
                            ui.painter().text(
                                slot.center(),
                                egui::Align2::CENTER_CENTER,
                                &initials,
                                text::headline(),
                                t.label,
                            );
                        }
                    }
                    if response.hovered() {
                        ui.painter().rect_filled(slot, corners, egui::Color32::from_black_alpha(90));
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if response.clicked() {
                        pick_icon = true;
                    }
                });
                rows.row(s.server_public, Some(s.server_public_hint), |ui, t| {
                    switch(ui, t, &mut draft.server_public);
                });
                if !draft.server_public {
                    rows.field(s.server_password, &mut draft.server_password, 64, true);
                }
            });
            if pick_icon {
                actions.push(SettingsAction::Admin(AdminAction::PickServerIcon));
            }

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

            // Overrides por cargo pertencem ao canal existente. Canal recém-criado
            // só ganha regras depois que o backend lhe deu um id.
            if let Some((channel_id, channel_kind)) = draft
                .channel
                .as_ref()
                .and_then(|editor| editor.id.clone().map(|id| (id, editor.kind.clone())))
                && let Some(channel) = channels.iter().find(|channel| channel.id == channel_id)
            {
                    section(ui, t, s.channel_permissions);

                    if channel.permissions.is_empty() {
                        group(ui, t, |rows| {
                            rows.row(
                                s.channel_permissions_open,
                                Some(s.channel_permissions_open_hint),
                                |_, _| {},
                            );
                        });
                        ui.add_space(space::SM);
                    }

                    if let Some(permission) = draft
                        .channel_permission
                        .as_mut()
                        .filter(|permission| permission.channel_id == channel_id)
                    {
                        group(ui, t, |rows| {
                            rows.row(&permission.role_name, None, |_, _| {});
                            rows.row(s.perm_read_channel, None, |ui, t| {
                                switch(ui, t, &mut permission.permissions.read_channel);
                            });
                            if channel_kind == "text" {
                                rows.row(s.perm_send_messages, None, |ui, t| {
                                    switch(ui, t, &mut permission.permissions.send_messages);
                                });
                                rows.row(s.perm_delete_messages, None, |ui, t| {
                                    switch(ui, t, &mut permission.permissions.delete_messages);
                                });
                            }
                            if channel_kind == "voice" {
                                rows.row(s.perm_connect_voice, None, |ui, t| {
                                    switch(ui, t, &mut permission.permissions.connect_voice);
                                });
                            }
                        });
                        ui.add_space(space::SM);
                        let mut cancel_permission = false;
                        let mut save_permission = None;
                        actions_row(ui, |ui| {
                            if row_button(ui, t, s.cancel, Emphasis::Quiet) {
                                cancel_permission = true;
                            }
                            if row_button(ui, t, s.save, Emphasis::Primary) {
                                save_permission = Some((
                                    permission.channel_id.clone(),
                                    permission.role_id.clone(),
                                    permission.permissions,
                                ));
                            }
                        });
                        if cancel_permission {
                            draft.channel_permission = None;
                        }
                        if let Some((channel_id, role_id, permissions)) = save_permission {
                            actions.push(SettingsAction::Chat(
                                ChatAction::SetChannelPermissions {
                                    channel_id,
                                    role_id,
                                    permissions,
                                },
                            ));
                            draft.channel_permission = None;
                        }
                    } else {
                        group(ui, t, |rows| {
                            for entry in &channel.permissions {
                                rows.row(&entry.role_name, None, |ui, t| {
                                    if row_button(ui, t, s.edit, Emphasis::Quiet) {
                                        draft.channel_permission = Some(ChannelPermissionDraft {
                                            channel_id: channel_id.clone(),
                                            role_id: entry.role_id.clone(),
                                            role_name: entry.role_name.clone(),
                                            permissions: entry.permissions,
                                        });
                                    }
                                });
                            }

                            for role in data.store.roles.iter().filter(|role| {
                                !channel
                                    .permissions
                                    .iter()
                                    .any(|entry| entry.role_id == role.id)
                            }) {
                                rows.row(&role.name, None, |ui, t| {
                                    if row_button(
                                        ui,
                                        t,
                                        s.channel_permission_add_role,
                                        Emphasis::Quiet,
                                    ) {
                                        draft.channel_permission = Some(ChannelPermissionDraft {
                                            channel_id: channel_id.clone(),
                                            role_id: role.id.clone(),
                                            role_name: role.name.clone(),
                                            permissions: Default::default(),
                                        });
                                    }
                                });
                            }
                        });
                    }
                    ui.add_space(space::LG);
            }

            // Apagar pede o nome digitado: é o que separa "quis" de
            // "esbarrou". A confirmação fica acima da árvore.
            if let Some((id, expected, typed)) = draft.deleting.as_mut() {
                let name = channels.iter().find(|c| &c.id == id).map(|c| c.name.clone()).unwrap_or_default();
                let id = id.clone();
                group(ui, t, |rows| {
                    rows.row(&name, Some(s.delete_type_name), |ui, t| {
                        if row_button(ui, t, s.cancel, Emphasis::Quiet) {
                            cancel_delete = true;
                        }
                    });
                    let sent = rows.field(s.channel_name, typed, 32, false);
                    let matches = typed.trim() == expected.as_str();
                    rows.row("", None, |ui, t| {
                        if (row_button(ui, t, s.delete_forever, Emphasis::Danger) || sent) && matches {
                            confirmed_delete = Some(id.clone());
                        }
                    });
                });
                ui.add_space(space::MD);
            }

            if channels.is_empty() {
                group(ui, t, |rows| rows.row(s.no_channels_yet, None, |_, _| {}));
            } else {
                match channel_tree(ui, t, s, data.store, data.collapsed, &mut state.drag_channel) {
                    Some(TreeAction::Edit(id)) => {
                        draft.deleting = None;
                        draft.channel_permission = None;
                        actions.push(SettingsAction::Chat(ChatAction::EditChannel(id)));
                    }
                    Some(TreeAction::Delete(id, name)) => {
                        draft.channel = None;
                        draft.deleting = Some((id, name, String::new()));
                    }
                    Some(TreeAction::Move { channel_id, old_position, new_position, parent_id }) => {
                        actions.push(SettingsAction::Chat(ChatAction::MoveChannel {
                            channel_id,
                            old_position,
                            new_position,
                            parent_id,
                        }));
                    }
                    None => {}
                }
            }

            if close_editor {
                draft.channel = None;
                draft.channel_permission = None;
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
                    draft.channel_permission = None;
                    draft.channel = Some(ChannelDraft::create());
                }
            });
        }

        ServerPane::Roles => roles_pane(ui, data, t, s, actions),

        ServerPane::Emojis => {
            // Escolher o arquivo abre o editor: recorte, prévia na conversa
            // e o nome, tudo num lugar só; Salvar já sobe.
            section(ui, t, s.sticker_new);
            group(ui, t, |rows| {
                if rows.action(s.add_emoji, Some(s.sticker_hint), false) {
                    actions.push(SettingsAction::Admin(AdminAction::PickSticker));
                }
            });

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

        ServerPane::Audit => audit_pane(ui, data, state, t, s, actions),
    }
}

fn audit_pane(
    ui: &mut egui::Ui,
    data: &mut Context<'_>,
    state: &mut SettingsState,
    t: &Tokens,
    s: &Strings,
    actions: &mut Vec<SettingsAction>,
) {
    use super::admin::AdminAction;
    use crate::ui::audit::{self, Kind, Period, Piece};
    let now = chrono::Local::now();
    // O registro é pedido ao abrir o painel, não ao abrir a folha: quem só
    // queria renomear o servidor não precisa esperá-lo.
    if !state.draft.loaded_audit {
        state.draft.loaded_audit = true;
        actions.push(SettingsAction::Admin(AdminAction::LoadAuditLogs(state.draft.audit.query(now))));
    }
    let before = state.draft.audit.clone();
    let filter = &mut state.draft.audit;
    ui.add_space(space::MD);

    // Pessoa: casa com quem fez ou com quem sofreu a ação.
    let (field, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    ui.painter().rect_filled(field, CornerRadius::same(radius::FIELD), t.fill_soft);
    ui.painter().text(
        egui::pos2(field.min.x + space::MD + 6.0, field.center().y),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::MAGNIFYING_GLASS,
        text::icon(12.0),
        t.label_tertiary,
    );
    let inner = Rect::from_min_max(
        egui::pos2(field.min.x + space::MD + 18.0, field.min.y),
        egui::pos2(field.max.x - space::SM, field.max.y),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(&mut filter.person)
            .hint_text(s.audit_person_search)
            .frame(egui::Frame::NONE)
            .font(text::callout())
            .vertical_align(Align::Center),
    );
    ui.add_space(space::SM);

    let kinds: Vec<(Kind, &str)> = Kind::ALL
        .iter()
        .map(|kind| {
            (*kind, match kind {
                Kind::All => s.audit_kind_all,
                Kind::Deleted => s.audit_kind_deleted,
                Kind::Messages => s.audit_kind_messages,
                Kind::Channels => s.audit_kind_channels,
                Kind::Roles => s.audit_kind_roles,
                Kind::Members => s.audit_kind_members,
                Kind::Server => s.audit_kind_server,
            })
        })
        .collect();
    let periods: Vec<(Period, &str)> = Period::ALL
        .iter()
        .map(|period| {
            (*period, match period {
                Period::All => s.audit_period_all,
                Period::Today => s.audit_period_today,
                Period::Week => s.audit_period_week,
                Period::Month => s.audit_period_month,
            })
        })
        .collect();
    let mut channels: Vec<(Option<&str>, String)> = vec![(None, s.audit_all.to_owned())];
    channels.extend(
        data.store
            .channels
            .iter()
            .filter(|channel| channel.kind != crate::state::ChannelKind::Category)
            .map(|channel| (Some(channel.id.as_str()), format!("#{}", channel.name))),
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(space::SM, space::SM);
        let deleted = filter.kind == Kind::Deleted;
        if audit_chip(ui, t, egui_phosphor::regular::TRASH, s.audit_kind_deleted, deleted, false).clicked() {
            filter.kind = if deleted { Kind::All } else { Kind::Deleted };
        }
        let kind_label = kinds.iter().find(|(kind, _)| *kind == filter.kind).map(|(_, label)| *label).unwrap_or("");
        let response = audit_chip(
            ui,
            t,
            egui_phosphor::regular::SHAPES,
            &format!("{}: {}", s.audit_kind, if filter.kind == Kind::All { s.audit_all } else { kind_label }),
            filter.kind != Kind::All,
            true,
        );
        crate::ui::widgets::dropdown(&response, response.rect, response.rect, true).show(|ui| {
            for (kind, label) in &kinds {
                if crate::ui::widgets::menu_option(ui, t, label, *kind == filter.kind) {
                    filter.kind = *kind;
                }
            }
        });
        let channel_label = channels
            .iter()
            .find(|(id, _)| *id == filter.channel.as_deref())
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| s.audit_all.to_owned());
        let response = audit_chip(
            ui,
            t,
            egui_phosphor::regular::HASH,
            &format!("{}: {channel_label}", s.audit_channel),
            filter.channel.is_some(),
            true,
        );
        crate::ui::widgets::dropdown(&response, response.rect, response.rect, true).show(|ui| {
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for (id, label) in &channels {
                    if crate::ui::widgets::menu_option(ui, t, label, *id == filter.channel.as_deref()) {
                        filter.channel = id.map(str::to_owned);
                    }
                }
            });
        });
        let period_label = periods.iter().find(|(period, _)| *period == filter.period).map(|(_, label)| *label).unwrap_or("");
        let response = audit_chip(ui, t, egui_phosphor::regular::CALENDAR_BLANK, period_label, filter.period != Period::All, true);
        crate::ui::widgets::dropdown(&response, response.rect, response.rect, true).show(|ui| {
            for (period, label) in &periods {
                if crate::ui::widgets::menu_option(ui, t, label, *period == filter.period) {
                    filter.period = *period;
                }
            }
        });
        if !filter.is_empty() {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.painter().layout_no_wrap(s.audit_clear.to_owned(), text::footnote(), t.label).size().x + space::MD * 2.0, 26.0), Sense::click());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                s.audit_clear,
                text::footnote(),
                if response.hovered() { t.label } else { t.label_tertiary },
            );
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *filter = Default::default();
            }
        }
    });
    let filter = state.draft.audit.clone();
    // Só o que o servidor sabe filtrar pede de novo; a pessoa filtra aqui.
    if filter.query(now) != before.query(now) {
        actions.push(SettingsAction::Admin(AdminAction::LoadAuditLogs(filter.query(now))));
    }
    ui.add_space(space::MD);

    let store = data.store;
    let lookup = |id: &str| store.member(id).map(|member| (member.name.clone(), member.username.clone()));
    let person = |id: &str| store.member(id).map(|member| member.name.clone());
    let channel = |id: &str| store.channel(id).map(|channel| channel.name.clone());
    let names = audit::Names { person: &person, channel: &channel };
    let shown: Vec<&crate::api::models::AuditLogEntry> =
        store.audit_logs.iter().filter(|entry| filter.keeps(entry, &lookup, now)).collect();

    if shown.is_empty() {
        ui.add_space(space::LG);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(if state.draft.loaded_audit && store.audit_logs.is_empty() && filter.is_empty() {
                    s.audit_loading
                } else {
                    s.audit_empty
                })
                .font(text::footnote())
                .color(t.label_tertiary),
            );
        });
    }
    let semibold = egui::FontId::new(text::body().size, egui::FontFamily::Name("semibold".into()));
    let mut day = None;
    for entry in &shown {
        let label = entry.created_at.map(|at| audit::day_label(at, now, *data.lang));
        if label != day {
            day = label.clone();
            if let Some(label) = label {
                ui.add_space(space::MD);
                ui.horizontal(|ui| {
                    ui.add_space(space::XS);
                    ui.label(RichText::new(label).font(text::footnote()).strong().color(t.label_secondary));
                });
                ui.add_space(space::XS);
            }
        }
        let danger = audit::is_danger(&entry.action);
        let icon = match entry.action.split('.').next().unwrap_or("") {
            _ if entry.action == "message.delete" => egui_phosphor::regular::TRASH,
            "message" | "media" => egui_phosphor::regular::CHAT_TEXT,
            "channel" => egui_phosphor::regular::HASH,
            "role" | "user_role" => egui_phosphor::regular::SHIELD,
            "user" | "auth" => egui_phosphor::regular::USER,
            "emoji" => egui_phosphor::regular::SMILEY,
            _ => egui_phosphor::regular::GEAR,
        };
        let time = entry
            .created_at
            .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_default();
        let width = ui.available_width();
        let text_left = space::MD + 26.0 + space::MD;
        let time_w = 44.0;
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = (width - text_left - time_w).max(80.0);
        for piece in audit::sentence(entry, *data.lang, &names) {
            let (words, font, color) = match piece {
                Piece::Text(words) => (words, text::body(), t.label_secondary),
                Piece::Person(words) => (words, semibold.clone(), t.label),
                Piece::Channel(words) => (words, text::body(), t.accent),
            };
            job.append(&words, 0.0, egui::TextFormat::simple(font, color));
        }
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let quote = audit::deleted_text(entry).map(|content| {
            let mut job = egui::text::LayoutJob::simple(
                format!("“{content}”"),
                text::footnote(),
                t.label_secondary,
                (width - text_left - time_w).max(80.0),
            );
            job.wrap.max_rows = 4;
            job.wrap.overflow_character = Some('…');
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        });
        let body_h = galley.size().y + quote.as_ref().map_or(0.0, |quote| quote.size().y + space::XS);
        let height = body_h.max(26.0) + space::MD * 2.0;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        if response.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
        }
        let tint = if danger { t.danger } else { t.label_secondary };
        let badge = egui::pos2(rect.min.x + space::MD + 13.0, rect.min.y + space::MD + 13.0);
        ui.painter().circle_filled(badge, 13.0, tint.gamma_multiply(if danger { 0.18 } else { 0.12 }));
        ui.painter().text(badge, egui::Align2::CENTER_CENTER, icon, text::icon(13.0), tint);
        let top = rect.min.y + space::MD + (26.0 - galley.rows.first().map_or(18.0, |row| row.height())).max(0.0) / 2.0;
        let sentence_h = galley.size().y;
        ui.painter().galley(egui::pos2(rect.min.x + text_left, top), galley, t.label);
        ui.painter().text(
            egui::pos2(rect.max.x - space::MD, badge.y),
            egui::Align2::RIGHT_CENTER,
            time,
            text::footnote(),
            t.label_tertiary,
        );
        if let Some(quote) = quote {
            let at = egui::pos2(rect.min.x + text_left, top + sentence_h + space::XS);
            ui.painter().rect_filled(
                Rect::from_min_size(egui::pos2(at.x - space::SM, at.y), Vec2::new(2.0, quote.size().y)),
                CornerRadius::same(1),
                t.separator,
            );
            ui.painter().galley(at, quote, t.label_secondary);
        }
        // O código cru continua a um hover de distância, para quem precisa.
        response.on_hover_text_at_pointer(format!("{} · {}", entry.action, entry.actor_username));
    }

    if store.audit_has_more {
        ui.add_space(space::MD);
        ui.vertical_centered(|ui| {
            if row_button(ui, t, s.audit_load_more, Emphasis::Quiet) {
                let mut query = filter.query(now);
                query.last_id = store.audit_logs.last().map(|entry| entry.id.clone());
                actions.push(SettingsAction::Admin(AdminAction::LoadAuditLogs(query)));
            }
        });
    }
    ui.add_space(space::LG);
}

/// Ficha de filtro: acesa quando restringe alguma coisa; `menu` põe a setinha.
fn audit_chip(ui: &mut egui::Ui, t: &Tokens, icon: &str, label: &str, on: bool, menu: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), text::footnote(), t.label);
    let width = space::MD + 14.0 + galley.size().x + if menu { 16.0 } else { 0.0 } + space::MD;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width.min(ui.available_width().max(80.0)), 26.0), Sense::click());
    let fill = if on {
        t.accent.gamma_multiply(0.22)
    } else if response.hovered() {
        t.fill_medium
    } else {
        t.fill_soft
    };
    let color = if on { t.accent } else { t.label_secondary };
    ui.painter().rect_filled(rect, CornerRadius::same(13), fill);
    ui.painter().text(
        egui::pos2(rect.min.x + space::MD + 6.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        icon,
        text::icon(11.0),
        color,
    );
    let right = rect.max.x - space::MD - if menu { 16.0 } else { 0.0 };
    crate::ui::widgets::text_fit(
        ui.painter(),
        egui::pos2(rect.min.x + space::MD + 14.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        text::footnote(),
        if on { t.accent } else { t.label },
        right - rect.min.x - space::MD - 14.0,
    );
    if menu {
        ui.painter().text(
            egui::pos2(rect.max.x - space::MD - 5.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::CARET_DOWN,
            text::icon(9.0),
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}


#[cfg(test)]
mod tree_tests {
    use super::tree_drop;

    fn item(id: &str, position: i32, category: bool, parent: Option<&str>) -> (String, i32, bool, Option<String>, bool) {
        (id.into(), position, category, parent.map(str::to_owned), false)
    }

    /// geral(1) · Projetos(2) [dev(3), design(4)] · voz(5)
    fn items() -> Vec<(String, i32, bool, Option<String>, bool)> {
        vec![
            item("geral", 1, false, None),
            item("proj", 2, true, None),
            item("dev", 3, false, Some("proj")),
            item("design", 4, false, Some("proj")),
            item("voz", 5, false, None),
        ]
    }

    #[test]
    fn dropping_right_under_an_open_category_puts_the_channel_in_it() {
        // geral solto logo abaixo do cabeçalho de Projetos.
        assert_eq!(tree_drop(&items(), 0, 2, 5), (2, Some("proj".into())));
    }

    #[test]
    fn dropping_between_children_keeps_the_category() {
        // voz entre dev e design.
        assert_eq!(tree_drop(&items(), 4, 3, 5), (4, Some("proj".into())));
    }

    #[test]
    fn dropping_at_the_top_takes_the_channel_out() {
        assert_eq!(tree_drop(&items(), 3, 0, 5), (1, None));
    }

    #[test]
    fn dropping_at_the_end_is_the_last_position() {
        assert_eq!(tree_drop(&items(), 0, 5, 5), (5, None));
    }

    #[test]
    fn a_category_never_lands_inside_another_category() {
        let mut list = items();
        list.push(item("arquivo", 6, true, None));
        // A categoria "arquivo" solta entre dev e design vai para antes de
        // Projetos, e sem categoria.
        assert_eq!(tree_drop(&list, 5, 3, 6), (2, None));
    }
}

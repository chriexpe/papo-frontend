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
    Activity,
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
    const ALL: [Self; 8] = [
        Self::Account,
        Self::Appearance,
        Self::Activity,
        Self::Alerts,
        Self::Language,
        Self::Files,
        Self::Sessions,
        Self::Diagnostics,
    ];

    /// Ícone do painel: o mesmo na coluna do desktop e na lista do celular.
    fn glyph(self) -> &'static str {
        use egui_phosphor::regular as icon;
        match self {
            Self::Account => icon::USER,
            Self::Activity => icon::GAME_CONTROLLER,
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
            Self::Activity => s.pane_activity,
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
    /// Temporary value while dragging the desktop UI scale slider.
    /// Applying viewport zoom mid-drag moves the slider under the pointer.
    #[cfg(not(target_os = "android"))]
    pub ui_scale_draft: Option<f32>,
    /// Rascunhos dos campos de texto, para não reescrever o estado a cada
    /// tecla digitada.
    pub draft: Draft,
    /// Quem tem acesso a cada canal, por pessoa. Só vive no app por ora: o
    /// servidor ainda não guarda isso.
    pub channel_people: std::collections::HashMap<String, Vec<String>>,
    /// Pessoas de um canal que acabou de ser pedido: casam com o id quando
    /// ele aparece na lista.
    pub pending_channel_people: Option<(String, Vec<String>)>,
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
            #[cfg(not(target_os = "android"))]
            ui_scale_draft: None,
            draft: Draft::default(),
            channel_people: std::collections::HashMap::new(),
            pending_channel_people: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChannelDraft {
    pub id: Option<String>,
    pub name: String,
    pub emoji_query: String,
    pub emoji_group: usize,
    pub topic: String,
    /// `text`, `voice` ou `category`, como o contrato espera.
    pub kind: String,
    /// Categoria escolhida no editor (só de canal que já existe) e a de
    /// origem, para saber se mudou.
    pub parent: Option<String>,
    pub original_parent: Option<String>,
    /// Cargos que têm acesso (ids) e como estavam ao abrir.
    pub access_roles: Vec<String>,
    pub original_access_roles: Vec<String>,
    /// Pessoas com acesso: por enquanto só no app, até o servidor guardar.
    pub people: Vec<String>,
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
            emoji_query: String::new(),
            emoji_group: 0,
            topic: String::new(),
            kind: "text".to_owned(),
            parent: None,
            original_parent: None,
            access_roles: Vec::new(),
            original_access_roles: Vec::new(),
            people: Vec::new(),
        }
    }

    pub fn edit(channel: &crate::state::Channel) -> Self {
        use crate::state::ChannelKind;
        Self {
            id: Some(channel.id.clone()),
            name: channel.name.clone(),
            emoji_query: String::new(),
            emoji_group: 0,
            topic: channel.topic.clone().unwrap_or_default(),
            kind: match channel.kind {
                ChannelKind::Voice => "voice".to_owned(),
                ChannelKind::Category => "category".to_owned(),
                ChannelKind::Text => "text".to_owned(),
                // DMs are not server-admin channels and never appear in this
                // settings projection.
                ChannelKind::Direct => "dm".to_owned(),
            },
            parent: channel.parent_id.clone(),
            original_parent: channel.parent_id.clone(),
            access_roles: channel.permissions.iter().filter(|entry| entry.permissions.read_channel).map(|entry| entry.role_id.clone()).collect(),
            original_access_roles: channel.permissions.iter().filter(|entry| entry.permissions.read_channel).map(|entry| entry.role_id.clone()).collect(),
            people: Vec::new(),
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
    /// A caixa de senha nova está aberta.
    pub password_open: bool,
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
    /// Host, porta e intervalo do Rich Presence enquanto são digitados.
    pub presence: Option<PresenceDraft>,
}

/// Campos do Rich Presence que reconectam: só valem quando a pessoa sai do
/// campo, e um valor inválido volta ao que estava.
#[derive(Clone, Debug, Default)]
pub struct PresenceDraft {
    pub host: String,
    pub port: String,
    pub interval: String,
}

impl PresenceDraft {
    #[cfg(not(target_os = "android"))]
    fn from_settings(settings: &crate::rich_presence::Settings) -> Self {
        Self {
            host: settings.external_host.clone(),
            port: settings.external_port.clone(),
            interval: settings.reconnect_interval.clone(),
        }
    }
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
        let mut editor = ChannelDraft::edit(channel);
        editor.people = self.channel_people.get(&channel.id).cloned().unwrap_or_default();
        self.draft.channel = Some(editor);
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

const ROW_HEIGHT: f32 = 42.0;
const ROW_INSET: f32 = space::LG;
/// Largura reservada ao controle, encostado à direita da linha.
const CONTROL_COLUMN: f32 = 290.0;
/// Linhas que mostram figurinha: a arte e a altura que ela pede.
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
    // 22 entre grupos, menos no primeiro, que já nasce no topo do corpo.
    if ui.min_rect().height() > 8.0 {
        ui.add_space(22.0);
    }
    ui.horizontal(|ui| {
        // Mesmo recuo das linhas: o título tem de nascer na mesma coluna
        // que os rótulos que ele cobre.
        ui.add_space(ROW_INSET);
        ui.label(RichText::new(label).font(px(11.5, "semibold")).color(t.label_secondary));
    });
    ui.add_space(6.0);
}

/// A largura que cabe de verdade na janela de rolagem: um texto sem quebra
/// pode esticar o `available_width` além da folha, e as linhas o seguiam.
fn fit_width(ui: &egui::Ui) -> f32 {
    let right = ui.ctx().data(|data| data.get_temp::<f32>(egui::Id::new(PANE_RIGHT_KEY)));
    match right {
        Some(right) => ui.available_width().min((right - ui.cursor().min.x).max(0.0)),
        None => ui.available_width().min((ui.clip_rect().max.x - ui.cursor().min.x).max(0.0)),
    }
}

const PANE_RIGHT_KEY: &str = "ajustes-borda-direita";

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
    // O cartão: o fundo é reservado antes e pintado depois, quando se sabe
    // a altura. No celular (peças) cada linha tem o seu.
    let card_top = ui.cursor().min.y;
    let card_w = fit_width(ui);
    let card = (!tiled).then(|| ui.painter().add(egui::Shape::Noop));
    let mut rows = Rows {
        ui,
        t: *t,
        separators: Vec::new(),
        first: true,
        lines: 1,
        tiled,
        pieces: Vec::new(),
        stack_next: false,
        no_separator_next: false,
        editing: false,
        min_height_next: ROW_HEIGHT,
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
    if let Some(slot) = card {
        let body = Rect::from_min_max(egui::pos2(rect.min.x, card_top), egui::pos2(rect.min.x + card_w, rect.max.y));
        ui.painter().set(
            slot,
            egui::epaint::RectShape::new(
                body,
                CornerRadius::same(10),
                t.card,
                Stroke::new(0.5, t.separator),
                egui::StrokeKind::Inside,
            ),
        );
    }
    for y in separators {
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x + ROW_INSET, y),
                egui::pos2(rect.min.x + card_w, y),
            ],
            Stroke::new(0.5, t.separator),
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
    /// A próxima linha nasce colada na anterior, sem o fio entre elas.
    no_separator_next: bool,
    /// O último campo desenhado está com o foco (a pessoa ainda digita).
    editing: bool,
    /// Altura mínima de uma linha especial (por exemplo, um preview de 36px).
    /// Volta ao padrão depois de consumir uma linha.
    min_height_next: f32,
}

impl Rows<'_> {
    /// Reserva a faixa de uma linha e devolve a área útil dela.
    fn band(&mut self, height: f32) -> (Rect, Rect) {
        let width = fit_width(self.ui);
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
        let joined = std::mem::take(&mut self.no_separator_next);
        if !self.first && !joined {
            self.separators.push(rect.min.y);
        }
        self.first = false;
        let inner = Rect::from_min_max(
            egui::pos2(rect.min.x + ROW_INSET, rect.min.y),
            egui::pos2(rect.max.x - ROW_INSET, rect.max.y),
        );
        (rect, inner)
    }

    /// A próxima linha empilha o controle embaixo do rótulo, mesmo com espaço.
    fn stack_next(&mut self) {
        self.stack_next = true;
    }

    /// A próxima linha continua a anterior, sem fio no meio.
    fn join_next(&mut self) {
        self.no_separator_next = true;
    }

    /// Um painel de aviso dentro do cartão: pontinhos e uma frase, como a
    /// prévia de "digitando…".
    fn typing_panel(&mut self, text: &str) {
        let t = self.t;
        let (_, inner) = self.band(52.0);
        let panel = Rect::from_min_max(egui::pos2(inner.min.x, inner.min.y), egui::pos2(inner.max.x, inner.max.y - 12.0));
        self.ui.painter().rect(panel, CornerRadius::same(10), t.elevated_bg, Stroke::new(0.5, t.separator), egui::StrokeKind::Inside);
        for i in 0..3 {
            self.ui.painter().circle_filled(egui::pos2(panel.min.x + 16.0 + i as f32 * 8.0, panel.center().y), 2.5, t.label_tertiary);
        }
        crate::ui::widgets::text_fit(
            self.ui.painter(),
            egui::pos2(panel.min.x + 14.0 + 3.0 * 8.0 + 8.0, panel.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            px(12.5, "regular"),
            t.label_secondary,
            panel.width() - 60.0,
        );
    }

    /// As regras da senha nova, uma por linha, e o botão de trocar à direita.
    fn password_rules(&mut self, rules: &[(&str, bool)], button: &str, ready: bool) -> bool {
        let t = self.t;
        let (rect, inner) = self.band(20.0 + rules.len() as f32 * 17.0);
        let mut y = rect.min.y + 10.0;
        for (label, ok) in rules {
            let ink = if *ok { t.online } else { t.label_tertiary };
            self.ui.painter().text(
                egui::pos2(inner.min.x, y),
                egui::Align2::LEFT_TOP,
                format!("{} {label}", if *ok { "●" } else { "○" }),
                px(11.5, "regular"),
                ink,
            );
            y += 17.0;
        }
        let width = self.ui.painter().layout_no_wrap(button.to_owned(), px(12.0, "regular"), t.label).size().x + 24.0;
        let slot = Rect::from_min_size(egui::pos2(inner.max.x - width, rect.max.y - 10.0 - 26.0), Vec2::new(width, 26.0));
        let response = self.ui.interact(slot, self.ui.id().with("trocar-senha-ok"), if ready { Sense::click() } else { Sense::hover() });
        let fill = if ready { if response.hovered() { t.accent.gamma_multiply(0.85) } else { t.accent } } else { t.fill_soft };
        self.ui.painter().rect_filled(slot, CornerRadius::same(6), fill);
        self.ui.painter().text(slot.center(), egui::Align2::CENTER_CENTER, button, px(12.0, "regular"), if ready { t.accent_label } else { t.label_tertiary });
        if ready && response.hovered() {
            self.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ready && response.clicked()
    }

    /// Rótulo + apoio + controle. Em espaço largo mantém duas colunas;
    /// quando o próprio painel fica estreito, empilha o controle embaixo.
    fn row(
        &mut self,
        label: &str,
        hint: Option<&str>,
        mut control: impl FnMut(&mut egui::Ui, &Tokens),
    ) {
        let t = self.t;
        let min_height = std::mem::replace(&mut self.min_height_next, ROW_HEIGHT);
        let inset = if self.tiled { TILE_INSET * 2.0 } else { ROW_INSET * 2.0 };
        let full = (fit_width(self.ui) - inset).max(120.0);
        // Em peças o controle fica à direita (chave, menu) e só os campos
        // descem para baixo do rótulo.
        let force = std::mem::take(&mut self.stack_next);
        let stacked = if self.tiled { force || full < 220.0 } else { force || full < 430.0 };
        let control_width = if stacked {
            full
        } else if self.tiled {
            (full * 0.42).min(200.0)
        } else {
            CONTROL_COLUMN.min(full * 0.48)
        };
        // O texto ocupa o que o controle deixa: mede o controle de verdade
        // (num passe invisível e sem toque) em vez de reservar uma coluna fixa.
        let natural = if stacked || self.tiled {
            control_width
        } else {
            let probe = Rect::from_min_size(egui::pos2(-10_000.0, -10_000.0), Vec2::new(control_width, 30.0));
            let mut ui = self.ui.new_child(
                UiBuilder::new()
                    .id_salt(("medida-do-controle", label))
                    .max_rect(probe)
                    .layout(Layout::right_to_left(Align::Center))
                    .sizing_pass()
                    .invisible(),
            );
            control(&mut ui, &t);
            ui.min_rect().width().clamp(0.0, control_width)
        };
        let text_width = if stacked {
            full
        } else {
            (full - natural - space::LG).max(120.0)
        };

        let label_galley = self.ui.painter().layout(
            label.to_owned(),
            px(13.0, "medium"),
            t.label,
            text_width,
        );
        let hint_galley = hint.map(|hint| {
            self.ui.painter().layout(
                hint.to_owned(),
                px(11.5, "regular"),
                t.label_secondary,
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
            (space::MD + text_height + space::SM + control_h + space::MD).max(min_height)
        } else {
            // Campo de várias linhas ao lado do rótulo: a linha cresce com ele.
            (text_height.max(if self.lines > 1 { control_h } else { 0.0 }) + 20.0)
                .max(min_height)
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
                t.label_secondary,
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
        let width = (fit_width(self.ui) - inset).max(120.0);
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



    /// Linha com um campo de texto ocupando a direita.
    fn field(&mut self, label: &str, value: &mut String, limit: usize, secret: bool) -> bool {
        self.field_hinted(label, None, value, limit, secret)
    }

    /// Campo cujo valor só vale quando a pessoa termina de digitar: devolve
    /// `true` quando ele não está com o foco, e aí o rascunho pode ser
    /// aplicado. Serve para o que reconecta ou reinicia algo a cada mudança
    /// (host e porta do Rich Presence), que não pode receber "1", "13",
    /// "133" no caminho até "1337".
    #[cfg(not(target_os = "android"))]
    fn field_settled(&mut self, label: &str, draft: &mut String, limit: usize) -> bool {
        self.field_hinted(label, None, draft, limit, false);
        !self.editing
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
        #[cfg_attr(target_os = "android", allow(unused_mut))]
        let mut editing = false;
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
                    .vertical_align(if lines > 1 { Align::Min } else { Align::Center })
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
                editing = response.has_focus();
            }
        });
        self.editing = editing;
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
fn draw_sticker(ui: &egui::Ui, t: &Tokens, rect: Rect, texture: Option<&egui::TextureHandle>) {
    match texture {
        Some(texture) => {
            super::emoji::paint_custom_image(ui.painter(), texture, rect, egui::Color32::WHITE);
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
        t.online
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
        .circle_filled(knob, SWITCH_H / 2.0 - 2.0, egui::Color32::WHITE);
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
                .layout_no_wrap((*label).to_owned(), px(13.0, "medium"), t.label)
                .size()
                .x
                + 24.0
        })
        .collect();
    let total: f32 = widths.iter().sum();
    // Sem espaço para as opções lado a lado (celular, painel estreito),
    // vira menu suspenso: o valor atual e a seta, e a lista abre por cima.
    // No celular (peças), escolha é sempre menu suspenso: o rótulo fica
    // com a linha, e o menu abre por cima do valor tocado.
    // Segmentado só para poucas opções curtas (Tema, presença); o resto é
    // menu suspenso, também no desktop — é o que não transborda.
    let short = options.len() <= 3 && options.iter().all(|(_, label)| label.chars().count() <= 18);
    if !always && (tiles(ui) || !short || total > ui.available_width() + 0.5) {
        return dropdown_choice(ui, t, current, options);
    }
    // O id vem do próprio controle, não do rótulo: numa lista de canais
    // cada linha tem os mesmos rótulos, e `ui.id().with(label)` repetia o
    // mesmo id em todas — o aviso vermelho de colisão do egui.
    let gap = 2.0;
    let total = total + gap * (options.len().saturating_sub(1)) as f32 + 4.0;
    let (rect, base) = ui.allocate_exact_size(Vec2::new(total, height), Sense::hover());
    let base = base.id;
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), t.fill_soft);

    let mut x = rect.min.x + 2.0;
    for ((value, label), width) in options.iter().zip(widths) {
        let slot = Rect::from_min_size(egui::pos2(x, rect.min.y), Vec2::new(width, height));
        x += width + gap;
        let selected = *current == *value;
        let response = ui.interact(slot, base.with(*label), Sense::click());
        if selected {
            ui.painter().rect(
                slot.shrink(2.0),
                CornerRadius::same(6),
                t.glass_opaque,
                Stroke::new(0.5, t.separator),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            if selected { px(13.0, "medium") } else { px(13.0, "regular") },
            t.label,
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
    // On desktop the menu belongs beside the value instead of covering the
    // rows underneath it. On mobile/tiled settings it stays over the tapped
    // value, where there is no side room.
    crate::ui::widgets::dropdown(&response, rect, rect, tiles(ui)).show(|ui| {
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

const SHEET_W: f32 = 880.0;
/// A altura acompanha a janela (80%) entre estes limites: Conta ficou
/// grande demais para 440, e uma folha enorme numa tela alta vira janela.
const SHEET_H_MIN: f32 = 440.0;
const SHEET_H_MAX: f32 = 620.0;
const RAIL_W: f32 = 276.0;
/// Altura de um item da coluna de painéis.
const RAIL_ENTRY_H: f32 = 46.0;
const HEADER_H: f32 = 54.0;

/// O que a folha pediu. O `app` traduz em comandos de rede; os ajustes que
/// são só locais ela mesma já escreveu em `Settings`.
#[derive(Clone, Debug)]
pub enum SettingsAction {
    Admin(super::admin::AdminAction),
    Role(super::roles::RoleAction),
    Chat(super::shell::ChatAction),
    Menu(crate::platform::menu::MenuCommand),
    RestartRichPresence,
    /// Abre o seletor de pasta do sistema para os downloads.
    PickDownloadFolder,
    /// Seletor e recorte da imagem da atividade manual.
    PickActivityImage,
    ClearActivityImage,
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
    pub ui_scale: &'a mut f32,
    pub translucency: &'a mut bool,
    /// O desfoque do vidro, quando o renderizador sabe fazê-lo.
    pub glass: Option<crate::ui::glass::SharedGlass>,
    pub notifications: &'a mut bool,
    pub reply_notifications: &'a mut bool,
    pub close_to_tray: &'a mut bool,
    pub autostart: &'a mut bool,
    pub badge: &'a mut bool,
    pub topic_reveal: &'a mut bool,
    pub open_at_newest: &'a mut bool,
    pub mark_read_at_newest: &'a mut bool,
    pub prefer_backend_embeds: &'a mut bool,
    pub record_button: &'a mut bool,
    pub channel_emoji_monochrome: &'a mut bool,
    pub self_card: &'a mut crate::ui::profile::SelfCardStyle,
    pub webembed_offscreen: &'a mut crate::webembed::OffscreenBehavior,
    pub webembed_scope: &'a mut crate::webembed::FloatScope,
    pub rich_presence: &'a mut crate::rich_presence::Settings,
    pub rich_presence_snapshot: &'a crate::rich_presence::Snapshot,
    pub ask_download: &'a mut bool,
    pub download_dir: Option<String>,
    pub diagnostics: &'a [WorkspaceDiagnostics],
    pub preview: papo_core::preview::PreviewStatsSnapshot,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    pub update_status: Option<&'a str>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    pub update_enabled: bool,
}

/// A paleta do mock de Ajustes: a folha, o cartão e as cores de texto são
/// as dele, e valem só dentro da folha do desktop.
fn sheet_tokens(base: &Tokens) -> Tokens {
    use egui::Color32;
    let dark = base.appearance == super::theme::Appearance::Dark;
    let mut t = *base;
    if dark {
        t.elevated_bg = Color32::from_rgb(0x26, 0x26, 0x29);
        t.glass_opaque = Color32::from_rgb(0x26, 0x26, 0x29);
        t.label = Color32::from_rgb(0xF2, 0xF2, 0xF5);
        t.label_secondary = Color32::from_rgb(0xA0, 0xA0, 0xAA);
        t.label_tertiary = Color32::from_rgb(0x6F, 0x6F, 0x78);
        t.separator = Color32::from_white_alpha(23);
        t.fill_soft = Color32::from_white_alpha(23);
        t.fill_medium = Color32::from_white_alpha(56);
    } else {
        t.elevated_bg = Color32::from_rgb(0xF6, 0xF6, 0xF8);
        t.glass_opaque = Color32::from_rgb(0xF6, 0xF6, 0xF8);
        t.label = Color32::from_rgb(0x1C, 0x1C, 0x1E);
        t.label_secondary = Color32::from_rgb(0x6B, 0x6B, 0x73);
        t.label_tertiary = Color32::from_rgb(0x9A, 0x9A, 0xA2);
        t.separator = Color32::from_rgba_unmultiplied(60, 60, 67, 36);
        t.fill_soft = Color32::from_rgba_unmultiplied(120, 120, 128, 31);
        t.fill_medium = Color32::from_rgba_unmultiplied(120, 120, 128, 82);
    }
    t.online = Color32::from_rgb(0x30, 0xB3, 0x50);
    t
}

/// Tamanho fixo com o peso pedido (o mock fala em px: 11.5, 14, 17).
fn px(size: f32, weight: &str) -> egui::FontId {
    let family = match weight {
        "medium" | "semibold" | "display" => egui::FontFamily::Name(weight.into()),
        _ => egui::FontFamily::Proportional,
    };
    egui::FontId::new(size, family)
}

/// A legenda de uma prévia, logo embaixo dela.
fn caption_below(ui: &mut egui::Ui, t: &Tokens, caption: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_INSET);
        ui.add(egui::Label::new(RichText::new(caption).font(px(11.5, "regular")).color(t.label_secondary)).wrap());
    });
}

/// A prévia ao vivo de Aparência: um pedaço de conversa que muda com o tema,
/// a translucidez e o emoji de uma cor, e a legenda embaixo.
fn preview_chat(ui: &mut egui::Ui, t: &Tokens, data: &mut Context<'_>, caption: &str) {
    let width = fit_width(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 133.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(10), t.elevated_bg, Stroke::new(0.5, t.separator), egui::StrokeKind::Inside);
    let left = rect.min.x + 14.0;
    let mono = *data.channel_emoji_monochrome;
    super::emoji::draw_channel_name(
        ui,
        t,
        data.media,
        data.store,
        "🎨 design-ui",
        egui::pos2(left, rect.min.y + 14.0 + 8.0),
        px(12.0, "semibold"),
        t.label_secondary,
        rect.width() - 28.0,
        mono,
        false,
    );
    let mut y = rect.min.y + 14.0 + 16.0 + 8.0;
    let messages: [(&str, egui::Color32, &str, &str, &str); 2] = [
        ("AN", egui::Color32::from_rgb(0xFF, 0x6A, 0x3D), "Ana", "14:02", "olha o render novo"),
        ("CH", egui::Color32::from_rgb(0x0A, 0x84, 0xFF), "chriexpe", "14:03", "@Ana ficou limpo, gostei."),
    ];
    for (initials, color, name, time, body) in messages {
        let avatar = Rect::from_min_size(egui::pos2(left, y), Vec2::splat(28.0));
        ui.painter().circle_filled(avatar.center(), 14.0, color);
        ui.painter().text(avatar.center(), egui::Align2::CENTER_CENTER, initials, px(11.0, "semibold"), egui::Color32::WHITE);
        let x = avatar.max.x + 9.0;
        let galley = ui.painter().layout_no_wrap(name.to_owned(), px(13.0, "semibold"), t.label);
        ui.painter().galley(egui::pos2(x, y), galley.clone(), t.label);
        ui.painter().text(egui::pos2(x + galley.size().x + 6.0, y + 2.0), egui::Align2::LEFT_TOP, time, px(11.0, "regular"), t.label_tertiary);
        let line = y + 17.0;
        if let Some(rest) = body.strip_prefix("@Ana") {
            let tag = ui.painter().layout_no_wrap("@Ana".to_owned(), px(13.0, "regular"), t.accent);
            let pill = Rect::from_min_size(egui::pos2(x, line), Vec2::new(tag.size().x + 6.0, tag.size().y + 1.0));
            ui.painter().rect_filled(pill, CornerRadius::same(4), t.accent.gamma_multiply(0.16));
            ui.painter().galley(egui::pos2(x + 3.0, line), tag, t.accent);
            ui.painter().text(egui::pos2(pill.max.x, line), egui::Align2::LEFT_TOP, rest, px(13.0, "regular"), t.label);
        } else {
            ui.painter().text(egui::pos2(x, line), egui::Align2::LEFT_TOP, body, px(13.0, "regular"), t.label);
        }
        y += 28.0 + 8.0 + 2.0;
    }
    caption_below(ui, t, caption);
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
        ctx.data_mut(|data| data.remove::<f32>(egui::Id::new(PANE_RIGHT_KEY)));
        mobile(ctx, state, data, t, s, surface, screen, &mut actions);
        return actions;
    }
    let mut sheet_t = sheet_tokens(t);
    // Vidro fosco como nas pastilhas: com o desfoque disponível a tinta pode
    // ser bem translúcida; sem ele, só um toque, para o texto do chat que
    // passa por baixo não atrapalhar a leitura.
    let frosted = *data.translucency && data.glass.is_some();
    if *data.translucency {
        let (sheet_alpha, side_alpha) = if frosted { (219, 179) } else { (244, 236) };
        let [r, g, b, _] = sheet_t.elevated_bg.to_array();
        sheet_t.elevated_bg = egui::Color32::from_rgba_unmultiplied(r, g, b, sheet_alpha);
        let [r, g, b, _] = sheet_t.side.to_array();
        sheet_t.side = egui::Color32::from_rgba_unmultiplied(r, g, b, side_alpha);
    }
    let t = &sheet_t;

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
            const SHEET_RADIUS: u8 = 14;
            super::shell::glass_blur(ui, &data.glass, *data.translucency, rect, SHEET_RADIUS as f32);
            ui.painter().rect(
                rect,
                CornerRadius::same(SHEET_RADIUS),
                t.elevated_bg,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            );

            // Coluna lateral de cima a baixo, mais escura que a folha.
            let rail_rect = Rect::from_min_size(rect.min, Vec2::new(RAIL_W, rect.height()));
            ui.painter().rect_filled(
                rail_rect,
                CornerRadius { nw: SHEET_RADIUS, sw: SHEET_RADIUS, ne: 0, se: 0 },
                t.side,
            );
            ui.painter().line_segment(
                [egui::pos2(rail_rect.max.x, rect.min.y + 1.0), egui::pos2(rail_rect.max.x, rect.max.y - 1.0)],
                Stroke::new(1.0, t.separator),
            );
            rail(ui, state, data, t, s, rail_rect, surface);

            // À direita: o título do painel e o X, e o corpo logo abaixo.
            let main = Rect::from_min_max(egui::pos2(rail_rect.max.x, rect.min.y), rect.max);
            let title = match surface {
                Surface::App if state.app_pane == AppPane::Account => s.profile,
                Surface::App => state.app_pane.title(s),
                Surface::Server => state.server_pane.title(s),
            };
            header(ui, state, t, s, main, surface, title);
            let pane_rect = Rect::from_min_max(egui::pos2(main.min.x, main.min.y + HEADER_H), main.max);
            ctx.data_mut(|data| data.insert_temp(egui::Id::new(PANE_RIGHT_KEY), main.max.x - 24.0));
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(pane_rect.shrink2(Vec2::new(0.0, 1.0)))
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    // Um rolamento por painel: trocar de painel começa do
                    // topo (com a prévia), não onde o anterior parou. A barra
                    // fica na borda da folha; o miolo é que tem a margem.
                    egui::ScrollArea::vertical()
                        .id_salt(("corpo-do-painel", title))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin { left: 24, right: 24, top: 4, bottom: 28 })
                                .show(ui, |ui| match surface {
                                    Surface::App => app_pane(ui, state, data, t, s, &mut actions),
                                    Surface::Server => server_pane(ui, state, data, t, s, &mut actions),
                                });
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
    role_list(ui, data, t, s);
    role_detail(ui, data, t, s, actions);
}

fn role_list(ui: &mut egui::Ui, data: &mut Context<'_>, t: &Tokens, s: &Strings) {
    use super::roles::{Draft, Tab};
    let roles = data.store.roles.clone();
    let mut new_role = false;
    // A barra do topo: o nome da lista e o botão de criar, como no mock.
    ui.horizontal(|ui| {
        ui.add_space(ROW_INSET);
        ui.label(RichText::new(s.roles).font(px(11.5, "semibold")).color(t.label_secondary));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if row_button(ui, t, s.new_role, Emphasis::Quiet) {
                new_role = true;
            }
        });
    });
    ui.add_space(10.0);
    if roles.is_empty() {
        ui.add(egui::Label::new(RichText::new(s.no_roles).font(text::footnote()).color(t.label_tertiary)).wrap());
    }
    let selected_id = data.roles.draft.as_ref().and_then(|draft| draft.id.clone());
    let width = fit_width(ui);
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    let card_slot = ui.painter().add(egui::Shape::Noop);
    let card_top = ui.cursor().min.y;
    let last = roles.len().saturating_sub(1);
    for (position, role) in roles.iter().enumerate() {
        let count = data.store.members.iter().filter(|m| m.roles.iter().any(|id| id == &role.id)).count();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 42.0), Sense::click());
        let selected = selected_id.as_deref() == Some(role.id.as_str());
        if selected || response.hovered() {
            let top = if position == 0 { 10 } else { 0 };
            let bottom = if position == last { 10 } else { 0 };
            ui.painter().rect_filled(
                rect,
                CornerRadius { nw: top, ne: top, sw: bottom, se: bottom },
                if selected { t.accent.gamma_multiply(0.16) } else { t.fill_soft },
            );
        }
        if position > 0 {
            ui.painter().line_segment(
                [egui::pos2(rect.min.x + ROW_INSET, rect.min.y), egui::pos2(rect.max.x, rect.min.y)],
                Stroke::new(0.5, t.separator),
            );
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
        ui.painter().circle_filled(egui::pos2(rect.min.x + 12.0 + 5.0, rect.center().y), 4.0, color);
        let count_text = format!("{count} {}", if count == 1 { s.person_one } else { s.count_members });
        let count_w = ui.painter().layout_no_wrap(count_text.clone(), px(13.0, "regular"), t.label_secondary).size().x;
        ui.painter().text(egui::pos2(rect.max.x - 12.0, rect.center().y), egui::Align2::RIGHT_CENTER, count_text, px(13.0, "regular"), t.label_secondary);
        let left = rect.min.x + 12.0 + 10.0 + 16.0;
        crate::ui::widgets::text_fit(
            ui.painter(),
            egui::pos2(left, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &role.name,
            px(13.0, "regular"),
            t.label,
            rect.max.x - 12.0 - count_w - 8.0 - left,
        );
        if response.clicked() {
            let people: Vec<String> = data
                .store
                .members
                .iter()
                .filter(|member| member.roles.iter().any(|id| id == &role.id))
                .map(|member| member.id.clone())
                .collect();
            data.roles.draft = Some(Draft {
                id: Some(role.id.clone()),
                name: role.name.clone(),
                color: role.color.clone().unwrap_or_default(),
                permissions: role.permissions,
                original_people: people.clone(),
                people,
            });
            data.roles.member_query.clear();
        }
    }
    let bottom = ui.cursor().min.y;
    if bottom > card_top {
        ui.painter().set(
            card_slot,
            egui::epaint::RectShape::new(
                Rect::from_min_max(egui::pos2(ui.max_rect().min.x, card_top), egui::pos2(ui.max_rect().min.x + width, bottom)),
                CornerRadius::same(10),
                t.card,
                Stroke::new(0.5, t.separator),
                egui::StrokeKind::Inside,
            ),
        );
    }
    ui.spacing_mut().item_spacing.y = spacing;
    if new_role {
        // Cargo novo começa pelo nome e pela cor.
        data.roles.draft = Some(Draft::default());
        data.roles.tab = Tab::Look;
    }
    ui.add_space(22.0);
}

/// O campo do nome do canal, com o seletor de emoji embutido à esquerda.
fn channel_name_input(
    ui: &mut egui::Ui,
    t: &Tokens,
    editor: &mut ChannelDraft,
    store: &Store,
    media: &mut crate::media::MediaStore,
    s: &Strings,
) {
            let width = ui.available_width();
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::hover());
            ui.painter().rect(
                rect,
                CornerRadius::same(radius::FIELD),
                t.fill_soft,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            );

            let picker_rect =
                Rect::from_min_size(rect.min, Vec2::new(30.0, rect.height()));
            let picker = ui.interact(
                picker_rect,
                ui.id().with((
                    "settings-channel-name-emoji",
                    editor.id.as_deref().unwrap_or("new"),
                )),
                Sense::click(),
            );
            if picker.hovered() {
                ui.painter().rect_filled(
                    picker_rect.shrink(2.0),
                    CornerRadius::same(radius::CONTROL),
                    t.fill_medium,
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            ui.painter().text(
                picker_rect.center(),
                egui::Align2::CENTER_CENTER,
                egui_phosphor::regular::SMILEY,
                text::icon(14.0),
                if picker.hovered() {
                    t.label
                } else {
                    t.label_tertiary
                },
            );
            ui.painter().line_segment(
                [
                    egui::pos2(picker_rect.max.x, rect.min.y + 4.0),
                    egui::pos2(picker_rect.max.x, rect.max.y - 4.0),
                ],
                Stroke::new(1.0, t.separator),
            );

            let field_rect = Rect::from_min_max(
                egui::pos2(picker_rect.max.x + 1.0, rect.min.y),
                rect.max,
            );
            let edit_id = ui.id().with((
                "settings-channel-name",
                editor.id.as_deref().unwrap_or("new"),
            ));

            #[cfg(target_os = "android")]
            {
                let key = format!("settings:{edit_id:?}");
                let _ = crate::platform::native_field::show(
                    ui.ctx(),
                    &key,
                    &mut editor.name,
                    field_rect.shrink2(Vec2::new(space::MD, space::XS)),
                    "",
                    crate::platform::native_field::Mode::Text,
                    32,
                    false,
                    t.label,
                    t.label_tertiary,
                    text::body().size,
                );
            }

            #[cfg(not(target_os = "android"))]
            {
                let _ =
                    crate::platform::ime::prepare_text_edit(ui.ctx(), edit_id, &mut editor.name);
                let response = ui.put(
                    field_rect,
                    egui::TextEdit::singleline(&mut editor.name)
                        .id(edit_id)
                        .char_limit(32)
                        .font(text::body())
                        .vertical_align(Align::Center)
                        .margin(egui::Margin::symmetric(
                            space::MD as i8,
                            space::XS as i8,
                        )),
                );
                let _ = crate::platform::ime::sync_text_edit(
                    ui.ctx(),
                    edit_id,
                    &mut editor.name,
                    response.has_focus(),
                    crate::platform::ime::Kind::Text,
                );
            }

            let mut chosen = None;
            egui::Popup::menu(&picker).frame(crate::ui::widgets::menu_frame(ui.ctx())).show(|ui| {
                ui.set_max_height(380.0);
                chosen = super::emoji::picker(
                    ui,
                    t,
                    s,
                    store,
                    media,
                    &mut editor.emoji_query,
                    &mut editor.emoji_group,
                    false,
                );
                if chosen.is_some() {
                    ui.close();
                }
            });

            if let Some(chosen) = chosen {
                let prefix = match chosen {
                    crate::state::Emoji::Unicode(glyph) => glyph,
                    crate::state::Emoji::Custom(id) => store
                        .emojis
                        .iter()
                        .find(|custom| custom.id == id)
                        .map(|custom| format!(":{}:", custom.name))
                        .unwrap_or_default(),
                };
                if !prefix.is_empty() {
                    let next = if editor.name.trim().is_empty() {
                        format!("{prefix} ")
                    } else {
                        format!("{prefix} {}", editor.name.trim_start())
                    };
                    if next.chars().count() <= 32 {
                        editor.name = next;
                    }
                }
            }
}

// ---------------------------------------------------------------------------
// Peças dos editores (cartão com seções, fichas, seletor com busca)
// ---------------------------------------------------------------------------

/// O cartão do editor: seções separadas por fio, cada uma com a legenda em
/// versalete em cima.
struct CardUi<'a> {
    ui: &'a mut egui::Ui,
    t: Tokens,
    first: bool,
    width: f32,
}

impl CardUi<'_> {
    fn section(&mut self, caption: &str, contents: impl FnOnce(&mut egui::Ui)) {
        let t = self.t;
        if !self.first {
            let y = self.ui.cursor().min.y;
            let left = self.ui.max_rect().min.x;
            self.ui.painter().line_segment(
                [egui::pos2(left, y), egui::pos2(left + self.width, y)],
                Stroke::new(0.5, t.separator),
            );
        }
        self.first = false;
        self.ui.add_space(12.0);
        let inner = self.width - 36.0;
        self.ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.set_width(inner);
                ui.spacing_mut().item_spacing.y = 0.0;
                if !caption.is_empty() {
                    ui.label(RichText::new(caption.to_uppercase()).font(px(10.0, "semibold")).color(t.label_tertiary));
                    ui.add_space(6.0);
                }
                contents(ui);
            });
        });
        self.ui.add_space(14.0);
    }
}

fn sheet_card(ui: &mut egui::Ui, t: &Tokens, build: impl FnOnce(&mut CardUi)) {
    let width = fit_width(ui);
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    let slot = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().min.y;
    let left = ui.max_rect().min.x;
    {
        let mut card = CardUi { ui, t: *t, first: true, width };
        build(&mut card);
    }
    let bottom = ui.cursor().min.y;
    ui.painter().set(
        slot,
        egui::epaint::RectShape::new(
            Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + width, bottom)),
            CornerRadius::same(10),
            t.card,
            Stroke::new(0.5, t.separator),
            egui::StrokeKind::Inside,
        ),
    );
    ui.spacing_mut().item_spacing.y = spacing;
}

/// Um item de seletor: nome, bolinha de cor ou foto com iniciais.
struct PickItem {
    label: String,
    dot: Option<egui::Color32>,
    avatar: Option<(String, egui::Color32)>,
}

fn mix(base: egui::Color32, over: egui::Color32, amount: f32) -> egui::Color32 {
    let a = |x: u8, y: u8| (x as f32 * (1.0 - amount) + y as f32 * amount).round() as u8;
    egui::Color32::from_rgb(a(base.r(), over.r()), a(base.g(), over.g()), a(base.b(), over.b()))
}

fn mini_avatar(painter: &egui::Painter, center: egui::Pos2, radius: f32, initials: &str, color: egui::Color32) {
    painter.circle_filled(center, radius, color);
    painter.text(center, egui::Align2::CENTER_CENTER, initials, px(radius * 1.0, "semibold"), egui::Color32::WHITE);
}

/// Uma ficha: cargo (com a cor), pessoa (com a foto) ou "Todos". Devolve
/// `true` quando o X é clicado.
fn chip(ui: &mut egui::Ui, t: &Tokens, id: &str, label: &str, dot: Option<egui::Color32>, avatar: Option<(&str, egui::Color32)>, removable: bool) -> bool {
    let font = px(12.0, "regular");
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), t.label).size().x;
    let lead = if dot.is_some() { 14.0 } else if avatar.is_some() { 22.0 } else { 0.0 };
    let width = 10.0 + lead + text_w + if removable { 6.0 + 16.0 + 4.0 } else { 10.0 };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    let fill = match dot {
        Some(color) => mix(t.card, color, 0.20),
        None => t.fill_soft,
    };
    ui.painter().rect_filled(rect, CornerRadius::same(12), fill);
    let mut x = rect.min.x + 10.0;
    if let Some(color) = dot {
        ui.painter().circle_filled(egui::pos2(x + 4.0, rect.center().y), 4.0, color);
        x += 14.0;
    }
    if let Some((initials, color)) = avatar {
        mini_avatar(ui.painter(), egui::pos2(x + 6.0, rect.center().y), 8.0, initials, color);
        x += 22.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, label, font, t.label);
    if !removable {
        return false;
    }
    let cross = Rect::from_center_size(egui::pos2(rect.max.x - 4.0 - 8.0, rect.center().y), Vec2::splat(16.0));
    let response = ui.interact(cross, egui::Id::new(("ficha-x", id)), Sense::click());
    if response.hovered() {
        ui.painter().circle_filled(cross.center(), 8.0, t.fill_medium);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(cross.center(), egui::Align2::CENTER_CENTER, egui_phosphor::regular::X, text::icon(9.0), t.label_secondary);
    response.clicked()
}

/// O botão "+ Cargo" / "+ Pessoa" que abre o seletor.
fn add_chip(ui: &mut egui::Ui, t: &Tokens, label: &str) -> egui::Response {
    let font = px(12.0, "regular");
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), t.label).size().x;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(10.0 + 14.0 + text_w + 12.0, 24.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(12), t.fill_soft);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(egui::pos2(rect.min.x + 10.0, rect.center().y), egui::Align2::LEFT_CENTER, egui_phosphor::regular::PLUS, text::icon(11.0), t.accent);
    ui.painter().text(egui::pos2(rect.min.x + 10.0 + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, label, font, t.accent);
    response
}

/// Campo de uma linha dentro de um retângulo, com a integração certa de cada
/// plataforma: campo nativo no Android (teclado e IME de verdade) e `TextEdit`
/// com a ponte do IME no desktop. Devolve se está com o foco.
#[allow(clippy::too_many_arguments)]
fn text_input(
    ui: &mut egui::Ui,
    t: &Tokens,
    key: &str,
    value: &mut String,
    rect: Rect,
    hint: &str,
    font: egui::FontId,
    limit: usize,
    focus: bool,
) -> bool {
    #[cfg(target_os = "android")]
    {
        crate::platform::native_field::show(
            ui.ctx(),
            key,
            value,
            rect,
            hint,
            crate::platform::native_field::Mode::Search,
            limit,
            focus,
            t.label,
            t.label_tertiary,
            font.size,
        )
        .focused
    }
    #[cfg(not(target_os = "android"))]
    {
        let id = ui.id().with(key);
        let _ = crate::platform::ime::prepare_text_edit(ui.ctx(), id, value);
        let response = ui.put(
            rect,
            egui::TextEdit::singleline(value)
                .id(id)
                .char_limit(if limit == 0 { usize::MAX } else { limit })
                .hint_text(RichText::new(hint).font(font.clone()))
                .frame(egui::Frame::NONE)
                .font(font)
                .text_color(t.label)
                .vertical_align(Align::Center),
        );
        let _ = crate::platform::ime::sync_text_edit(ui.ctx(), id, value, response.has_focus(), crate::platform::ime::Kind::Text);
        if focus {
            response.request_focus();
        }
        response.has_focus()
    }
}

/// Campo de busca: o campo de texto da plataforma mais o X redondo que limpa.
/// `field` é a caixa inteira (o fundo já foi pintado) e `left` é onde termina
/// a lupa. Limpar pelo app precisa forçar o campo nativo a aceitar o texto
/// novo, porque o Android é quem manda enquanto o campo tem o foco.
#[allow(clippy::too_many_arguments)]
fn search_input(
    ui: &mut egui::Ui,
    t: &Tokens,
    key: &str,
    value: &mut String,
    field: Rect,
    left: f32,
    hint: &str,
    font: egui::FontId,
    focus: bool,
) {
    let ctx = ui.ctx().clone();
    let force_id = egui::Id::new(("limpar-busca-pendente", key));
    let force = ctx.data_mut(|data| data.remove_temp::<bool>(force_id)).unwrap_or(false);
    let has_text = !value.is_empty();
    let right = if has_text { 30.0 } else { 8.0 };
    let input = Rect::from_min_max(egui::pos2(field.min.x + left, field.min.y), egui::pos2(field.max.x - right, field.max.y));
    text_input(ui, t, key, value, input, hint, font, 0, focus || force);
    if has_text {
        let cross = Rect::from_center_size(egui::pos2(field.max.x - 8.0 - 9.0, field.center().y), Vec2::splat(18.0));
        let response = ui.interact(cross, egui::Id::new(("limpar-busca", key)), Sense::click());
        ui.painter().circle_filled(cross.center(), 9.0, t.fill_medium);
        ui.painter().text(cross.center(), egui::Align2::CENTER_CENTER, egui_phosphor::regular::X, text::icon(10.0), t.label);
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            value.clear();
            ctx.data_mut(|data| data.insert_temp(force_id, true));
            ctx.request_repaint();
        }
    }
}

/// O seletor com busca que abre do botão: devolve o índice escolhido.
///
/// É uma camada própria (Area), com fundo opaco: ela fica por cima, segura o
/// toque e a rolagem para si, e a busca usa o campo certo de cada plataforma.
fn picker_menu(response: &egui::Response, t: &Tokens, placeholder: &str, empty: &str, items: &[PickItem]) -> Option<usize> {
    let ctx = response.ctx.clone();
    let open_id = response.id.with("aberto");
    let query_id = response.id.with("busca");
    let mut open: bool = ctx.data(|data| data.get_temp(open_id)).unwrap_or(false);
    let mut just_opened = false;
    if response.clicked() {
        open = !open;
        just_opened = open;
        ctx.data_mut(|data| data.insert_temp(query_id, String::new()));
    }
    if !open {
        ctx.data_mut(|data| data.insert_temp(open_id, false));
        return None;
    }

    let mut query: String = ctx.data(|data| data.get_temp(query_id)).unwrap_or_default();
    let needle = query.trim().to_lowercase();
    let matching: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| needle.is_empty() || item.label.to_lowercase().contains(&needle))
        .map(|(index, _)| index)
        .collect();
    // O tamanho e o lado são decididos uma vez, na abertura: se o seletor
    // encolhesse ou trocasse de lado a cada letra, o campo de busca nativo
    // sairia do lugar a cada toque no teclado.
    let max_list = if cfg!(target_os = "android") { 150.0 } else { 210.0 };
    let list_h = (items.len().max(1) as f32 * 30.0).min(max_list);
    let size = Vec2::new(264.0, 8.0 + 34.0 + 6.0 + list_h + 8.0);
    let screen = ctx.content_rect();
    let side_id = response.id.with("lado");
    if just_opened {
        // Embaixo do botão quando ele está na metade de cima da tela; senão
        // por cima (o teclado cobre a parte de baixo).
        let above = response.rect.center().y > screen.center().y;
        ctx.data_mut(|data| data.insert_temp(side_id, above));
    }
    let above: bool = ctx.data(|data| data.get_temp(side_id)).unwrap_or(false);
    let mut pos = egui::pos2(
        response.rect.min.x,
        if above { response.rect.min.y - 6.0 - size.y } else { response.rect.max.y + 6.0 },
    );
    pos.y = pos.y.clamp(screen.min.y + 8.0, (screen.max.y - size.y - 8.0).max(screen.min.y + 8.0));
    pos.x = pos.x.min(screen.max.x - size.x - 8.0).max(screen.min.x + 8.0);
    let rect = Rect::from_min_size(pos, size);

    let mut chosen = None;
    let area = egui::Area::new(response.id.with("camada"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            let (_, _) = ui.allocate_exact_size(size, Sense::click_and_drag());
            // Fundo opaco, sombra e fio: nada de texto aparecendo por trás.
            ui.painter().add(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(90) }.as_shape(rect, CornerRadius::same(12)));
            ui.painter().rect(rect, CornerRadius::same(12), t.card.to_opaque(), Stroke::new(0.5, t.separator), egui::StrokeKind::Inside);
            let field = Rect::from_min_size(egui::pos2(rect.min.x + 8.0, rect.min.y + 8.0), Vec2::new(rect.width() - 16.0, 34.0));
            ui.painter().rect_filled(field, CornerRadius::same(8), t.fill_soft);
            ui.painter().text(egui::pos2(field.min.x + 12.0, field.center().y), egui::Align2::LEFT_CENTER, egui_phosphor::regular::MAGNIFYING_GLASS, text::icon(13.0), t.label_tertiary);
            search_input(ui, t, "busca", &mut query, field, 30.0, placeholder, px(13.0, "regular"), just_opened);
            let list = Rect::from_min_size(egui::pos2(rect.min.x + 8.0, field.max.y + 6.0), Vec2::new(rect.width() - 16.0, list_h));
            let mut inner = ui.new_child(UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
            egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show(&mut inner, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if matching.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(empty).font(px(12.0, "regular")).color(t.label_tertiary));
                }
                for index in &matching {
                    let item = &items[*index];
                    let (row_rect, row) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                    if row.hovered() {
                        ui.painter().rect_filled(row_rect, CornerRadius::same(8), t.fill_soft);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let mut x = row_rect.min.x + 8.0;
                    if let Some(color) = item.dot {
                        ui.painter().circle_filled(egui::pos2(x + 4.0, row_rect.center().y), 4.0, color);
                        x += 16.0;
                    }
                    if let Some((initials, color)) = &item.avatar {
                        mini_avatar(ui.painter(), egui::pos2(x + 10.0, row_rect.center().y), 10.0, initials, *color);
                        x += 28.0;
                    }
                    crate::ui::widgets::text_fit(ui.painter(), egui::pos2(x, row_rect.center().y), egui::Align2::LEFT_CENTER, &item.label, px(13.0, "regular"), t.label, row_rect.max.x - x - 8.0);
                    if row.clicked() {
                        chosen = Some(*index);
                    }
                }
            });
        });
    ctx.move_to_top(area.response.layer_id);

    // Fecha ao escolher, com Esc ou com um toque fora (do seletor e do botão).
    let outside = ctx.input(|input| {
        input.pointer.any_click()
            && input.pointer.interact_pos().is_some_and(|pos| !rect.contains(pos) && !response.rect.contains(pos))
    });
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if chosen.is_some() || (outside && !just_opened) || escape {
        open = false;
        ctx.data_mut(|data| data.remove::<String>(query_id));
    } else {
        ctx.data_mut(|data| data.insert_temp(query_id, query));
    }
    ctx.data_mut(|data| data.insert_temp(open_id, open));
    chosen
}

fn person_tint(store: &Store, member: &crate::state::Member, accent: egui::Color32) -> egui::Color32 {
    let _ = store;
    member.role_color.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)).unwrap_or(accent)
}

/// A pastilha de salvar dos editores: o motivo quando ainda não dá.
#[allow(clippy::too_many_arguments)]
fn save_pill(ctx: &egui::Context, id: &str, t: &Tokens, translucent: bool, glass: &Option<crate::ui::glass::SharedGlass>, s: &Strings, label: &str, blocked: Option<&str>) -> Option<bool> {
    let mut result = None;
    let pill_id = egui::Id::new(id);
    crate::ui::widgets::floating_pill(ctx, pill_id, t, translucent, glass, space::LG, 420.0, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            ui.label(RichText::new(blocked.unwrap_or(s.unsaved_changes)).font(text::body()).color(t.label_secondary));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if blocked.is_none() && row_button(ui, t, label, Emphasis::Primary) {
                    result = Some(true);
                }
                if row_button(ui, t, s.discard, Emphasis::Quiet) {
                    result = Some(false);
                }
            });
        });
    });
    ctx.move_to_top(egui::LayerId::new(egui::Order::Foreground, pill_id));
    result
}

/// O cargo aberto, como no mock: nome, cor, permissões, pessoas e excluir,
/// num cartão só; a pastilha confirma.
fn role_detail(ui: &mut egui::Ui, data: &mut Context<'_>, t: &Tokens, s: &Strings, actions: &mut Vec<SettingsAction>) {
    use super::roles::RoleAction;
    let ctx = ui.ctx().clone();
    let translucent = *data.translucency;
    let glass = data.glass.clone();
    let Some(draft) = data.roles.draft.as_mut() else {
        return;
    };
    let stored = draft.id.as_ref().and_then(|id| data.store.roles.iter().find(|role| &role.id == id));
    let dirty = match stored {
        Some(role) => {
            draft.name.trim() != role.name.trim()
                || draft.color.to_lowercase() != role.color.clone().unwrap_or_default().to_lowercase()
                || draft.permissions != role.permissions
                || draft.people != draft.original_people
        }
        None => true,
    };
    section(ui, t, s.edit_role_title);
    const COLORS: [&str; 8] = ["#ff6a3d", "#f5b50a", "#30b350", "#38bdf8", "#0a84ff", "#7c5cff", "#ff4f8b", "#9a9aa2"];
    let mut delete = false;
    let store = data.store;
    sheet_card(ui, t, |card| {
        card.section(s.role_name, |ui| {
            ui.horizontal(|ui| {
                let color = crate::api::models::parse_hex_color(&draft.color)
                    .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
                    .unwrap_or(t.label_tertiary);
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 7.0, color);
                ui.add_space(4.0);
                crate::ui::profile::inline_field(ui, t, "cargo-nome", &mut draft.name, s.role_name, px(13.0, "regular"), t.label, 32, (1, 1));
            });
        });
        card.section(s.role_color, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(10.0);
                for hex in COLORS {
                    let color = crate::api::models::parse_hex_color(hex)
                        .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
                        .unwrap_or(t.label_tertiary);
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                    ui.painter().circle_filled(rect.center(), 12.0, color);
                    let chosen = draft.color.eq_ignore_ascii_case(hex);
                    if chosen {
                        ui.painter().circle_stroke(rect.center(), 15.0, Stroke::new(2.0, t.accent));
                    }
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if response.clicked() {
                        draft.color = hex.to_owned();
                    }
                }
            });
        });
        card.section(s.role_tab_permissions, |ui| {
            let list = [
                (s.perm_manage_server, &mut draft.permissions.manage_server),
                (s.perm_manage_channels, &mut draft.permissions.manage_channels),
                (s.perm_manage_roles, &mut draft.permissions.manage_roles),
                (s.perm_ban_members, &mut draft.permissions.ban_members),
                (s.perm_pin_message, &mut draft.permissions.pin_message),
                (s.perm_everyone_message, &mut draft.permissions.everyone_message),
                (s.perm_send_attachment, &mut draft.permissions.send_attachment),
            ];
            let width = ui.available_width();
            let count = list.len();
            for (index, (label, value)) in list.into_iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::hover());
                ui.painter().text(egui::pos2(rect.min.x, rect.center().y), egui::Align2::LEFT_CENTER, label, px(13.0, "medium"), t.label);
                let slot = Rect::from_min_size(egui::pos2(rect.max.x - SWITCH_W, rect.center().y - SWITCH_H / 2.0), Vec2::new(SWITCH_W, SWITCH_H));
                let mut child = ui.new_child(UiBuilder::new().max_rect(slot));
                switch(&mut child, t, value);
                if index + 1 < count {
                    ui.painter().line_segment([egui::pos2(rect.min.x, rect.max.y), egui::pos2(rect.max.x, rect.max.y)], Stroke::new(0.5, t.separator));
                }
            }
        });
        {
            card.section(s.people_caption, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                    if draft.people.is_empty() {
                        ui.label(RichText::new(s.nobody_yet).font(px(12.0, "regular")).color(t.label_secondary));
                    }
                    let mut remove = None;
                    for id in &draft.people {
                        if let Some(member) = store.member(id) {
                            let tint = person_tint(store, member, t.accent);
                            if chip(ui, t, &format!("cargo-pessoa-{id}"), &member.name, None, Some((&member.initials(), tint)), true) {
                                remove = Some(id.clone());
                            }
                        }
                    }
                    if let Some(id) = remove {
                        draft.people.retain(|x| *x != id);
                    }
                    let button = add_chip(ui, t, s.add_person);
                    let candidates: Vec<&crate::state::Member> = store.members.iter().filter(|m| !draft.people.contains(&m.id)).collect();
                    let items: Vec<PickItem> = candidates
                        .iter()
                        .map(|m| PickItem { label: m.name.clone(), dot: None, avatar: Some((m.initials(), person_tint(store, m, t.accent))) })
                        .collect();
                    if let Some(index) = picker_menu(&button, t, s.search_person, s.nothing_to_add, &items) {
                        draft.people.push(candidates[index].id.clone());
                    }
                });
            });
        }
        if draft.id.is_some() {
            card.section("", |ui| {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.painter().layout_no_wrap(s.delete_role.to_owned(), px(12.0, "regular"), t.danger).size().x + 24.0, 26.0), Sense::click());
                ui.painter().rect_filled(rect, CornerRadius::same(6), t.danger.gamma_multiply(if response.hovered() { 0.24 } else { 0.14 }));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, s.delete_role, px(12.0, "regular"), t.danger);
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() {
                    delete = true;
                }
            });
        }
    });

    let color = (!draft.color.trim().is_empty()).then(|| draft.color.trim().to_owned());
    if delete {
        if let Some(role_id) = draft.id.clone() {
            actions.push(SettingsAction::Role(RoleAction::Delete(role_id)));
        }
        return;
    }
    if dirty {
        let blocked = draft.name.trim().is_empty().then_some(s.role_name_required);
        match save_pill(&ctx, "cargo-salvar", t, translucent, &glass, s, s.save_role, blocked) {
            Some(true) => {
                match draft.id.clone() {
                    Some(role_id) => {
                        actions.push(SettingsAction::Role(RoleAction::Update { role_id: role_id.clone(), name: draft.name.trim().to_owned(), color, permissions: draft.permissions }));
                        for user_id in draft.people.iter().filter(|id| !draft.original_people.contains(id)) {
                            actions.push(SettingsAction::Role(RoleAction::Assign { user_id: user_id.clone(), role_id: role_id.clone() }));
                        }
                        for user_id in draft.original_people.iter().filter(|id| !draft.people.contains(id)) {
                            actions.push(SettingsAction::Role(RoleAction::Unassign { user_id: user_id.clone(), role_id: role_id.clone() }));
                        }
                        draft.original_people = draft.people.clone();
                    }
                    None => actions.push(SettingsAction::Role(RoleAction::Create { name: draft.name.trim().to_owned(), color, permissions: draft.permissions, people: draft.people.clone() })),
                }
            }
            Some(false) => data.roles.draft = None,
            None => {}
        }
    }
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

/// O pedido de mover `items[dragged]` para a fresta `gap`, se isso muda algo.
fn move_action(
    items: &[(String, i32, bool, Option<String>, bool)],
    dragged: usize,
    gap: usize,
    total: i32,
) -> Option<TreeAction> {
    let (new_position, parent) = tree_drop(items, dragged, gap, total);
    let (id, old_position, _, old_parent, _) = &items[dragged];
    let parent_change = (parent != *old_parent).then(|| parent.clone().unwrap_or_default());
    (new_position != *old_position || parent_change.is_some()).then(|| TreeAction::Move {
        channel_id: id.clone(),
        old_position: *old_position,
        new_position,
        parent_id: parent_change,
    })
}

/// Os quatro botões de uma linha: subir, descer, editar e apagar. Aparecem
/// com o ponteiro em cima e, no toque, ficam sempre à mostra.
fn row_tools(
    ui: &mut egui::Ui,
    t: &Tokens,
    id: &str,
    rect: Rect,
    can_up: bool,
    can_down: bool,
    show: bool,
) -> Option<usize> {
    use egui_phosphor::regular as icon;
    let side = 28.0;
    let gap = 4.0;
    let mut right = rect.max.x - 12.0;
    let mut hit = None;
    for (glyph, danger, which) in [
        (icon::TRASH, true, 3),
        (icon::PENCIL_SIMPLE, false, 2),
        (icon::ARROW_DOWN, false, 1),
        (icon::ARROW_UP, false, 0),
    ] {
        let enabled = match which {
            0 => can_up,
            1 => can_down,
            _ => true,
        };
        let tool = Rect::from_min_size(egui::pos2(right - side, rect.center().y - side / 2.0), Vec2::splat(side));
        right -= side + gap;
        // O espaço é reservado mesmo escondido: o texto não muda de largura.
        let response = ui.interact(tool, egui::Id::new(("ferramenta", id, which)), if show && enabled { Sense::click() } else { Sense::hover() });
        if !show {
            continue;
        }
        if response.hovered() && enabled {
            ui.painter().rect_filled(tool, CornerRadius::same(8), t.fill_soft);
        }
        ui.painter().text(
            tool.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            text::icon(15.0),
            if danger && response.hovered() { t.danger } else { t.label_secondary }.gamma_multiply(if enabled { 1.0 } else { 0.3 }),
        );
        if response.clicked() {
            hit = Some(which);
        }
    }
    hit
}

/// Categorias e canais como no mock: cada categoria é um cartão com o nome
/// em cima, a alça para arrastar à esquerda e as ações à direita.
/// Soltar reordena (e troca de categoria) num pedido só.
fn channel_tree(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    store: &Store,
    media: &mut crate::media::MediaStore,
    channel_emoji_monochrome: bool,
    _collapsed: &mut std::collections::HashSet<String>,
    dragging: &mut Option<String>,
) -> Option<TreeAction> {
    use egui_phosphor::regular as icon;
    let layout = store.channel_layout();
    let total = layout.len() as i32;
    let visible: Vec<(&crate::state::Channel, Option<String>)> = layout
        .iter()
        .map(|(index, parent)| (&store.channels[*index], parent.clone()))
        .collect();
    let items: Vec<(String, i32, bool, Option<String>, bool)> = visible
        .iter()
        .map(|(c, parent)| (c.id.clone(), c.position, c.kind == crate::state::ChannelKind::Category, parent.clone(), false))
        .collect();

    let mut action = None;
    let mut rects: Vec<Rect> = Vec::with_capacity(visible.len());
    let touch = tiles(ui);
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    let width = fit_width(ui);
    let last = items.len().saturating_sub(1);
    let mut index = 0;
    let mut first_group = true;
    while index < visible.len() {
        let (head_channel, _) = &visible[index];
        let is_category = items[index].2;
        if !first_group {
            ui.add_space(22.0);
        }
        first_group = false;

        // Cabeçalho do grupo: a categoria (com alça e ações) ou "Sem categoria".
        if is_category {
            let channel = *head_channel;
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::hover());
            rects.push(rect);
            let being = dragging.as_deref() == Some(channel.id.as_str());
            let alpha = if being { 0.45 } else { 1.0 };
            let grip = Rect::from_min_size(egui::pos2(rect.min.x + 6.0, rect.min.y), Vec2::new(18.0, 28.0));
            let grip_response = ui.interact(grip, egui::Id::new(("alca-canal", &channel.id)), Sense::drag());
            if grip_response.hovered() || grip_response.dragged() {
                ui.ctx().set_cursor_icon(if grip_response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            if grip_response.drag_started() {
                *dragging = Some(channel.id.clone());
            }
            ui.painter().text(grip.center(), egui::Align2::CENTER_CENTER, icon::DOTS_SIX_VERTICAL, text::icon(13.0), t.label_tertiary.gamma_multiply(alpha));
            let tools_w = 4.0 * 32.0 + 12.0;
            super::emoji::draw_channel_name(
                ui,
                t,
                media,
                store,
                &channel.name,
                egui::pos2(rect.min.x + 28.0, rect.center().y),
                px(11.5, "semibold"),
                t.label_secondary.gamma_multiply(alpha),
                rect.width() - 28.0 - tools_w,
                channel_emoji_monochrome,
                false,
            );
            let show = (response.hovered() || ui.rect_contains_pointer(rect) || touch) && dragging.is_none();
            if let Some(which) = row_tools(ui, t, &channel.id, rect, index > 0, index < last, show) {
                action = match which {
                    0 => move_action(&items, index, index - 1, total),
                    1 => move_action(&items, index, index + 2, total),
                    2 => Some(TreeAction::Edit(channel.id.clone())),
                    _ => Some(TreeAction::Delete(channel.id.clone(), channel.name.clone())),
                };
            }
            ui.add_space(6.0);
            index += 1;
        } else {
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                ui.label(RichText::new(s.no_category).font(px(11.5, "semibold")).color(t.label_secondary));
            });
            ui.add_space(6.0);
        }

        // Os canais do grupo: os filhos da categoria, ou a fileira de canais
        // soltos até a próxima categoria.
        let owner = is_category.then(|| head_channel.id.clone());
        let start = index;
        while index < visible.len() {
            let belongs = if let Some(owner) = &owner {
                visible[index].1.as_deref() == Some(owner.as_str())
            } else {
                !items[index].2 && visible[index].1.is_none()
            };
            if !belongs {
                break;
            }
            index += 1;
        }
        let card_slot = ui.painter().add(egui::Shape::Noop);
        let card_top = ui.cursor().min.y;
        if start == index {
            // Categoria vazia: um aviso no lugar das linhas.
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::hover());
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, s.category_empty, px(12.0, "regular"), t.label_secondary);
        }
        for (row, (channel, _)) in visible.iter().enumerate().take(index).skip(start) {
            let h = if channel.topic.as_deref().is_some_and(|topic| !topic.is_empty()) { 54.0 } else { 48.0 };
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
            rects.push(rect);
            let being = dragging.as_deref() == Some(channel.id.as_str());
            let alpha = if being { 0.45 } else { 1.0 };
            let hovered = ui.rect_contains_pointer(rect) || being;
            if hovered {
                let top = if row == start { 10 } else { 0 };
                let bottom = if row + 1 == index { 10 } else { 0 };
                ui.painter().rect_filled(rect, CornerRadius { nw: top, ne: top, sw: bottom, se: bottom }, t.fill_soft);
            }
            if row > start {
                ui.painter().line_segment(
                    [egui::pos2(rect.min.x + ROW_INSET, rect.min.y), egui::pos2(rect.max.x, rect.min.y)],
                    Stroke::new(0.5, t.separator),
                );
            }
            let _ = response;
            let grip = Rect::from_min_size(egui::pos2(rect.min.x + 6.0, rect.min.y), Vec2::new(18.0, h));
            let grip_response = ui.interact(grip, egui::Id::new(("alca-canal", &channel.id)), Sense::drag());
            if grip_response.hovered() || grip_response.dragged() {
                ui.ctx().set_cursor_icon(if grip_response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            if grip_response.drag_started() {
                *dragging = Some(channel.id.clone());
            }
            ui.painter().text(grip.center(), egui::Align2::CENTER_CENTER, icon::DOTS_SIX_VERTICAL, text::icon(13.0), t.label_tertiary.gamma_multiply(alpha));

            let tile = Rect::from_center_size(egui::pos2(rect.min.x + 34.0 + 14.0, rect.center().y), Vec2::splat(28.0));
            ui.painter().rect_filled(tile, CornerRadius::same(8), t.fill_soft.gamma_multiply(alpha));
            ui.painter().text(
                tile.center(),
                egui::Align2::CENTER_CENTER,
                if channel.kind == crate::state::ChannelKind::Voice { icon::SPEAKER_HIGH } else { icon::HASH },
                text::icon(14.0),
                t.label_secondary.gamma_multiply(alpha),
            );
            let left = rect.min.x + 78.0;
            let room = rect.width() - 78.0 - (4.0 * 32.0 + 12.0);
            let topic = channel.topic.as_deref().filter(|topic| !topic.is_empty());
            let name_y = if topic.is_some() { rect.center().y - 9.0 } else { rect.center().y };
            super::emoji::draw_channel_name(
                ui,
                t,
                media,
                store,
                &channel.name,
                egui::pos2(left, name_y),
                px(13.0, "medium"),
                t.label.gamma_multiply(alpha),
                room,
                channel_emoji_monochrome,
                false,
            );
            if let Some(topic) = topic {
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, rect.center().y + 9.0), egui::Align2::LEFT_CENTER, topic, px(11.5, "regular"), t.label_secondary.gamma_multiply(alpha), room);
            }
            let show = (hovered || touch) && dragging.is_none();
            if let Some(which) = row_tools(ui, t, &channel.id, rect, row > 0, row < last, show) {
                action = match which {
                    0 => move_action(&items, row, row - 1, total),
                    1 => move_action(&items, row, row + 2, total),
                    2 => Some(TreeAction::Edit(channel.id.clone())),
                    _ => Some(TreeAction::Delete(channel.id.clone(), channel.name.clone())),
                };
            }
        }
        // O cartão do grupo, pintado atrás das linhas agora que se sabe a altura.
        let bottom = ui.cursor().min.y;
        if bottom > card_top {
            ui.painter().set(
                card_slot,
                egui::epaint::RectShape::new(
                    Rect::from_min_max(egui::pos2(ui.max_rect().min.x, card_top), egui::pos2(ui.max_rect().min.x + width, bottom)),
                    CornerRadius::same(10),
                    t.card,
                    Stroke::new(0.5, t.separator),
                    egui::StrokeKind::Inside,
                ),
            );
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
                if let Some(moved) = move_action(&items, dragged, gap, total) {
                    action = Some(moved);
                }
                *dragging = None;
            }
        } else if released {
            *dragging = None;
        }
    }
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
        (AppPane::Activity, s.rich_presence_enabled),
        (AppPane::Activity, s.rich_presence_builtin),
        (AppPane::Activity, s.rich_presence_game_detection),
        (AppPane::Activity, s.rich_presence_auto_reconnect),
        (AppPane::Activity, s.rich_presence_reconnect_interval),
        (AppPane::Activity, s.rich_presence_debug),
        (AppPane::Activity, s.rich_presence_external),
        (AppPane::Activity, s.rich_presence_override),
        (AppPane::Appearance, s.theme),
        #[cfg(not(target_os = "android"))]
        (AppPane::Appearance, s.ui_scale),
        (AppPane::Appearance, s.translucency),
        (AppPane::Appearance, s.topic_reveal),
        (AppPane::Appearance, s.open_at_newest),
        (AppPane::Appearance, s.mark_read_at_newest),
        (AppPane::Appearance, s.prefer_backend_embeds),
        (AppPane::Appearance, s.record_button),
        (AppPane::Appearance, s.channel_emoji_monochrome),
        (AppPane::Appearance, s.webembed_offscreen),
        (AppPane::Appearance, s.webembed_scope),
        (AppPane::Appearance, s.self_card),
        (AppPane::Alerts, s.menu_notifications),
        (AppPane::Alerts, s.reply_notifications_default),
        (AppPane::Alerts, s.badge),
        (AppPane::Alerts, s.close_to_tray_title),
        (AppPane::Alerts, s.start_at_login),
        (AppPane::Files, s.downloads),
        (AppPane::Files, s.download_folder),
        (AppPane::Sessions, s.version),
    ]
}

fn rich_presence_source_label(source: crate::rich_presence::Source, s: &Strings) -> &'static str {
    match source {
        crate::rich_presence::Source::External => s.rich_presence_source_external,
        crate::rich_presence::Source::BuiltIn => s.rich_presence_source_builtin,
        crate::rich_presence::Source::Override => s.rich_presence_source_override,
        crate::rich_presence::Source::Connecting => s.rich_presence_source_connecting,
        crate::rich_presence::Source::Disabled => s.rich_presence_source_disabled,
        crate::rich_presence::Source::Error => s.rich_presence_source_error,
        crate::rich_presence::Source::Unsupported => s.rich_presence_source_disabled,
    }
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
    let tiled = tiles(ui);
    if tiled {
        ui.add_space(space::MD);
    }
    // Desktop: o cartão da prévia do mock, na cor da folha e com 14 de miolo.
    let pad = if tiled { space::LG } else { 14.0 };
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(fit_width(ui), height + pad * 2.0),
        Sense::hover(),
    );
    if tiled {
        ui.painter().rect_filled(rect, CornerRadius::same(radius::SHEET), t.fill_soft);
    } else {
        ui.painter().rect(rect, CornerRadius::same(10), t.elevated_bg, Stroke::new(0.5, t.separator), egui::StrokeKind::Inside);
    }
    draw(ui, rect.shrink(pad));
    // Some previews draw through a bounded child Ui. Keep the parent cursor
    // explicitly below the reserved card so the following settings section
    // can never start on top of the preview.
    ui.advance_cursor_after_rect(rect);
}

/// Texto de apoio à direita da prévia, quando sobra espaço.
fn notification_preview(ui: &egui::Ui, t: &Tokens, s: &Strings, rect: Rect, _initials: &str, on: bool) {
    let painter = ui.painter();
    let alpha = if on { 1.0 } else { 0.4 };
    painter.add(ui.visuals().popup_shadow.as_shape(rect, CornerRadius::same(12)));
    painter.rect(rect, CornerRadius::same(12), t.card, Stroke::new(0.5, t.separator), egui::StrokeKind::Inside);
    let icon = Rect::from_min_size(egui::pos2(rect.min.x + 12.0, rect.center().y - 15.0), Vec2::splat(30.0));
    painter.rect_filled(icon, CornerRadius::same(8), t.accent.gamma_multiply(alpha));
    let x = icon.max.x + 10.0;
    let clip = painter.with_clip_rect(rect.shrink(2.0));
    clip.text(egui::pos2(x, rect.center().y - 15.0), egui::Align2::LEFT_TOP, s.preview_notify_title, px(13.0, "semibold"), t.label.gamma_multiply(alpha));
    clip.text(egui::pos2(x, rect.center().y + 2.0), egui::Align2::LEFT_TOP, s.preview_notify_body, px(13.0, "regular"), t.label_secondary.gamma_multiply(alpha));
    clip.text(egui::pos2(rect.max.x - 12.0, rect.center().y), egui::Align2::RIGHT_CENTER, s.preview_notify_meta, px(11.0, "regular"), t.label_tertiary);
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
            let _ = t;
            ui.painter().rect_filled(rect, CornerRadius::same(corner), egui::Color32::from_rgb(0xFF, 0x5C, 0x63));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                px((rect.height() * 0.36).max(11.0), "semibold"),
                egui::Color32::WHITE,
            );
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
    search_input(ui, t, "ajustes-celular:busca", query, rect, space::XL + 24.0, s.settings_search, egui::FontId::new(15.0, egui::FontFamily::Proportional), false);
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

/// A linha de resumo de cada painel do usuário: a mesma na lista do celular
/// e na coluna do desktop.
fn app_summaries(data: &Context<'_>, s: &Strings) -> Vec<(AppPane, String)> {
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
    let presence_summary = rich_presence_source_label(data.rich_presence_snapshot.source.clone(), s).to_owned();
    vec![
        (AppPane::Appearance, appearance),
        (AppPane::Activity, presence_summary),
        (AppPane::Alerts, alerts),
        (AppPane::Language, data.lang.endonym().to_owned()),
        (AppPane::Files, files),
        (AppPane::Sessions, devices),
        (AppPane::Diagnostics, s.sum_diagnostics.to_owned()),
    ]
}

/// Lista dos ajustes do usuário.
fn app_list(ui: &mut egui::Ui, state: &mut SettingsState, data: &mut Context<'_>, t: &Tokens, s: &Strings) {
    use egui_phosphor::regular as icon;
    search_field(ui, t, s, &mut state.mobile_query);

    let summaries = app_summaries(data, s);
    let summary = |pane: AppPane| summaries.iter().find(|(p, _)| *p == pane).map(|(_, text)| text.clone()).unwrap_or_default();
    let rows: Vec<(AppPane, Entry<'_>)> = vec![
        (AppPane::Appearance, Entry { glyph: icon::PALETTE, title: AppPane::Appearance.title(s), summary: summary(AppPane::Appearance) }),
        (AppPane::Activity, Entry { glyph: icon::GAME_CONTROLLER, title: AppPane::Activity.title(s), summary: summary(AppPane::Activity) }),
        (AppPane::Alerts, Entry { glyph: icon::BELL, title: AppPane::Alerts.title(s), summary: summary(AppPane::Alerts) }),
        (AppPane::Language, Entry { glyph: icon::GLOBE, title: AppPane::Language.title(s), summary: summary(AppPane::Language) }),
        (AppPane::Files, Entry { glyph: icon::FOLDER, title: AppPane::Files.title(s), summary: summary(AppPane::Files) }),
        (AppPane::Sessions, Entry { glyph: icon::DEVICES, title: AppPane::Sessions.title(s), summary: summary(AppPane::Sessions) }),
        (AppPane::Diagnostics, Entry { glyph: icon::PULSE, title: AppPane::Diagnostics.title(s), summary: summary(AppPane::Diagnostics) }),
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

/// A linha de resumo de cada painel do servidor: a mesma na lista do celular
/// e na coluna do desktop.
fn server_summaries(store: &Store, s: &Strings) -> Vec<(ServerPane, String)> {
    vec![
        (ServerPane::General, s.sum_identity.to_owned()),
        (
            ServerPane::Channels,
            format!(
                "{} {}",
                store.channels.iter().filter(|channel| channel.kind != crate::state::ChannelKind::Direct).count(),
                s.sum_channels
            ),
        ),
        (ServerPane::Roles, format!("{} {}", store.roles.len(), s.sum_roles)),
        (ServerPane::Emojis, format!("{} {}", store.emojis.len(), s.sum_stickers)),
        (ServerPane::Audit, s.sum_audit.to_owned()),
    ]
}

/// Lista dos ajustes do servidor: só o que o cargo permite.
fn server_list(ui: &mut egui::Ui, state: &mut SettingsState, data: &mut Context<'_>, t: &Tokens, s: &Strings) {
    search_field(ui, t, s, &mut state.mobile_query);
    let store = data.store;
    let summaries = server_summaries(store, s);
    let all: Vec<(ServerPane, Entry<'_>)> = ServerPane::ALL
        .into_iter()
        .filter(|pane| server_pane_allowed(store, *pane))
        .map(|pane| {
            let summary = summaries.iter().find(|(p, _)| *p == pane).map(|(_, text)| text.clone()).unwrap_or_default();
            (pane, Entry { glyph: pane.glyph(), title: pane.title(s), summary })
        })
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
        let access = if store.server.as_ref().is_none_or(|server| server.public) { s.access_public } else { s.access_private };
        let head = Entry { glyph: "", title: &name, summary: format!("{access} · {} {}", store.members.len(), s.count_members) };
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

/// Cabeçalho do painel: o nome dele em negrito e o X redondo à direita.
fn header(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    t: &Tokens,
    _s: &Strings,
    rect: Rect,
    _surface: Surface,
    title: &str,
) {
    let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H));
    let mid = head.min.y + 16.0 + 13.0;
    ui.painter().text(
        egui::pos2(head.min.x + 24.0, mid),
        egui::Align2::LEFT_CENTER,
        title,
        px(17.0, "display"),
        t.label,
    );
    const CLOSE: f32 = 26.0;
    let close = Rect::from_center_size(egui::pos2(head.max.x - 16.0 - CLOSE / 2.0, mid), Vec2::splat(CLOSE));
    let response = ui.interact(close, ui.id().with("fechar-ajustes"), Sense::click());
    ui.painter().circle_filled(close.center(), CLOSE / 2.0, if response.hovered() { t.fill_medium } else { t.fill_soft });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(
        close.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::X,
        text::icon(12.0),
        if response.hovered() { t.label } else { t.label_secondary },
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
    data: &mut Context<'_>,
    t: &Tokens,
    s: &Strings,
    rect: Rect,
    surface: Surface,
) {
    let store = data.store;
    let summaries = app_summaries(data, s);
    let server_sums = server_summaries(store, s);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(12.0, 14.0)))
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Busca no topo da coluna: pílula de 36, lupa na cor de destaque
            // e um X redondo para limpar quando há texto.
            {
                let (field, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::hover());
                ui.painter().rect(field, CornerRadius::same(18), t.card, Stroke::new(0.5, t.separator), egui::StrokeKind::Inside);
                ui.painter().text(
                    egui::pos2(field.min.x + 13.0 + 7.5, field.center().y),
                    egui::Align2::CENTER_CENTER,
                    egui_phosphor::regular::MAGNIFYING_GLASS,
                    text::icon(15.0),
                    t.accent,
                );
                let inner = Rect::from_min_max(egui::pos2(field.min.x + 34.0, field.min.y), egui::pos2(field.max.x - 32.0, field.max.y));
                let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
                child.add_sized(
                    inner.size(),
                    egui::TextEdit::singleline(&mut state.mobile_query)
                        .hint_text(RichText::new(s.rail_search).font(px(13.0, "regular")))
                        .frame(egui::Frame::NONE)
                        .font(px(13.0, "regular"))
                        .vertical_align(Align::Center),
                );
                if !state.mobile_query.is_empty() {
                    let x = Rect::from_center_size(egui::pos2(field.max.x - 8.0 - 9.0, field.center().y), Vec2::splat(18.0));
                    let response = ui.interact(x, ui.id().with("limpar-busca"), Sense::click());
                    ui.painter().circle_filled(x.center(), 9.0, t.fill_medium);
                    ui.painter().text(x.center(), egui::Align2::CENTER_CENTER, egui_phosphor::regular::X, text::icon(10.0), t.label);
                    if response.clicked() {
                        state.mobile_query.clear();
                    }
                }
                ui.add_space(10.0);
            }
            let query = state.mobile_query.trim().to_lowercase();
            let shown = |title: &str| query.is_empty() || title.to_lowercase().contains(&query);
            // Cada item: bolinha de 34 com o ícone (acesa no painel aberto), o
            // nome e, embaixo, o resumo do que está valendo.
            let entry = |ui: &mut egui::Ui, glyph: &str, label: &str, summary: Option<&str>, active: bool| -> bool {
                let height = if summary.is_some() { RAIL_ENTRY_H } else { 36.0 };
                let (slot, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
                if active {
                    ui.painter().rect_filled(slot, CornerRadius::same(10), t.accent.gamma_multiply(0.16));
                } else if response.hovered() {
                    ui.painter().rect_filled(slot, CornerRadius::same(10), t.fill_soft);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let badge_d = if summary.is_some() { 34.0 } else { 26.0 };
                let badge = Rect::from_center_size(egui::pos2(slot.min.x + 12.0 + badge_d / 2.0, slot.center().y), Vec2::splat(badge_d));
                ui.painter().circle_filled(badge.center(), badge_d / 2.0, if active { t.accent } else { t.fill_soft });
                ui.painter().text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    text::icon(if summary.is_some() { 17.0 } else { 13.0 }),
                    if active { t.accent_label } else { t.label },
                );
                let left = badge.max.x + 12.0;
                let room = slot.max.x - 12.0 - left;
                let ink = if active { t.accent } else { t.label };
                match summary {
                    Some(summary) => {
                        crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, slot.center().y - 8.0), egui::Align2::LEFT_CENTER, label, px(14.0, "medium"), ink, room);
                        crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, slot.center().y + 9.0), egui::Align2::LEFT_CENTER, summary, px(11.5, "regular"), t.label_secondary, room);
                    }
                    None => {
                        crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, slot.center().y), egui::Align2::LEFT_CENTER, label, px(13.0, "medium"), ink, room);
                    }
                }
                ui.add_space(4.0);
                response.clicked()
            };

            // Você no topo (Conta): foto de 42, apelido e o que cabe em "Conta".
            if surface == Surface::App && query.is_empty() {
                let me = store.member(&store.me);
                let tint = me
                    .and_then(|member| member.role_color)
                    .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
                    .unwrap_or(t.accent);
                let initials = me.map(|member| member.initials()).unwrap_or_default();
                let ctx = ui.ctx().clone();
                let face = data
                    .media
                    .avatar(&store.me, store.avatars.get(&store.me).map(String::as_str))
                    .and_then(|texture| texture.frame(&ctx))
                    .map(|handle| handle.id());
                let active = state.app_pane == AppPane::Account;
                let (slot, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::click());
                if active {
                    ui.painter().rect_filled(slot, CornerRadius::same(10), t.accent.gamma_multiply(0.16));
                } else if response.hovered() {
                    ui.painter().rect_filled(slot, CornerRadius::same(10), t.fill_soft);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let avatar = Rect::from_center_size(egui::pos2(slot.min.x + 12.0 + 21.0, slot.center().y), Vec2::splat(42.0));
                match face {
                    Some(texture) => crate::ui::widgets::round_photo(ui.painter(), avatar, texture, egui::Color32::WHITE),
                    None => {
                        ui.painter().circle_filled(avatar.center(), 21.0, tint.gamma_multiply(0.30));
                        ui.painter().text(avatar.center(), egui::Align2::CENTER_CENTER, &initials, px(15.0, "semibold"), tint);
                    }
                }
                let left = avatar.max.x + 12.0;
                let room = slot.max.x - 12.0 - left;
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, slot.center().y - 8.0), egui::Align2::LEFT_CENTER, &store.my_name, px(14.0, "medium"), if active { t.accent } else { t.label }, room);
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, slot.center().y + 9.0), egui::Align2::LEFT_CENTER, s.sum_account, px(11.5, "regular"), t.label_secondary, room);
                if response.clicked() {
                    state.app_pane = AppPane::Account;
                    state.draft.loaded_account = false;
                }
                ui.add_space(10.0);
            }

            // O servidor no topo: ícone, nome, endereço e quantos são. Só
            // identifica; "Geral" é que abre a identidade.
            if surface == Surface::Server && query.is_empty() {
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
                let (slot, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 54.0), Sense::hover());
                let tile = Rect::from_center_size(egui::pos2(slot.min.x + 12.0 + 21.0, slot.center().y), Vec2::splat(42.0));
                match texture {
                    Some(texture) => crate::ui::widgets::photo(ui.painter(), tile, texture, crate::ui::widgets::FULL_UV, CornerRadius::same(10), egui::Color32::WHITE),
                    None => {
                        ui.painter().rect_filled(tile, CornerRadius::same(10), t.accent.gamma_multiply(0.30));
                        ui.painter().text(tile.center(), egui::Align2::CENTER_CENTER, &initials, px(15.0, "semibold"), t.accent);
                    }
                }
                let left = tile.max.x + 12.0;
                let room = slot.max.x - 12.0 - left;
                let access = if store.server.as_ref().is_none_or(|server| server.public) { s.access_public } else { s.access_private };
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, slot.center().y - 8.0), egui::Align2::LEFT_CENTER, &name, px(14.0, "medium"), t.label, room);
                crate::ui::widgets::text_fit(ui.painter(), egui::pos2(left, slot.center().y + 9.0), egui::Align2::LEFT_CENTER, &format!("{access} · {} {}", store.members.len(), s.count_members), px(11.5, "regular"), t.label_secondary, room);
                ui.add_space(10.0);
            }

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
                            // Conta mora no cartão do topo; com busca ela
                            // volta à lista como qualquer painel.
                            if (pane == AppPane::Account && query.is_empty()) || !shown(pane.title(s)) {
                                continue;
                            }
                            let summary = summaries.iter().find(|(p, _)| *p == pane).map(|(_, text)| text.as_str()).or((pane == AppPane::Account).then_some(s.sum_account));
                            if entry(ui, pane.glyph(), pane.title(s), summary, state.app_pane == pane) {
                                state.app_pane = pane;
                            }
                        }
                    }
                    Surface::Server => {
                        for pane in ServerPane::ALL {
                            if !server_pane_allowed(store, pane) || !shown(pane.title(s)) {
                                continue;
                            }
                            let summary = server_sums.iter().find(|(p, _)| *p == pane).map(|(_, text)| text.as_str());
                            if entry(ui, pane.glyph(), pane.title(s), summary, state.server_pane == pane) {
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
                        if entry(ui, glyph, label, None, false) {
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

            // O cartão como ele fica para os outros, e cada parte é o campo:
            // recado no balão, apelido no nome, bio em "Sobre".
            {
                match crate::ui::profile::edit_card(
                    ui,
                    t,
                    s,
                    data.store,
                    data.media,
                    crate::ui::profile::CardDraft {
                        nickname: &mut draft.nickname,
                        status: &mut draft.status_message,
                        description: &mut draft.description,
                    },
                ) {
                    crate::ui::profile::PreviewHit::Avatar => pick_avatar = true,
                    crate::ui::profile::PreviewHit::Banner => pick_banner = true,
                    crate::ui::profile::PreviewHit::None => {}
                }
            }
            // Os botões moram com a prévia: são dela, não linhas à parte.
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::MD;
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
            if pick_avatar {
                actions.push(SettingsAction::Admin(AdminAction::PickAvatar));
            }
            if pick_banner {
                actions.push(SettingsAction::Admin(AdminAction::PickBanner));
            }
            if remove_banner {
                actions.push(SettingsAction::Admin(AdminAction::RemoveBanner));
            }

            // O recado da frase de digitação não aparece no cartão, mas os
            // outros veem na hora em que você digita: fica à parte do cartão.
            let stored = |pick: fn(&Store, Option<&crate::state::Member>) -> String| {
                pick(data.store, data.store.member(&data.store.me))
            };
            let saved_nickname = stored(|store, _| store.my_name.clone());
            let saved_status = stored(|_, me| {
                me.and_then(|member| member.status_message.clone()).unwrap_or_default()
            });
            let saved_typing = stored(|_, me| {
                me.and_then(|member| member.typing_label.clone()).unwrap_or_default()
            });
            let saved_description = stored(|store, _| {
                store
                    .profiles
                    .get(&store.me)
                    .and_then(|profile| profile.description.clone())
                    .unwrap_or_default()
            });
            let dirty = draft.nickname.trim() != saved_nickname.trim()
                || draft.status_message.trim() != saved_status.trim()
                || draft.description.trim() != saved_description.trim()
                || draft.typing.trim() != saved_typing.trim();
            let mut save_profile = false;
            let mut discard_profile = false;
            if dirty {
                // A pastilha flutuante de confirmação: só existe enquanto há
                // algo por salvar.
                let pill_id = egui::Id::new("perfil-salvar");
                crate::ui::widgets::floating_pill(
                    ui.ctx(),
                    pill_id,
                    t,
                    *data.translucency,
                    &data.glass,
                    space::LG,
                    420.0,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = space::SM;
                            ui.label(
                                RichText::new(s.unsaved_changes)
                                    .font(text::body())
                                    .color(t.label_secondary),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if row_button(ui, t, s.save_profile, Emphasis::Primary) {
                                    save_profile = true;
                                }
                                if row_button(ui, t, s.discard, Emphasis::Quiet) {
                                    discard_profile = true;
                                }
                            });
                        });
                    },
                );
                // Clicar na folha a traz para frente; a pastilha fica por cima.
                ui.ctx()
                    .move_to_top(egui::LayerId::new(egui::Order::Foreground, pill_id));
            }
            if save_profile {
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
            if discard_profile {
                // Recarrega o rascunho do que está valendo, no quadro seguinte.
                draft.loaded_account = false;
            }

            section(ui, t, s.presence);
            group(ui, t, |rows| {
                let mine = data
                    .store
                    .member(&data.store.me)
                    .map(|member| member.presence);
                rows.row(s.presence, Some(s.presence_hint), |ui, t| {
                    let mut chosen = mine.unwrap_or(crate::state::Presence::Online);
                    if segmented(
                        ui,
                        t,
                        &mut chosen,
                        &[
                            (crate::state::Presence::Online, s.presence_online),
                            (crate::state::Presence::Away, s.presence_away),
                            (crate::state::Presence::Busy, s.presence_busy),
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

            section(ui, t, s.typing_section);
            let typing_preview = {
                let name = if draft.nickname.trim().is_empty() { data.store.my_name.as_str() } else { draft.nickname.trim() };
                let phrase = if draft.typing.trim().is_empty() { s.typing_one } else { draft.typing.trim() };
                format!("{name} {phrase}")
            };
            group(ui, t, |rows| {
                rows.stack_next();
                rows.field_hinted(
                    s.typing_phrase,
                    Some(s.typing_phrase_hint),
                    &mut draft.typing,
                    64,
                    false,
                );
                rows.join_next();
                rows.typing_panel(&typing_preview);
            });

            // Segurança: senha e sair, juntos — as duas coisas que não são
            // perfil e não mudam na hora como a presença.
            section(ui, t, s.security);
            group(ui, t, |rows| {
                let open = draft.password_open;
                let mut toggle = false;
                rows.row(s.change_password, Some(s.password_hint), |ui, t| {
                    if row_button(ui, t, if open { s.cancel } else { s.change_password_open }, Emphasis::Quiet) {
                        toggle = true;
                    }
                });
                if toggle {
                    draft.password_open = !open;
                    draft.password.clear();
                }
                if draft.password_open {
                    rows.stack_next();
                    let sent = rows.field(s.new_password, &mut draft.password, 64, true);
                    let value = draft.password.clone();
                    let checks = [
                        (s.password_rule_length, value.chars().count() >= 8),
                        (s.password_rule_uppercase, value.chars().any(char::is_uppercase)),
                        (s.password_rule_special, value.chars().any(|c| !c.is_alphanumeric())),
                    ];
                    let ready = checks.iter().all(|(_, ok)| *ok);
                    rows.join_next();
                    if rows.password_rules(&checks, s.change_password, ready) || (sent && ready) {
                        actions.push(SettingsAction::Admin(AdminAction::ChangePassword(
                            draft.password.clone(),
                        )));
                        draft.password.clear();
                        draft.password_open = false;
                    }
                }
                rows.row(s.sign_out, Some(s.sign_out_hint), |ui, t| {
                    if row_button(ui, t, s.sign_out_short, Emphasis::Danger) {
                        actions.push(SettingsAction::Menu(
                            crate::platform::menu::MenuCommand::SignOut,
                        ));
                    }
                });
            });
        }

        AppPane::Appearance => {
            preview_chat(ui, t, data, s.preview_theme_hint);
            section(ui, t, s.theme);
            group(ui, t, |rows| {
                use crate::ui::profile::SelfCardStyle;
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
                #[cfg(not(target_os = "android"))]
                rows.row(s.ui_scale, Some(s.ui_scale_hint), |ui, _t| {
                    // Keep the value local during pointer interaction. Zooming
                    // the viewport while the pointer is held would change the
                    // slider geometry, causing a moving-target feedback loop.
                    let current = state.ui_scale_draft.unwrap_or(*data.ui_scale);
                    let mut percent = (current * 100.0).round() as i32;
                    if ui.add_sized(
                        [175.0, 26.0],
                        egui::Slider::new(&mut percent, 75..=200).step_by(5.0).suffix("%"),
                    ).changed() {
                        state.ui_scale_draft = Some(percent as f32 / 100.0);
                    }
                });
                rows.row(s.translucency, Some(s.translucency_hint), |ui, t| {
                    switch(ui, t, data.translucency);
                });
                rows.row(
                    s.channel_emoji_monochrome,
                    Some(s.channel_emoji_monochrome_hint),
                    |ui, t| {
                        switch(ui, t, data.channel_emoji_monochrome);
                    },
                );
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

            section(ui, t, s.section_chat);
            group(ui, t, |rows| {
                rows.row(s.topic_reveal, Some(s.topic_reveal_hint), |ui, t| {
                    switch(ui, t, data.topic_reveal);
                });
                rows.row(s.open_at_newest, Some(s.open_at_newest_hint), |ui, t| {
                    switch(ui, t, data.open_at_newest);
                });
                rows.row(
                    s.mark_read_at_newest,
                    Some(s.mark_read_at_newest_hint),
                    |ui, t| {
                        switch(ui, t, data.mark_read_at_newest);
                    },
                );
                rows.row(s.record_button, Some(s.record_button_hint), |ui, t| {
                    switch(ui, t, data.record_button);
                });
                rows.row(s.prefer_backend_embeds, Some(s.prefer_backend_embeds_hint), |ui, t| {
                    switch(ui, t, data.prefer_backend_embeds);
                });
            });

            section(ui, t, s.section_video);
            group(ui, t, |rows| {
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
                if matches!(*data.webembed_offscreen, crate::webembed::OffscreenBehavior::Float) {
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
                }
            });
        }

        AppPane::Activity => {
            use crate::rich_presence::Source;
            // The manager already applies the manual override, so the hero
            // shows exactly what the profile shows, with a stable timer.
            let snapshot = data.rich_presence_snapshot;
            let preview = snapshot.activity.as_ref();
            // The activity card is taller than the empty state (and can grow
            // a timer/progress row), so reserve enough room for whichever
            // preview is actually visible.
            let preview_height = if preview.is_some() { 132.0 } else { 58.0 };
            hero(ui, t, preview_height, |ui, area| {
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(area)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.horizontal(|ui| {
                            let color = match snapshot.source {
                                Source::External | Source::BuiltIn | Source::Override => t.online,
                                Source::Connecting => t.away,
                                Source::Error => t.danger,
                                _ => t.label_tertiary,
                            };
                            ui.label(
                                RichText::new("●")
                                    .font(text::callout())
                                    .color(color),
                            );
                            ui.label(
                                RichText::new(rich_presence_source_label(snapshot.source.clone(), s))
                                    .font(text::subheadline())
                                    .color(t.label),
                            );
                            if !snapshot.detail.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("· {}", snapshot.detail))
                                            .font(text::footnote())
                                            .color(t.label_tertiary),
                                    )
                                    .truncate(),
                                );
                            }
                        });
                        ui.add_space(space::SM);
                        if let Some(activity) = preview {
                            super::profile::activity_block(ui, t, s, activity, true);
                        } else {
                            ui.label(
                                RichText::new(s.rich_presence_no_activity)
                                    .font(text::body())
                                    .color(t.label_secondary),
                            );
                        }
                    },
                );
            });

            section(ui, t, s.rich_presence);
            group(ui, t, |rows| {
                rows.row(
                    s.rich_presence_enabled,
                    Some(s.rich_presence_enabled_hint),
                    |ui, t| {
                        switch(ui, t, &mut data.rich_presence.enabled);
                    },
                );
                // Android only renders activity; there is no provider to
                // configure there.
                #[cfg(not(target_os = "android"))]
                {
                    rows.row(
                        s.rich_presence_builtin,
                        Some(s.rich_presence_builtin_hint),
                        |ui, t| {
                            switch(ui, t, &mut data.rich_presence.built_in);
                        },
                    );
                    rows.row(
                        s.rich_presence_game_detection,
                        Some(s.rich_presence_game_detection_hint),
                        |ui, t| {
                            switch(ui, t, &mut data.rich_presence.game_detection);
                        },
                    );
                }
            });

            section(ui, t, s.rich_presence_override);
            group(ui, t, |rows| {
                rows.row(
                    s.rich_presence_override,
                    Some(s.rich_presence_override_hint),
                    |ui, t| {
                        switch(ui, t, &mut data.rich_presence.override_activity.enabled);
                    },
                );
                if data.rich_presence.override_activity.enabled {
                    rows.row(s.rich_presence_override_type, None, |ui, t| {
                        segmented(
                            ui,
                            t,
                            &mut data.rich_presence.override_activity.kind,
                            &[
                                (crate::rich_presence::OverrideKind::Playing, s.activity_playing),
                                (crate::rich_presence::OverrideKind::Listening, s.activity_listening),
                                (crate::rich_presence::OverrideKind::Working, s.activity_working),
                            ],
                        );
                    });
                    let image = data
                        .rich_presence
                        .override_activity
                        .image
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned());
                    rows.row(s.rich_presence_override_image, None, |ui, t| {
                        use egui_phosphor::regular as icon;
                        // Right to left: remove, choose, then the thumbnail,
                        // which also opens the picker.
                        if image.is_some()
                            && crate::ui::widgets::icon_button(
                                ui,
                                t,
                                icon::TRASH,
                                s.rich_presence_override_image_remove,
                            )
                            .clicked()
                        {
                            actions.push(SettingsAction::ClearActivityImage);
                        }
                        if crate::ui::widgets::icon_button(
                            ui,
                            t,
                            icon::IMAGE,
                            s.rich_presence_override_image_choose,
                        )
                        .clicked()
                        {
                            actions.push(SettingsAction::PickActivityImage);
                        }
                        let (slot, response) =
                            ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                        let corners = CornerRadius::same(radius::FIELD);
                        match image
                            .as_deref()
                            .and_then(|path| super::profile::activity_art(ui.ctx(), path))
                        {
                            Some(texture) => {
                                egui::Image::new((texture.id(), slot.size()))
                                    .corner_radius(corners)
                                    .paint_at(ui, slot);
                            }
                            None => {
                                ui.painter().rect_filled(slot, corners, t.fill_soft);
                                ui.painter().text(
                                    slot.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icon::IMAGE,
                                    text::icon(14.0),
                                    t.label_tertiary,
                                );
                            }
                        }
                        if response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(s.rich_presence_override_image_choose)
                            .clicked()
                        {
                            actions.push(SettingsAction::PickActivityImage);
                        }
                    });
                    rows.field(
                        s.rich_presence_override_name,
                        &mut data.rich_presence.override_activity.name,
                        128,
                        false,
                    );
                    rows.field(
                        s.rich_presence_override_details,
                        &mut data.rich_presence.override_activity.details,
                        128,
                        false,
                    );
                    rows.field(
                        s.rich_presence_override_state,
                        &mut data.rich_presence.override_activity.state,
                        128,
                        false,
                    );
                    rows.row(s.rich_presence_override_elapsed, None, |ui, t| {
                        switch(ui, t, &mut data.rich_presence.override_activity.elapsed);
                    });
                    rows.field(
                        s.rich_presence_override_duration,
                        &mut data.rich_presence.override_activity.duration_minutes,
                        6,
                        false,
                    );
                }
            });

            #[cfg(not(target_os = "android"))]
            {
                let settings = &mut *data.rich_presence;
                let draft = state
                    .draft
                    .presence
                    .get_or_insert_with(|| PresenceDraft::from_settings(settings));

                section(ui, t, s.rich_presence_external);
                group(ui, t, |rows| {
                    if rows.field_settled(s.rich_presence_host, &mut draft.host, 255) {
                        let host = draft.host.trim();
                        let host = if host.is_empty() { "127.0.0.1" } else { host };
                        if settings.external_host != host {
                            settings.external_host = host.to_owned();
                        }
                        draft.host = settings.external_host.clone();
                    }
                    if rows.field_settled(s.rich_presence_port, &mut draft.port, 5) {
                        match draft.port.trim().parse::<u16>() {
                            Ok(port) if port > 0 => settings.external_port = port.to_string(),
                            _ => {}
                        }
                        draft.port = settings.external_port.clone();
                    }
                });
                footnote(ui, t, s.rich_presence_external_hint);

                section(ui, t, s.rich_presence_advanced);
                group(ui, t, |rows| {
                    rows.row(
                        s.rich_presence_auto_reconnect,
                        Some(s.rich_presence_auto_reconnect_hint),
                        |ui, t| {
                            switch(ui, t, &mut settings.auto_reconnect);
                        },
                    );
                    if settings.auto_reconnect
                        && rows.field_settled(
                            s.rich_presence_reconnect_interval,
                            &mut draft.interval,
                            3,
                        )
                    {
                        if let Ok(seconds) = draft.interval.trim().parse::<u64>() {
                            settings.reconnect_interval = seconds.clamp(1, 300).to_string();
                        }
                        draft.interval = settings.reconnect_interval.clone();
                    }
                    rows.row(
                        s.rich_presence_debug,
                        Some(s.rich_presence_debug_hint),
                        |ui, t| {
                            switch(ui, t, &mut settings.debug);
                        },
                    );
                    if rows.action(
                        s.rich_presence_restart,
                        Some(s.rich_presence_restart_hint),
                        false,
                    ) {
                        actions.push(SettingsAction::RestartRichPresence);
                    }
                });
            }
        }

        AppPane::Alerts => {
            hero(ui, t, 50.0, |ui, area| {
                let card = Rect::from_min_size(area.min, Vec2::new(area.width(), 50.0));
                let initials: String = data
                    .store
                    .server
                    .as_ref()
                    .map(|server| server.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase())
                    .unwrap_or_else(|| "P".to_owned());
                notification_preview(ui, t, s, card, &initials, *data.notifications);
            });
            caption_below(ui, t, s.preview_notify_hint);
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
                rows.row(s.close_to_tray_title, Some(s.close_to_tray_hint), |ui, t| {
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
                state.draft.server_public = data
                    .store
                    .server
                    .as_ref()
                    .is_none_or(|server| server.public);
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
            // O cabeçalho da identidade: o ícone grande (clicar troca), o nome
            // editável ali mesmo e, embaixo, quantos são e o atalho do ícone.
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                ui.spacing_mut().item_spacing.x = 18.0;
                let (slot, response) = ui.allocate_exact_size(Vec2::splat(84.0), Sense::click());
                paint_server_icon(ui, t, slot, icon, &initials, 22);
                if response.hovered() {
                    ui.painter().rect_filled(slot, CornerRadius::same(22), egui::Color32::from_black_alpha(115));
                    ui.painter().text(slot.center(), egui::Align2::CENTER_CENTER, s.server_icon_swap, px(11.5, "regular"), egui::Color32::WHITE);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() {
                    pick_icon = true;
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space(8.0);
                    crate::ui::profile::inline_field_fit(
                        ui,
                        t,
                        "servidor-nome",
                        &mut draft.server_name,
                        s.server_name,
                        px(22.0, "display"),
                        t.label,
                        64,
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("{} {}", data.store.members.len(), s.count_members))
                            .font(px(12.0, "regular"))
                            .color(t.label_secondary),
                    );
                    ui.add_space(6.0);
                    let link = ui.add(
                        egui::Label::new(RichText::new(s.change_server_icon).font(px(12.0, "regular")).color(t.accent))
                            .sense(Sense::click()),
                    );
                    if link.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if link.clicked() {
                        pick_icon = true;
                    }
                });
            });
            section(ui, t, s.server_access);
            group(ui, t, |rows| {
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

            let (saved_name, saved_public) = data
                .store
                .server
                .as_ref()
                .map(|server| (server.name.clone(), server.public))
                .unwrap_or_default();
            let making_private = saved_public && !draft.server_public;
            let name_ok = !draft.server_name.trim().is_empty();
            let password_ok = !making_private || draft.server_password.chars().count() >= 8;
            let dirty = draft.server_name.trim() != saved_name.trim()
                || draft.server_public != saved_public
                || !draft.server_password.is_empty();
            let mut save_server = false;
            let mut discard_server = false;
            if dirty {
                // Mesma pastilha do perfil: só existe enquanto há algo por salvar
                // e diz o que falta quando ainda não dá.
                let pill_id = egui::Id::new("servidor-salvar");
                let blocked = if !name_ok {
                    Some(s.server_name_required)
                } else if !password_ok {
                    Some(s.server_password_short)
                } else {
                    None
                };
                crate::ui::widgets::floating_pill(
                    ui.ctx(),
                    pill_id,
                    t,
                    *data.translucency,
                    &data.glass,
                    space::LG,
                    420.0,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = space::SM;
                            ui.label(
                                RichText::new(blocked.unwrap_or(s.unsaved_changes))
                                    .font(text::body())
                                    .color(t.label_secondary),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if blocked.is_none() && row_button(ui, t, s.save_server, Emphasis::Primary) {
                                    save_server = true;
                                }
                                if row_button(ui, t, s.discard, Emphasis::Quiet) {
                                    discard_server = true;
                                }
                            });
                        });
                    },
                );
                ui.ctx().move_to_top(egui::LayerId::new(egui::Order::Foreground, pill_id));
            }
            if save_server {
                actions.push(SettingsAction::Admin(AdminAction::SaveServer(Box::new(
                    crate::api::models::PatchServerRequest {
                        name: Some(draft.server_name.trim().to_owned()),
                        public: Some(draft.server_public),
                        password: (!draft.server_password.is_empty())
                            .then(|| draft.server_password.clone()),
                        ..Default::default()
                    },
                ))));
            }
            if discard_server {
                draft.loaded_server = false;
            }
        }

        ServerPane::Channels => {
            let mut create_kind: Option<&str> = None;
            // A barra do topo: o nome da lista e os dois botões de criar.
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                ui.label(RichText::new(s.channels_list_title).font(px(11.5, "semibold")).color(t.label_secondary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if row_button(ui, t, s.create_channel, Emphasis::Quiet) {
                        create_kind = Some("text");
                    }
                    if row_button(ui, t, s.create_category, Emphasis::Quiet) {
                        create_kind = Some("category");
                    }
                });
            });
            ui.add_space(10.0);
            let channels: Vec<_> = data
                .store
                .channels
                .iter()
                .filter(|channel| channel.kind != crate::state::ChannelKind::Direct)
                .cloned()
                .collect();
            if let Some((name, _)) = &state.pending_channel_people
                && let Some(created) = channels.iter().find(|c| c.name == *name && !state.channel_people.contains_key(&c.id))
                && let Some((_, people)) = state.pending_channel_people.take()
            {
                state.channel_people.insert(created.id.clone(), people);
            }
            let draft = &mut state.draft;
            let mut close_editor = false;
            let mut cancel_delete = false;
            let mut confirmed_delete: Option<String> = None;

            if let Some(editor) = draft.channel.as_mut() {
                let ctx = ui.ctx().clone();
                let translucent = *data.translucency;
                let glass = data.glass.clone();
                let store = data.store;
                let existing = editor.id.clone();
                section(ui, t, if existing.is_some() { s.edit_channel_title } else { s.create_channel });
                let mut finish: Option<bool> = None;
                sheet_card(ui, t, |card| {
                    card.section(s.role_name, |ui| {
                        channel_name_input(ui, t, editor, store, data.media, s);
                    });
                    card.section(s.channel_kind, |ui| {
                        let locked = existing.is_some();
                        ui.add_enabled_ui(!locked, |ui| {
                            let mut kind: &'static str = match editor.kind.as_str() {
                                "voice" => "voice",
                                "category" => "category",
                                _ => "text",
                            };
                            // Sempre lado a lado, também no celular: são três opções curtas.
                            if segmented_inner(ui, t, &mut kind, &[("text", s.channel_kind_text), ("voice", s.channel_kind_voice), ("category", s.channel_kind_category)], true) {
                                editor.kind = kind.to_owned();
                            }
                        });
                        if locked {
                            ui.add_space(8.0);
                            ui.label(RichText::new(s.channel_kind_locked).font(px(12.0, "regular")).color(t.label_secondary));
                        }
                    });
                    if editor.kind != "category" {
                        card.section(s.channel_topic, |ui| {
                            crate::ui::profile::inline_field(ui, t, "canal-topico", &mut editor.topic, s.channel_topic_hint, px(13.0, "regular"), t.label, 512, (1, 3));
                        });
                    }
                    if editor.kind != "category" {
                        card.section(s.channel_category, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                                let mut options: Vec<(Option<String>, String)> = vec![(None, s.no_category.to_owned())];
                                options.extend(channels.iter().filter(|c| c.kind == crate::state::ChannelKind::Category).map(|c| (Some(c.id.clone()), c.name.clone())));
                                for (value, label) in options {
                                    let selected = editor.parent == value;
                                    let font = px(12.0, "regular");
                                    let text_w = ui.painter().layout_no_wrap(label.clone(), font.clone(), t.label).size().x;
                                    let (rect, response) = ui.allocate_exact_size(Vec2::new(text_w + 22.0, 24.0), Sense::click());
                                    let fill = if selected { t.accent.gamma_multiply(0.16) } else if response.hovered() { t.fill_medium } else { t.fill_soft };
                                    ui.painter().rect(rect, CornerRadius::same(12), fill, if selected { Stroke::new(1.0, t.accent) } else { Stroke::NONE }, egui::StrokeKind::Inside);
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &label, font, if selected { t.accent } else { t.label });
                                    if response.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if response.clicked() {
                                        editor.parent = value;
                                    }
                                }
                            });
                        });
                    }
                    if editor.kind != "category" {
                        card.section(s.who_access, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                                // "Todos" é o que o canal tem até alguém ser adicionado.
                                if editor.access_roles.is_empty() && editor.people.is_empty() {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.painter().layout_no_wrap(s.everyone_chip.to_owned(), px(12.0, "regular"), t.accent).size().x + 34.0, 24.0), Sense::hover());
                                    ui.painter().rect_filled(rect, CornerRadius::same(12), t.accent.gamma_multiply(0.16));
                                    ui.painter().text(egui::pos2(rect.min.x + 10.0, rect.center().y), egui::Align2::LEFT_CENTER, egui_phosphor::regular::GLOBE, text::icon(12.0), t.accent);
                                    ui.painter().text(egui::pos2(rect.min.x + 26.0, rect.center().y), egui::Align2::LEFT_CENTER, s.everyone_chip, px(12.0, "regular"), t.accent);
                                }
                                let mut drop_role = None;
                                for id in &editor.access_roles {
                                    if let Some(role) = store.roles.iter().find(|role| &role.id == id) {
                                        let color = role.color.as_deref().and_then(crate::api::models::parse_hex_color).map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)).unwrap_or(t.label_tertiary);
                                        if chip(ui, t, &format!("canal-cargo-{id}"), &role.name, Some(color), None, true) {
                                            drop_role = Some(id.clone());
                                        }
                                    }
                                }
                                if let Some(id) = drop_role {
                                    editor.access_roles.retain(|x| *x != id);
                                }
                                let mut drop_person = None;
                                for id in &editor.people {
                                    if let Some(member) = store.member(id) {
                                        let tint = person_tint(store, member, t.accent);
                                        if chip(ui, t, &format!("canal-pessoa-{id}"), &member.name, None, Some((&member.initials(), tint)), true) {
                                            drop_person = Some(id.clone());
                                        }
                                    }
                                }
                                if let Some(id) = drop_person {
                                    editor.people.retain(|x| *x != id);
                                }
                                let role_button = add_chip(ui, t, s.add_role);
                                let free_roles: Vec<&crate::api::models::Role> = store.roles.iter().filter(|role| !editor.access_roles.contains(&role.id)).collect();
                                let items: Vec<PickItem> = free_roles
                                    .iter()
                                    .map(|role| PickItem {
                                        label: role.name.clone(),
                                        dot: Some(role.color.as_deref().and_then(crate::api::models::parse_hex_color).map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)).unwrap_or(t.label_tertiary)),
                                        avatar: None,
                                    })
                                    .collect();
                                if let Some(index) = picker_menu(&role_button, t, s.search_role, s.nothing_to_add, &items) {
                                    editor.access_roles.push(free_roles[index].id.clone());
                                }
                                let person_button = add_chip(ui, t, s.add_person);
                                let free_people: Vec<&crate::state::Member> = store.members.iter().filter(|m| !editor.people.contains(&m.id)).collect();
                                let items: Vec<PickItem> = free_people
                                    .iter()
                                    .map(|m| PickItem { label: m.name.clone(), dot: None, avatar: Some((m.initials(), person_tint(store, m, t.accent))) })
                                    .collect();
                                if let Some(index) = picker_menu(&person_button, t, s.search_person, s.nothing_to_add, &items) {
                                    editor.people.push(free_people[index].id.clone());
                                }
                            });
                        });
                    }
                });

                let blocked = editor.name.trim().is_empty().then_some(s.channel_name_required);
                let label = if existing.is_some() { s.save_channel } else { s.create_channel };
                match save_pill(&ctx, "canal-salvar", t, translucent, &glass, s, label, blocked) {
                    Some(true) => {
                        let name = editor.name.trim().to_owned();
                        let topic = (editor.kind != "category").then(|| editor.topic.trim().to_owned());
                        match existing.clone() {
                            Some(channel_id) => {
                                if editor.parent != editor.original_parent
                                    && let Some(channel) = channels.iter().find(|c| c.id == channel_id)
                                {
                                    actions.push(SettingsAction::Chat(ChatAction::MoveChannel {
                                        channel_id: channel_id.clone(),
                                        old_position: channel.position,
                                        new_position: channel.position,
                                        parent_id: Some(editor.parent.clone().unwrap_or_default()),
                                    }));
                                }
                                for role_id in editor.access_roles.iter().filter(|id| !editor.original_access_roles.contains(id)) {
                                    actions.push(SettingsAction::Chat(ChatAction::SetChannelPermissions {
                                        channel_id: channel_id.clone(),
                                        role_id: role_id.clone(),
                                        permissions: crate::api::models::ChannelPermissions {
                                            read_channel: true,
                                            send_messages: true,
                                            delete_messages: false,
                                            connect_voice: true,
                                        },
                                    }));
                                }
                                for role_id in editor.original_access_roles.iter().filter(|id| !editor.access_roles.contains(id)) {
                                    actions.push(SettingsAction::Chat(ChatAction::RemoveChannelRolePermission {
                                        channel_id: channel_id.clone(),
                                        role_id: role_id.clone(),
                                    }));
                                }
                                state.channel_people.insert(channel_id.clone(), editor.people.clone());
                                actions.push(SettingsAction::Chat(ChatAction::UpdateChannel { channel_id, name, topic }));
                            }
                            None => {
                                let is_category = editor.kind == "category";
                                if !is_category && !editor.people.is_empty() {
                                    state.pending_channel_people = Some((name.clone(), editor.people.clone()));
                                }
                                actions.push(SettingsAction::Chat(ChatAction::CreateChannel {
                                    name,
                                    kind: editor.kind.clone(),
                                    topic: topic.filter(|topic| !topic.is_empty()),
                                    parent_id: if is_category { None } else { editor.parent.clone() },
                                    access_roles: if is_category { Vec::new() } else { editor.access_roles.clone() },
                                }));
                            }
                        }
                        finish = Some(true);
                    }
                    Some(false) => finish = Some(false),
                    None => {}
                }
                if finish.is_some() {
                    close_editor = true;
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
                match channel_tree(
                    ui,
                    t,
                    s,
                    data.store,
                    data.media,
                    *data.channel_emoji_monochrome,
                    data.collapsed,
                    &mut state.drag_channel,
                ) {
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

            if let Some(kind) = create_kind {
                draft.deleting = None;
                draft.channel_permission = None;
                let mut editor = ChannelDraft::create();
                editor.kind = kind.to_owned();
                draft.channel = Some(editor);
            }
        }

        ServerPane::Roles => roles_pane(ui, data, t, s, actions),

        ServerPane::Emojis => {
            // Escolher o arquivo abre o editor: recorte, prévia na conversa
            // e o nome, tudo num lugar só; Salvar já sobe.
            let width = fit_width(ui);
            let (drop, response) = ui.allocate_exact_size(Vec2::new(width, 62.0), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(drop, CornerRadius::same(12), t.fill_soft);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let dash = Stroke::new(1.5, t.fill_medium);
            let outline = [drop.left_top(), drop.right_top(), drop.right_bottom(), drop.left_bottom(), drop.left_top()];
            ui.painter().extend(egui::Shape::dashed_line(&outline, dash, 5.0, 4.0));
            ui.painter().text(egui::pos2(drop.center().x, drop.center().y - 8.0), egui::Align2::CENTER_CENTER, format!("+ {}", s.add_emoji), px(13.0, "regular"), t.label_secondary);
            ui.painter().text(egui::pos2(drop.center().x, drop.center().y + 10.0), egui::Align2::CENTER_CENTER, s.sticker_hint, px(11.5, "regular"), t.label_tertiary);
            if response.clicked() {
                actions.push(SettingsAction::Admin(AdminAction::PickSticker));
            }

            section(ui, t, s.server_emojis);
            let stickers: Vec<(String, String, Option<String>)> = data
                .store
                .emojis
                .iter()
                .map(|emoji| (emoji.id.clone(), emoji.name.clone(), emoji.blob.clone()))
                .collect();
            if stickers.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(ROW_INSET);
                    ui.label(RichText::new(s.no_stickers).font(px(12.0, "regular")).color(t.label_secondary));
                });
            }
            // Grade de cartões: 104 de largura mínima, 10 entre eles.
            let gap = 10.0;
            let columns = (((width + gap) / (104.0 + gap)).floor() as usize).max(1);
            let card_w = (width - gap * (columns as f32 - 1.0)) / columns as f32;
            let ctx = ui.ctx().clone();
            for chunk in stickers.chunks(columns) {
                let (row, _) = ui.allocate_exact_size(Vec2::new(width, 104.0), Sense::hover());
                for (column, (id, name, blob)) in chunk.iter().enumerate() {
                    let rect = Rect::from_min_size(egui::pos2(row.min.x + column as f32 * (card_w + gap), row.min.y), Vec2::new(card_w, 104.0));
                    let hovered = ui.rect_contains_pointer(rect);
                    ui.painter().rect(rect, CornerRadius::same(12), t.card, Stroke::new(0.5, t.separator), egui::StrokeKind::Inside);
                    let texture = data
                        .media
                        .emoji(id, blob.as_deref())
                        .and_then(|texture| texture.frame(&ctx));
                    let art = Rect::from_center_size(egui::pos2(rect.center().x, rect.min.y + 10.0 + 28.0), Vec2::splat(56.0));
                    ui.painter().rect_filled(art, CornerRadius::same(14), t.fill_soft);
                    draw_sticker(ui, t, art.shrink(8.0), texture);
                    crate::ui::widgets::text_fit(
                        ui.painter(),
                        egui::pos2(rect.center().x, rect.max.y - 18.0),
                        egui::Align2::CENTER_CENTER,
                        &format!(":{name}:"),
                        px(11.5, "regular"),
                        t.label_secondary,
                        rect.width() - 12.0,
                    );
                    let trash = Rect::from_min_size(egui::pos2(rect.max.x - 4.0 - 28.0, rect.min.y + 4.0), Vec2::splat(28.0));
                    let tool = ui.interact(trash, egui::Id::new(("apagar-figurinha", id)), if hovered || tiles(ui) { Sense::click() } else { Sense::hover() });
                    if hovered || tiles(ui) {
                        if tool.hovered() {
                            ui.painter().rect_filled(trash, CornerRadius::same(8), t.fill_soft);
                        }
                        ui.painter().text(trash.center(), egui::Align2::CENTER_CENTER, egui_phosphor::regular::TRASH, text::icon(15.0), if tool.hovered() { t.danger } else { t.label_secondary });
                    }
                    if tool.clicked() {
                        actions.push(SettingsAction::Admin(AdminAction::DeleteEmoji(id.clone())));
                    }
                }
                ui.add_space(gap);
            }
            // O backend não tem como renomear: `/emojis` só aceita criar e
            // apagar. Dizer isso é melhor do que oferecer um campo que não
            // salvaria, ou apagar e recriar por baixo — o id mudaria e as
            // reações já feitas com ela iriam junto.
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                ui.label(
                    RichText::new(s.sticker_rename_note)
                        .font(px(11.5, "regular"))
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
    search_input(ui, t, "auditoria-pessoa", &mut filter.person, field, space::MD + 18.0, s.audit_person_search, text::callout(), false);
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
            .filter(|channel| {
                !matches!(
                    channel.kind,
                    crate::state::ChannelKind::Category | crate::state::ChannelKind::Direct
                )
            })
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
    use super::{TreeAction, move_action, tree_drop};

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

    #[test]
    fn step_up_from_the_first_child_leaves_the_category() {
        let items = items();
        // dev é o índice 2; subir um passo é a fresta antes de Projetos.
        match move_action(&items, 2, 1, 5) {
            Some(TreeAction::Move { channel_id, new_position, parent_id, .. }) => {
                assert_eq!(channel_id, "dev");
                assert_eq!(new_position, 2);
                assert_eq!(parent_id, Some(String::new()));
            }
            _ => panic!("devia mover"),
        }
    }

    #[test]
    fn step_down_into_an_open_category_enters_it() {
        let items = items();
        // geral (0) desce: depois de Projetos, logo na primeira vaga dela.
        match move_action(&items, 0, 2, 5) {
            Some(TreeAction::Move { channel_id, parent_id, .. }) => {
                assert_eq!(channel_id, "geral");
                assert_eq!(parent_id, Some("proj".to_owned()));
            }
            _ => panic!("devia mover"),
        }
    }
}

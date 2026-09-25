//! Janela principal: três colunas (canais · conversa · membros).
//!
//! As barras funcionais (laterais, topo e caixa de mensagem) formam a camada
//! translúcida; a lista de mensagens é a camada de conteúdo, sempre opaca.

use chrono::{DateTime, Datelike, Local};
use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Rect, RichText, Sense, Stroke,
    TextEdit, UiBuilder, Vec2,
};
use egui_phosphor::regular as icon;

use crate::api::client::Upload;
use crate::i18n::Strings;
use crate::media::MediaStore;
use crate::platform::menu::MenuCommand;
use crate::state::{ChannelKind, Emoji, MentionBinding, Message, Presence, Store};

use super::attachments::{self, MediaAction};
use super::emoji;
use super::glass::SharedGlass;
use super::theme::{radius, space, text, Tokens, HIT_TARGET};
use super::viewer::{self, Viewer, ViewerAction};
use super::widgets::{avatar, floating_pill, icon_button, scroll_edge_fade, section_caption, sidebar_frame};

pub const SIDEBAR_WIDTH: f32 = 232.0;
pub const MEMBERS_WIDTH: f32 = 196.0;
/// Abaixo disto a conversa vira a superfície raiz e as laterais viram drawers.
pub const COMPACT_BREAKPOINT: f32 = 820.0;
/// Um arrasto só vira gesto depois de andar isto, em pontos. Antes disso
/// ainda pode ser um toque, e roubar o movimento cedo demais faria a rolagem
/// engasgar a cada encostada.
const SWIPE_SLOP: f32 = 6.0;
/// O quanto o movimento precisa ser mais horizontal que vertical para ser
/// nosso. Sem isto, rolar a conversa arrastaria a gaveta junto.
const SWIPE_AXIS_BIAS: f32 = 1.25;
/// Quanto da gaveta precisa estar à mostra, ao soltar o dedo, para ela
/// terminar de abrir. Abaixo disso ela volta.
///
/// É uma fração da largura da gaveta, não uma distância fixa: assim a de
/// navegação e a de pessoas, que têm larguras diferentes, pedem o mesmo
/// gesto proporcional.
const DRAWER_COMMIT: f32 = 0.4;
/// Velocidade a partir da qual um piparote decide sozinho, em pontos por
/// segundo, mesmo que o dedo não tenha andado o bastante.
const FLICK_SPEED: f32 = 450.0;
/// Quanto a mensagem acompanha o dedo coladinha antes de começar a resistir
/// — e também a distância que dispara a resposta.
const REPLY_TRAVEL: f32 = 72.0;
const SIDEBAR_HEADER_HEIGHT: f32 = IDENTITY_PILL_HEIGHT + PILL_INSET * 2.0;
/// As duas pastilhas de identidade: a do servidor e a da conta.
pub const IDENTITY_PILL_HEIGHT: f32 = 46.0;
/// Respiro das pastilhas contra a coluna — o mesmo nos quatro lados, e não
/// em dois valores diferentes como estava.
pub const PILL_INSET: f32 = space::MD;
/// Altura das pastilhas flutuantes e respiro entre elas e a borda.
const PILL_HEIGHT: f32 = 36.0;
const PILL_MARGIN: f32 = 12.0;
/// Raio das pastilhas flutuantes — o mesmo canto do realce interno.
const PILL_RADIUS: f32 = 12.0;
/// Largura da pastilha esticada, e teto da parte de baixo dela.
const PANEL_WIDTH: f32 = 380.0;
const PANEL_MAX_BODY: f32 = 360.0;
/// Quanto tempo a mensagem alcançada fica piscando, e quantas piscadas.
const BLINK_SECONDS: f64 = 1.4;
const BLINKS: f64 = 2.0;
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
const COMPOSER_LINE_H: f32 = 52.0;
/// O compositor cresce pelo número de linhas VISUAIS (incluindo wrap), até
/// quatro linhas. Depois disso a própria área de texto rola internamente.
const COMPOSER_MAX_ROWS: usize = 4;
const TOAST_SECONDS: f64 = 6.0;

/// O que a conversa pede para a camada de cima fazer.
#[derive(Debug, Clone)]
pub enum ChatAction {
    Send {
        content: String,
        reply_to: Option<String>,
        notify_reply: bool,
        attachments: Vec<Upload>,
    },
    Edit {
        message_id: String,
        content: String,
    },
    Delete(String),
    RetryOutgoing(String),
    DismissOutgoing(String),
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
    /// Abre a criação inline em Ajustes do servidor → Canais.
    NewChannel,
    /// Abre a edição inline de um canal existente.
    EditChannel(String),
    /// Abre a confirmação inline de exclusão.
    RequestDeleteChannel(String),
    /// Cria um canal depois de confirmar o formulário inline.
    CreateChannel {
        name: String,
        kind: String,
        topic: Option<String>,
    },
    /// Salva nome/tópico de um canal depois da edição inline.
    UpdateChannel {
        channel_id: String,
        name: String,
        topic: Option<String>,
    },
    /// Apaga o canal depois da confirmação por nome.
    DeleteChannel(String),
    /// Busca no servidor, a partir da pastilha.
    Search(String),
    /// `off`, `only_mentions` ou `all` para este canal.
    ChannelNotifications {
        channel_id: String,
        setting: &'static str,
    },
    /// Troca o canal de lugar com o vizinho.
    MoveChannel {
        channel_id: String,
        old_position: i32,
        new_position: i32,
    },
    BanUser {
        user_id: String,
        banned: bool,
    },
    ResetUser(String),
    /// Entra na call do canal de voz.
    JoinVoice(String),
    LeaveVoice,
    ToggleMute,
    ToggleCamera,
    /// Encolhe a folha da call numa pastilha (ou a abre de volta).
    CollapseCall(bool),
    /// Volta para a call: leva à sala em que já se está, como o clique que
    /// levou na primeira vez.
    OpenCall,
    /// Mostra/esconde o mini-overlay da call sobre a conversa.
    FloatCall(bool),
    /// Joga a call numa janela do sistema só dela (ou a traz de volta).
    PopOutCall(bool),
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

/// Qual lista a pastilha está mostrando quando está aberta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Search,
    Pinned,
}

/// Superfície que ocupa a frente no layout estreito.
///
/// A conversa é a raiz. Navegação e pessoas só cobrem a conversa enquanto
/// estão abertas; por isso não são páginas independentes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MobileSurface {
    #[default]
    Chat,
    Navigation,
    People,
}

/// O shell só precisa saber desenhar o trilho quando ele está dentro do drawer.
/// A aplicação continua sendo dona da troca/remoção dos workspaces.
pub struct MobileServers<'a> {
    pub entries: &'a [super::rail::Entry],
    pub active: usize,
}

#[derive(Clone, Debug)]
struct MobileGesture {
    origin: egui::Pos2,
    last: egui::Pos2,
    /// Quando `last` foi visto, para tirar a velocidade do piparote.
    last_time: f64,
    /// Velocidade horizontal recente, em pontos por segundo. Vai sendo
    /// suavizada para um tranco isolado não decidir o gesto sozinho.
    velocity: f32,
    message_id: Option<String>,
    blocked: bool,
    /// O que este arrasto resolveu ser. Decide-se uma vez, no primeiro
    /// movimento que passa da folga, e não muda mais: a meio caminho o dedo
    /// não deve trocar de gesto por acidente.
    intent: Option<Intent>,
}

/// O que um arrasto virou.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Intent {
    /// Mostra ou esconde esta gaveta.
    Drawer(MobileSurface),
    /// Puxa esta mensagem para responder.
    Reply(String),
    /// Vertical: o movimento é da rolagem, não nosso.
    Scroll,
}

/// A gaveta que aparece na tela e o quanto dela aparece.
///
/// Enquanto o dedo está na tela é ele quem manda em `shown`; ao soltar, o
/// valor corre sozinho até `target` (0 ou 1). `surface` continua valendo
/// durante a saída, que é o que deixa a gaveta terminar de sair depois de
/// `mobile_surface` já ter voltado a ser a conversa.
#[derive(Clone, Debug)]
struct Drawer {
    surface: MobileSurface,
    /// De 0 (fora da tela) a 1 (inteira à mostra).
    shown: f32,
    target: f32,
    dragging: bool,
}

impl Default for Drawer {
    fn default() -> Self {
        Self {
            surface: MobileSurface::Chat,
            shown: 0.0,
            target: 0.0,
            dragging: false,
        }
    }
}

/// A mensagem sendo puxada para responder.
#[derive(Clone, Debug, Default)]
struct ReplyDrag {
    id: String,
    /// Quanto ela já andou para a esquerda, em pontos, depois da resistência.
    shown: f32,
    /// O dedo ainda está segurando.
    dragging: bool,
}

/// Retorna se a largura pede a navegação de uma coluna.
pub fn is_compact(rect: Rect) -> bool {
    rect.width() < COMPACT_BREAKPOINT
}

/// A pastilha de ações, esticada para mostrar busca ou fixadas.
#[derive(Clone, Debug)]
pub struct Panel {
    pub kind: PanelKind,
    pub query: String,
    /// O campo de busca recebe o foco uma vez, ao abrir.
    pub focus: bool,
    /// Distingue "ainda não buscou" de uma busca válida com zero resultados.
    pub searched: bool,
}

/// Teclas que pertencem à lista de sugestões neste quadro.
#[derive(Clone, Copy, Debug, Default)]
struct SuggestKeys {
    up: bool,
    down: bool,
    accept: bool,
    dismiss: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestKind {
    Sticker,
    Mention,
}

/// Sugestão contextual do compositor: `:figurinha` ou `@pessoa`.
#[derive(Clone, Debug)]
pub struct Suggest {
    pub kind: SuggestKind,
    /// Onde está o caractere que abriu a sugestão, em caracteres.
    pub start: usize,
    /// Fim exato do fragmento que originou esta lista. A aceitação substitui
    /// start..end em vez de anexar a escolha ao texto parcial.
    pub end: usize,
    /// Ids de emoji ou de membro, conforme `kind`.
    pub matches: Vec<String>,
    /// Qual delas está marcada.
    pub index: usize,
}

/// Mensagem que a janela está tentando alcançar, vinda de um resultado.
#[derive(Clone, Debug)]
pub struct Jump {
    pub message_id: String,
    /// Instante em que a mensagem foi encontrada; antes disso ela ainda
    /// pode estar num canal cujas mensagens não chegaram.
    pub found: Option<f64>,
    /// Quando a busca começou, para desistir se o canal nunca carregar.
    pub since: f64,
}

#[derive(Clone, Debug)]
pub struct LinkViewer {
    pub id: String,
    pub url: String,
    /// O clique que abriu o overlay não pode fechá-lo no mesmo quadro.
    pub opened: f64,
}

#[derive(Clone, Debug)]
pub struct ExternalLinkPrompt {
    pub url: String,
    pub host: String,
    pub remember: bool,
    pub opened: f64,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftState {
    pub text: String,
    pub mentions: Vec<MentionBinding>,
    pub reply_to: Option<String>,
    pub notify_reply: bool,
}

impl DraftState {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.reply_to.is_none()
    }

    fn from_ui(ui: &UiState) -> Self {
        Self {
            text: ui.composer.clone(),
            mentions: ui.composer_mentions.clone(),
            reply_to: ui.replying.clone(),
            notify_reply: ui.reply_notify,
        }
    }

    fn apply_to(self, ui: &mut UiState) {
        ui.composer = self.text;
        ui.composer_mentions = self.mentions;
        ui.replying = self.reply_to;
        ui.reply_notify = self.notify_reply;
    }

    fn from_cached(draft: papo_core::cache::CachedDraft) -> Self {
        let text = draft.text;
        let chars: Vec<char> = text.chars().collect();
        let mentions = draft
            .mentions
            .into_iter()
            .filter_map(|binding| {
                if binding.user_id.is_empty() || binding.label.is_empty() || binding.start >= chars.len() {
                    return None;
                }
                if chars[binding.start] != '@' {
                    return None;
                }
                let label: Vec<char> = binding.label.chars().collect();
                let end = binding.start + 1 + label.len();
                if end > chars.len() || chars[binding.start + 1..end] != label[..] {
                    return None;
                }
                Some(MentionBinding {
                    start: binding.start,
                    label: binding.label,
                    user_id: binding.user_id,
                })
            })
            .collect();
        Self {
            text,
            mentions,
            reply_to: draft.reply_to,
            notify_reply: draft.notify_reply,
        }
    }

    fn to_cached(
        &self,
        owner_user_id: &str,
        channel_id: &str,
    ) -> papo_core::cache::CachedDraft {
        papo_core::cache::CachedDraft {
            owner_user_id: owner_user_id.to_owned(),
            channel_id: channel_id.to_owned(),
            text: self.text.clone(),
            mentions: self
                .mentions
                .iter()
                .map(|binding| papo_core::cache::CachedMentionBinding {
                    start: binding.start,
                    label: binding.label.clone(),
                    user_id: binding.user_id.clone(),
                })
                .collect(),
            reply_to: self.reply_to.clone(),
            notify_reply: self.notify_reply,
            updated_at: papo_core::cache::now_millis(),
        }
    }
}

#[derive(Debug, Default)]
pub struct DraftBook {
    owner_user_id: Option<String>,
    loaded: bool,
    by_channel: std::collections::HashMap<String, DraftState>,
    dirty: std::collections::HashMap<String, std::time::Instant>,
}

impl DraftBook {
    pub fn owner(&self) -> Option<&str> {
        self.owner_user_id.as_deref()
    }

    pub fn is_loaded_for(&self, owner_user_id: &str) -> bool {
        self.loaded && self.owner() == Some(owner_user_id)
    }

    pub fn reset_owner(&mut self, owner_user_id: &str) {
        if self.owner() == Some(owner_user_id) {
            return;
        }
        self.owner_user_id = Some(owner_user_id.to_owned());
        self.loaded = false;
        self.by_channel.clear();
        self.dirty.clear();
    }

    pub fn load(&mut self, owner_user_id: &str, drafts: Vec<papo_core::cache::CachedDraft>) {
        self.owner_user_id = Some(owner_user_id.to_owned());
        self.loaded = true;
        self.by_channel.clear();
        self.dirty.clear();
        for draft in drafts {
            if draft.owner_user_id != owner_user_id {
                continue;
            }
            let channel_id = draft.channel_id.clone();
            let state = DraftState::from_cached(draft);
            if !state.is_empty() {
                self.by_channel.insert(channel_id, state);
            }
        }
    }

    pub fn capture(&mut self, channel_id: &str, state: DraftState) {
        if channel_id.is_empty() {
            return;
        }
        let changed = self.by_channel.get(channel_id) != Some(&state);
        if !changed {
            return;
        }
        if state.is_empty() {
            self.by_channel.remove(channel_id);
        } else {
            self.by_channel.insert(channel_id.to_owned(), state);
        }
        self.dirty.insert(channel_id.to_owned(), std::time::Instant::now());
    }

    pub fn put(&mut self, channel_id: &str, state: DraftState) {
        self.capture(channel_id, state);
    }

    pub fn get(&self, channel_id: &str) -> Option<DraftState> {
        self.by_channel.get(channel_id).cloned()
    }

    pub fn take_persistence_ops(
        &mut self,
        owner_user_id: &str,
        force: bool,
    ) -> Vec<papo_core::cache::CacheOp> {
        if self.owner() != Some(owner_user_id) {
            return Vec::new();
        }
        let now = std::time::Instant::now();
        let due: Vec<String> = self
            .dirty
            .iter()
            .filter(|(_, changed)| force || now.duration_since(**changed) >= std::time::Duration::from_millis(750))
            .map(|(channel_id, _)| channel_id.clone())
            .collect();
        let mut ops = Vec::with_capacity(due.len());
        for channel_id in due {
            self.dirty.remove(&channel_id);
            if let Some(draft) = self.by_channel.get(&channel_id) {
                ops.push(papo_core::cache::CacheOp::UpsertDraft(
                    draft.to_cached(owner_user_id, &channel_id),
                ));
            } else {
                ops.push(papo_core::cache::CacheOp::DeleteDraft {
                    owner_user_id: owner_user_id.to_owned(),
                    channel_id,
                });
            }
        }
        ops
    }
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
    pub composer_mentions: Vec<MentionBinding>,
    pub drafts: DraftBook,
    pub attachments: Vec<Upload>,
    pub replying: Option<String>,
    pub reply_notify: bool,
    pub editing: Option<(String, String)>,
    pub editing_mentions: Vec<MentionBinding>,
    /// A edição acabou de abrir e deve pedir foco exatamente uma vez.
    pub edit_focus_pending: bool,
    pub viewer: Option<Viewer>,
    pub link_viewer: Option<LinkViewer>,
    pub popup: Option<Popup>,
    pub last_channel: String,
    pub topic_since: Option<f64>,
}

impl Stash {
    pub fn new(media: MediaStore) -> Self {
        Self {
            media,
            composer: String::new(),
            composer_mentions: Vec::new(),
            drafts: DraftBook::default(),
            attachments: Vec::new(),
            replying: None,
            reply_notify: true,
            editing: None,
            editing_mentions: Vec::new(),
            edit_focus_pending: false,
            viewer: None,
            link_viewer: None,
            popup: None,
            last_channel: String::new(),
            topic_since: None,
        }
    }

    /// Troca este guardado com o que está na tela.
    pub fn swap(&mut self, ui: &mut UiState) {
        std::mem::swap(&mut self.media, &mut ui.media);
        std::mem::swap(&mut self.composer, &mut ui.composer);
        std::mem::swap(&mut self.composer_mentions, &mut ui.composer_mentions);
        std::mem::swap(&mut self.drafts, &mut ui.drafts);
        std::mem::swap(&mut self.attachments, &mut ui.attachments);
        std::mem::swap(&mut self.replying, &mut ui.replying);
        std::mem::swap(&mut self.reply_notify, &mut ui.reply_notify);
        std::mem::swap(&mut self.editing, &mut ui.editing);
        std::mem::swap(&mut self.editing_mentions, &mut ui.editing_mentions);
        std::mem::swap(&mut self.edit_focus_pending, &mut ui.edit_focus_pending);
        std::mem::swap(&mut self.viewer, &mut ui.viewer);
        std::mem::swap(&mut self.link_viewer, &mut ui.link_viewer);
        std::mem::swap(&mut self.popup, &mut ui.popup);
        std::mem::swap(&mut self.last_channel, &mut ui.last_channel);
        std::mem::swap(&mut self.topic_since, &mut ui.topic_since);
    }
}

pub struct UiState {
    pub composer: String,
    pub composer_mentions: Vec<MentionBinding>,
    pub drafts: DraftBook,
    pub show_members: bool,
    /// Layout estreito ativo neste quadro.
    pub compact: bool,
    pub mobile_surface: MobileSurface,
    /// Quantos vídeos o overlay compacto tenta manter visíveis (1, 2 ou 4).
    pub call_video_tiles: usize,
    mobile_gesture: Option<MobileGesture>,
    /// Quanto da gaveta está à mostra neste quadro.
    drawer: Drawer,
    /// A mensagem que está sendo puxada para responder, se houver.
    reply_drag: Option<ReplyDrag>,
    /// Retângulos das mensagens deste quadro, usados para swipe-to-reply sem
    /// roubar o drag vertical do ScrollArea.
    message_rows: Vec<(String, Rect)>,
    /// Zonas de timeline/waveform que, no Android, arbitram o próprio gesto:
    /// horizontal = scrub; vertical = rolagem. O swipe da mensagem não entra.
    media_seek_zones: Vec<Rect>,
    pub translucent: bool,
    pub pending: Vec<MenuCommand>,
    /// Renderizador do vidro fosco; ausente quando o backend não é o glow.
    pub glass: Option<SharedGlass>,
    /// O texto mudou neste quadro (dispara o evento de digitação).
    pub typed: bool,
    /// Coordenador global de previews. Não pertence a um servidor e por isso
    /// não entra no Stash quando o usuário troca de workspace.
    pub previews: Option<std::sync::Arc<papo_core::preview::PreviewCoordinator>>,
    /// Mídia baixada, decodificada e tocando.
    pub media: MediaStore,
    /// Browser rico é estado da janela/processo, nunca entra no Stash de servidor.
    pub webembed: crate::webembed::WebEmbedManager,
    /// O que fazer quando o cartão dono sai da viewport.
    pub webembed_behavior: crate::webembed::OffscreenBehavior,
    /// Até onde um WebEmbed flutuante acompanha a navegação.
    pub webembed_scope: crate::webembed::FloatScope,
    /// Largura lembrada do player flutuante em desktop. Mobile é edge-to-edge.
    pub webembed_float_width: f32,
    /// Canto superior esquerdo do player flutuante, em pontos. `None` usa o
    /// canto inferior padrão; um arrasto grava a posição escolhida.
    pub webembed_float_pos: Option<(f32, f32)>,
    /// Retângulo do player flutuante no quadro anterior. Uma pressão aqui
    /// pertence ao player, não a um gesto de gaveta/resposta por baixo.
    pub webembed_float_rect: Option<Rect>,
    /// Faixa realmente disponível para browser nativo dentro da conversa.
    /// O ScrollArea corre por baixo das pastilhas/compositor, mas uma WebView
    /// nativa não pode fazer isso: ela precisa ser recortada antes do chrome.
    pub webembed_chat_clip: Option<Rect>,
    /// Superfície egui que deve ficar por cima de qualquer browser nativo.
    pub webembed_blocked: bool,
    /// Arquivos escolhidos, ainda não enviados.
    pub attachments: Vec<Upload>,
    /// Mensagem sendo respondida.
    pub replying: Option<String>,
    /// Estado atual do @ desta resposta.
    pub reply_notify: bool,
    /// Valor usado sempre que uma nova resposta começa.
    pub reply_notify_default: bool,
    /// Mensagem sendo editada, com o texto em edição.
    pub editing: Option<(String, String)>,
    pub editing_mentions: Vec<MentionBinding>,
    /// Só o primeiro quadro da edição pede foco ao TextEdit.
    pub edit_focus_pending: bool,
    pub actions: Vec<ChatAction>,
    pub viewer: Option<Viewer>,
    pub link_viewer: Option<LinkViewer>,
    /// Hosts explicitly trusted by the user for opening links without asking.
    /// This is window/global state and is mirrored to persisted Settings.
    pub trusted_link_hosts: std::collections::BTreeSet<String>,
    pub external_link_prompt: Option<ExternalLinkPrompt>,
    pub popup: Option<Popup>,
    /// Pastilha de ações esticada em busca ou fixadas.
    pub panel: Option<Panel>,
    /// Mensagem a alcançar e piscar, vinda de um resultado.
    pub jump: Option<Jump>,
    /// Sugestão contextual de figurinha ou pessoa no compositor.
    pub suggest: Option<Suggest>,
    /// Esc dispensou a lista: ela não volta até o apelido mudar.
    pub suggest_muted: bool,
    /// Onde começava o apelido quando o Esc foi apertado.
    pub suggest_start: Option<usize>,
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
    /// Evita descartar vários quadros seguidos quando várias mídias terminam
    /// quase juntas; egui mostra um PERF WARNING depois de três consecutivos.
    last_relayout_discard: f64,
}

impl UiState {
    pub fn request_external_url(&mut self, ctx: &egui::Context, raw_url: impl Into<String>) {
        let url = raw_url.into();
        if !papo_core::preview::safe_remote_url(&url) {
            return;
        }
        let Some(host) = url::Url::parse(&url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
        else {
            return;
        };

        if self.trusted_link_hosts.contains(&host) {
            ctx.open_url(egui::OpenUrl::new_tab(url));
            return;
        }

        self.external_link_prompt = Some(ExternalLinkPrompt {
            url,
            host,
            remember: false,
            opened: ctx.input(|input| input.time),
        });
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            composer: String::new(),
            composer_mentions: Vec::new(),
            drafts: DraftBook::default(),
            show_members: true,
            compact: false,
            mobile_surface: MobileSurface::Chat,
            call_video_tiles: 2,
            mobile_gesture: None,
            drawer: Drawer::default(),
            reply_drag: None,
            message_rows: Vec::new(),
            media_seek_zones: Vec::new(),
            translucent: true,
            pending: Vec::new(),
            glass: None,
            typed: false,
            previews: None,
            media: MediaStore::new(None),
            webembed: crate::webembed::WebEmbedManager::default(),
            webembed_behavior: crate::webembed::OffscreenBehavior::default(),
            webembed_scope: crate::webembed::FloatScope::default(),
            webembed_float_width: 360.0,
            webembed_float_pos: None,
            webembed_float_rect: None,
            webembed_chat_clip: None,
            webembed_blocked: false,
            attachments: Vec::new(),
            replying: None,
            reply_notify: true,
            reply_notify_default: true,
            editing: None,
            editing_mentions: Vec::new(),
            edit_focus_pending: false,
            actions: Vec::new(),
            viewer: None,
            link_viewer: None,
            trusted_link_hosts: std::collections::BTreeSet::new(),
            external_link_prompt: None,
            popup: None,
            panel: None,
            jump: None,
            suggest: None,
            suggest_muted: false,
            suggest_start: None,
            emoji_query: String::new(),
            emoji_group: 0,
            last_channel: String::new(),
            topic_since: None,
            reveal_topic: true,
            show_record: true,
            recorder: None,
            error: None,
            relayout: false,
            last_relayout_discard: f64::NEG_INFINITY,
        }
    }
}

impl UiState {
    pub fn draft_state(&self) -> DraftState {
        DraftState::from_ui(self)
    }

    pub fn capture_draft(&mut self, channel_id: &str) {
        let state = DraftState::from_ui(self);
        self.drafts.capture(channel_id, state);
    }

    pub fn restore_draft(&mut self, channel_id: &str) {
        if let Some(draft) = self.drafts.get(channel_id) {
            draft.apply_to(self);
        } else {
            self.composer.clear();
            self.composer_mentions.clear();
            self.replying = None;
            self.reply_notify = self.reply_notify_default;
        }
    }

    pub fn switch_draft_channel(&mut self, channel_id: &str) {
        let previous = self.last_channel.clone();
        if !previous.is_empty() {
            self.capture_draft(&previous);
        }
        self.restore_draft(channel_id);
        self.last_channel = channel_id.to_owned();
    }

    fn start_reply(&mut self, message_id: String) {
        self.replying = Some(message_id);
        self.reply_notify = self.reply_notify_default;
    }

    fn close_popup(&mut self) {
        self.popup = None;
        self.emoji_query.clear();
    }
}

pub fn draw(
    ui: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    call: Option<&mut crate::voice::Call>,
    t: &Tokens,
    s: &Strings,
    mobile_servers: Option<MobileServers<'_>>,
) -> Option<super::rail::RailAction> {
    state.message_rows.clear();
    state.media_seek_zones.clear();

    // Mídia que acabou de chegar muda a altura das mensagens.
    if state.media.pump(ui.ctx()) {
        state.relayout = true;
    }

    // Resultado de um canal que nunca carregou: a busca não pode ficar
    // pendurada para sempre, ou a próxima mensagem com esse id piscaria do
    // nada muito depois.
    if let Some(jump) = &state.jump {
        let now = ui.input(|input| input.time);
        if jump.found.is_none() && now - jump.since > 10.0 {
            state.jump = None;
        }
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
        let next_channel = store.selected_channel.clone();
        state.switch_draft_channel(&next_channel);
        state.topic_since = Some(ui.input(|input| input.time));
        state.media.pause_all();
        if state.webembed_scope == crate::webembed::FloatScope::CurrentChannel {
            state.webembed.destroy_active();
        }
        state.media.saved = None;
        state.editing = None;
        state.editing_mentions.clear();
        state.edit_focus_pending = false;
        state.close_popup();

        // Mudar de canal também muda a apresentação da call:
        // - vídeo continua visível por cima da conversa;
        // - só voz vira apenas a pastilha compacta;
        // - voltar ao canal da call devolve a sala inteira.
        if store.call.active() {
            if store.selected_channel == store.call.channel_id {
                store.call.floating = false;
                store.call.collapsed = false;
            } else if store.call.has_video() {
                store.call.floating = true;
                store.call.collapsed = true;
                store.call.popped_out = false;
            } else {
                store.call.floating = false;
                store.call.collapsed = true;
            }
        }
    }

    // A altura da lista mudou no quadro anterior (uma reação a mais, por
    // exemplo): a rolagem ainda está no lugar antigo e a conversa daria um
    // pulo. Refazer o quadro antes de mostrá-lo resolve na origem.
    if std::mem::take(&mut state.relayout) {
        let now = ui.input(|input| input.time);
        if now - state.last_relayout_discard >= 0.20 {
            state.last_relayout_discard = now;
            ui.ctx().request_discard("a lista mudou de altura");
        } else {
            // A correção anterior ainda é recente: mais um discard só cria
            // uma sequência de quadros invisíveis. O repaint seguinte já
            // recebe a geometria nova sem disparar o diagnóstico do egui.
            ui.ctx().request_repaint();
        }
    }

    // A call de vídeo mora numa folha por cima da conversa; só voz fica no
    // próprio canal. Quem decide é o estado, não esta função.
    let stage = crate::ui::call::stage_of(store);
    let live = call.as_ref().is_some_and(|call| call.is_live());
    let shell_rect = ui.max_rect();
    state.compact = is_compact(shell_rect);

    if state.compact {
        conversation(ui, store, state, call, t, s, stage);
        let rail_action = mobile_drawers(
            ui,
            store,
            state,
            t,
            s,
            live,
            mobile_servers,
            shell_rect,
        );
        overlays(ui, store, state, t, s);
        if state.media.any_playing() {
            state.webembed.destroy_active();
        }
        webembed_floating(ui, state, t);
        rail_action
    } else {
        state.mobile_surface = MobileSurface::Chat;
        state.mobile_gesture = None;
        // A janela alargou no meio de um gesto: as gavetas não existem mais
        // aqui, e um arrasto pela metade não pode sobrar guardado.
        state.drawer = Drawer::default();
        state.reply_drag = None;
        channels_sidebar(ui, store, state, t, s, live);
        if state.show_members {
            members_sidebar(ui, store, state, t, s, false);
        }
        conversation(ui, store, state, call, t, s, stage);
        overlays(ui, store, state, t, s);
        if state.media.any_playing() {
            state.webembed.destroy_active();
        }
        webembed_floating(ui, state, t);
        None
    }
}

/// Leva a gaveta e a mensagem puxada até onde elas deviam estar.
///
/// Enquanto o dedo está na tela quem manda é ele, e aqui não se mexe em
/// nada. Solto o dedo, os dois correm sozinhos — e é também por aqui que a
/// gaveta aberta por um toque (no botão de pessoas, por exemplo) entra
/// deslizando em vez de aparecer de uma vez.
fn advance_surfaces(ui: &egui::Ui, state: &mut UiState) {
    if !state.drawer.dragging {
        let wanted = state.mobile_surface;
        if wanted == MobileSurface::Chat {
            state.drawer.target = 0.0;
        } else {
            if state.drawer.surface != wanted {
                // Trocou de gaveta sem passar pela conversa: a nova entra
                // do zero, senão ela apareceria já pela metade.
                state.drawer.surface = wanted;
                state.drawer.shown = 0.0;
            }
            state.drawer.target = 1.0;
        }
        let mut shown = state.drawer.shown;
        advance(&mut shown, state.drawer.target, 1.0, ui);
        state.drawer.shown = shown;
    }

    if let Some(drag) = state.reply_drag.as_mut()
        && !drag.dragging
    {
        let mut shown = drag.shown;
        advance(&mut shown, 0.0, REPLY_TRAVEL * 2.0, ui);
        drag.shown = shown;
        if shown <= 0.0 {
            state.reply_drag = None;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mobile_drawers(
    root: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    live: bool,
    servers: Option<MobileServers<'_>>,
    area: Rect,
) -> Option<super::rail::RailAction> {
    advance_surfaces(&*root, state);

    // Fora da tela e ninguém segurando: não há gaveta nenhuma para desenhar.
    if state.drawer.shown <= 0.0 && !state.drawer.dragging {
        return None;
    }
    let surface = state.drawer.surface;
    let width = drawer_width(surface, area);
    if width <= 0.0 {
        return None;
    }
    let shown = state.drawer.shown.clamp(0.0, 1.0);
    // Só o que já está à mostra fica dentro da tela; o resto espera do lado
    // de fora. É isto que faz a gaveta acompanhar o dedo.
    let hidden = width * (1.0 - shown);

    // Enquanto ela não está inteira à mostra, o toque fora dela ainda é do
    // arrasto: fechar no meio do caminho tiraria a gaveta da mão de quem a
    // está puxando.
    let settled = shown >= 1.0;

    match surface {
        MobileSurface::Chat => None,
        MobileSurface::Navigation => {
            let servers = servers?;
            let rect = Rect::from_min_size(
                egui::pos2(area.min.x - hidden, area.min.y),
                Vec2::new(width, area.height()),
            );
            if settled {
                let dismiss =
                    root.interact(area, Id::new("mobile-navigation-dismiss"), Sense::click());
                if dismiss.clicked()
                    && root
                        .ctx()
                        .pointer_interact_pos()
                        .is_some_and(|pos| !rect.contains(pos))
                {
                    // Fecha, mas segue desenhando a gaveta neste quadro.
                    // Sair daqui fazia ela faltar um quadro e voltar já
                    // saindo, e era isso que parecia uma segunda cópia dela.
                    // Fechar deslizando nunca passou por aqui — por isso só
                    // o toque fora tinha o defeito.
                    state.mobile_surface = MobileSurface::Chat;
                }
            }
            let response = egui::Area::new(Id::new("mobile-navigation-drawer"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.min)
                .default_size(rect.size())
                .constrain_to(area.expand2(Vec2::new(width, 0.0)))
                .show(root.ctx(), |ui| {
                    ui.set_min_size(rect.size());
                    ui.set_max_size(rect.size());
                    let action = super::rail::draw(ui, servers.entries, servers.active, t, s);
                    channels_sidebar(ui, store, state, t, s, live);
                    action
                });
            response.inner
        }
        MobileSurface::People => {
            let rect = Rect::from_min_size(
                egui::pos2(area.max.x - width + hidden, area.min.y),
                Vec2::new(width, area.height()),
            );
            if settled {
                let dismiss =
                    root.interact(area, Id::new("mobile-people-dismiss"), Sense::click());
                if dismiss.clicked()
                    && root
                        .ctx()
                        .pointer_interact_pos()
                        .is_some_and(|pos| !rect.contains(pos))
                {
                    // Fecha, mas segue desenhando a gaveta neste quadro.
                    // Sair daqui fazia ela faltar um quadro e voltar já
                    // saindo, e era isso que parecia uma segunda cópia dela.
                    // Fechar deslizando nunca passou por aqui — por isso só
                    // o toque fora tinha o defeito.
                    state.mobile_surface = MobileSurface::Chat;
                }
            }
            egui::Area::new(Id::new("mobile-people-drawer"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.min)
                .default_size(rect.size())
                .constrain_to(area.expand2(Vec2::new(width, 0.0)))
                .show(root.ctx(), |ui| {
                    ui.set_min_size(rect.size());
                    ui.set_max_size(rect.size());
                    members_sidebar(ui, store, state, t, s, true);
                });
            None
        }
    }
}

/// Desenha o fundo embaçado de uma barra: o que já foi pintado por baixo
/// dela entra borrado, e a tinta translúcida vem por cima.
pub(super) fn glass_backdrop(ui: &egui::Ui, state: &UiState, rect: Rect, corner: f32) {
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
    live: bool,
) {
    egui::Panel::left("channels")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(sidebar_frame(t))
        .show(root, |ui| {
            let full = ui.max_rect();

            // Pastilha do servidor: mesma anatomia da pastilha da conta lá
            // embaixo — ícone, nome, o que ele é, e a engrenagem do outro
            // lado. Uma é o servidor, a outra é você.
            let (name, subtitle) = match &store.server {
                Some(server) => (
                    server.name.clone(),
                    server
                        .description
                        .clone()
                        .unwrap_or_else(|| format!("{} · {}", store.members.len(), s.members)),
                ),
                None => ("Papo".to_owned(), String::new()),
            };
            let server_pill = Rect::from_min_size(
                egui::pos2(full.min.x + PILL_INSET, full.min.y + PILL_INSET),
                Vec2::new(full.width() - PILL_INSET * 2.0, IDENTITY_PILL_HEIGHT),
            );
            if identity_pill(
                ui,
                state,
                t,
                server_pill,
                &initials_of(&name),
                &name,
                &subtitle,
                None,
                t.accent,
                "pastilha-do-servidor",
            ) {
                state.pending.push(MenuCommand::ServerSettings);
            }

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
                                        if state.compact {
                                            state.mobile_surface = MobileSurface::Chat;
                                        }
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
                                    let here = store.call.channel_id == channel.id;
                                    let row = channel_row(
                                        ui,
                                        t,
                                        icon::SPEAKER_HIGH,
                                        &channel.name,
                                        here,
                                        false,
                                        0,
                                        SIDEBAR_WIDTH - indent * 2.0,
                                    );
                                    // Um clique entra, como se espera de uma
                                    // sala de voz: ela não é uma tela para
                                    // visitar, é um lugar onde se está. Na
                                    // sala em que já se está, o mesmo clique
                                    // leva de volta a ela — era o que faltava
                                    // para quem saiu para ler outro canal ter
                                    // caminho de volta.
                                    if row.clicked() {
                                        state.actions.push(if here {
                                            ChatAction::OpenCall
                                        } else {
                                            ChatAction::JoinVoice(channel.id.clone())
                                        });
                                        if state.compact {
                                            state.mobile_surface = MobileSurface::Chat;
                                        }
                                    }
                                    channel_menu(&row, channel, state, s);
                                    crate::ui::call::roster(
                                        ui,
                                        store,
                                        state,
                                        t,
                                        s,
                                        &channel.id,
                                        SIDEBAR_WIDTH - indent * 2.0,
                                    );
                                }
                                // Espaço para a pastilha da conta (e a barra
                                // da call, quando existe) não cobrirem o
                                // último canal quando a lista chega ao fim.
                                let reserved = if store.call.active() {
                                    IDENTITY_PILL_HEIGHT + crate::ui::call::BAR_HEIGHT + space::XS
                                } else {
                                    IDENTITY_PILL_HEIGHT
                                };
                                ui.add_space(reserved + PILL_INSET * 2.0);
                            });
                        });
                    });
            });

            if store.call.active() {
                let bar = Rect::from_min_size(
                    egui::pos2(
                        full.min.x + PILL_INSET,
                        full.max.y
                            - IDENTITY_PILL_HEIGHT
                            - PILL_INSET
                            - crate::ui::call::BAR_HEIGHT
                            - space::XS,
                    ),
                    Vec2::new(
                        full.width() - PILL_INSET * 2.0,
                        crate::ui::call::BAR_HEIGHT,
                    ),
                );
                crate::ui::call::bar(ui, store, state, t, s, bar, live);
            }
            account_pill(ui, store, state, t, s, full);
        });
}

/// Pastilha da conta, no pé da coluna. Gêmea da pastilha do servidor lá em
/// cima: as duas têm ícone, nome, uma linha de contexto e a engrenagem na
/// ponta oposta, e as duas abrem a mesma folha. O que muda é de quem são os
/// ajustes — seus, embaixo; do servidor, em cima.
fn rgb([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

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
        username: store.my_username.clone(),
        name: store.my_name.clone(),
        presence: crate::state::Presence::Online,
        role_color: None,
        roles: Vec::new(),
    });
    if me.name.is_empty() {
        return;
    }
    let subtitle = if store.my_username.is_empty() {
        s.presence_online.to_owned()
    } else {
        format!("@{}", store.my_username)
    };
    let rect = Rect::from_min_size(
        egui::pos2(
            sidebar.min.x + PILL_INSET,
            sidebar.max.y - IDENTITY_PILL_HEIGHT - PILL_INSET,
        ),
        Vec2::new(sidebar.width() - PILL_INSET * 2.0, IDENTITY_PILL_HEIGHT),
    );
    let ctx = ui.ctx().clone();
    let avatar = state
        .media
        .avatar(&me.id, store.avatars.get(&me.id).map(String::as_str))
        .and_then(|texture| texture.frame(&ctx))
        .map(|handle| handle.id());

    if identity_pill(
        ui,
        state,
        t,
        rect,
        &me.initials(),
        &me.name,
        &subtitle,
        Some((presence_color(t, me.presence), avatar)),
        me.role_color.map(rgb).unwrap_or(t.accent),
        "pastilha-da-conta",
    ) {
        state.pending.push(MenuCommand::Preferences);
    }
}

/// Desenho comum das duas pastilhas. Devolve `true` no clique.
///
/// `presence` só existe na pastilha da conta: é o ponto de status e a foto.
#[allow(clippy::too_many_arguments)]
fn identity_pill(
    ui: &mut egui::Ui,
    state: &UiState,
    t: &Tokens,
    rect: Rect,
    initials: &str,
    name: &str,
    subtitle: &str,
    presence: Option<(Color32, Option<egui::TextureId>)>,
    tint: Color32,
    id: &str,
) -> bool {
    pill_surface(ui, state, t, rect);
    let response = ui.interact(rect, Id::new(id), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(PILL_RADIUS as u8), t.fill_soft);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let avatar_size = 28.0;
    let avatar_rect = Rect::from_center_size(
        egui::pos2(rect.min.x + space::SM + avatar_size / 2.0, rect.center().y),
        Vec2::splat(avatar_size),
    );
    match presence.and_then(|(_, texture)| texture) {
        Some(texture) => {
            let mut mesh = egui::Mesh::with_texture(texture);
            mesh.add_rect_with_uv(
                avatar_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.painter()
                .with_clip_rect(avatar_rect)
                .add(egui::Shape::mesh(mesh));
        }
        None => {
            ui.painter().circle_filled(
                avatar_rect.center(),
                avatar_size / 2.0,
                tint.gamma_multiply(0.30),
            );
            ui.painter().text(
                avatar_rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                egui::FontId::new(10.0, egui::FontFamily::Name("semibold".into())),
                tint,
            );
        }
    }
    if let Some((dot_color, _)) = presence {
        let dot = avatar_rect.center() + Vec2::splat(avatar_size / 2.0 * 0.72);
        ui.painter().circle_filled(dot, 5.0, t.glass_opaque);
        ui.painter().circle_filled(dot, 3.5, dot_color);
    }

    // A engrenagem mora na ponta oposta ao ícone; o texto vive entre as duas.
    let gear = Rect::from_center_size(
        egui::pos2(rect.max.x - space::SM - 14.0, rect.center().y),
        Vec2::splat(28.0),
    );
    let gear_hovered = ui.rect_contains_pointer(gear);
    if gear_hovered {
        ui.painter()
            .rect_filled(gear, CornerRadius::same(radius::FIELD), t.fill_medium);
    }
    ui.painter().text(
        gear.center(),
        egui::Align2::CENTER_CENTER,
        icon::GEAR_SIX,
        text::icon(15.0),
        if gear_hovered { t.label } else { t.label_secondary },
    );

    let text_left = avatar_rect.max.x + space::MD;
    let text_right = gear.min.x - space::XS;
    let painter = ui.painter().with_clip_rect(Rect::from_x_y_ranges(
        egui::Rangef::new(text_left, text_right),
        rect.y_range(),
    ));
    let has_subtitle = !subtitle.is_empty();
    painter.text(
        egui::pos2(
            text_left,
            rect.center().y - if has_subtitle { 7.0 } else { 0.0 },
        ),
        egui::Align2::LEFT_CENTER,
        name,
        text::headline(),
        t.label,
    );
    if has_subtitle {
        painter.text(
            egui::pos2(text_left, rect.center().y + 8.0),
            egui::Align2::LEFT_CENTER,
            subtitle,
            text::footnote(),
            t.label_tertiary,
        );
    }

    response.clicked()
}

/// Iniciais de um nome, para o ícone de quem não tem imagem.
fn initials_of(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
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
        ui.label(s.channel_notifications);
        for (label, setting) in [
            (s.notify_all, "all"),
            (s.notify_mentions, "only_mentions"),
            (s.notify_off, "off"),
        ] {
            if ui.button(label).clicked() {
                state.actions.push(ChatAction::ChannelNotifications {
                    channel_id: channel.id.clone(),
                    setting,
                });
                ui.close();
            }
        }
        ui.separator();
        // A posição é trocada com o vizinho; o backend recebe as duas.
        if ui.button(s.move_up).clicked() {
            state.actions.push(ChatAction::MoveChannel {
                channel_id: channel.id.clone(),
                old_position: channel.position,
                new_position: channel.position - 1,
            });
            ui.close();
        }
        if ui.button(s.move_down).clicked() {
            state.actions.push(ChatAction::MoveChannel {
                channel_id: channel.id.clone(),
                old_position: channel.position,
                new_position: channel.position + 1,
            });
            ui.close();
        }
        ui.separator();
        if ui.button(s.edit).clicked() {
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
                .push(ChatAction::RequestDeleteChannel(channel.id.clone()));
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

fn members_sidebar(
    root: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    mobile: bool,
) {
    let ctx = root.ctx().clone();
    egui::Panel::right("members")
        .exact_size(MEMBERS_WIDTH)
        .resizable(false)
        .frame(sidebar_frame(t))
        .show(root, |ui| {
            if mobile {
                ui.horizontal(|ui| {
                    ui.add_space(space::LG);
                    ui.label(
                        RichText::new(s.members)
                            .font(text::headline())
                            .color(t.label),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, t, icon::X, s.close).clicked() {
                            state.mobile_surface = MobileSurface::Chat;
                        }
                    });
                });
                ui.separator();
            }
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
                                    let avatar = state
                                        .media
                                        .avatar(
                                            &member.id,
                                            store.avatars.get(&member.id).map(String::as_str),
                                        )
                                        .and_then(|texture| texture.frame(&ctx))
                                        .map(|handle| handle.id());
                                    let row = member_row(
                                        ui,
                                        t,
                                        &member.initials(),
                                        &member.name,
                                        presence_color(t, member.presence),
                                        member.role_color.map(rgb),
                                        presence == Presence::Offline,
                                        MEMBERS_WIDTH - space::LG * 2.0,
                                        avatar,
                                    );
                                    member_menu(&row, member, state, s);
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
    avatar: Option<egui::TextureId>,
) -> egui::Response {
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
    match avatar {
        // Foto redonda: a malha recorta o círculo, senão sobrariam os
        // cantos quadrados da textura.
        Some(texture) => {
            let mut mesh = egui::Mesh::with_texture(texture);
            mesh.add_rect_with_uv(
                avatar_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(alpha),
            );
            painter.with_clip_rect(avatar_rect).add(egui::Shape::mesh(mesh));
        }
        None => {
            painter.circle_filled(avatar_rect.center(), 11.0, tint.gamma_multiply(0.30));
            painter.text(
                avatar_rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                egui::FontId::new(9.0, egui::FontFamily::Name("semibold".into())),
                tint,
            );
        }
    }
    painter.circle_filled(avatar_rect.right_bottom() - Vec2::splat(1.0), 4.5, t.glass_opaque);
    painter.circle_filled(avatar_rect.right_bottom() - Vec2::splat(1.0), 3.0, dot.gamma_multiply(alpha));
    painter.text(
        egui::pos2(avatar_rect.max.x + space::MD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        text::body(),
        if dimmed { t.label_tertiary } else { t.label_secondary },
    );
    response
}

/// Menu do botão direito de uma pessoa: banir e redefinir a conta. As duas
/// pedem permissão no servidor; quem não a tem recebe o 403 no aviso.
fn member_menu(
    response: &egui::Response,
    member: &crate::state::Member,
    state: &mut UiState,
    s: &Strings,
) {
    response.context_menu(|ui| {
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

#[allow(clippy::too_many_arguments)]
fn conversation(
    root: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    mut call: Option<&mut crate::voice::Call>,
    t: &Tokens,
    s: &Strings,
    stage: Option<crate::state::Stage>,
) {
    use crate::state::Stage;

    let voice = store
        .channel(&store.selected_channel)
        .is_some_and(|channel| channel.kind == ChannelKind::Voice);
    let frame = Frame::new().fill(t.content_bg);
    egui::CentralPanel::default().frame(frame).show(root, |ui| {
        let full = ui.max_rect();

        // Canal de voz na tela: a conversa dá lugar à sala. Dentro da call,
        // a grade; fora, quem está lá e o caminho para entrar.
        if voice {
            state.webembed_chat_clip = None;
            if stage == Some(Stage::Docked) && store.call.channel_id == store.selected_channel {
                crate::ui::call::dock(ui, store, state, call.as_deref_mut(), t, s);
            } else {
                crate::ui::call::lobby(ui, store, state, t, s);
            }
            let channel_rect = channel_pill(ui, store, state, t, full);
            call_layers(
                ui,
                store,
                state,
                call,
                t,
                s,
                full,
                stage,
                channel_rect,
                None,
            );
            if state.compact {
                handle_mobile_gesture(
                    ui,
                    state,
                    full,
                    PILL_MARGIN * 2.0 + PILL_HEIGHT,
                    72.0,
                );
            }
            return;
        }

        let composer_height = composer_height(ui, state, full);
        let top_inset = PILL_MARGIN * 2.0 + PILL_HEIGHT;
        let bottom_inset = PILL_MARGIN * 2.0 + composer_height;
        state.webembed_chat_clip = Some(Rect::from_min_max(
            egui::pos2(full.min.x, full.min.y + top_inset),
            egui::pos2(full.max.x, full.max.y - bottom_inset),
        ));

        // Camada de conteúdo: ocupa a janela inteira e corre por baixo das
        // pastilhas.
        ui.scope_builder(UiBuilder::new().max_rect(full), |ui| {
            egui::ScrollArea::vertical()
                .scroll_source(if state.panel.is_some() {
                    egui::containers::scroll_area::ScrollSource::NONE
                } else {
                    egui::containers::scroll_area::ScrollSource::ALL
                })
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let web_scroll = state
                        .webembed
                        .take_scroll_delta_points(ui.ctx().pixels_per_point());
                    if web_scroll.abs() > f32::EPSILON {
                        ui.scroll_with_delta(Vec2::new(0.0, web_scroll));
                    }
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
        let channel_rect = channel_pill(ui, store, state, t, full);
        let actions_rect = actions_pill(ui, store, state, t, s, full);
        composer(ui, store, state, t, s, full, composer_height);
        call_layers(
            ui,
            store,
            state,
            call,
            t,
            s,
            full,
            stage,
            channel_rect,
            Some(actions_rect),
        );

        if state.compact {
            handle_mobile_gesture(ui, state, full, top_inset, bottom_inset);
        }
    });
}

/// Largura que a gaveta ocupa. A mesma conta de `mobile_drawers`: o gesto
/// precisa saber de quanto é o caminho inteiro para dizer que fração dele já
/// foi andada.
fn drawer_width(surface: MobileSurface, area: Rect) -> f32 {
    match surface {
        MobileSurface::Chat => 0.0,
        MobileSurface::Navigation => (super::rail::RAIL_WIDTH + SIDEBAR_WIDTH)
            .min((area.width() - space::XXL).max(SIDEBAR_WIDTH)),
        MobileSurface::People => MEMBERS_WIDTH.min((area.width() - space::XXL).max(120.0)),
    }
}

/// De que lado a gaveta entra: a navegação vem da esquerda, as pessoas vêm
/// da direita. Serve para a mesma conta valer para as duas — basta trocar o
/// sinal do que o dedo andou.
fn open_sign(surface: MobileSurface) -> f32 {
    match surface {
        MobileSurface::People => -1.0,
        _ => 1.0,
    }
}

/// O elástico: até `full` a coisa anda colada ao dedo; depois anda cada vez
/// menos e nunca chega ao dobro. É a resistência que avisa o polegar de que
/// já foi longe o bastante, sem precisar de nada escrito na tela.
fn rubber_band(raw: f32, full: f32) -> f32 {
    if raw <= 0.0 {
        return 0.0;
    }
    if raw <= full {
        return raw;
    }
    let over = raw - full;
    full + over / (1.0 + over / full)
}

/// Ao soltar o dedo: a gaveta termina de abrir ou volta para onde estava.
///
/// Um piparote decide sozinho, para os dois lados — quem joga a gaveta com
/// força espera que ela vá, mesmo que o dedo tenha andado pouco. Sem
/// piparote, vale o quanto dela já está à mostra.
fn commits(shown: f32, opening_velocity: f32) -> bool {
    if opening_velocity >= FLICK_SPEED {
        return true;
    }
    if opening_velocity <= -FLICK_SPEED {
        return false;
    }
    shown >= DRAWER_COMMIT
}

/// O que este arrasto vai ser, decidido uma vez só.
fn decide_intent(delta: Vec2, surface: MobileSurface, message: &Option<String>) -> Intent {
    // Mais vertical que horizontal: o movimento é da rolagem.
    if delta.x.abs() < delta.y.abs() * SWIPE_AXIS_BIAS {
        return Intent::Scroll;
    }
    match (surface, delta.x > 0.0) {
        (MobileSurface::Chat, true) => Intent::Drawer(MobileSurface::Navigation),
        (MobileSurface::Chat, false) => match message {
            Some(id) => Intent::Reply(id.clone()),
            None => Intent::Scroll,
        },
        (MobileSurface::Navigation, false) => Intent::Drawer(MobileSurface::Navigation),
        (MobileSurface::People, true) => Intent::Drawer(MobileSurface::People),
        _ => Intent::Scroll,
    }
}

fn handle_mobile_gesture(
    ui: &egui::Ui,
    state: &mut UiState,
    area: Rect,
    top_inset: f32,
    bottom_inset: f32,
) {
    let (pressed, released, down, pos, time) = ui.input(|input| {
        (
            input.pointer.any_pressed(),
            input.pointer.any_released(),
            input.pointer.any_down(),
            input.pointer.interact_pos(),
            input.time,
        )
    });

    if pressed
        && let Some(origin) = pos
    {
        let message_id = if state.mobile_surface == MobileSurface::Chat {
            state
                .message_rows
                .iter()
                .find(|(_, rect)| rect.contains(origin))
                .map(|(id, _)| id.clone())
        } else {
            None
        };
        let controls = origin.y < area.min.y + top_inset || origin.y > area.max.y - bottom_inset;
        #[cfg(target_os = "android")]
        let media_seek = state
            .media_seek_zones
            .iter()
            .any(|rect| rect.contains(origin));
        #[cfg(not(target_os = "android"))]
        let media_seek = false;

        let blocked = state.popup.is_some()
            || state.viewer.is_some()
            || state.panel.is_some()
            || media_seek
            || state
                .webembed_float_rect
                .is_some_and(|rect| rect.contains(origin))
            || (state.mobile_surface == MobileSurface::Chat && controls);
        state.mobile_gesture = Some(MobileGesture {
            origin,
            last: origin,
            last_time: time,
            velocity: 0.0,
            message_id,
            blocked,
            intent: None,
        });
    }

    // Tirar o gesto do estado enquanto se mexe nele evita brigar com o
    // empréstimo do resto do `state`, que também muda aqui.
    let mut gesture = state.mobile_gesture.take();

    if let Some(active) = gesture.as_mut()
        && let Some(pos) = pos
        && !active.blocked
    {
        let elapsed = (time - active.last_time) as f32;
        if elapsed > 0.0 {
            // Média que esquece depressa: um tranco isolado no meio do
            // arrasto não decide o gesto sozinho.
            let instant = (pos.x - active.last.x) / elapsed;
            active.velocity = active.velocity * 0.7 + instant * 0.3;
        }
        active.last = pos;
        active.last_time = time;

        let delta = pos - active.origin;
        if active.intent.is_none() && delta.length() >= SWIPE_SLOP {
            active.intent = Some(decide_intent(delta, state.mobile_surface, &active.message_id));
        }

        match &active.intent {
            Some(Intent::Drawer(surface)) => {
                let surface = *surface;
                let width = drawer_width(surface, area).max(1.0);
                // Se a gaveta já estava aberta, o dedo parte de 1 e a
                // fecha; se não, parte de 0 e a abre.
                let base = if state.mobile_surface == surface { 1.0 } else { 0.0 };
                state.drawer.surface = surface;
                state.drawer.shown =
                    (base + delta.x * open_sign(surface) / width).clamp(0.0, 1.0);
                state.drawer.dragging = true;
            }
            Some(Intent::Reply(id)) => {
                state.reply_drag = Some(ReplyDrag {
                    id: id.clone(),
                    shown: rubber_band(-delta.x, REPLY_TRAVEL),
                    dragging: true,
                });
            }
            _ => {}
        }
    }

    if (released || (!down && gesture.is_some() && !pressed))
        && let Some(finished) = gesture.take()
    {
        settle_mobile_gesture(state, finished);
    }

    state.mobile_gesture = gesture;
}

/// O dedo saiu da tela: decidir o que fica.
fn settle_mobile_gesture(state: &mut UiState, gesture: MobileGesture) {
    state.drawer.dragging = false;
    if let Some(drag) = state.reply_drag.as_mut() {
        // Solta sempre volta: a mensagem não fica torta esperando resposta.
        drag.dragging = false;
    }

    let Some(intent) = gesture.intent else {
        return;
    };
    match intent {
        Intent::Scroll => {}
        Intent::Drawer(surface) => {
            let opening_velocity = gesture.velocity * open_sign(surface);
            let open = commits(state.drawer.shown, opening_velocity);
            state.drawer.target = if open { 1.0 } else { 0.0 };
            state.mobile_surface = if open { surface } else { MobileSurface::Chat };
        }
        Intent::Reply(id) => {
            let far_enough = state
                .reply_drag
                .as_ref()
                .is_some_and(|drag| drag.shown >= REPLY_TRAVEL);
            if far_enough {
                state.start_reply(id);
                state.close_popup();
            }
        }
    }
}

/// Leva `shown` até `target` no tempo de animação da casa.
///
/// Quem desligou as animações no sistema recebe o salto direto: o fator zero
/// vale aqui como vale no resto da interface.
/// `span` é o caminho inteiro na unidade de `shown` — 1 para a gaveta, que
/// anda de 0 a 1, e a distância máxima para a mensagem, que anda em pontos.
/// Sem ele o passo seria uma fração por quadro tanto faz a unidade, e a
/// mensagem voltaria a um oitavo de ponto por quadro: parada, na prática.
fn advance(shown: &mut f32, target: f32, span: f32, ui: &egui::Ui) -> bool {
    if (*shown - target).abs() < f32::EPSILON {
        return false;
    }
    let time = ui.style().animation_time;
    if time <= 0.0 {
        *shown = target;
        return false;
    }
    let ctx = ui.ctx();
    let dt = ctx.input(|input| input.stable_dt).min(0.1);
    let step = span * dt / time;
    let remaining = target - *shown;
    if remaining.abs() <= step {
        *shown = target;
        return false;
    }
    *shown += step * remaining.signum();
    ctx.request_repaint();
    true
}

/// O que a call põe por cima da conversa: a folha de vidro, ou a pastilha
/// dela encolhida. No canal da própria call, nenhum dos dois — ali a call já
/// é a tela.
#[allow(clippy::too_many_arguments)]
fn call_layers(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut crate::voice::Call>,
    t: &Tokens,
    s: &Strings,
    full: Rect,
    stage: Option<crate::state::Stage>,
    channel_rect: Option<Rect>,
    actions_rect: Option<Rect>,
) {
    use crate::state::Stage;

    let left = channel_rect
        .map(|rect| rect.max.x + space::SM)
        .unwrap_or(full.min.x + PILL_MARGIN);
    let right = actions_rect
        .map(|rect| rect.min.x - space::SM)
        .unwrap_or(full.max.x - PILL_MARGIN);
    let pill_area = if right > left {
        Rect::from_min_max(
            egui::pos2(left, full.min.y),
            egui::pos2(right, full.max.y),
        )
    } else {
        full
    };

    match stage {
        Some(Stage::Floating) => {
            crate::ui::call::floating(ui, store, state, call, t, s, full, pill_area);
        }
        Some(Stage::Sheet) => crate::ui::call::sheet(ui, store, state, call, t, s, full),
        Some(Stage::Docked) if store.call.channel_id != store.selected_channel => {
            crate::ui::call::pill(ui, store, state, t, s, pill_area);
        }
        _ => {}
    }
}

/// Pastilha de identidade do canal, no alto à esquerda.
///
/// A descrição entra junto com o canal, fica alguns segundos e escorrega na
/// direção do nome até sumir — a pastilha encolhe junto.
fn channel_pill(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    area: Rect,
) -> Option<Rect> {
    let channel = store.channel(&store.selected_channel).cloned()?;

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
    let limit = (area.width() - PILL_MARGIN * 2.0 - ACTIONS_PILL_WIDTH - space::MD)
        .max(PILL_HEIGHT);
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

    Some(rect)
}

/// Pastilha de ações do canal, no alto à direita.
///
/// Fechada, são três ícones. Aberta em busca ou em fixadas, vira uma camada
/// modal leve: fica acima da conversa, bloqueia a rolagem de trás e fecha ao
/// clicar fora, pelo X ou pelo mesmo ícone que a abriu.
fn actions_pill(
    ui: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) -> Rect {
    let open = state.panel.as_ref().map(|panel| panel.kind);
    // Fechada, esta pastilha precisa morar na camada normal. Antes ela
    // criava uma camada Foreground do tamanho da tela inteira e roubava o
    // ponteiro da pastilha da call mesmo sem painel aberto.
    let layer = if open.is_some() {
        egui::LayerId::new(egui::Order::Foreground, Id::new("actions-panel-layer"))
    } else {
        ui.layer_id()
    };
    let screen = ui.ctx().content_rect();
    let mut top = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(screen));
    let ui = &mut top;
    let width = if open.is_some() {
        PANEL_WIDTH.min((area.width() - PILL_MARGIN * 2.0).max(ACTIONS_PILL_WIDTH))
    } else {
        ACTIONS_PILL_WIDTH
    };
    // A altura acompanha o conteúdo até um teto; a conversa continua visível
    // embaixo, que é a vantagem de esticar em vez de abrir janela.
    let body = match open {
        None => 0.0,
        Some(_) => (area.height() - PILL_MARGIN * 2.0 - PILL_HEIGHT).min(PANEL_MAX_BODY),
    };
    let rect = Rect::from_min_size(
        egui::pos2(area.max.x - PILL_MARGIN - width, area.min.y + PILL_MARGIN),
        Vec2::new(width, PILL_HEIGHT + body),
    );
    pill_surface(ui, state, t, rect);

    let header = Rect::from_min_size(rect.min, Vec2::new(width, PILL_HEIGHT));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(header.shrink2(Vec2::new(space::XS, space::XS)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = space::XXS;
            // O X mora à esquerda da pastilha esticada.
            if open.is_some() && icon_button(ui, t, icon::X, s.close).clicked() {
                state.panel = None;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::XXS;
                if icon_button(ui, t, icon::USERS, s.members).clicked() {
                    if state.compact {
                        state.mobile_surface = MobileSurface::People;
                    } else {
                        state.pending.push(MenuCommand::ToggleMembers);
                    }
                }
                if pill_toggle(ui, t, icon::PUSH_PIN, s.pinned, open == Some(PanelKind::Pinned)) {
                    toggle_panel(state, PanelKind::Pinned);
                }
                if pill_toggle(
                    ui,
                    t,
                    icon::MAGNIFYING_GLASS,
                    s.search,
                    open == Some(PanelKind::Search),
                ) {
                    toggle_panel(state, PanelKind::Search);
                }
            });
        },
    );

    let Some(kind) = open else { return rect };
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + space::MD, header.max.y),
            egui::pos2(rect.max.x - space::MD, header.max.y),
        ],
        Stroke::new(1.0, t.separator),
    );

    let body_rect = Rect::from_min_max(
        egui::pos2(rect.min.x, header.max.y),
        egui::pos2(rect.max.x, rect.max.y),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(body_rect.shrink(space::SM))
            .layout(Layout::top_down(Align::Min)),
        |ui| match kind {
            PanelKind::Search => search_panel(ui, store, state, t, s),
            PanelKind::Pinned => pinned_panel(ui, store, state, t, s),
        },
    );

    // Quatro zonas cobrem tudo que não é a pastilha. Como vivem na camada
    // Foreground, o clique não atravessa para mensagens/canais atrás.
    let outside = [
        Rect::from_min_max(screen.min, egui::pos2(rect.min.x, screen.max.y)),
        Rect::from_min_max(
            egui::pos2(rect.min.x, screen.min.y),
            egui::pos2(screen.max.x, rect.min.y),
        ),
        Rect::from_min_max(
            egui::pos2(rect.max.x, rect.min.y),
            egui::pos2(screen.max.x, rect.max.y),
        ),
        Rect::from_min_max(
            egui::pos2(rect.min.x, rect.max.y),
            screen.max,
        ),
    ];
    let dismiss = outside.into_iter().enumerate().any(|(index, zone)| {
        zone.width() > 0.0
            && zone.height() > 0.0
            && ui
                .interact(zone, Id::new(("actions-panel-dismiss", index)), Sense::click())
                .clicked()
    });
    if dismiss || ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.panel = None;
    }

    rect
}

/// Abre a lista pedida, ou fecha se ela já era a que estava aberta.
pub fn toggle_panel(state: &mut UiState, kind: PanelKind) {
    match &state.panel {
        Some(panel) if panel.kind == kind => state.panel = None,
        _ => {
            state.panel = Some(Panel {
                kind,
                query: String::new(),
                focus: kind == PanelKind::Search,
                searched: false,
            })
        }
    }
}

/// Ícone da pastilha que fica aceso enquanto a sua lista está aberta.
fn pill_toggle(ui: &mut egui::Ui, t: &Tokens, glyph: &str, tip: &str, active: bool) -> bool {
    let response = icon_button(ui, t, glyph, tip);
    if active {
        ui.painter().rect_filled(
            response.rect,
            CornerRadius::same(radius::FIELD),
            t.accent.gamma_multiply(0.20),
        );
        ui.painter().text(
            response.rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            text::icon(15.0),
            t.accent,
        );
    }
    response.clicked()
}

/// Busca dentro da pastilha: campo em cima, resultados embaixo.
fn search_panel(ui: &mut egui::Ui, store: &mut Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    let mut run = false;
    let mut query = state
        .panel
        .as_ref()
        .map(|panel| panel.query.clone())
        .unwrap_or_default();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::XS;
        let field_width = (ui.available_width() - HIT_TARGET - space::XS).max(80.0);
        #[cfg(target_os = "android")]
        {
            let focus = state
                .panel
                .as_mut()
                .map(|panel| std::mem::take(&mut panel.focus))
                .unwrap_or(false);
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(field_width, 30.0), Sense::hover());
            ui.painter().rect(
                rect,
                CornerRadius::same(radius::FIELD),
                t.fill_soft,
                Stroke::new(1.0, t.separator),
                egui::StrokeKind::Inside,
            );
            let events = crate::platform::native_field::show(
                ui.ctx(),
                "search:messages",
                &mut query,
                rect.shrink2(Vec2::new(space::MD, space::XXS)),
                s.search_placeholder,
                crate::platform::native_field::Mode::Search,
                0,
                focus,
                t.label,
                t.label_tertiary,
                text::body().size,
            );
            run |= events.submit;
        }

        #[cfg(not(target_os = "android"))]
        {
            let search_id = Id::new("busca-mensagens");
            let _ = crate::platform::ime::prepare_text_edit(ui.ctx(), search_id, &mut query);
            let field = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .id(search_id)
                    .hint_text(s.search_placeholder)
                    .desired_width(field_width)
                    .font(text::body()),
            );
            let _ = crate::platform::ime::sync_text_edit(
                ui.ctx(),
                search_id,
                &mut query,
                field.has_focus(),
                crate::platform::ime::Kind::Search,
            );
            if let Some(panel) = state.panel.as_mut() {
                panel.query = query.clone();
                if panel.focus {
                    field.request_focus();
                    panel.focus = false;
                }
            }
            if ui.input(|input| input.key_pressed(egui::Key::Enter))
                && (field.has_focus() || field.lost_focus())
            {
                run = true;
            }
        }
        if icon_button(ui, t, icon::MAGNIFYING_GLASS, s.search).clicked() {
            run = true;
        }
    });
    #[cfg(target_os = "android")]
    if let Some(panel) = state.panel.as_mut() {
        panel.query.clone_from(&query);
    }
    if run && !query.trim().is_empty() {
        if let Some(panel) = state.panel.as_mut() {
            panel.searched = true;
        }
        store.searching = true;
        state.actions.push(ChatAction::Search(query.trim().to_owned()));
    }

    ui.add_space(space::XS);
    if store.searching {
        ui.label(
            RichText::new(s.searching)
                .font(text::footnote())
                .color(t.label_tertiary),
        );
        return;
    }
    if store.search_results.is_empty() {
        // Antes da primeira busca o que falta é a instrução, não o "nada
        // encontrado": quem acabou de abrir ainda não procurou coisa alguma.
        let searched = state
            .panel
            .as_ref()
            .is_some_and(|panel| panel.searched);
        ui.label(
            RichText::new(if searched { s.search_empty } else { s.search_hint })
                .font(text::footnote())
                .color(t.label_tertiary),
        );
        return;
    }

    let found: Vec<_> = store
        .search_results
        .iter()
        .map(|result| {
            let member = store.member_by_username(&result.author_username);
            (
                result.channel_id.clone(),
                result.id.clone(),
                result.channel_name.clone(),
                member.map(|member| member.id.clone()),
                member
                    .map(|member| member.name.clone())
                    .unwrap_or_else(|| result.author_username.clone()),
                result.created_at.map(|at| at.with_timezone(&Local)),
                result.content.clone(),
            )
        })
        .collect();
    egui::ScrollArea::vertical()
        .id_salt("resultados-da-busca")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (channel_id, message_id, channel_name, author_id, author_name, at, body) in found {
                if result_row(
                    ui,
                    store,
                    state,
                    t,
                    s,
                    ResultPreview {
                        message_id: &message_id,
                        author_id: author_id.as_deref(),
                        author_name: &author_name,
                        at,
                        channel_name: Some(&channel_name),
                        body: &body,
                    },
                ) {
                    go_to(store, state, ui, &channel_id, &message_id);
                }
            }
        });
}

/// Fixadas do canal aberto, dentro da mesma pastilha.
fn pinned_panel(ui: &mut egui::Ui, store: &mut Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    let channel_id = store.selected_channel.clone();
    let pinned: Vec<_> = store
        .messages_in(&channel_id)
        .filter(|message| message.pinned)
        .map(|message| {
            let body = if !message.content.trim().is_empty() {
                store.display_mentions(&message.content)
            } else {
                message
                    .attachments
                    .first()
                    .and_then(|attachment| attachment.original_file_name.clone())
                    .map(|name| format!("📎 {name}"))
                    .unwrap_or_default()
            };
            (
                message.id.clone(),
                message.author_id.clone(),
                store
                    .member(&message.author_id)
                    .map(|member| member.name.clone())
                    .unwrap_or_else(|| "?".into()),
                message.at,
                body,
            )
        })
        .collect();

    if pinned.is_empty() {
        ui.label(
            RichText::new(s.no_pinned)
                .font(text::footnote())
                .color(t.label_tertiary),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("lista-de-fixadas")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (message_id, author_id, author_name, at, body) in pinned {
                if result_row(
                    ui,
                    store,
                    state,
                    t,
                    s,
                    ResultPreview {
                        message_id: &message_id,
                        author_id: Some(&author_id),
                        author_name: &author_name,
                        at: Some(at),
                        channel_name: None,
                        body: &body,
                    },
                ) {
                    let channel = channel_id.clone();
                    go_to(store, state, ui, &channel, &message_id);
                }
            }
        });
}

struct ResultPreview<'a> {
    message_id: &'a str,
    author_id: Option<&'a str>,
    author_name: &'a str,
    at: Option<DateTime<Local>>,
    channel_name: Option<&'a str>,
    body: &'a str,
}

/// Miniatura de uma mensagem: avatar, autor, idade e o texto. O realce acompanha
/// o bloco da miniatura em vez de ocupar toda a largura da pastilha.
fn result_row(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    preview: ResultPreview<'_>,
) -> bool {
    let ResultPreview {
        message_id,
        author_id,
        author_name,
        at,
        channel_name,
        body,
    } = preview;
    let body = store.display_mentions(body);
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let avatar_size = 30.0;
    let row_width = ui.available_width();
    let max_text_width =
        (row_width - space::SM * 2.0 - avatar_size - space::MD).max(80.0);
    let member = author_id.and_then(|id| store.member(id));
    let initials = member
        .map(|member| member.initials())
        .unwrap_or_else(|| initials_for_name(author_name));
    let texture = author_id.and_then(|id| {
        state
            .media
            .avatar(id, store.avatars.get(id).map(String::as_str))
            .and_then(|texture| texture.frame(ui.ctx()))
            .map(|handle| handle.id())
    });

    let inner = ui.scope(|ui| {
        ui.set_min_width(row_width);
        ui.set_max_width(row_width);
        ui.add_space(space::XS);
        ui.horizontal(|ui| {
            ui.add_space(space::SM);
            avatar(
                ui,
                t,
                &initials,
                avatar_size,
                member.and_then(|member| member.role_color.map(rgb)),
                texture,
            );
            ui.add_space(space::MD);
            ui.vertical(|ui| {
                ui.set_max_width(max_text_width);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(author_name)
                            .font(text::headline())
                            .color(member.and_then(|member| member.role_color.map(rgb)).unwrap_or(t.label)),
                    );
                    if let Some(channel_name) = channel_name.filter(|name| !name.is_empty()) {
                        ui.add_space(space::XXS);
                        ui.label(
                            RichText::new(format!("#{channel_name}"))
                                .font(text::footnote())
                                .color(t.label_tertiary),
                        );
                    }
                    if let Some(at) = at {
                        ui.add_space(space::XXS);
                        ui.label(
                            RichText::new(relative_time(at, s))
                                .font(text::footnote())
                                .color(t.label_tertiary),
                        );
                    }
                });
                if !body.trim().is_empty() {
                    ui.add_space(space::XXS);
                    ui.label(RichText::new(&body).font(text::body()).color(t.label));
                }
            });
            ui.add_space(space::SM);
        });
        ui.add_space(space::XS);
    });

    let row = inner.response.rect;
    let response = ui.interact(
        row,
        ui.id().with(("message-preview", message_id)),
        Sense::click(),
    );
    if response.hovered() {
        ui.painter().set(
            backdrop,
            egui::epaint::RectShape::filled(row, CornerRadius::same(radius::CARD), t.fill_soft),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.add_space(space::XXS);
    response.clicked()
}

fn initials_for_name(name: &str) -> String {
    let mut words = name.split_whitespace();
    let Some(first) = words.next() else {
        return "?".into();
    };
    if let Some(second) = words.next() {
        format!(
            "{}{}",
            first.chars().next().unwrap_or('?'),
            second.chars().next().unwrap_or('?')
        )
        .to_uppercase()
    } else {
        first.chars().take(2).collect::<String>().to_uppercase()
    }
}

fn relative_time(at: DateTime<Local>, s: &Strings) -> String {
    let seconds = (Local::now() - at).num_seconds().max(0);
    let (value, singular, plural) = if seconds < 60 {
        return s.relative_now.to_owned();
    } else if seconds < 60 * 60 {
        (seconds / 60, s.time_minute, s.time_minutes)
    } else if seconds < 24 * 60 * 60 {
        (seconds / (60 * 60), s.time_hour, s.time_hours)
    } else if seconds < 7 * 24 * 60 * 60 {
        (seconds / (24 * 60 * 60), s.time_day, s.time_days)
    } else if seconds < 30 * 24 * 60 * 60 {
        (seconds / (7 * 24 * 60 * 60), s.time_week, s.time_weeks)
    } else if seconds < 365 * 24 * 60 * 60 {
        (seconds / (30 * 24 * 60 * 60), s.time_month, s.time_months)
    } else {
        (seconds / (365 * 24 * 60 * 60), s.time_year, s.time_years)
    };
    let unit = if value == 1 { singular } else { plural };
    format!(
        "{}{} {}{}",
        s.relative_ago_prefix, value, unit, s.relative_ago_suffix
    )
}

/// Intensidade do realce desta mensagem neste quadro, entre 0 e 1.
///
/// Na primeira vez que a mensagem procurada aparece, a lista rola até ela e
/// o relógio começa. Depois disso o valor é `|sen|`, que dá duas piscadas
/// limpas em [`BLINK_SECONDS`] e volta a zero sem corte.
fn blink_alpha(state: &mut UiState, message_id: &str, ui: &egui::Ui, row: Rect) -> Option<f32> {
    let jump = state.jump.as_mut()?;
    if jump.message_id != message_id {
        return None;
    }
    let now = ui.input(|input| input.time);
    let started = match jump.found {
        Some(started) => started,
        None => {
            // Só agora a mensagem existe na tela: é aqui que dá para rolar.
            ui.scroll_to_rect(row, Some(Align::Center));
            jump.found = Some(now);
            now
        }
    };
    let elapsed = now - started;
    if elapsed > BLINK_SECONDS {
        state.jump = None;
        return None;
    }
    ui.ctx().request_repaint();
    let phase = (elapsed / BLINK_SECONDS) * BLINKS * std::f64::consts::PI;
    Some(phase.sin().abs() as f32)
}

/// Leva a janela até a mensagem: troca de canal se precisar e marca o alvo
/// para a lista rolar até ele e piscá-lo.
fn go_to(
    store: &mut Store,
    state: &mut UiState,
    ui: &egui::Ui,
    channel_id: &str,
    message_id: &str,
) {
    if !channel_id.is_empty() && store.selected_channel != channel_id {
        store.selected_channel = channel_id.to_owned();
    }
    state.jump = Some(Jump {
        message_id: message_id.to_owned(),
        found: None,
        since: ui.input(|input| input.time),
    });
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

    let ctx = ui.ctx().clone();
    let width = area.width();
    let gutter = space::XL;
    let avatar_size = 36.0;
    let text_indent = gutter + avatar_size + space::LG;
    let text_width = width - text_indent - space::XL;
    let rows = egui::Rangef::new(area.min.x + space::MD, area.max.x - space::MD);
    // Com um popup aberto, só a mensagem dona dele fica em destaque: o resto
    // da lista não deve reagir ao ponteiro que está a caminho do menu.
    let interactive = state.viewer.is_none() && state.panel.is_none();
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

        // Puxada para responder: a linha anda para a esquerda junto com o
        // dedo. Quem anda é **só o desenho**. O retângulo do toque fica onde
        // estava — senão a mensagem fugiria do próprio gesto que a move — e
        // o lugar que ela ocupa na lista também, que é o que importa aqui:
        // mexer no retângulo do filho mexia na conta do pai, e o empurrão
        // escorria para todas as mensagens de baixo.
        //
        // Por isso a linha puxada vai para uma camada só dela, e é a camada
        // que se desloca depois de desenhada. A conta da lista não vê nada
        // disso. O preço é a linha passar por cima das pastilhas flutuantes
        // enquanto o dedo a segura, em vez de por baixo.
        let slide = state
            .reply_drag
            .as_ref()
            .filter(|drag| drag.id == message.id)
            .map_or(0.0, |drag| drag.shown);
        let sliding = (slide > 0.5).then(|| {
            egui::LayerId::new(egui::Order::Middle, Id::new(("mensagem-puxada", &message.id)))
        });

        let author = store.member(&message.author_id);
        // O escopo da linha é o alvo de toque do layout compacto — duplo
        // toque abre as reações, toque longo abre o menu.
        //
        // Registrar o alvo **aqui**, e não depois da linha pronta, é o que
        // importa: o egui dá o clique ao último widget registrado sobre o
        // ponto, e um `interact` no fim engolia tudo o que estivesse dentro
        // da mensagem. Era por isso que tocar no play de um vídeo, no play
        // de um áudio ou numa imagem não fazia nada além de acender a
        // linha. Como o escopo entra antes do conteúdo, o conteúdo ganha, e
        // a linha só recebe o toque que sobra.
        let mut builder = UiBuilder::new().sense(Sense::click());
        if let Some(layer) = sliding {
            builder = builder.layer_id(layer);
        }
        let inner = ui.scope_builder(builder, |ui| {
            ui.horizontal_top(|ui| {
                ui.add_space(gutter);
                if grouped {
                    ui.allocate_exact_size(Vec2::new(avatar_size, 0.0), Sense::hover());
                    ui.add_space(space::LG);
                } else {
                    let initials = author.map(|a| a.initials()).unwrap_or_else(|| "?".into());
                    // A mesma foto que a lista de membros e a pastilha da
                    // conta mostram; sem ela, sobram as iniciais.
                    let texture = author.and_then(|a| {
                        state
                            .media
                            .avatar(&a.id, store.avatars.get(&a.id).map(String::as_str))
                            .and_then(|texture| texture.frame(&ctx))
                            .map(|handle| handle.id())
                    });
                    avatar(
                        ui,
                        t,
                        &initials,
                        avatar_size,
                        author.and_then(|a| a.role_color.map(rgb)),
                        texture,
                    );
                    ui.add_space(space::LG);
                }
                ui.vertical(|ui| {
                    ui.set_max_width(text_width);
                    if !grouped {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(author.map(|a| a.name.as_str()).unwrap_or("?"))
                                    .font(text::headline())
                                    .color(author.and_then(|a| a.role_color.map(rgb)).unwrap_or(t.label)),
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
                        reply_quote(ui, store, state, t, s, reply_to, text_width);
                    }
                    message_body(ui, store, state, t, s, message, text_width);
                });
            });
        });

        if let Some(layer) = sliding {
            ui.ctx().transform_layer_shapes(
                layer,
                egui::emath::TSTransform::from_translation(Vec2::new(-slide, 0.0)),
            );
        }

        let row = Rect::from_x_y_ranges(rows, inner.response.rect.y_range());
        if state.compact && state.panel.is_none() {
            let touch_rect = row.expand2(Vec2::new(0.0, ROW_PADDING));
            state
                .message_rows
                .push((message.id.clone(), touch_rect));
            if !message.pending {
                let touch = &inner.response;
                if touch.double_clicked() {
                    let at = ui.ctx().pointer_interact_pos().unwrap_or(row.center());
                    state.popup = Some(Popup {
                        kind: PopupKind::Emoji,
                        message_id: message.id.clone(),
                        anchor: Rect::from_min_size(at, Vec2::ZERO),
                        at_pointer: true,
                        opened: ui.input(|input| input.time),
                    });
                } else if touch.secondary_clicked() {
                    let at = ui.ctx().pointer_interact_pos().unwrap_or(row.center());
                    state.popup = Some(Popup {
                        kind: PopupKind::Menu,
                        message_id: message.id.clone(),
                        anchor: Rect::from_min_size(at, Vec2::ZERO),
                        at_pointer: true,
                        opened: ui.input(|input| input.time),
                    });
                }
            }
        }
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

        // Mensagem alcançada por um resultado: rola até ela e pisca duas
        // vezes, de borda a borda. O realce vai por cima do de menção, que
        // pode estar na mesma linha.
        if let Some(blink) = blink_alpha(state, &message.id, ui, row) {
            let band = row.expand2(Vec2::new(0.0, ROW_PADDING));
            ui.painter().rect_filled(
                band,
                CornerRadius::same(radius::CARD),
                t.accent.gamma_multiply(0.22 * blink),
            );
            ui.painter().rect_stroke(
                band,
                CornerRadius::same(radius::CARD),
                Stroke::new(1.5, t.accent.gamma_multiply(blink)),
                egui::StrokeKind::Inside,
            );
        }
        if hovered {
            if focused_message.is_none() && !state.compact {
                hover_pill(ui, state, t, s, message, row, store);
            }

            let secondary = !state.compact
                && focused_message.is_none()
                && ui.input(|input| input.pointer.secondary_clicked());
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
    state: &mut UiState,
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

    // A citação leva à original, como um resultado de busca ou uma fixada.
    // Ela some do banco quando a original é apagada; aí não há aonde ir.
    let exists = store.message(reply_to).is_some();
    let sense = if exists { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 18.0), sense);
    if exists && state.panel.is_none() && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if exists && state.panel.is_none() && response.clicked() {
        state.jump = Some(Jump {
            message_id: reply_to.to_owned(),
            found: None,
            since: ui.input(|input| input.time),
        });
    }
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
    if let Some((id, buffer)) = &mut state.editing
        && id == &message.id
    {
            let mut buffer_copy = buffer.clone();

            #[cfg(target_os = "android")]
            let save = {
                // O Android desenha e edita este texto de verdade. O egui só
                // reserva o lugar e mantém a moldura alinhada com a mensagem.
                let inner_width = (width - space::MD * 2.0).max(80.0);
                let rows = ui
                    .painter()
                    .layout(
                        buffer_copy.clone(),
                        text::message(),
                        Color32::WHITE,
                        inner_width,
                    )
                    .rows
                    .len()
                    .clamp(1, 6);
                let line_height = ui.fonts_mut(|fonts| fonts.row_height(&text::message()))
                    + ui.spacing().extra_text_line_spacing;
                let height = rows as f32 * line_height + space::SM * 2.0;
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(radius::CONTROL),
                    t.fill_soft,
                    Stroke::new(1.0, t.separator),
                    egui::StrokeKind::Inside,
                );

                let focus = std::mem::take(&mut state.edit_focus_pending);
                if focus {
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
                let key = format!("edit:{}", id);
                let editor_rect = rect.shrink2(Vec2::new(space::MD, space::SM));
                let events = crate::platform::native_text::show(
                    ui.ctx(),
                    &key,
                    &mut buffer_copy,
                    editor_rect,
                    "",
                    crate::platform::native_text::Mode::Edit,
                    6,
                    focus,
                    false,
                    t.label,
                    t.label_tertiary,
                    text::message().size,
                );
                events.submit
            };

            #[cfg(not(target_os = "android"))]
            let save = {
                let edit_id = Id::new(("editar-mensagem", id));
                let response = ui.add(
                    TextEdit::multiline(&mut buffer_copy)
                        .id(edit_id)
                        .font(text::message())
                        .desired_width(width)
                        .desired_rows(1)
                        .margin(Margin::symmetric(space::MD as i8, space::SM as i8)),
                );
                if std::mem::take(&mut state.edit_focus_pending) {
                    response.request_focus();
                }
                response.has_focus()
                    && ui.input(|input| {
                        input.key_pressed(egui::Key::Enter) && !input.modifiers.shift
                    })
            };

            *buffer = buffer_copy;
            reanchor_mentions(buffer, &mut state.editing_mentions);

            let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(s.edit_hint)
                        .font(text::footnote())
                        .color(t.label_tertiary),
                );
            });
            if cancel {
                state.editing = None;
                state.editing_mentions.clear();
                state.edit_focus_pending = false;
            } else if save
                && let Some((id, content)) = state.editing.take()
            {
                state.edit_focus_pending = false;
                let content = content.trim().to_owned();
                if content.is_empty() {
                    state.editing_mentions.clear();
                    state.actions.push(ChatAction::Delete(id));
                } else {
                    let content = store.encode_mentions(&content, &state.editing_mentions);
                    state.editing_mentions.clear();
                    state.actions.push(ChatAction::Edit {
                        message_id: id,
                        content,
                    });
                }
            }
            return;
    }

    if !message.content.is_empty() {
        let shown_content = store.display_mentions(&message.content);
        let color = if message.pending {
            t.label_secondary
        } else {
            t.label
        };
        let tokens = emoji::tokenize(&shown_content, &store.emojis);
        // URL hit-testing needs individual widgets even for otherwise plain
        // text. Keeping a separate LayoutJob fast path made normal messages
        // skip the hyperlink path entirely.
        rich_body(ui, t, s, store, state, &tokens, color, message.edited, width);
    }

    if message.pending {
        let outgoing_state = store.outgoing_state(&message.id);
        let (label, color) = match outgoing_state {
            Some(papo_core::cache::OutgoingState::Queued) => {
                (s.outgoing_waiting, t.label_tertiary)
            }
            Some(papo_core::cache::OutgoingState::Sending) => {
                (s.outgoing_sending, t.label_tertiary)
            }
            Some(papo_core::cache::OutgoingState::UnknownOutcome) => {
                (s.outgoing_uncertain, t.away)
            }
            Some(papo_core::cache::OutgoingState::FailedPermanent) => {
                (s.outgoing_failed, t.danger)
            }
            None => (s.outgoing_waiting, t.label_tertiary),
        };
        ui.add_space(space::XXS);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = space::SM;
            ui.label(RichText::new(label).font(text::footnote()).color(color));
            if matches!(
                outgoing_state,
                Some(
                    papo_core::cache::OutgoingState::UnknownOutcome
                        | papo_core::cache::OutgoingState::FailedPermanent
                )
            ) {
                if ui
                    .small_button(RichText::new(s.outgoing_retry_anyway).font(text::footnote()))
                    .clicked()
                {
                    state
                        .actions
                        .push(ChatAction::RetryOutgoing(message.id.clone()));
                }
                if ui
                    .small_button(RichText::new(s.outgoing_dismiss).font(text::footnote()))
                    .clicked()
                {
                    state
                        .actions
                        .push(ChatAction::DismissOutgoing(message.id.clone()));
                }
            }
        });
    }

    if !message.attachments.is_empty()
        && let Some(action) = attachments::draw(
            ui,
            t,
            s,
            &mut state.media,
            &message.id,
            &message.attachments,
            width,
            &mut state.media_seek_zones,
        )
    {
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

    if !message.previews.is_empty() {
        link_previews(ui, state, t, &message.id, &message.previews, width);
    }
    rich_links_from_message(
        ui,
        state,
        t,
        &message.id,
        &message.content,
        &message.previews,
        width,
    );

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

                        let visible = word.trim_end();
                        let trailing = &word[visible.len()..];
                        let url = papo_core::preview::extract_https_urls(visible)
                            .into_iter()
                            .next();

                        if let Some(url) = url {
                            let response = ui.add(
                                egui::Label::new(
                                    RichText::new(visible)
                                        .font(font.clone())
                                        .color(ui.visuals().hyperlink_color)
                                        .underline(),
                                )
                                .sense(Sense::click()),
                            );
                            if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if response.clicked() {
                                state.request_external_url(ui.ctx(), url);
                            }
                            if !trailing.is_empty() {
                                ui.label(
                                    RichText::new(trailing)
                                        .font(font.clone())
                                        .color(color),
                                );
                            }
                        } else {
                            ui.label(RichText::new(word).font(font.clone()).color(color));
                        }
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


/// Um único cartão serve tanto para o LinkPreview que veio do backend quanto
/// para uma URL encontrada diretamente no texto. A descoberta rica é sempre
/// client-side e capability-driven; metadados do backend entram apenas como
/// fallback imediato enquanto o coordenador resolve/atualiza a URL.
fn link_previews(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    message_id: &str,
    previews: &[crate::api::models::LinkPreview],
    width: f32,
) {
    for preview in previews {
        let Some(url) = preview.url.as_deref().filter(|url| !url.is_empty()) else {
            continue;
        };
        let embed_id = format!("embed:{message_id}:backend:{}", preview.id);
        preview_card(ui, state, t, &preview.id, &embed_id, url, Some(preview), width);
    }
}

fn rich_links_from_message(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    message_id: &str,
    content: &str,
    previews: &[crate::api::models::LinkPreview],
    width: f32,
) {
    use std::hash::{Hash, Hasher};

    let backend_urls: std::collections::HashSet<String> = previews
        .iter()
        .filter_map(|preview| preview.url.as_deref())
        .filter_map(papo_core::preview::canonical_url)
        .collect();

    let mut seen = std::collections::HashSet::new();
    for url in papo_core::preview::extract_https_urls(content) {
        if backend_urls.contains(&url) || !seen.insert(url.clone()) {
            continue;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut hasher);
        let id = format!("rich-{:016x}", hasher.finish());
        let embed_id = format!("embed:{message_id}:{id}");
        preview_card(ui, state, t, &id, &embed_id, &url, None, width);
    }
}

fn preview_card(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    id: &str,
    embed_id: &str,
    url: &str,
    backend: Option<&crate::api::models::LinkPreview>,
    width: f32,
) {
    use papo_core::preview::{PreviewKind, PreviewState};

    const MAX_W: f32 = 420.0;
    const IMAGE_MAX_H: f32 = 300.0;

    let resolved = state
        .previews
        .as_ref()
        .and_then(|coordinator| coordinator.get_or_request(url));
    let ready = match resolved.as_ref() {
        Some(PreviewState::Ready(preview)) => Some(preview.clone()),
        _ => None,
    };

    let backend_image = backend
        .and_then(|preview| state.media.preview(preview))
        .and_then(|texture| texture.frame(ui.ctx()))
        .cloned();

    let image_url = ready.as_ref().and_then(|preview| preview.image_url.clone());
    let remote_image = image_url
        .as_deref()
        .and_then(|remote| state.media.remote_image(id, remote))
        .and_then(|texture| texture.frame(ui.ctx()))
        .cloned();
    let image = remote_image.as_ref().or(backend_image.as_ref());

    let video_url = ready
        .as_ref()
        .filter(|preview| preview.kind == PreviewKind::Video)
        .and_then(|preview| preview.media_url.clone());
    let embed_url = ready
        .as_ref()
        .and_then(|preview| preview.embed_url.clone())
        .or_else(|| backend.and_then(|preview| preview.embed_url.clone()))
        .filter(|embed| papo_core::preview::safe_remote_url(embed));

    let title = ready
        .as_ref()
        .and_then(|preview| preview.title.as_deref())
        .or_else(|| backend.and_then(|preview| preview.title.as_deref()))
        .filter(|value| !value.is_empty());
    let description = ready
        .as_ref()
        .and_then(|preview| preview.description.as_deref())
        .or_else(|| backend.and_then(|preview| preview.description.as_deref()))
        .filter(|value| !value.is_empty());
    let provider = ready
        .as_ref()
        .and_then(|preview| preview.provider_name.as_deref())
        .or_else(|| backend.and_then(|preview| preview.provider_name.as_deref()))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            url::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_owned))
        });

    ui.add_space(space::SM);
    let card_width = width.clamp(160.0, MAX_W);
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let player_id = video_url
        .as_deref()
        .and_then(crate::media::remote_player_key)
        .unwrap_or_else(|| format!("link-preview-player:{id}"));

    let (frame, aspect, playing, position, duration) = if video_url.is_some() {
        match state.media.existing_player(&player_id) {
            Some(player) => {
                let aspect = player.aspect().clamp(0.4, 3.0);
                let playing = player.is_playing();
                let position = player.position();
                let duration = player.duration();
                let frame = player.frame(ui.ctx()).cloned();
                (frame, aspect, playing, position, duration)
            }
            None => (None, 16.0 / 9.0, false, 0.0, 0.0),
        }
    } else {
        (None, 16.0 / 9.0, false, 0.0, 0.0)
    };

    let mut media_clicked = false;
    let mut media_rect: Option<Rect> = None;
    // The preview itself is a real click target, registered before its media
    // children. egui gives overlapping clicks to the later child widget, so
    // image/video/play controls still win; otherwise the card wins instead
    // of the surrounding message row.
    let inner = ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        ui.set_width(card_width);

        if let Some(remote) = video_url.as_deref() {
            let aspect = if frame.is_some() {
                aspect
            } else {
                image
                    .map(|texture| {
                        let size = texture.size_vec2();
                        (size.x / size.y).clamp(0.4, 3.0)
                    })
                    .unwrap_or(16.0 / 9.0)
            };
            let frame_size = Vec2::new(card_width, (card_width / aspect).min(320.0));
            let controls_h = 30.0;
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(card_width, frame_size.y + controls_h),
                Sense::click(),
            );
            media_rect = Some(rect);
            let video_rect = Rect::from_min_size(rect.min, frame_size);
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius::CARD), Color32::BLACK);

            if let Some(texture) = frame.as_ref().or(image) {
                let source = texture.size_vec2();
                let scale =
                    (video_rect.width() / source.x).min(video_rect.height() / source.y);
                let painted = Rect::from_center_size(video_rect.center(), source * scale);
                ui.painter().image(
                    texture.id(),
                    painted,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }

            if !playing {
                ui.painter()
                    .circle_filled(video_rect.center(), 28.0, Color32::from_black_alpha(155));
                ui.painter().text(
                    video_rect.center() + Vec2::new(1.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    icon::PLAY,
                    text::icon(22.0),
                    Color32::WHITE,
                );
            }

            let controls =
                Rect::from_min_max(egui::pos2(rect.min.x, video_rect.max.y), rect.max);
            ui.painter().rect_filled(
                controls,
                CornerRadius::ZERO,
                Color32::from_black_alpha(190),
            );
            let progress = if duration > 0.0 {
                (position / duration).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let line = Rect::from_min_max(
                egui::pos2(controls.min.x + space::MD, controls.center().y - 2.0),
                egui::pos2(controls.max.x - 94.0, controls.center().y + 2.0),
            );
            ui.painter()
                .rect_filled(line, CornerRadius::same(2), Color32::from_white_alpha(45));
            if line.width() > 0.0 {
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        line.min,
                        Vec2::new(line.width() * progress, line.height()),
                    ),
                    CornerRadius::same(2),
                    t.accent,
                );
            }
            ui.painter().text(
                egui::pos2(controls.max.x - space::MD, controls.center().y),
                egui::Align2::RIGHT_CENTER,
                format!(
                    "{} / {}",
                    attachments::clock(position),
                    attachments::clock(duration)
                ),
                text::footnote(),
                Color32::from_white_alpha(205),
            );

            if response.clicked() {
                let ctx = ui.ctx().clone();
                state.webembed.destroy_active();
                state.media.toggle_remote_player(&player_id, remote, &ctx);
                state.media.solo(&player_id);
                media_clicked = true;
            }
            if playing {
                ui.ctx().request_repaint();
            }
            ui.add_space(space::SM);
        } else if let Some(texture) = image {
            let source = texture.size_vec2();
            let scale = (card_width / source.x)
                .min(IMAGE_MAX_H / source.y)
                .min(1.0);
            let size = Vec2::new(
                (source.x * scale).max(1.0),
                (source.y * scale).max(1.0),
            );
            let (image_rect, response) = ui.allocate_exact_size(size, Sense::click());
            media_rect = Some(image_rect);
            ui.painter().image(
                texture.id(),
                image_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            if let Some(embed) = embed_url.as_deref() {
                if state.webembed.is_active(embed_id) {
                    let allowed = webembed_inline_allowed(state);
                    let clip = state
                        .webembed_chat_clip
                        .map_or_else(|| ui.clip_rect(), |safe| safe.intersect(ui.clip_rect()));
                    state.webembed.present_inline(
                        embed_id,
                        image_rect,
                        clip,
                        ui.ctx().pixels_per_point(),
                        allowed,
                    );
                } else {
                    ui.painter().circle_filled(
                        image_rect.center(),
                        24.0,
                        Color32::from_black_alpha(170),
                    );
                    ui.painter().text(
                        image_rect.center() + Vec2::new(1.0, 0.0),
                        egui::Align2::CENTER_CENTER,
                        icon::PLAY,
                        text::icon(19.0),
                        Color32::WHITE,
                    );
                }

                if response.clicked() {
                    if state.webembed.activate(embed_id.to_owned(), embed.to_owned()) {
                        state.media.pause_all();
                        ui.ctx().request_repaint();
                    } else {
                        state.request_external_url(ui.ctx(), url.to_owned());
                    }
                    media_clicked = true;
                }
            } else if response.clicked() {
                if let Some(remote) = image_url.as_ref() {
                    state.link_viewer = Some(LinkViewer {
                        id: id.to_owned(),
                        url: remote.clone(),
                        opened: ui.input(|input| input.time),
                    });
                } else {
                    state.request_external_url(ui.ctx(), url.to_owned());
                }
                media_clicked = true;
            }
            ui.add_space(space::SM);
        } else if let Some(embed) = embed_url.as_deref() {
            let frame_size = Vec2::new(card_width, (card_width * 9.0 / 16.0).min(260.0));
            let (rect, response) = ui.allocate_exact_size(frame_size, Sense::click());
            media_rect = Some(rect);
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(radius::CARD),
                Color32::from_black_alpha(225),
            );

            if state.webembed.is_active(embed_id) {
                let allowed = webembed_inline_allowed(state);
                let clip = state
                    .webembed_chat_clip
                    .map_or_else(|| ui.clip_rect(), |safe| safe.intersect(ui.clip_rect()));
                state.webembed.present_inline(
                    embed_id,
                    rect,
                    clip,
                    ui.ctx().pixels_per_point(),
                    allowed,
                );
            } else {
                ui.painter()
                    .circle_filled(rect.center(), 28.0, Color32::from_black_alpha(155));
                ui.painter().text(
                    rect.center() + Vec2::new(1.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    icon::PLAY,
                    text::icon(22.0),
                    Color32::WHITE,
                );
            }
            if response.clicked() {
                if state.webembed.activate(embed_id.to_owned(), embed.to_owned()) {
                    state.media.pause_all();
                    ui.ctx().request_repaint();
                } else {
                    state.request_external_url(ui.ctx(), url.to_owned());
                }
                media_clicked = true;
            }
            ui.add_space(space::SM);
        }

        if let Some(provider) = provider.as_deref() {
            ui.label(
                RichText::new(provider)
                    .font(text::caption())
                    .color(t.label_tertiary),
            );
            ui.add_space(space::XXS);
        }

        if let Some(title) = title {
            ui.label(RichText::new(title).font(text::headline()).color(t.label));
        }
        if let Some(description) = description {
            ui.add_space(space::XXS);
            ui.label(
                RichText::new(description)
                    .font(text::body())
                    .color(t.label_secondary),
            );
        }

        if title.is_none() && description.is_none() {
            let label = match resolved {
                Some(PreviewState::Loading) => "Carregando preview…",
                Some(PreviewState::RetryLater { .. }) => url,
                Some(PreviewState::Negative) | None => url,
                Some(PreviewState::Ready(_)) => url,
            };
            ui.label(
                RichText::new(label)
                    .font(text::footnote())
                    .color(t.label_secondary),
            );
        }
    });

    let rect = inner.response.rect.expand2(Vec2::new(space::MD, space::SM));

    // The scope response owns background clicks. Do not add another click
    // interaction here: anything registered after the media children would
    // steal their taps.
    // Keyed by card occurrence, not by preview id: one preview can back
    // several messages, and a sliding row moves to another layer, which made
    // egui see the same id in two layers in one frame (debug assert).
    let hover = ui.interact(rect, Id::new(("link-preview-hover", embed_id)), Sense::hover());
    let fill = if hover.hovered() {
        t.fill_medium
    } else {
        t.fill_soft
    };
    ui.painter().set(
        backdrop,
        egui::epaint::RectShape::new(
            rect,
            CornerRadius::same(radius::CARD),
            fill,
            Stroke::new(1.0, t.separator),
            egui::StrokeKind::Inside,
        ),
    );

    if hover.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // The card background is a link, but the actual media surface is not:
    // video/image/WebEmbed keeps its own interaction. Register only the
    // rectangles around that media so we never steal its tap from a child.
    let mut open_card = false;
    let mut zones = Vec::new();
    if let Some(media) = media_rect {
        if rect.min.y < media.min.y {
            zones.push(Rect::from_min_max(rect.min, egui::pos2(rect.max.x, media.min.y)));
        }
        if media.max.y < rect.max.y {
            zones.push(Rect::from_min_max(
                egui::pos2(rect.min.x, media.max.y),
                rect.max,
            ));
        }
        if rect.min.x < media.min.x {
            zones.push(Rect::from_min_max(
                egui::pos2(rect.min.x, media.min.y),
                egui::pos2(media.min.x, media.max.y),
            ));
        }
        if media.max.x < rect.max.x {
            zones.push(Rect::from_min_max(
                egui::pos2(media.max.x, media.min.y),
                egui::pos2(rect.max.x, media.max.y),
            ));
        }
    } else {
        zones.push(rect);
    }
    for (index, zone) in zones.into_iter().filter(|zone| zone.is_positive()).enumerate() {
        let response = ui.interact(
            zone,
            Id::new(("link-preview-open", embed_id, index)),
            Sense::click(),
        );
        open_card |= response.clicked();
    }
    if open_card && !media_clicked {
        state.request_external_url(ui.ctx(), url.to_owned());
    }
    ui.add_space(space::XS);
}


fn webembed_inline_allowed(state: &UiState) -> bool {
    !state.webembed_blocked
        && state.mobile_surface == MobileSurface::Chat
        && state.panel.is_none()
        && state.popup.is_none()
        && state.viewer.is_none()
        && state.link_viewer.is_none()
        && state.external_link_prompt.is_none()
}

/// A floating player is PiP-like: it stays on top while drawers, panels and
/// menus come and go, so it is not hidden just because the chat is covered —
/// suspending it there is what made the browser reload and lose its position.
/// Only a full-screen modal (preferences sheet, a viewer) or an explicit block
/// takes it away, and even then it is suspended, not destroyed.
fn webembed_floating_allowed(state: &UiState) -> bool {
    !state.webembed_blocked
        && state.viewer.is_none()
        && state.link_viewer.is_none()
        && state.external_link_prompt.is_none()
}

fn webembed_floating(ui: &mut egui::Ui, state: &mut UiState, t: &Tokens) {
    if !state
        .webembed
        .should_float(state.webembed_behavior, state.webembed_scope)
        || !webembed_floating_allowed(state)
    {
        state.webembed_float_rect = None;
        return;
    }

    let screen = ui.ctx().content_rect();
    let safe = state.webembed_chat_clip.unwrap_or(screen);
    let controls_h = 32.0;
    let margin = if state.compact { 0.0 } else { space::LG };
    let max_width = (screen.width() - margin * 2.0).clamp(200.0, 720.0);

    // Phones start edge-to-edge; once the player has been moved or resized,
    // the remembered width wins.
    let customized = state.webembed_float_pos.is_some();
    let width = if state.compact && !customized {
        screen.width()
    } else {
        state.webembed_float_width.clamp(200.0, max_width)
    };
    let size = Vec2::new(width, width * 9.0 / 16.0 + controls_h);

    let default_min = egui::pos2(
        if state.compact {
            safe.min.x
        } else {
            safe.max.x - width - margin
        },
        (safe.max.y - size.y - margin).max(safe.min.y + margin),
    );
    let mut min = state
        .webembed_float_pos
        .map_or(default_min, |(x, y)| egui::pos2(x, y));
    let x_lo = safe.min.x;
    let x_hi = (safe.max.x - size.x).max(x_lo);
    let y_lo = safe.min.y;
    let y_hi = (safe.max.y - size.y).max(y_lo);
    min.x = min.x.clamp(x_lo, x_hi);
    min.y = min.y.clamp(y_lo, y_hi);

    let outer = Rect::from_min_size(min, size);

    // Above the navigation/members drawers and action panels, all of which
    // live in Order::Foreground. The float is PiP-like: while it is up, the
    // pointer over it belongs to it, not to whatever it overlaps.
    let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("webembed-floating"));
    let top = ui.new_child(
        UiBuilder::new()
            .layer_id(layer)
            .max_rect(screen)
            .sense(Sense::hover()),
    );

    // The header is the drag handle; its top-left corner resizes and the close
    // button sits at the right. Interactions are registered from this frame's
    // rect, then the origin is applied before anything is painted.
    let header = Rect::from_min_size(outer.min, Vec2::new(outer.width(), controls_h));
    let resize_rect = Rect::from_min_size(header.min, Vec2::splat(controls_h));
    let close_rect = Rect::from_center_size(
        egui::pos2(header.max.x - controls_h / 2.0, header.center().y),
        Vec2::splat(controls_h),
    );
    let drag_rect = Rect::from_min_max(
        egui::pos2(resize_rect.max.x, header.min.y),
        egui::pos2(close_rect.min.x, header.max.y),
    );

    let drag = top.interact(drag_rect, Id::new("webembed-floating-drag"), Sense::drag());
    if drag.dragged() {
        let delta = ui.input(|input| input.pointer.delta());
        min += delta;
        min.x = min.x.clamp(x_lo, x_hi);
        min.y = min.y.clamp(y_lo, y_hi);
        state.webembed_float_pos = Some((min.x, min.y));
        ui.ctx().request_repaint();
    }
    if drag.hovered() || drag.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    let resize = top.interact(resize_rect, Id::new("webembed-floating-resize"), Sense::drag());
    if resize.dragged() {
        let dx = ui.input(|input| input.pointer.delta().x);
        state.webembed_float_width = (state.webembed_float_width - dx).clamp(200.0, max_width);
        state.webembed_float_pos = Some((min.x, min.y));
        ui.ctx().request_repaint();
    }
    if resize.hovered() || resize.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }

    // Repaint from the (possibly moved) origin.
    let outer = Rect::from_min_size(min, size);
    let header = Rect::from_min_size(outer.min, Vec2::new(outer.width(), controls_h));
    let resize_rect = Rect::from_min_size(header.min, Vec2::splat(controls_h));
    let close_rect = Rect::from_center_size(
        egui::pos2(header.max.x - controls_h / 2.0, header.center().y),
        Vec2::splat(controls_h),
    );

    top.painter().rect(
        outer,
        CornerRadius::same(radius::CARD),
        t.fill_medium,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    // Resize glyph at the top-left corner.
    top.painter().line_segment(
        [
            egui::pos2(resize_rect.min.x + 8.0, resize_rect.min.y + 20.0),
            egui::pos2(resize_rect.min.x + 20.0, resize_rect.min.y + 8.0),
        ],
        Stroke::new(1.5, t.label_tertiary),
    );
    top.painter().line_segment(
        [
            egui::pos2(resize_rect.min.x + 13.0, resize_rect.min.y + 22.0),
            egui::pos2(resize_rect.min.x + 22.0, resize_rect.min.y + 13.0),
        ],
        Stroke::new(1.5, t.label_tertiary),
    );
    top.painter().text(
        egui::pos2(header.min.x + space::SM, header.center().y),
        egui::Align2::LEFT_CENTER,
        "Web",
        text::caption(),
        t.label_secondary,
    );
    if top
        .interact(close_rect, Id::new("webembed-floating-close"), Sense::click())
        .clicked()
    {
        state.webembed.destroy_active();
        return;
    }
    top.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon::X,
        text::icon(14.0),
        t.label,
    );

    let browser_rect = Rect::from_min_max(
        egui::pos2(outer.min.x, header.max.y),
        outer.max,
    );
    state.webembed_float_rect = Some(outer);
    state.webembed.present_floating(
        browser_rect,
        safe,
        ui.ctx().pixels_per_point(),
        true,
    );
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
                icon::ARROW_BEND_UP_LEFT => state.start_reply(message.id.clone()),
                icon::PENCIL_SIMPLE => {
                    let (content, bindings) =
                        store.display_mentions_with_bindings(&message.content);
                    state.editing = Some((message.id.clone(), content));
                    state.editing_mentions = bindings;
                    state.edit_focus_pending = true;
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
    let screen = ui.ctx().content_rect();
    let mut top = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(screen));

    saved_toast(&mut top, state, t, s);

    if state.external_link_prompt.is_some() {
        external_link_prompt(&mut top, state, t, s);
    }

    if let Some(popup) = state.popup.clone() {
        match popup.kind {
            PopupKind::Emoji | PopupKind::ComposerEmoji | PopupKind::ComposerSticker => {
                emoji_popup(&mut top, store, state, t, s, &popup)
            }
            PopupKind::Menu => context_menu(&mut top, store, state, t, s, &popup),
        }
    }

    if state.link_viewer.is_some() {
        link_image_viewer(ui, state, t);
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

fn external_link_prompt(
    ui: &mut egui::Ui,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
) {
    let Some(mut prompt) = state.external_link_prompt.take() else {
        return;
    };

    let screen = ui.ctx().content_rect();
    let backdrop = ui.interact(
        screen,
        Id::new("external-link-backdrop"),
        Sense::click(),
    );
    ui.painter()
        .rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(150));

    let width = 440.0_f32.min((screen.width() - space::XXL * 2.0).max(260.0));
    let height = 188.0;
    let rect = Rect::from_center_size(screen.center(), Vec2::new(width, height));
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::SHEET),
        t.elevated_bg,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    let mut body = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink(space::XL))
            .layout(Layout::top_down(Align::Min)),
    );
    body.label(
        RichText::new(s.external_link_title)
            .font(text::headline())
            .color(t.label),
    );
    body.add_space(space::SM);
    body.label(
        RichText::new(s.external_link_body)
            .font(text::body())
            .color(t.label_secondary),
    );
    body.add_space(space::XS);
    body.label(
        RichText::new(&prompt.url)
            .font(text::footnote())
            .color(t.label),
    );
    body.add_space(space::MD);
    body.checkbox(
        &mut prompt.remember,
        format!("{} {}", s.external_link_trust_host, prompt.host),
    );

    let button_h = 32.0;
    let open_w = 112.0;
    let cancel_w = 88.0;
    let y = rect.max.y - space::XL - button_h;
    let open_rect = Rect::from_min_size(
        egui::pos2(rect.max.x - space::XL - open_w, y),
        Vec2::new(open_w, button_h),
    );
    let cancel_rect = Rect::from_min_size(
        egui::pos2(open_rect.min.x - space::SM - cancel_w, y),
        Vec2::new(cancel_w, button_h),
    );

    let cancel = ui.interact(cancel_rect, Id::new("external-link-cancel"), Sense::click());
    ui.painter().rect_filled(
        cancel_rect,
        CornerRadius::same((button_h / 2.0) as u8),
        if cancel.hovered() { t.fill_medium } else { t.fill_soft },
    );
    ui.painter().text(
        cancel_rect.center(),
        egui::Align2::CENTER_CENTER,
        s.cancel,
        text::body(),
        t.label,
    );

    let open = ui.interact(open_rect, Id::new("external-link-open"), Sense::click());
    ui.painter().rect_filled(
        open_rect,
        CornerRadius::same((button_h / 2.0) as u8),
        if open.hovered() { t.accent } else { t.accent.gamma_multiply(0.88) },
    );
    ui.painter().text(
        open_rect.center(),
        egui::Align2::CENTER_CENTER,
        s.external_link_open,
        text::body(),
        t.accent_label,
    );

    let now = ui.input(|input| input.time);
    let outside = now > prompt.opened + 0.05
        && backdrop.clicked()
        && backdrop
            .interact_pointer_pos()
            .is_some_and(|position| !rect.contains(position));
    let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));

    if open.clicked() {
        if prompt.remember {
            state.trusted_link_hosts.insert(prompt.host.clone());
        }
        ui.ctx().open_url(egui::OpenUrl::new_tab(prompt.url));
    } else if !(cancel.clicked() || outside || escape) {
        state.external_link_prompt = Some(prompt);
    }
}

fn link_image_viewer(ui: &mut egui::Ui, state: &mut UiState, t: &Tokens) {
    let Some(viewer) = state.link_viewer.clone() else {
        return;
    };
    let screen = ui.ctx().content_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("papo-link-viewer"));
    let top = ui.new_child(
        UiBuilder::new()
            .layer_id(layer)
            .max_rect(screen)
            .sense(Sense::click()),
    );

    top.painter()
        .rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(232));

    let texture = state
        .media
        .remote_image(&viewer.id, &viewer.url)
        .and_then(|texture| texture.frame(top.ctx()))
        .cloned();

    let mut content = Rect::NOTHING;
    if let Some(texture) = texture {
        let stage = screen.shrink2(Vec2::new(space::XXXL, 64.0));
        let natural = texture.size_vec2();
        let scale = (stage.width() / natural.x)
            .min(stage.height() / natural.y)
            .min(1.0);
        let size = natural * scale;
        content = Rect::from_center_size(stage.center(), size);
        top.painter().image(
            texture.id(),
            content,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        top.painter().text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            "Carregando imagem…",
            text::body(),
            t.label_secondary,
        );
        top.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }

    let close_rect = Rect::from_center_size(
        egui::pos2(screen.max.x - 28.0, screen.min.y + 28.0),
        Vec2::splat(36.0),
    );
    let close = top.interact(close_rect, Id::new("link-viewer-close"), Sense::click());
    top.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon::X,
        text::icon(20.0),
        Color32::WHITE,
    );

    let backdrop = top.interact(screen, Id::new("link-viewer-backdrop"), Sense::click());
    let escape = top.input(|input| input.key_pressed(egui::Key::Escape));
    let outside = top.input(|input| input.time) > viewer.opened + 0.05
        && backdrop.clicked()
        && backdrop
            .interact_pointer_pos()
            .is_some_and(|position| !content.expand(space::MD).contains(position));
    if close.clicked() || escape || outside {
        state.link_viewer = None;
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
    let safe = ui.ctx().content_rect();
    let desired = Vec2::new(316.0, if custom_only { 300.0 } else { 380.0 });
    let size = Vec2::new(
        desired.x.min((safe.width() - space::XL).max(220.0)),
        desired.y.min((safe.height() - space::XL).max(220.0)),
    );
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
            MessageCommand::Reply => state.start_reply(message.id.clone()),
            MessageCommand::Edit => {
                let (content, bindings) =
                    store.display_mentions_with_bindings(&message.content);
                state.editing = Some((message.id.clone(), content));
                state.editing_mentions = bindings;
                state.edit_focus_pending = true;
            }
            MessageCommand::Copy => {
                ui.ctx().copy_text(store.display_mentions(&message.content));
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

/// Avisos transitórios. A superfície mede o conteúdo e só cresce até o
/// limite confortável; depois disso os rótulos quebram linha.
fn saved_toast(ui: &mut egui::Ui, state: &mut UiState, t: &Tokens, s: &Strings) {
    if let Some((message, at)) = state.error.clone() {
        let now = ui.ctx().input(|input| input.time);
        if now - at > TOAST_SECONDS {
            state.error = None;
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
            floating_pill(
                ui.ctx(),
                Id::new("toast-error"),
                t,
                state.translucent,
                space::XXXL * 2.0,
                420.0,
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(icon::WARNING)
                                .font(text::icon(14.0))
                                .color(t.danger),
                        );
                        ui.label(
                            egui::RichText::new(message)
                                .font(text::body())
                                .color(t.danger),
                        );
                    });
                },
            );
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
    if now - at > TOAST_SECONDS || scrolled {
        state.media.saved = None;
        return;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(250));

    let mut open_clicked = false;
    floating_pill(
        ui.ctx(),
        Id::new("toast-saved"),
        t,
        state.translucent,
        space::XXXL * 2.0,
        420.0,
        |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(icon::CHECK_CIRCLE)
                        .font(text::icon(14.0))
                        .color(t.label),
                );
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&name)
                                .font(text::body())
                                .color(t.label),
                        )
                        .wrap(),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(path.display().to_string())
                                .font(text::footnote())
                                .color(t.label_tertiary),
                        )
                        .wrap(),
                    );
                });
                if ui.button(s.open).clicked() {
                    open_clicked = true;
                }
            });
        },
    );
    if open_clicked {
        state.actions.push(ChatAction::OpenExternally(path));
        state.media.saved = None;
    }
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

/// Reencontra no texto as menções escolhidas pelo autocomplete.
///
/// O offset salvo é só uma âncora. Se o usuário escreve/apaga antes da
/// menção, escolhemos a ocorrência válida mais próxima do offset anterior.
/// Isso preserva a identidade de nicknames duplicados sem pôr ids invisíveis
/// dentro do EditText.
fn reanchor_mentions(text: &str, bindings: &mut Vec<MentionBinding>) {
    bindings.sort_by_key(|binding| binding.start);
    let mut used = Vec::<usize>::new();
    let mut kept = Vec::with_capacity(bindings.len());

    for mut binding in bindings.drain(..) {
        let pattern = format!("@{}", binding.label);
        let best = text
            .match_indices(&pattern)
            .filter_map(|(byte, _)| {
                let before_ok = byte == 0
                    || text[..byte]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_whitespace() || (!c.is_alphanumeric() && c != '_'));
                let end = byte + pattern.len();
                let after_ok = end == text.len()
                    || text[end..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_whitespace() || (!c.is_alphanumeric() && c != '_'));
                if !before_ok || !after_ok {
                    return None;
                }
                let start = text[..byte].chars().count();
                (!used.contains(&start)).then_some(start)
            })
            .min_by_key(|start| start.abs_diff(binding.start));

        if let Some(start) = best {
            binding.start = start;
            used.push(start);
            kept.push(binding);
        }
    }

    *bindings = kept;
}

/// Tira o texto da caixa e o coloca na fila de envio.
fn submit(store: &Store, state: &mut UiState) {
    let visible = state.composer.trim().to_owned();
    if visible.is_empty() && state.attachments.is_empty() {
        return;
    }
    let content = store.encode_mentions(&visible, &state.composer_mentions);
    state.composer.clear();
    state.composer_mentions.clear();
    state.actions.push(ChatAction::Send {
        content,
        reply_to: state.replying.take(),
        notify_reply: state.reply_notify,
        attachments: std::mem::take(&mut state.attachments),
    });
}

fn composer_height(ui: &egui::Ui, state: &UiState, area: Rect) -> f32 {
    let recording = state.recorder.is_some();
    let with_record = state.show_record || recording;
    let left_count = 1 + usize::from(with_record);
    let left_width =
        left_count as f32 * PILL_HEIGHT + (left_count as f32 - 1.0) * space::SM + space::MD;
    let composer_width =
        (area.width() - PILL_MARGIN * 2.0 - left_width).max(COMPOSER_LINE_H);

    // Reserva dos quatro controles à direita e das margens do campo.
    let controls_width = space::MD * 2.0 + HIT_TARGET * 4.0 + space::XXS * 4.0;
    let text_width = (composer_width - controls_width).max(80.0);

    let rows = if state.composer.is_empty() {
        1
    } else {
        ui.painter()
            .layout(
                state.composer.clone(),
                text::message(),
                Color32::WHITE,
                text_width,
            )
            .rows
            .len()
            .clamp(1, COMPOSER_MAX_ROWS)
    };

    let row_h = ui.fonts_mut(|fonts| fonts.row_height(&text::message()))
        + ui.spacing().extra_text_line_spacing;
    let text_h = rows as f32 * row_h + space::SM * 2.0;
    let mut height = COMPOSER_LINE_H.max(text_h);

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

    // Faixa de resposta: o corpo inteiro leva até a mensagem original; o X
    // continua sendo um alvo separado para cancelar a resposta.
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

        let close = Rect::from_center_size(
            egui::pos2(band.max.x - space::LG - 8.0, band.center().y),
            Vec2::splat(20.0),
        );
        let notify = Rect::from_center_size(
            egui::pos2(close.min.x - space::MD - 10.0, band.center().y),
            Vec2::splat(20.0),
        );
        let jump_rect = Rect::from_min_max(
            band.min,
            egui::pos2(notify.min.x - space::SM, band.max.y),
        );
        let jump = ui.interact(jump_rect, Id::new(("reply-jump", &reply_to)), Sense::click());
        if jump.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui.painter().text(
            egui::pos2(band.min.x + space::LG, band.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{} {name}", s.replying_to),
            text::footnote(),
            if jump.hovered() { t.label } else { t.label_secondary },
        );
        if jump.clicked() && store.message(&reply_to).is_some() {
            state.jump = Some(Jump {
                message_id: reply_to.clone(),
                found: None,
                since: ui.input(|input| input.time),
            });
        }

        let notify_response = ui
            .interact(notify, Id::new("reply-notify"), Sense::click())
            .on_hover_text(s.reply_notification);
        if notify_response.hovered() {
            ui.painter()
                .circle_filled(notify.center(), 10.0, t.fill_soft);
        }
        ui.painter().text(
            notify.center(),
            egui::Align2::CENTER_CENTER,
            icon::AT,
            text::icon(13.0),
            if state.reply_notify {
                t.accent
            } else {
                t.label_tertiary
            },
        );
        if notify_response.clicked() {
            state.reply_notify = !state.reply_notify;
        }

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
        if inline_button(ui, t, cancel, icon::TRASH, s.record_cancel, "record-cancel").clicked()
            && let Some(recorder) = state.recorder.take()
        {
            recorder.cancel();
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
        submit(store, state);
    }

    // O campo de texto ocupa o que sobrou entre a borda e os botões.
    let field = Rect::from_min_max(
        egui::pos2(line.min.x + space::MD, line.min.y),
        egui::pos2(emoji_rect.min.x - space::XXS, line.max.y),
    );

    #[cfg(target_os = "android")]
    let android_composer_tap = if state.editing.is_some() {
        ui.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| field.contains(position))
        })
    } else {
        false
    };

    #[cfg(target_os = "android")]
    if state.editing.is_some() && ui.input(|input| input.pointer.any_pressed()) {
        // Um toque que chegou ao egui aconteceu fora da EditText da mensagem
        // (toques dentro dela são consumidos pela própria View Android).
        // Portanto ele encerra a edição. Se caiu no compositor, o foco é
        // transferido para ele logo abaixo.
        state.editing = None;
        state.editing_mentions.clear();
        state.edit_focus_pending = false;
        crate::platform::native_text::dismiss();
    }

    #[cfg(target_os = "android")]
    let android_chat_editor_visible = !state.compact
        || (state.mobile_surface == MobileSurface::Chat
            && state.drawer.shown <= 0.0
            && !state.drawer.dragging);

    #[cfg(target_os = "android")]
    if !android_chat_editor_visible {
        // A View Android fica acima da superfície do egui. Enquanto uma
        // gaveta cobre a conversa ela precisa desaparecer de verdade, senão
        // rouba toques dos controles desenhados por cima.
        crate::platform::native_text::dismiss();
    }

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(field)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let hint = format!("{} #{}…", s.composer_hint, channel_name);
            let edit_id = Id::new("caixa-de-mensagem");
            // As setas e o Enter pertencem à lista de sugestões enquanto ela
            // estiver aberta; sem tirá-los da caixa de texto, a seta moveria
            // o cursor e o Enter mandaria `:parc` como mensagem.
            // Com a lista aberta, estas teclas são dela. Elas têm de ser
            // lidas **no mesmo gesto** em que saem da fila de eventos: ler
            // depois de tirá-las era perguntar por eventos que eu mesmo
            // acabara de apagar, e por isso nada respondia.
            let keys = if state.suggest.is_some() {
                ui.input_mut(|input| {
                    let taken = SuggestKeys {
                        up: input.key_pressed(egui::Key::ArrowUp),
                        down: input.key_pressed(egui::Key::ArrowDown),
                        accept: input.key_pressed(egui::Key::Enter)
                            || input.key_pressed(egui::Key::Tab),
                        dismiss: input.key_pressed(egui::Key::Escape),
                    };
                    input.events.retain(|event| {
                        !matches!(
                            event,
                            egui::Event::Key {
                                key: egui::Key::ArrowUp
                                    | egui::Key::ArrowDown
                                    | egui::Key::Enter
                                    | egui::Key::Tab,
                                pressed: true,
                                ..
                            }
                        )
                    });
                    taken
                })
            } else {
                SuggestKeys::default()
            };

            #[cfg(target_os = "android")]
            let (field_focused, caret) =
                if state.editing.is_none() && android_chat_editor_visible {
                    let native_rect = field.shrink2(Vec2::new(space::XS, 0.0));
                    let events = crate::platform::native_text::show(
                        ui.ctx(),
                        "composer",
                        &mut state.composer,
                        native_rect,
                        &hint,
                        crate::platform::native_text::Mode::Composer,
                        COMPOSER_MAX_ROWS,
                        android_composer_tap,
                        state.suggest.is_some(),
                        t.label,
                        t.label_tertiary,
                        text::message().size,
                    );
                    state.typed = events.changed;
                    if events.submit && state.suggest.is_some() {
                        accept_suggestion(store, state, ui.ctx(), edit_id);
                    }
                    (events.focused, events.caret)
                } else {
                    // Durante edição de mensagem ou enquanto uma gaveta cobre
                    // a conversa, o compositor é só desenho egui. A View
                    // Android não pode continuar existindo por cima dela.
                    let shown = if state.composer.is_empty() {
                        hint.as_str()
                    } else {
                        state.composer.as_str()
                    };
                    ui.painter().text(
                        field.left_center(),
                        egui::Align2::LEFT_CENTER,
                        shown,
                        text::message(),
                        if state.composer.is_empty() {
                            t.label_tertiary
                        } else {
                            t.label
                        },
                    );
                    (false, None)
                };

            #[cfg(not(target_os = "android"))]
            let (field_focused, caret) = {
                let ime_changed =
                    crate::platform::ime::prepare_text_edit(ui.ctx(), edit_id, &mut state.composer);
                let viewport_h =
                    (line.height() - space::XS * 2.0).max(COMPOSER_LINE_H - space::XS * 2.0);
                let response = egui::ScrollArea::vertical()
                    .id_salt("composer-text-scroll")
                    .max_height(viewport_h)
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.set_min_height(viewport_h);
                        TextEdit::multiline(&mut state.composer)
                            .id(edit_id)
                            .hint_text(RichText::new(hint).color(t.label_tertiary))
                            .frame(Frame::NONE)
                            .font(text::message())
                            .desired_rows(1)
                            .vertical_align(Align::Center)
                            .min_size(Vec2::new(0.0, viewport_h))
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(space::XS as i8, space::SM as i8))
                            .show(ui)
                            .response
                            .response
                    })
                    .inner;
                let _ = crate::platform::ime::sync_text_edit(
                    ui.ctx(),
                    edit_id,
                    &mut state.composer,
                    response.has_focus(),
                    crate::platform::ime::Kind::Multiline,
                );
                state.typed = response.changed() || ime_changed;
                (response.has_focus(), caret_of(ui.ctx(), edit_id))
            };

            reanchor_mentions(&state.composer, &mut state.composer_mentions);

            if keys.dismiss {
                state.suggest = None;
                state.suggest_muted = true;
            } else {
                refresh_suggestions(store, state, field_focused, caret);
            }

            if let Some(suggest) = state.suggest.as_mut() {
                let last = suggest.matches.len().saturating_sub(1);
                if keys.up {
                    suggest.index = if suggest.index == 0 { last } else { suggest.index - 1 };
                }
                if keys.down {
                    suggest.index = if suggest.index >= last { 0 } else { suggest.index + 1 };
                }
            }
            let accepted = keys.accept && state.suggest.is_some();
            if accepted {
                accept_suggestion(store, state, ui.ctx(), edit_id);
            }

            // Enter envia; Shift+Enter quebra linha. Com a lista aberta o
            // Enter já foi gasto escolhendo a figurinha.
            #[cfg(not(target_os = "android"))]
            {
                let enter = field_focused
                    && !accepted
                    && state.suggest.is_none()
                    && ui.input(|input| {
                        input.key_pressed(egui::Key::Enter) && !input.modifiers.shift
                    });
                if enter {
                    submit(store, state);
                }
            }
        },
    );

    // A lista sobe da caixa: ela é resposta ao que está sendo digitado, e
    // tem de ficar onde os olhos já estão.
    suggestions(ui, store, state, t, line);
}

/// Posição do cursor na caixa de mensagem, em caracteres.
fn caret_of(ctx: &egui::Context, id: Id) -> Option<usize> {
    egui::TextEdit::load_state(ctx, id)?
        .cursor
        .char_range()
                // `CharIndex` é um newtype sobre `usize`; contamos caracteres.
        .map(|range| range.primary.index.0)
}

/// Atualiza as sugestões de `@pessoa` ou `:figurinha` em torno do cursor.
fn refresh_suggestions(store: &Store, state: &mut UiState, focused: bool, caret: Option<usize>) {
    let Some(caret) = caret.filter(|_| focused) else {
        state.suggest = None;
        return;
    };

    let candidate = typing_mention(&state.composer, caret)
        .map(|(start, query)| (SuggestKind::Mention, start, query))
        .or_else(|| {
            typing_shortcode(&state.composer, caret)
                .map(|(start, query)| (SuggestKind::Sticker, start, query))
        });
    let Some((kind, start, query)) = candidate else {
        state.suggest = None;
        state.suggest_muted = false;
        return;
    };

    if state.suggest_muted {
        if state.suggest_start == Some(start) {
            return;
        }
        state.suggest_muted = false;
    }
    state.suggest_start = Some(start);

    let needle = query.to_lowercase();
    let matches: Vec<String> = match kind {
        SuggestKind::Sticker => store
            .emojis
            .iter()
            .filter(|emoji| emoji.name.to_lowercase().contains(&needle))
            .map(|emoji| emoji.id.clone())
            .take(8)
            .collect(),
        SuggestKind::Mention => {
            let mut people: Vec<_> = store
                .members
                .iter()
                .filter(|member| member.id != store.me)
                .filter(|member| {
                    needle.is_empty()
                        || member.username.to_lowercase().contains(&needle)
                        || member.name.to_lowercase().contains(&needle)
                })
                .collect();
            people.sort_by_key(|member| {
                let username = member.username.to_lowercase();
                let name = member.name.to_lowercase();
                (
                    !(needle.is_empty() || name.starts_with(&needle)),
                    !(needle.is_empty() || username.starts_with(&needle)),
                    member.presence == Presence::Offline,
                    name,
                    username,
                )
            });
            people
                .into_iter()
                .map(|member| member.id.clone())
                .take(8)
                .collect()
        }
    };

    if matches.is_empty() {
        state.suggest = None;
        return;
    }
    let index = match &state.suggest {
        Some(previous) if previous.kind == kind && previous.matches == matches => previous.index,
        _ => 0,
    };
    state.suggest = Some(Suggest {
        kind,
        start,
        end: caret,
        matches,
        index: index.min(7),
    });
}

/// O `@nome` imediatamente antes do cursor. Diferente da figurinha, um
/// `@` sozinho já abre a lista inteira para citar alguém rapidamente.
fn typing_mention(text: &str, caret: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    if caret > chars.len() {
        return None;
    }
    let mut index = caret;
    while index > 0 {
        let c = chars[index - 1];
        if c == '@' {
            let start = index - 1;
            if start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
                return None;
            }
            let query: String = chars[index..caret].iter().collect();
            return Some((start, query));
        }
        if c.is_whitespace() || !c.is_alphanumeric() && c != '_' && c != '-' {
            return None;
        }
        index -= 1;
    }
    None
}

/// O `:alguma` imediatamente antes do cursor, se houver. Devolve onde o
/// `:` está e o que veio depois dele.
fn typing_shortcode(text: &str, caret: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    if caret > chars.len() {
        return None;
    }
    let mut index = caret;
    while index > 0 {
        let c = chars[index - 1];
        if c == ':' {
            // Pelo menos uma letra depois do `:`; um `:` solto não sugere
            // nada, e `:algo:` já fechado também não.
            let query: String = chars[index..caret].iter().collect();
            if query.is_empty() {
                return None;
            }
            return Some((index - 1, query));
        }
        if c.is_whitespace() || !c.is_alphanumeric() && c != '_' && c != '-' {
            return None;
        }
        index -= 1;
    }
    None
}

fn replace_char_range(text: &str, start: usize, end: usize, replacement: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if start > end || end > chars.len() {
        return None;
    }
    let mut next: String = chars[..start].iter().collect();
    next.push_str(replacement);
    next.extend(chars[end..].iter());
    Some(next)
}

/// Troca o fragmento ativo pelo valor escolhido e põe o cursor depois dele.
fn accept_suggestion(
    store: &Store,
    state: &mut UiState,
    ctx: &egui::Context,
    id: Id,
) {
    #[cfg(target_os = "android")]
    let _ = (ctx, id);
    let Some(suggest) = state.suggest.take() else {
        return;
    };
    let Some(chosen) = suggest.matches.get(suggest.index) else {
        return;
    };
    let caret = suggest.end;

    let replacement = match suggest.kind {
        SuggestKind::Sticker => {
            let Some(emoji) = store.emojis.iter().find(|emoji| &emoji.id == chosen) else {
                return;
            };
            format!(":{}:", emoji.name)
        }
        SuggestKind::Mention => {
            let Some(member) = store.member(chosen) else {
                return;
            };
            format!("@{} ", member.name)
        }
    };

    let Some(next) = replace_char_range(&state.composer, suggest.start, caret, &replacement)
    else {
        return;
    };
    state.composer = next;
    if suggest.kind == SuggestKind::Mention
        && let Some(member) = store.member(chosen)
    {
        // Remove binding anterior que ocupava o mesmo início e conserva a
        // identidade exata escolhida mesmo se houver nicknames iguais.
        state.composer_mentions.retain(|binding| binding.start != suggest.start);
        state.composer_mentions.push(MentionBinding {
            start: suggest.start,
            label: member.name.clone(),
            user_id: member.id.clone(),
        });
    }
    state.typed = true;

    let after = suggest.start + replacement.chars().count();

    #[cfg(target_os = "android")]
    crate::platform::native_text::set_selection("composer", &state.composer, after);

    #[cfg(not(target_os = "android"))]
    if let Some(mut edit) = egui::TextEdit::load_state(ctx, id) {
        edit.cursor.set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(after),
        )));
        edit.store(ctx, id);
    }
}

/// Lista de sugestões do compositor, logo acima da caixa de mensagem.
///
/// Mora numa camada própria à frente de tudo. Desenhada solta dentro da
/// conversa, ela ficava por baixo do que já tinha sido registrado ali e o
/// clique do mouse nunca chegava nela.
fn suggestions(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, line: Rect) {
    let Some(suggest) = state.suggest.clone() else {
        return;
    };
    let row = 30.0;
    let height = suggest.matches.len() as f32 * row + space::SM * 2.0;
    let width = 260.0_f32.min(line.width());
    let rect = Rect::from_min_size(
        egui::pos2(line.min.x, line.min.y - space::SM - height),
        Vec2::new(width, height),
    );

    // Use an actual foreground UI whose max_rect is the visible popup.
    // Painting in an Area while interacting with absolute rects made the
    // visuals appear in one place and the hit targets live somewhere else.
    let ctx = ui.ctx().clone();
    let layer = egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("sugestoes-do-compositor"),
    );
    let mut overlay = ui.new_child(
        UiBuilder::new()
            .layer_id(layer)
            .max_rect(rect)
            .sense(Sense::click()),
    );
    pill_surface(&mut overlay, state, t, rect);

    let mut chosen = None;
    for (index, id) in suggest.matches.iter().enumerate() {
        let slot = Rect::from_min_size(
            egui::pos2(
                rect.min.x + space::SM,
                rect.min.y + space::SM + index as f32 * row,
            ),
            Vec2::new(rect.width() - space::SM * 2.0, row),
        );
        let response = overlay.interact(slot, Id::new(("sugestao", id)), Sense::click());
        if index == suggest.index || response.hovered() {
            overlay.painter().rect_filled(
                slot,
                CornerRadius::same(radius::CONTROL),
                if index == suggest.index {
                    t.accent.gamma_multiply(0.22)
                } else {
                    t.fill_soft
                },
            );
        }
        if response.hovered() {
            overlay.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let art = Rect::from_center_size(
            egui::pos2(slot.min.x + space::SM + 10.0, slot.center().y),
            Vec2::splat(20.0),
        );
        match suggest.kind {
            SuggestKind::Sticker => {
                let Some(emoji) = store.emojis.iter().find(|emoji| &emoji.id == id) else {
                    continue;
                };
                let texture = state
                    .media
                    .emoji(&emoji.id, emoji.blob.as_deref())
                    .and_then(|texture| texture.frame(&ctx))
                    .map(|handle| handle.id());
                if let Some(texture) = texture {
                    let mut mesh = egui::Mesh::with_texture(texture);
                    mesh.add_rect_with_uv(
                        art,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    overlay.painter().add(egui::Shape::mesh(mesh));
                }
                overlay.painter().text(
                    egui::pos2(art.max.x + space::MD, slot.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!(":{}:", emoji.name),
                    text::body(),
                    if index == suggest.index { t.label } else { t.label_secondary },
                );
            }
            SuggestKind::Mention => {
                let Some(member) = store.member(id) else {
                    continue;
                };
                let avatar = state
                    .media
                    .avatar(&member.id, store.avatars.get(&member.id).map(String::as_str))
                    .and_then(|texture| texture.frame(&ctx))
                    .map(|handle| handle.id());
                if let Some(texture) = avatar {
                    let mut mesh = egui::Mesh::with_texture(texture);
                    mesh.add_rect_with_uv(
                        art,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    overlay.painter().add(egui::Shape::mesh(mesh));
                } else {
                    overlay.painter().circle_filled(
                        art.center(),
                        art.width() / 2.0,
                        t.accent.gamma_multiply(0.28),
                    );
                    overlay.painter().text(
                        art.center(),
                        egui::Align2::CENTER_CENTER,
                        member.initials(),
                        text::footnote(),
                        t.label,
                    );
                }
                let x = art.max.x + space::MD;
                overlay.painter().text(
                    egui::pos2(x, slot.center().y - 4.0),
                    egui::Align2::LEFT_CENTER,
                    &member.name,
                    text::body(),
                    if index == suggest.index { t.label } else { t.label_secondary },
                );
                overlay.painter().text(
                    egui::pos2(x, slot.center().y + 8.0),
                    egui::Align2::LEFT_CENTER,
                    format!("@{}", member.username),
                    text::footnote(),
                    t.label_tertiary,
                );
                overlay.painter().circle_filled(
                    egui::pos2(art.max.x - 2.0, art.max.y - 2.0),
                    3.0,
                    presence_color(t, member.presence),
                );
            }
        }
        if response.clicked() {
            chosen = Some(index);
        }
    }

    if let Some(index) = chosen {
        if let Some(suggest) = state.suggest.as_mut() {
            suggest.index = index;
        }
        let edit_id = Id::new("caixa-de-mensagem");
        accept_suggestion(store, state, &ctx, edit_id);
        // Desktop TextEdit lost focus to the popup; Android's native editor
        // keeps its own focus and ignores this egui request harmlessly.
        ctx.memory_mut(|memory| memory.request_focus(edit_id));
    }
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



#[cfg(test)]
mod sugestao {
    use super::{typing_mention, typing_shortcode};

    #[test]
    fn arroba_sozinho_abre_lista_de_pessoas() {
        assert_eq!(typing_mention("@", 1), Some((0, String::new())));
    }

    #[test]
    fn arroba_filtra_username() {
        let text = "fala @chr";
        assert_eq!(
            typing_mention(text, text.chars().count()),
            Some((5, "chr".to_owned()))
        );
    }

    #[test]
    fn email_nao_e_mencao() {
        let text = "ana@exemplo";
        assert_eq!(typing_mention(text, text.chars().count()), None);
    }

    #[test]
    fn acha_o_apelido_sendo_digitado() {
        let text = "olha essa :gat";
        assert_eq!(
            typing_shortcode(text, text.chars().count()),
            Some((10, "gat".to_owned()))
        );
    }

    /// Dois pontos sozinho não sugere nada: quem escreveu `:` ainda não
    /// disse o que procura, e a lista inteira não é sugestão.
    #[test]
    fn dois_pontos_sozinho_nao_sugere() {
        assert_eq!(typing_shortcode("olha :", 6), None);
    }

    /// Depois do segundo `:` o apelido já está fechado.
    #[test]
    fn apelido_fechado_nao_sugere() {
        let text = "olha :gato:";
        assert_eq!(typing_shortcode(text, text.chars().count()), None);
    }

    /// O espaço corta: `um dois` não é apelido de nada.
    #[test]
    fn espaco_encerra_a_busca() {
        let text = "olha :gato preto";
        assert_eq!(typing_shortcode(text, text.chars().count()), None);
    }

    /// O cursor no meio do texto sugere pelo que está à esquerda dele.
    #[test]
    fn cursor_no_meio_olha_para_tras() {
        let text = ":ga do resto";
        assert_eq!(typing_shortcode(text, 3), Some((0, "ga".to_owned())));
    }

    /// Acento não quebra a contagem: o índice é em caracteres.
    #[test]
    fn acento_nao_desloca_o_indice() {
        let text = "ação :co";
        assert_eq!(
            typing_shortcode(text, text.chars().count()),
            Some((5, "co".to_owned()))
        );
    }

    #[test]
    fn aceitar_mencao_substitui_fragmento_parcial() {
        assert_eq!(
            super::replace_char_range("@chr", 0, 4, "@chris "),
            Some("@chris ".to_owned())
        );
        assert_eq!(
            super::replace_char_range("oi @chr tudo", 3, 7, "@chris "),
            Some("oi @chris  tudo".to_owned())
        );
    }

    #[test]
    fn aceitar_shortcode_substitui_fragmento_parcial() {
        assert_eq!(
            super::replace_char_range("usa :gat agora", 4, 8, ":gato:"),
            Some("usa :gato: agora".to_owned())
        );
    }
}


#[cfg(test)]
mod mobile_tests {
    use super::*;

    #[test]
    fn arrasto_vertical_e_da_rolagem_nao_da_gaveta() {
        // Mais vertical que horizontal: o movimento não é nosso, mesmo
        // sendo longo. Sem isto, rolar a conversa abriria a gaveta.
        assert_eq!(
            decide_intent(Vec2::new(40.0, 70.0), MobileSurface::Chat, &None),
            Intent::Scroll
        );
        // Horizontal para a direita, na conversa: abre a navegação.
        assert_eq!(
            decide_intent(Vec2::new(40.0, 5.0), MobileSurface::Chat, &None),
            Intent::Drawer(MobileSurface::Navigation)
        );
        // Para a esquerda em cima de uma mensagem: responder. Fora dela,
        // não há o que puxar.
        assert_eq!(
            decide_intent(Vec2::new(-40.0, 5.0), MobileSurface::Chat, &Some("m-1".into())),
            Intent::Reply("m-1".into())
        );
        assert_eq!(
            decide_intent(Vec2::new(-40.0, 5.0), MobileSurface::Chat, &None),
            Intent::Scroll
        );
        // Com a gaveta aberta, o caminho de volta é o inverso de cada uma.
        assert_eq!(
            decide_intent(Vec2::new(-40.0, 5.0), MobileSurface::Navigation, &None),
            Intent::Drawer(MobileSurface::Navigation)
        );
        assert_eq!(
            decide_intent(Vec2::new(40.0, 5.0), MobileSurface::People, &None),
            Intent::Drawer(MobileSurface::People)
        );
    }

    #[test]
    fn o_piparote_decide_mesmo_com_a_gaveta_quase_fechada() {
        // Mal saiu do lugar, mas saiu voando: abre.
        assert!(commits(0.05, FLICK_SPEED + 1.0));
        // Quase toda à mostra e jogada de volta: fecha.
        assert!(!commits(0.95, -FLICK_SPEED - 1.0));
        // Sem piparote, vale o quanto dela está à mostra.
        assert!(!commits(DRAWER_COMMIT - 0.01, 0.0));
        assert!(commits(DRAWER_COMMIT, 0.0));
    }

    #[test]
    fn o_elastico_segura_a_mensagem_perto_do_dedo() {
        // Até o limite a mensagem anda colada ao dedo.
        assert_eq!(rubber_band(0.0, 72.0), 0.0);
        assert_eq!(rubber_band(72.0, 72.0), 72.0);
        // Puxar para o outro lado não a move.
        assert_eq!(rubber_band(-30.0, 72.0), 0.0);
        // Depois dele ela resiste, e por mais que se puxe nunca chega ao
        // dobro — é o que a impede de sair sozinha pela borda.
        let longe = rubber_band(1000.0, 72.0);
        assert!(longe > 72.0 && longe < 144.0, "andou {longe}");
        assert!(rubber_band(144.0, 72.0) < 120.0);
    }

    #[test]
    fn breakpoint_preserva_layout_largo() {
        assert!(is_compact(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(COMPACT_BREAKPOINT - 1.0, 700.0),
        )));
        assert!(!is_compact(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(COMPACT_BREAKPOINT, 700.0),
        )));
    }
}


#[cfg(test)]
mod draft_tests {
    use super::*;

    fn state(text: &str, reply: Option<&str>, notify: bool) -> DraftState {
        DraftState {
            text: text.to_owned(),
            mentions: Vec::new(),
            reply_to: reply.map(str::to_owned),
            notify_reply: notify,
        }
    }

    #[test]
    fn canais_guardam_e_restauram_rascunhos_independentes() {
        let mut ui = UiState::default();
        ui.drafts.reset_owner("owner");
        ui.last_channel = "A".to_owned();
        ui.composer = "alpha".to_owned();
        ui.replying = Some("m-a".to_owned());
        ui.reply_notify = false;

        ui.switch_draft_channel("B");
        assert!(ui.composer.is_empty());
        assert!(ui.replying.is_none());
        assert_eq!(ui.reply_notify, ui.reply_notify_default);

        ui.composer = "beta".to_owned();
        ui.switch_draft_channel("A");
        assert_eq!(ui.composer, "alpha");
        assert_eq!(ui.replying.as_deref(), Some("m-a"));
        assert!(!ui.reply_notify);

        ui.switch_draft_channel("B");
        assert_eq!(ui.composer, "beta");
    }

    #[test]
    fn mencao_exata_sobrevive_round_trip_e_binding_ruim_e_descartado() {
        let cached = papo_core::cache::CachedDraft {
            owner_user_id: "owner".to_owned(),
            channel_id: "general".to_owned(),
            text: "oi @Alex".to_owned(),
            mentions: vec![
                papo_core::cache::CachedMentionBinding {
                    start: 3,
                    label: "Alex".to_owned(),
                    user_id: "id-exato".to_owned(),
                },
                papo_core::cache::CachedMentionBinding {
                    start: 1,
                    label: "Alex".to_owned(),
                    user_id: "id-errado".to_owned(),
                },
            ],
            reply_to: None,
            notify_reply: true,
            updated_at: 1,
        };
        let restored = DraftState::from_cached(cached);
        assert_eq!(restored.mentions.len(), 1);
        assert_eq!(restored.mentions[0].user_id, "id-exato");
        assert_eq!(restored.mentions[0].start, 3);
    }

    #[test]
    fn coalescencia_grava_apenas_o_ultimo_valor_do_canal() {
        let mut book = DraftBook::default();
        book.reset_owner("owner");
        book.capture("general", state("h", None, true));
        book.capture("general", state("he", None, true));
        book.capture("general", state("hello", None, true));

        let ops = book.take_persistence_ops("owner", true);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            papo_core::cache::CacheOp::UpsertDraft(draft) => {
                assert_eq!(draft.text, "hello");
                assert_eq!(draft.channel_id, "general");
            }
            _ => panic!("esperava upsert de draft"),
        }
    }

    #[test]
    fn limpar_um_canal_nao_apaga_o_outro_e_owner_reset_isola_contas() {
        let mut book = DraftBook::default();
        book.reset_owner("owner-a");
        book.capture("A", state("um", None, true));
        book.capture("B", state("dois", Some("m2"), false));
        book.capture("A", DraftState::default());

        assert!(book.get("A").is_none());
        assert_eq!(book.get("B").unwrap().text, "dois");

        book.reset_owner("owner-b");
        assert!(book.get("B").is_none());
        assert_eq!(book.owner(), Some("owner-b"));
    }
}

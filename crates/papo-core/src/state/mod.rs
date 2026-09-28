//! Estado da aplicação, alimentado pelas respostas REST e pelos eventos do
//! WebSocket.

pub mod call;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Local, Utc};
use crate::api::models::{self, parse_hex_color, Attachment};
use crate::api::net::{RefreshTicket, Update};
use crate::api::ws::{Connection, Event};
use crate::cache::{
    now_millis, CachedChannel, CachedMember, CachedMessage, CachedMessagePage, CachedOutgoing,
    CachedServer, CachedServerMetadata, CachedServerSnapshot, CacheOp, OutgoingState,
};

pub use call::{CallState, Phase, Stage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
    Category,
}

impl ChannelKind {
    fn parse(kind: &str) -> Self {
        match kind {
            "voice" => Self::Voice,
            "category" => Self::Category,
            _ => Self::Text,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub topic: Option<String>,
    pub position: i32,
    /// Overrides por cargo. Vetor vazio significa canal sem restrição
    /// específica no backend.
    pub permissions: Vec<models::ChannelPermissionEntry>,
    /// Preferência deste usuário neste canal: off, only_mentions ou all.
    pub notification_settings: String,
    /// Categoria à qual o canal pertence, quando o servidor informa. Sem
    /// ela, a categoria é dona dos canais que vêm depois dela na ordem.
    pub parent_id: Option<String>,
    /// Chegou coisa nova desde a última vez que o canal foi visto.
    pub unread: bool,
    /// Quantas dessas citam você.
    pub mentions: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Online,
    Away,
    Busy,
    Offline,
}

impl Presence {
    fn parse(status: &str) -> Self {
        match status {
            "online" => Self::Online,
            "away" => Self::Away,
            "busy" => Self::Busy,
            _ => Self::Offline,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Member {
    pub id: String,
    /// Nome de conta estável; serve para ligar resultados de busca, que hoje
    /// chegam do backend com `author_username`, ao membro carregado.
    pub username: String,
    pub name: String,
    pub presence: Presence,
    pub status_message: Option<String>,
    pub typing_label: Option<String>,
    pub role_color: Option<[u8; 3]>,
    /// Ids dos cargos que a pessoa tem; é o que a tela de cargos marca.
    pub roles: Vec<String>,
}

/// O que o cartão de perfil mostra além do que a lista de pessoas já tem.
/// Chega com o mesmo lote de perfis que traz as fotos.
#[derive(Debug, Clone, Default)]
pub struct ProfileDetails {
    pub description: Option<String>,
    /// sha256 do banner na mídia endereçada por conteúdo.
    pub banner: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    /// Cargos com nome e cor, do mais alto para o mais baixo.
    pub roles: Vec<models::RoleSummary>,
}

/// O que a pessoa está fazendo agora: ouvindo, jogando, trabalhando.
///
/// Hoje só a demonstração preenche isto. O backend ainda não transporta
/// atividade; quando transportar, o evento alimenta este mesmo modelo e o
/// cartão não muda.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub kind: ActivityKind,
    /// Aplicativo ou jogo.
    pub name: String,
    /// Primeira linha: faixa, fase, arquivo.
    pub details: Option<String>,
    /// Segunda linha: artista, modo, projeto.
    pub state: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    /// Com início e fim, o cartão desenha a barra de progresso.
    pub ends_at: Option<DateTime<Utc>>,
    /// Arte quadrada da atividade: caminho local (a atividade manual deste
    /// aparelho). Sem ela o cartão desenha o ícone do tipo.
    pub image: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Listening,
    Playing,
    Working,
}

impl Member {
    pub fn initials(&self) -> String {
        self.name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionBinding {
    /// Índice em caracteres do @ visível no editor.
    pub start: usize,
    /// Nickname/display name visível, sem o @.
    pub label: String,
    /// Identidade persistida no wire.
    pub user_id: String,
}

/// Um emoji de reação: do teclado ou do próprio servidor.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Emoji {
    Unicode(String),
    Custom(String),
}

impl Emoji {
    pub fn from_parts(unicode: Option<String>, custom: Option<String>) -> Option<Self> {
        match (unicode, custom) {
            (Some(unicode), _) if !unicode.is_empty() => Some(Self::Unicode(unicode)),
            (_, Some(id)) if !id.is_empty() => Some(Self::Custom(id)),
            _ => None,
        }
    }

    pub fn request(&self) -> models::ReactionRequest {
        match self {
            Self::Unicode(emoji) => models::ReactionRequest::unicode(emoji.clone()),
            Self::Custom(id) => models::ReactionRequest::custom(id.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Reaction {
    pub emoji: Emoji,
    pub count: u32,
    pub mine: bool,
}

/// Emoji custom do servidor, com a imagem em base64 como ela chega da API.
#[derive(Clone, Debug)]
pub struct CustomEmoji {
    pub id: String,
    pub name: String,
    pub blob: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub content: String,
    pub at: DateTime<Local>,
    pub edited: bool,
    pub reply_to: Option<String>,
    pub attachments: Vec<Attachment>,
    pub previews: Vec<models::LinkPreview>,
    pub reactions: Vec<Reaction>,
    pub pinned: bool,
    /// Mensagem ainda não confirmada pelo servidor.
    pub pending: bool,
}

impl Message {
    pub fn mine(&self, me: &str) -> bool {
        self.author_id == me
    }
}

#[derive(Clone, Debug)]
pub struct Server {
    pub name: String,
    pub description: Option<String>,
    /// Dono do servidor; o backend concede a ele capacidades administrativas.
    pub owner_id: Option<String>,
    /// Ícone em base64, como o servidor entrega. Não vai para o cache em
    /// disco: chega de novo na carga inicial.
    pub icon: Option<String>,
}

/// Aviso que o servidor manda e a interface traduz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// O backend viu um token de sessão antigo voltar e, por segurança,
    /// derrubou todas as conexões da conta.
    ConnectionViolation,
}

/// Em que ponto da jornada o usuário está.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Ainda verificando a sessão guardada.
    Starting,
    /// Sem sessão: pede login ou cadastro.
    Auth,
    /// Autenticado numa instância sem servidor criado.
    NeedsServer,
    Chat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineStatus {
    Missing,
    Stale,
    Refreshing,
    Fresh,
}

#[derive(Clone, Debug)]
struct ActiveRefresh {
    ticket: RefreshTicket,
    barrier_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshDiagnostics {
    pub generation: u64,
    pub request_id: u64,
    pub barrier_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineDiagnostics {
    pub channel_id: String,
    pub status: TimelineStatus,
    pub fresh_generation: Option<u64>,
    pub active_refresh: Option<RefreshDiagnostics>,
    pub journal_revision: u64,
    pub journal_entries: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreDiagnostics {
    pub sync_generation: u64,
    pub selected_channel: String,
    pub timelines: Vec<TimelineDiagnostics>,
    pub outgoing_queued: usize,
    pub outgoing_sending: usize,
    pub outgoing_unknown: usize,
    pub outgoing_failed: usize,
    pub outgoing_oldest_age_ms: Option<i64>,
}

#[derive(Clone, Debug)]
enum TimelineMutation {
    MessageUpsert(Message),
    MessageEdit {
        id: String,
        content: String,
    },
    MessageDelete {
        id: String,
    },
    MessagePinned {
        message_id: String,
        pinned: bool,
    },
    PreviewRemoved {
        message_id: String,
        preview_id: String,
    },
    PreviewUpsert {
        message_id: String,
        preview: models::LinkPreview,
    },
    AttachmentModeration {
        message_id: String,
        attachment_id: String,
        status: String,
    },
    Reaction {
        message_id: String,
        emoji: Emoji,
        count: i64,
    },
    LocalReaction {
        message_id: String,
        emoji: Emoji,
        add: bool,
    },
    MessagePending {
        id: String,
        pending: bool,
    },
}

impl TimelineMutation {
    /// A mensagem que esta mutação toca, quando há uma.
    fn affected_message_id(&self) -> Option<&str> {
        match self {
            TimelineMutation::MessageUpsert(message) => Some(&message.id),
            TimelineMutation::MessageEdit { id, .. } => Some(id),
            TimelineMutation::MessageDelete { id } => Some(id),
            TimelineMutation::MessagePinned { message_id, .. } => Some(message_id),
            TimelineMutation::Reaction { message_id, .. } => Some(message_id),
            TimelineMutation::LocalReaction { message_id, .. } => Some(message_id),
            TimelineMutation::AttachmentModeration { message_id, .. } => Some(message_id),
            TimelineMutation::MessagePending { id, .. } => Some(id),
            TimelineMutation::PreviewRemoved { message_id, .. }
            | TimelineMutation::PreviewUpsert { message_id, .. } => Some(message_id),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationSource {
    CacheRestore,
    Reconcile,
    Live,
    Local,
}

#[derive(Clone, Debug)]
enum StoreMutation {
    Timeline {
        channel_id: Option<String>,
        mutation: TimelineMutation,
    },
    ReplaceChannelSnapshot {
        channel_id: String,
        messages: Vec<Message>,
        /// false means the backend returned only the newest window.
        complete: bool,
    },
    ConfirmSent {
        local_id: String,
        message: Message,
    },
}

#[derive(Clone, Debug)]
struct JournalEntry {
    revision: u64,
    mutation: TimelineMutation,
}

#[derive(Debug, Default)]
struct ChannelMutationJournal {
    revision: u64,
    entries: VecDeque<JournalEntry>,
}

#[derive(Debug)]
pub struct Store {
    pub screen: Screen,
    pub connection: Connection,
    pub server: Option<Server>,
    pub channels: Vec<Channel>,
    pub members: Vec<Member>,
    pub messages: Vec<Message>,
    pub emojis: Vec<CustomEmoji>,
    pub me: String,
    pub my_name: String,
    /// Nome de usuário (sem apelido): é o que aparece numa menção.
    pub my_username: String,
    /// Preferências portáteis da conta deste servidor.
    pub user_settings: Option<models::UserSettings>,
    pub selected_channel: String,
    /// Geração da continuidade atual do WebSocket.
    sync_generation: u64,
    /// Última geração em que cada canal recebeu uma carga REST autoritativa.
    channel_freshness: HashMap<String, u64>,
    /// Canais que possuem estado de timeline persistido no Turso. Um snapshot
    /// vazio também entra aqui: cacheado e "nunca persistido" são diferentes.
    cached_channels: HashSet<String>,
    /// Canais cuja primeira página local já foi projetada nesta Store.
    hydrated_channels: HashSet<String>,
    /// Se ainda há páginas normais mais antigas no Turso, por canal.
    cache_history_has_more: HashMap<String, bool>,
    /// Leituras locais em voo; separadas do refresh REST/freshness.
    cache_loading: HashSet<String>,
    /// Invalida respostas Turso assíncronas quando o contexto de cache muda
    /// (principalmente após detectar que o cache pertence a outra conta).
    cache_restore_epoch: u64,
    /// Efeitos de cache pendentes, drenados pelo coordenador de persistência.
    pending_cache: Vec<CacheOp>,
    /// Estado da fila local, indexado pelo id cliente. A linha confirmada do
    /// servidor nunca entra aqui.
    outgoing_states: HashMap<String, OutgoingState>,
    /// Refresh aceito atualmente por canal, incluindo o barrier local.
    loading_channels: HashMap<String, ActiveRefresh>,
    /// Se ainda há páginas mais antigas no backend, por canal.
    history_has_more: HashMap<String, bool>,
    /// Evita disparar duas páginas antigas do mesmo canal ao mesmo tempo.
    history_loading: HashSet<String>,
    /// Mutações live recebidas durante refreshes REST.
    mutation_journals: HashMap<String, ChannelMutationJournal>,
    next_refresh_request_id: u64,
    /// Quem está digitando, por canal.
    typing: HashMap<String, HashSet<String>>,
    /// Até quando cada canal foi visto; é o que define o não lido, já que o
    /// backend registra `last_read_message` mas nunca o escreve.
    pub read_marks: HashMap<String, DateTime<Utc>>,
    /// Notificações já contadas, para não somar a mesma menção duas vezes.
    counted_notifications: HashSet<String>,
    /// Notificações por canal ainda não confirmadas no servidor.
    open_notifications: HashMap<String, Vec<String>>,
    pub error: Option<String>,
    pub busy: bool,
    /// O servidor é fechado e ainda espera a senha do servidor. A tela de
    /// entrada só mostra esse campo quando o servidor de fato pede.
    pub locked: bool,
    /// Aviso do servidor que não é um erro de formulário — hoje, o reuso de
    /// token que derrubou as outras sessões.
    pub notice: Option<Notice>,
    /// Cargos do servidor, com as permissões de cada um.
    pub roles: Vec<models::Role>,
    /// Foto de perfil por pessoa, em base64, como o servidor a entrega.
    pub avatars: HashMap<String, String>,
    /// Descrição, banner, cargos e data de entrada, por pessoa.
    pub profiles: HashMap<String, ProfileDetails>,
    /// Atividade por pessoa (só a demonstração preenche, por enquanto).
    pub activities: HashMap<String, Activity>,
    /// Sessões abertas da conta neste servidor.
    pub devices: Vec<models::ConnectionInfo>,
    pub audit_logs: Vec<models::AuditLogEntry>,
    /// O servidor tem mais registro além do carregado ("carregar mais").
    pub audit_has_more: bool,
    /// Última resposta da busca, do servidor na tela.
    pub search_results: Vec<models::SearchResult>,
    pub search_has_more: bool,
    /// Uma busca saiu e ainda não voltou.
    pub searching: bool,
    /// Detalhes de reação carregados sob demanda, por mensagem.
    pub reaction_details: HashMap<String, Vec<models::ReactionGroup>>,
    pub reaction_details_has_more: HashMap<String, bool>,
    pub reaction_details_loading: HashSet<String>,
    /// A call: quem está em cada canal de voz e onde ela aparece na tela.
    pub call: CallState,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            screen: Screen::Starting,
            connection: Connection::Offline,
            server: None,
            channels: Vec::new(),
            members: Vec::new(),
            messages: Vec::new(),
            emojis: Vec::new(),
            me: String::new(),
            my_name: String::new(),
            my_username: String::new(),
            user_settings: None,
            selected_channel: String::new(),
            sync_generation: 0,
            channel_freshness: HashMap::new(),
            cached_channels: HashSet::new(),
            hydrated_channels: HashSet::new(),
            cache_history_has_more: HashMap::new(),
            cache_loading: HashSet::new(),
            cache_restore_epoch: 0,
            pending_cache: Vec::new(),
            outgoing_states: HashMap::new(),
            loading_channels: HashMap::new(),
            history_has_more: HashMap::new(),
            history_loading: HashSet::new(),
            mutation_journals: HashMap::new(),
            next_refresh_request_id: 0,
            typing: HashMap::new(),
            read_marks: HashMap::new(),
            counted_notifications: HashSet::new(),
            open_notifications: HashMap::new(),
            error: None,
            busy: false,
            locked: false,
            notice: None,
            roles: Vec::new(),
            avatars: HashMap::new(),
            profiles: HashMap::new(),
            activities: HashMap::new(),
            devices: Vec::new(),
            audit_logs: Vec::new(),
            audit_has_more: false,
            search_results: Vec::new(),
            search_has_more: false,
            searching: false,
            reaction_details: HashMap::new(),
            reaction_details_has_more: HashMap::new(),
            reaction_details_loading: HashSet::new(),
            call: CallState::default(),
        }
    }
}

fn mention_boundary(c: char) -> bool {
    c.is_whitespace() || (!c.is_alphanumeric() && c != '_')
}


impl Store {
    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.channels.iter().find(|channel| channel.id == id)
    }

    /// Canais na ordem de exibição, cada um com a sua categoria.
    ///
    /// A ordem é hierárquica: os de fora de categoria e as categorias pela
    /// posição, e logo depois de cada categoria os canais dela (também pela
    /// posição, entre si). Assim mover uma categoria leva os canais junto na
    /// tela, mesmo que as posições deles não tenham mudado.
    ///
    /// Com `parent_id` vindo do servidor, vale ele. Sem nenhum `parent_id`
    /// (servidor sem categoria de verdade), a categoria é dona dos canais
    /// que vêm depois dela até a próxima categoria.
    pub fn channel_layout(&self) -> Vec<(usize, Option<String>)> {
        let mut order: Vec<usize> = (0..self.channels.len()).collect();
        order.sort_by_key(|&index| self.channels[index].position);
        let explicit = self.channels.iter().any(|channel| channel.parent_id.is_some());
        let is_category = |id: &str| self.channel(id).is_some_and(|c| c.kind == ChannelKind::Category);

        // A categoria de cada canal.
        let mut current: Option<String> = None;
        let parents: Vec<(usize, Option<String>)> = order
            .iter()
            .map(|&index| {
                let channel = &self.channels[index];
                if channel.kind == ChannelKind::Category {
                    current = Some(channel.id.clone());
                    return (index, None);
                }
                let parent = if explicit {
                    channel.parent_id.clone().filter(|parent| is_category(parent))
                } else {
                    current.clone()
                };
                (index, parent)
            })
            .collect();

        // Raízes pela posição; cada categoria seguida dos filhos.
        let mut layout = Vec::with_capacity(parents.len());
        for (index, parent) in &parents {
            if parent.is_some() {
                continue;
            }
            layout.push((*index, None));
            let channel = &self.channels[*index];
            if channel.kind == ChannelKind::Category {
                for (child, child_parent) in &parents {
                    if child_parent.as_deref() == Some(channel.id.as_str()) {
                        layout.push((*child, child_parent.clone()));
                    }
                }
            }
        }
        layout
    }

    /// Move um canal localmente (demonstração, e o eco otimista de um
    /// arrasto): posições contíguas de 1 em diante, como o servidor faz.
    /// `parent` `None` não mexe na categoria; `Some("")` tira dela.
    pub fn move_channel_local(&mut self, id: &str, new_position: i32, parent: Option<String>) {
        let mut order: Vec<usize> = (0..self.channels.len()).collect();
        order.sort_by_key(|&index| self.channels[index].position);
        let Some(from) = order.iter().position(|&index| self.channels[index].id == id) else {
            return;
        };
        let moved = order.remove(from);
        let to = ((new_position.max(1) - 1) as usize).min(order.len());
        order.insert(to, moved);
        for (position, index) in order.into_iter().enumerate() {
            self.channels[index].position = position as i32 + 1;
        }
        if let Some(parent) = parent
            && let Some(channel) = self.channels.iter_mut().find(|channel| channel.id == id)
        {
            channel.parent_id = (!parent.is_empty()).then_some(parent);
        }
        self.channels.sort_by_key(|channel| channel.position);
    }

    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.id == id)
    }

    pub fn member_by_username(&self, username: &str) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| member.username.eq_ignore_ascii_case(username))
    }

    /// Canonicaliza menções visíveis (`@nickname`) para `<@user_id>`.
    ///
    /// Bindings produzidos pelo autocomplete/edit têm prioridade e eliminam
    /// a ambiguidade de nicknames repetidos. Texto digitado manualmente só é
    /// resolvido quando aquele nickname/display name identifica uma única
    /// pessoa no servidor.
    pub fn encode_mentions(&self, text: &str, bindings: &[MentionBinding]) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut bindings_by_start: HashMap<usize, &MentionBinding> =
            bindings.iter().map(|binding| (binding.start, binding)).collect();

        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] != '@' {
                out.push(chars[i]);
                i += 1;
                continue;
            }

            // Não interpretar e-mail nem o @ interno de um token já canônico.
            if i > 0 && (!mention_boundary(chars[i - 1]) || chars[i - 1] == '<') {
                out.push('@');
                i += 1;
                continue;
            }

            if let Some(binding) = bindings_by_start.remove(&i) {
                let label: Vec<char> = binding.label.chars().collect();
                let end = i + 1 + label.len();
                if end <= chars.len()
                    && chars[i + 1..end]
                        .iter()
                        .copied()
                        .eq(label.iter().copied())
                {
                    out.push_str("<@");
                    out.push_str(&binding.user_id);
                    out.push('>');
                    i = end;
                    continue;
                }
            }

            // Entrada manual: chamados globais continuam especiais.
            let rest: String = chars[i + 1..].iter().collect();
            let global = ["everyone", "todos"].iter().any(|word| {
                rest.len() >= word.len()
                    && rest[..word.len()].eq_ignore_ascii_case(word)
                    && rest[word.len()..]
                        .chars()
                        .next()
                        .is_none_or(mention_boundary)
            });
            if global {
                out.push('@');
                i += 1;
                continue;
            }

            // Maior nome visível primeiro. Se duas pessoas compartilham
            // exatamente o mesmo nickname, não adivinhamos.
            let mut candidates: Vec<&Member> = self
                .members
                .iter()
                .filter(|member| {
                    rest.len() >= member.name.len()
                        && rest.is_char_boundary(member.name.len())
                        && rest[..member.name.len()].eq_ignore_ascii_case(&member.name)
                })
                .collect();
            candidates.sort_by_key(|member| std::cmp::Reverse(member.name.len()));

            if let Some(first) = candidates.first().copied() {
                let same_label = candidates
                    .iter()
                    .filter(|member| member.name.eq_ignore_ascii_case(&first.name))
                    .count();
                let end = i + 1 + first.name.chars().count();
                let boundary_ok = end >= chars.len() || mention_boundary(chars[end]);
                if same_label == 1 && boundary_ok {
                    out.push_str("<@");
                    out.push_str(&first.id);
                    out.push('>');
                    i = end;
                    continue;
                }
            }

            out.push('@');
            i += 1;
        }
        out
    }

    /// Resolve tokens estáveis para o nickname/display name atual.
    pub fn display_mentions(&self, text: &str) -> String {
        self.display_mentions_with_bindings(text).0
    }

    /// Versão usada pelo editor: além do texto humano, devolve a ligação de
    /// cada nickname ao user_id original para preservar duplicatas.
    pub fn display_mentions_with_bindings(&self, text: &str) -> (String, Vec<MentionBinding>) {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut bindings = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '<' && i + 3 < chars.len() && chars[i + 1] == '@' {
                let mut end = i + 2;
                while end < chars.len() && chars[end] != '>' {
                    end += 1;
                }
                if end < chars.len() {
                    let id: String = chars[i + 2..end].iter().collect();
                    if let Some(member) = self.member(&id) {
                        let start = out.chars().count();
                        out.push('@');
                        out.push_str(&member.name);
                        bindings.push(MentionBinding {
                            start,
                            label: member.name.clone(),
                            user_id: member.id.clone(),
                        });
                        i = end + 1;
                        continue;
                    }
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        (out, bindings)
    }

    pub fn begin_reaction_details(&mut self, message_id: &str) -> bool {
        self.reaction_details_loading.insert(message_id.to_owned())
    }

    pub fn reaction_details_cursor(
        &self,
        message_id: &str,
    ) -> Option<(DateTime<Utc>, String)> {
        self.reaction_details
            .get(message_id)?
            .iter()
            .flat_map(|group| group.users.iter())
            .min_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)))
            .map(|user| (user.created_at, user.id.clone()))
    }

    pub fn message(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|message| message.id == id)
    }

    pub fn messages_in<'a>(&'a self, channel_id: &'a str) -> impl Iterator<Item = &'a Message> {
        self.messages
            .iter()
            .filter(move |message| message.channel_id == channel_id)
    }

    pub fn members_by_presence(&self, presence: Presence) -> impl Iterator<Item = &Member> {
        self.members
            .iter()
            .filter(move |member| member.presence == presence)
    }

    /// Nomes de quem está digitando no canal aberto.
    pub fn typing_names(&self) -> Vec<String> {
        self.typing
            .get(&self.selected_channel)
            .map(|users| {
                users
                    .iter()
                    .filter(|id| **id != self.me)
                    .filter_map(|id| self.member(id).map(|member| member.name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A frase de digitação de quem está sozinho digitando, se a pessoa
    /// escolheu uma. Com duas ou mais, cada frase no seu jeito vira ruído: o
    /// texto padrão serve para o grupo.
    pub fn typing_phrase(&self) -> Option<&str> {
        let users = self.typing.get(&self.selected_channel)?;
        let mut others = users.iter().filter(|id| **id != self.me);
        let only = others.next()?;
        if others.next().is_some() {
            return None;
        }
        self.member(only)?
            .typing_label
            .as_deref()
            .map(str::trim)
            .filter(|phrase| !phrase.is_empty())
    }

    pub fn sync_generation(&self) -> u64 {
        self.sync_generation
    }

    pub fn timeline_status(&self, channel_id: &str) -> TimelineStatus {
        if self.loading_channels.contains_key(channel_id) {
            return TimelineStatus::Refreshing;
        }
        match self.channel_freshness.get(channel_id) {
            Some(generation) if *generation == self.sync_generation => TimelineStatus::Fresh,
            Some(_) => TimelineStatus::Stale,
            // Cache restaurado nunca é frescura: só a reconciliação desta
            // geração marca um canal como atual.
            None if self.cached_channels.contains(channel_id) => TimelineStatus::Stale,
            None => TimelineStatus::Missing,
        }
    }

    /// Hidrata somente metadados persistidos antes da rede começar. Timelines
    /// são carregadas sob demanda e nunca tornam um canal Fresh.
    pub fn restore_cached_metadata(&mut self, metadata: CachedServerMetadata) {
        self.cache_restore_epoch = self.cache_restore_epoch.wrapping_add(1);
        if let Some(server) = &metadata.server {
            self.me = server.me_user_id.clone().unwrap_or_default();
            self.my_name = server.me_display_name.clone().unwrap_or_default();
            self.my_username = server.me_username.clone().unwrap_or_default();
            self.server = Some(Server {
                name: server.name.clone(),
                description: server.description.clone(),
                owner_id: metadata.owner_user_id.clone(),
                icon: None,
            });
        }

        self.channels = metadata
            .channels
            .into_iter()
            .map(|channel| Channel {
                id: channel.id,
                name: channel.name,
                kind: ChannelKind::parse(&channel.kind),
                topic: channel.topic,
                position: channel.position,
                permissions: Vec::new(),
                notification_settings: "only_mentions".to_owned(),
                parent_id: channel.parent_id,
                unread: channel.unread,
                mentions: channel.mentions,
            })
            .collect();
        self.channels.sort_by_key(|channel| channel.position);

        // Presença lida do disco é sempre velha; ninguém é "online" só por
        // causa dela.
        self.members = metadata
            .members
            .into_iter()
            .map(|member| Member {
                id: member.id,
                username: member.username,
                name: member.name,
                presence: Presence::Offline,
                status_message: None,
                typing_label: None,
                role_color: member.role_color,
                roles: member.roles,
            })
            .collect();

        self.messages.clear();
        self.cached_channels = metadata.cached_channels;
        self.hydrated_channels.clear();
        self.cache_history_has_more.clear();
        self.cache_loading.clear();

        if self.selected_channel.is_empty()
            && let Some(first) = self
                .channels
                .iter()
                .find(|channel| channel.kind == ChannelKind::Text)
        {
            self.selected_channel = first.id.clone();
        }

        if !self.channels.is_empty() || self.server.is_some() {
            self.screen = Screen::Chat;
            self.connection = Connection::Offline;
        }

        self.pending_cache.clear();
    }

    /// Compatibilidade interna/testes: restaura o snapshot integral antigo.
    /// O runtime interativo usa metadados + páginas por canal.
    pub fn restore_cached(&mut self, snapshot: CachedServerSnapshot) {
        let CachedServerSnapshot {
            owner_user_id,
            server,
            channels,
            members,
            messages,
            cached_channels,
        } = snapshot;
        let hydrated = cached_channels.clone();
        self.restore_cached_metadata(CachedServerMetadata {
            owner_user_id,
            server,
            channels,
            members,
            cached_channels,
        });
        for message in messages {
            self.apply_mutation(
                MutationSource::CacheRestore,
                StoreMutation::Timeline {
                    channel_id: Some(message.channel_id.clone()),
                    mutation: TimelineMutation::MessageUpsert(message.to_store()),
                },
            );
        }
        self.sort_messages();
        self.hydrated_channels = hydrated;
    }
    /// Esvazia o estado vindo do cache. Usado quando a conta verificada não é
    /// a dona do cache — nunca deixar a conversa de um usuário aparecer para
    /// outro.
    pub fn clear_cached_state(&mut self) {
        self.cache_restore_epoch = self.cache_restore_epoch.wrapping_add(1);
        self.server = None;
        self.channels.clear();
        self.members.clear();
        self.messages.clear();
        self.cached_channels.clear();
        self.hydrated_channels.clear();
        self.cache_history_has_more.clear();
        self.cache_loading.clear();
        self.history_has_more.clear();
        self.history_loading.clear();
        self.pending_cache.clear();
        self.outgoing_states.clear();
        self.selected_channel.clear();
    }

    /// Retira os efeitos de cache acumulados desde a última drenagem.
    pub fn take_cache_ops(&mut self) -> Vec<CacheOp> {
        std::mem::take(&mut self.pending_cache)
    }

    /// Projeção pequena e somente-leitura do estado de sincronização.
    /// Não contém mensagens, credenciais ou URLs privadas.
    pub fn diagnostics(&self) -> StoreDiagnostics {
        let mut ids = BTreeSet::new();
        ids.extend(self.channels.iter().map(|channel| channel.id.clone()));
        ids.extend(self.channel_freshness.keys().cloned());
        ids.extend(self.loading_channels.keys().cloned());
        ids.extend(self.mutation_journals.keys().cloned());
        if !self.selected_channel.is_empty() {
            ids.insert(self.selected_channel.clone());
        }

        let timelines = ids
            .into_iter()
            .map(|channel_id| {
                let active_refresh = self.loading_channels.get(&channel_id).map(|refresh| {
                    RefreshDiagnostics {
                        generation: refresh.ticket.generation,
                        request_id: refresh.ticket.request_id,
                        barrier_revision: refresh.barrier_revision,
                    }
                });
                let (journal_revision, journal_entries) = self
                    .mutation_journals
                    .get(&channel_id)
                    .map(|journal| (journal.revision, journal.entries.len()))
                    .unwrap_or_default();

                TimelineDiagnostics {
                    status: self.timeline_status(&channel_id),
                    fresh_generation: self.channel_freshness.get(&channel_id).copied(),
                    active_refresh,
                    journal_revision,
                    journal_entries,
                    channel_id,
                }
            })
            .collect();

        let outgoing_queued = self
            .outgoing_states
            .values()
            .filter(|state| **state == OutgoingState::Queued)
            .count();
        let outgoing_sending = self
            .outgoing_states
            .values()
            .filter(|state| **state == OutgoingState::Sending)
            .count();
        let outgoing_unknown = self
            .outgoing_states
            .values()
            .filter(|state| **state == OutgoingState::UnknownOutcome)
            .count();
        let outgoing_failed = self
            .outgoing_states
            .values()
            .filter(|state| **state == OutgoingState::FailedPermanent)
            .count();
        let oldest = self
            .messages
            .iter()
            .filter(|message| message.pending)
            .map(|message| message.at.with_timezone(&Utc).timestamp_millis())
            .min();

        StoreDiagnostics {
            sync_generation: self.sync_generation,
            selected_channel: self.selected_channel.clone(),
            timelines,
            outgoing_queued,
            outgoing_sending,
            outgoing_unknown,
            outgoing_failed,
            outgoing_oldest_age_ms: oldest
                .map(|created| now_millis().saturating_sub(created).max(0)),
        }
    }

    /// Canal selecionado cuja primeira página persistida ainda não entrou na
    /// Store. Funciona offline: cache não depende da conexão.
    pub fn cache_restore_epoch(&self) -> u64 {
        self.cache_restore_epoch
    }

    pub fn channel_needing_cache(&self) -> Option<String> {
        let id = &self.selected_channel;
        if id.is_empty()
            || !self.cached_channels.contains(id)
            || self.hydrated_channels.contains(id)
            || self.cache_loading.contains(id)
            || self.channel(id).is_some_and(|channel| channel.kind == ChannelKind::Voice)
        {
            return None;
        }
        Some(id.clone())
    }

    pub fn mark_cache_loading(&mut self, channel_id: &str) {
        self.cache_loading.insert(channel_id.to_owned());
    }

    /// Projeta uma página local sem atribuir freshness.
    pub fn restore_cached_page(&mut self, page: CachedMessagePage, older: bool) {
        let channel_id = page.channel_id.clone();
        let fresh = self.timeline_status(&channel_id) == TimelineStatus::Fresh;
        let authoritative_complete = fresh
            && !self
                .history_has_more
                .get(&channel_id)
                .copied()
                .unwrap_or(false);
        let fresh_cutoff = if !older && fresh && !authoritative_complete {
            self.messages_in(&channel_id)
                .filter(|message| !message.pending)
                .min_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)))
                .map(|message| {
                    (
                        message.at.with_timezone(&Utc).timestamp_millis(),
                        message.id.clone(),
                    )
                })
        } else {
            None
        };

        if !authoritative_complete {
            for message in page.messages {
                let older_than_fresh_head = fresh_cutoff.as_ref().is_none_or(|(at, id)| {
                    message.created_at < *at
                        || (message.created_at == *at && message.id < *id)
                });
                if older || !fresh || older_than_fresh_head {
                    self.apply_mutation(
                        MutationSource::CacheRestore,
                        StoreMutation::Timeline {
                            channel_id: Some(channel_id.clone()),
                            mutation: TimelineMutation::MessageUpsert(message.to_store()),
                        },
                    );
                }
            }
            self.sort_messages();
        }
        self.cached_channels.insert(channel_id.clone());
        self.hydrated_channels.insert(channel_id.clone());
        self.cache_history_has_more
            .insert(channel_id.clone(), page.has_more && !authoritative_complete);
        self.cache_loading.remove(&channel_id);
        if older {
            self.history_loading.remove(&channel_id);
        }
    }

    /// Falha de leitura local não pode virar retry por frame.
    pub fn cached_page_failed(&mut self, channel_id: &str, older: bool) {
        self.cache_loading.remove(channel_id);
        if older {
            self.cache_history_has_more
                .insert(channel_id.to_owned(), false);
            self.history_loading.remove(channel_id);
        } else {
            self.hydrated_channels.insert(channel_id.to_owned());
        }
    }

    pub fn can_load_older(&self, channel_id: &str) -> bool {
        !self.history_loading.contains(channel_id)
            && (self
                .cache_history_has_more
                .get(channel_id)
                .copied()
                .unwrap_or(false)
                || (self.connection == Connection::Online
                    && self.history_has_more.get(channel_id).copied().unwrap_or(false)))
    }

    pub fn loading_older(&self, channel_id: &str) -> bool {
        self.history_loading.contains(channel_id)
    }

    /// Reserva primeiro uma página antiga do Turso.
    pub fn begin_load_cached_older(&mut self, channel_id: &str) -> Option<(i64, String)> {
        if self.history_loading.contains(channel_id)
            || !self
                .cache_history_has_more
                .get(channel_id)
                .copied()
                .unwrap_or(false)
        {
            return None;
        }
        let oldest = self
            .messages_in(channel_id)
            .filter(|message| !message.pending && !message.pinned)
            .min_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)))?;
        let cursor = (
            oldest.at.with_timezone(&Utc).timestamp_millis(),
            oldest.id.clone(),
        );
        self.history_loading.insert(channel_id.to_owned());
        Some(cursor)
    }

    /// Depois que o Turso esgotou, reserva a próxima página do backend.
    pub fn begin_load_older(
        &mut self,
        channel_id: &str,
    ) -> Option<(DateTime<Utc>, String)> {
        if self.history_loading.contains(channel_id)
            || self
                .cache_history_has_more
                .get(channel_id)
                .copied()
                .unwrap_or(false)
            || self.connection != Connection::Online
            || !self.history_has_more.get(channel_id).copied().unwrap_or(false)
        {
            return None;
        }
        let oldest = self
            .messages_in(channel_id)
            .filter(|message| !message.pending)
            .min_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)))?;
        let cursor = (oldest.at.with_timezone(&Utc), oldest.id.clone());
        self.history_loading.insert(channel_id.to_owned());
        Some(cursor)
    }

    /// Canal que ainda precisa ter as mensagens buscadas.
    pub fn channel_needing_messages(&self) -> Option<String> {
        if self.connection != Connection::Online {
            return None;
        }
        let id = &self.selected_channel;
        if id.is_empty()
            || matches!(self.timeline_status(id), TimelineStatus::Fresh | TimelineStatus::Refreshing)
        {
            return None;
        }
        if self.channel(id).is_some_and(|channel| channel.kind == ChannelKind::Voice) {
            return None;
        }
        Some(id.clone())
    }

    pub fn mark_loading(&mut self, channel_id: &str) -> RefreshTicket {
        self.next_refresh_request_id = self.next_refresh_request_id.saturating_add(1);
        let journal = self
            .mutation_journals
            .entry(channel_id.to_owned())
            .or_default();
        // Uma tentativa nova supersede a anterior. Tudo que chegou antes
        // deste ponto deve estar incluído no novo snapshot REST; só mutações
        // posteriores ao barrier precisam de replay.
        journal.entries.clear();
        let barrier_revision = journal.revision;
        let ticket = RefreshTicket {
            channel_id: channel_id.to_owned(),
            generation: self.sync_generation,
            request_id: self.next_refresh_request_id,
        };
        self.loading_channels.insert(
            channel_id.to_owned(),
            ActiveRefresh {
                ticket: ticket.clone(),
                barrier_revision,
            },
        );
        ticket
    }

    fn refresh_ticket_is_current(&self, ticket: &RefreshTicket) -> bool {
        ticket.generation == self.sync_generation
            && self
                .loading_channels
                .get(&ticket.channel_id)
                .is_some_and(|refresh| refresh.ticket == *ticket)
    }

    fn record_timeline_mutation(&mut self, channel_id: &str, mutation: TimelineMutation) {
        if !self.loading_channels.contains_key(channel_id) {
            return;
        }
        let journal = self
            .mutation_journals
            .entry(channel_id.to_owned())
            .or_default();
        journal.revision = journal.revision.saturating_add(1);
        journal.entries.push_back(JournalEntry {
            revision: journal.revision,
            mutation,
        });
    }

    fn channel_for_message(&self, message_id: &str) -> Option<String> {
        self.messages
            .iter()
            .find(|message| message.id == message_id)
            .map(|message| message.channel_id.clone())
            .or_else(|| {
                self.mutation_journals.iter().find_map(|(channel_id, journal)| {
                    journal.entries.iter().rev().find_map(|entry| {
                        if let TimelineMutation::MessageUpsert(message) = &entry.mutation
                            && message.id == message_id
                        {
                            Some(channel_id.clone())
                        } else {
                            None
                        }
                    })
                })
            })
    }

    fn replay_timeline_mutations(&mut self, channel_id: &str, barrier_revision: u64) {
        let mutations: Vec<TimelineMutation> = self
            .mutation_journals
            .get(channel_id)
            .map(|journal| {
                journal
                    .entries
                    .iter()
                    .filter(|entry| entry.revision > barrier_revision)
                    .map(|entry| entry.mutation.clone())
                    .collect()
            })
            .unwrap_or_default();
        for mutation in mutations {
            self.apply_mutation(
                MutationSource::Reconcile,
                StoreMutation::Timeline {
                    channel_id: Some(channel_id.to_owned()),
                    mutation,
                },
            );
        }
    }

    fn apply_mutation(&mut self, source: MutationSource, mutation: StoreMutation) {
        match mutation {
            StoreMutation::Timeline {
                channel_id,
                mutation,
            } => {
                let live_message = if source == MutationSource::Live {
                    match &mutation {
                        TimelineMutation::MessageUpsert(message) => Some((
                            message.clone(),
                            !self.message_is_known(&message.id),
                        )),
                        _ => None,
                    }
                } else {
                    None
                };
                let cache_message_id = mutation.affected_message_id().map(str::to_owned);
                let deleted = matches!(mutation, TimelineMutation::MessageDelete { .. });

                if source == MutationSource::Live
                    && let Some(channel_id) = channel_id.as_deref()
                {
                    self.record_timeline_mutation(channel_id, mutation.clone());
                }

                let project = !matches!(
                    (source, &mutation),
                    (
                        MutationSource::Live,
                        TimelineMutation::MessageUpsert(message)
                    ) if self.timeline_status(&message.channel_id) != TimelineStatus::Fresh
                );
                if project {
                    self.apply_timeline_projection(mutation);
                }

                // Só Live e Reconcile persistem; CacheRestore nunca volta ao
                // banco e Local (eco pendente) ainda não é persistido.
                if matches!(source, MutationSource::Live | MutationSource::Reconcile)
                    && let Some(message_id) = cache_message_id
                {
                    self.emit_message_cache_effect(&message_id, deleted);
                }

                if let Some((message, first_delivery)) = live_message {
                    self.apply_live_message_effects(&message, first_delivery);
                }
            }
            StoreMutation::ReplaceChannelSnapshot {
                channel_id,
                messages,
                complete,
            } => {
                let returned_ids: HashSet<String> =
                    messages.iter().map(|message| message.id.clone()).collect();
                let oldest_returned = messages
                    .iter()
                    .min_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)))
                    .map(|message| (message.at, message.id.clone()));

                // A primeira página só é autoridade para a janela que ela
                // cobre. Se há páginas anteriores, conserva o histórico local
                // abaixo do cursor e converge apenas o head.
                let mut deleted_ids = Vec::new();
                if complete {
                    deleted_ids.extend(
                        self.messages
                            .iter()
                            .filter(|message| {
                                message.channel_id == channel_id
                                    && !message.pending
                                    && !returned_ids.contains(&message.id)
                            })
                            .map(|message| message.id.clone()),
                    );
                    self.messages
                        .retain(|message| message.channel_id != channel_id || message.pending);
                } else if let Some((oldest_at, oldest_id)) = oldest_returned.as_ref() {
                    self.messages.retain(|message| {
                        if message.channel_id != channel_id || message.pending {
                            return true;
                        }
                        let inside_authoritative_head = message.at > *oldest_at
                            || (message.at == *oldest_at && message.id >= *oldest_id);
                        let stale = inside_authoritative_head
                            && !returned_ids.contains(&message.id);
                        if stale {
                            deleted_ids.push(message.id.clone());
                        }
                        !stale
                    });
                }

                for message in &messages {
                    self.upsert_message(message.clone());
                }
                self.sort_messages();

                if source == MutationSource::Reconcile {
                    if complete {
                        let confirmed: Vec<CachedMessage> = self
                            .messages
                            .iter()
                            .filter(|message| message.channel_id == channel_id && !message.pending)
                            .map(CachedMessage::from_store)
                            .collect();
                        self.pending_cache.push(CacheOp::ReplaceChannelSnapshot {
                            channel_id,
                            messages: confirmed,
                            cached_at: now_millis(),
                        });
                    } else {
                        self.pending_cache.push(CacheOp::MergeChannelHead {
                            channel_id,
                            messages: messages.iter().map(CachedMessage::from_store).collect(),
                            deleted_ids,
                            cached_at: now_millis(),
                        });
                    }
                }
            }
            StoreMutation::ConfirmSent { local_id, message } => {
                // Uma confirmação resolve só a intenção correspondente. A
                // persistência da mensagem+remoção da fila já foi feita
                // transacionalmente pelo ClientDb no worker de rede.
                self.messages.retain(|existing| existing.id != local_id);
                self.outgoing_states.remove(&local_id);
                self.apply_timeline_projection(TimelineMutation::MessageUpsert(message));
            }
        }
    }

    /// Publica o efeito de cache de uma mutação já projetada. Deletar remove a
    /// linha; qualquer outra mudança regrava a mensagem resultante, e não a
    /// intenção — assim edição, reação e fixação convergem sozinhas.
    fn emit_message_cache_effect(&mut self, message_id: &str, deleted: bool) {
        if deleted {
            self.pending_cache.push(CacheOp::DeleteMessage {
                message_id: message_id.to_owned(),
            });
            return;
        }
        if let Some(message) = self.messages.iter().find(|message| message.id == message_id)
            && !message.pending
        {
            self.pending_cache
                .push(CacheOp::UpsertMessage(CachedMessage::from_store(message)));
        }
    }

    fn message_is_known(&self, message_id: &str) -> bool {
        self.channel_for_message(message_id).is_some()
    }

    fn apply_live_message_effects(&mut self, message: &Message, first_delivery: bool) {
        if first_delivery {
            let mention = self.mentions_me(message);
            if message.channel_id != self.selected_channel
                && message.author_id != self.me
                && let Some(channel) = self
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == message.channel_id)
            {
                channel.unread = true;
                if mention {
                    channel.mentions += 1;
                }
            }
        }

        if let Some(users) = self.typing.get_mut(&message.channel_id) {
            users.remove(&message.author_id);
        }
    }

    fn apply_timeline_projection(&mut self, mutation: TimelineMutation) {
        match mutation {
            TimelineMutation::MessageUpsert(message) => {
                self.upsert_message(message);
                self.sort_messages();
            }
            TimelineMutation::MessageEdit { id, content } => {
                if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
                    message.content = content;
                    message.edited = true;
                }
            }
            TimelineMutation::MessageDelete { id } => {
                self.messages.retain(|message| message.id != id);
            }
            TimelineMutation::MessagePinned { message_id, pinned } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    message.pinned = pinned;
                }
            }
            TimelineMutation::PreviewRemoved {
                message_id,
                preview_id,
            } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    message.previews.retain(|preview| preview.id != preview_id);
                }
            }
            TimelineMutation::PreviewUpsert {
                message_id,
                preview,
            } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    match message
                        .previews
                        .iter_mut()
                        .find(|existing| existing.id == preview.id)
                    {
                        Some(existing) => *existing = preview,
                        None => message.previews.push(preview),
                    }
                }
            }
            TimelineMutation::AttachmentModeration {
                message_id,
                attachment_id,
                status,
            } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                    && let Some(attachment) = message
                        .attachments
                        .iter_mut()
                        .find(|attachment| attachment.id == attachment_id)
                {
                    attachment.moderation_status = Some(status);
                }
            }
            TimelineMutation::Reaction {
                message_id,
                emoji,
                count,
            } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    match message
                        .reactions
                        .iter_mut()
                        .find(|reaction| reaction.emoji == emoji)
                    {
                        Some(reaction) => reaction.count = count.max(0) as u32,
                        None if count > 0 => message.reactions.push(Reaction {
                            emoji,
                            count: count as u32,
                            mine: false,
                        }),
                        None => {}
                    }
                    message.reactions.retain(|reaction| reaction.count > 0);
                }
            }
            TimelineMutation::LocalReaction {
                message_id,
                emoji,
                add,
            } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    match message
                        .reactions
                        .iter_mut()
                        .find(|reaction| reaction.emoji == emoji)
                    {
                        Some(reaction) if add && !reaction.mine => {
                            reaction.mine = true;
                            reaction.count = reaction.count.saturating_add(1);
                        }
                        Some(reaction) if !add && reaction.mine => {
                            reaction.mine = false;
                            reaction.count = reaction.count.saturating_sub(1);
                        }
                        None if add => message.reactions.push(Reaction {
                            emoji,
                            count: 1,
                            mine: true,
                        }),
                        _ => {}
                    }
                    message.reactions.retain(|reaction| reaction.count > 0);
                }
            }
            TimelineMutation::MessagePending { id, pending } => {
                if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
                    message.pending = pending;
                }
            }
        }
    }

    fn upsert_message(&mut self, mut message: Message) {
        match self
            .messages
            .iter_mut()
            .find(|existing| existing.id == message.id)
        {
            Some(existing) => {
                // Pin é mantido por uma trilha própria (snapshot de pins /
                // evento MessagePinned), portanto um upsert de mensagem não
                // pode apagá-lo só porque o payload comum não o carrega.
                message.pinned = existing.pinned;
                *existing = message;
            }
            None => self.messages.push(message),
        }
    }

    // -- Não lidos ---------------------------------------------------------

    /// Menções somadas de todos os canais: é o número do badge.
    pub fn mention_total(&self) -> u32 {
        self.channels.iter().map(|channel| channel.mentions).sum()
    }

    pub fn has_unread(&self) -> bool {
        self.channels.iter().any(|channel| channel.unread)
    }

    /// O canal foi visto agora: zera o realce e guarda a marca.
    pub fn mark_read(&mut self, channel_id: &str) {
        let now = Utc::now();
        self.read_marks.insert(channel_id.to_owned(), now);
        if let Some(channel) = self
            .channels
            .iter_mut()
            .find(|channel| channel.id == channel_id)
        {
            channel.unread = false;
            channel.mentions = 0;
        }
    }

    /// Ids de notificação do canal para confirmar no servidor; some da lista
    /// ao ser entregue.
    pub fn take_open_notifications(&mut self, channel_id: &str) -> Vec<String> {
        self.open_notifications
            .remove(channel_id)
            .unwrap_or_default()
    }

    pub fn mark_all_read(&mut self) {
        let ids: Vec<String> = self.channels.iter().map(|c| c.id.clone()).collect();
        for id in ids {
            self.mark_read(&id);
        }
    }

    /// A mensagem cita você? Menção estável, legado por nickname/display name ou chamado geral.
    pub fn mentions_me(&self, message: &Message) -> bool {
        crate::notification::mentions_user(
            &message.content,
            &self.me,
            &self.my_name,
            &message.author_id,
        )
    }

    /// Permissões globais acumuladas dos cargos do usuário atual.
    pub fn my_role_permissions(&self) -> models::RolePermissions {
        let Some(me) = self.member(&self.me) else {
            return models::RolePermissions::default();
        };
        let mut combined = models::RolePermissions::default();
        for role in self
            .roles
            .iter()
            .filter(|role| me.roles.iter().any(|role_id| role_id == &role.id))
        {
            let p = role.permissions;
            combined.manage_server |= p.manage_server;
            combined.manage_channels |= p.manage_channels;
            combined.manage_roles |= p.manage_roles;
            combined.ban_members |= p.ban_members;
            combined.pin_message |= p.pin_message;
            combined.everyone_message |= p.everyone_message;
            combined.send_attachment |= p.send_attachment;
        }
        combined
    }

    pub fn is_server_owner(&self) -> bool {
        self.server
            .as_ref()
            .and_then(|server| server.owner_id.as_deref())
            .is_some_and(|owner| owner == self.me)
    }

    pub fn can_manage_server(&self) -> bool {
        self.is_server_owner() || self.my_role_permissions().manage_server
    }

    pub fn can_manage_channels(&self) -> bool {
        self.is_server_owner() || self.my_role_permissions().manage_channels
    }

    pub fn can_manage_roles(&self) -> bool {
        self.is_server_owner() || self.my_role_permissions().manage_roles
    }

    pub fn can_open_server_admin(&self) -> bool {
        self.can_manage_server() || self.can_manage_channels() || self.can_manage_roles()
    }

    // -- Atualizações ------------------------------------------------------

    /// Aplica uma atualização vinda da rede.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Session(Some(me)) => {
                self.me = me.id.clone();
                self.my_name = me.display_name().to_owned();
                self.my_username = me.username.clone();
                self.user_settings = Some(models::UserSettings {
                    user_id: me.id.clone(),
                    version: me.settings.version,
                    config: me.settings.config.clone().normalised(),
                    updated_at: None,
                });
                self.screen = Screen::Chat;
                self.error = None;
                self.busy = false;
                self.pending_cache.push(CacheOp::SetOwner {
                    owner_user_id: me.id.clone(),
                    me_name: me.display_name().to_owned(),
                    me_username: me.username.clone(),
                });
            }
            Update::Session(None) => {
                let was = std::mem::take(self);
                self.screen = Screen::Auth;
                self.error = was.error;
                self.read_marks = was.read_marks;
                // Sessão inválida: estado reconstruível não atravessa a
                // autenticação, mas filas/ledger particionados pelo owner
                // antigo continuam duráveis caso a mesma conta retorne.
                self.pending_cache.push(CacheOp::ClearCachedData);
            }
            Update::AuthFailed(message) => {
                self.screen = Screen::Auth;
                self.error = Some(message);
                self.busy = false;
            }
            Update::ServerLocked => {
                self.screen = Screen::Auth;
                self.locked = true;
                self.error = None;
                self.busy = false;
            }
            Update::ServerUnlocked => {
                // A senha do servidor valeu: o campo some e o login segue.
                self.locked = false;
                self.error = None;
                self.busy = false;
            }
            Update::ConnectionViolation => self.notice = Some(Notice::ConnectionViolation),
            Update::Server(server) => {
                self.screen = match &server {
                    Some(_) => Screen::Chat,
                    None => Screen::NeedsServer,
                };
                self.server = server.map(|server| Server {
                    name: server.name.clone(),
                    description: server
                        .owner_username
                        .clone()
                        .map(|owner| format!("de {owner}")),
                    owner_id: server.owner_id.clone(),
                    icon: server.icon_blob.clone().filter(|blob| !blob.is_empty()),
                });
                self.busy = false;
                if let Some(server) = &self.server {
                    self.pending_cache
                        .push(CacheOp::UpsertServer(CachedServer::from_store(
                            server,
                            &self.me,
                            &self.my_name,
                            &self.my_username,
                        )));
                }
            }
            Update::Channels(channels) => {
                let previous: HashMap<String, (bool, u32)> = self
                    .channels
                    .iter()
                    .map(|channel| (channel.id.clone(), (channel.unread, channel.mentions)))
                    .collect();
                self.channels = channels
                    .into_iter()
                    .filter(|channel| channel.kind != "category")
                    .map(|channel| {
                        let (unread, mentions) = previous
                            .get(&channel.id)
                            .copied()
                            .unwrap_or((false, 0));
                        // Sem marca local, o canal conta como visto: o
                        // servidor não guarda o último lido.
                        let mark = self.read_marks.get(&channel.id).copied();
                        let fresh = channel
                            .last_message
                            .as_ref()
                            .and_then(|last| last.created_at)
                            .zip(mark)
                            .map(|(at, mark)| at > mark)
                            .unwrap_or(false);
                        Channel {
                            unread: unread || fresh,
                            id: channel.id,
                            name: channel.name,
                            kind: ChannelKind::parse(&channel.kind),
                            topic: channel.topic,
                            position: channel.position,
                            permissions: channel.permissions,
                            notification_settings: channel.notification_settings,
                            parent_id: channel.parent_id.filter(|id| !id.is_empty()),
                            mentions,
                        }
                    })
                    .collect();
                self.channels.sort_by_key(|channel| channel.position);
                // O canal escolhido pode ter sido apagado — daqui ou de outra
                // janela. Sem isto a conversa ficaria apontando para um id
                // que não existe mais e o envio cairia no vazio.
                let gone = !self.selected_channel.is_empty()
                    && !self
                        .channels
                        .iter()
                        .any(|channel| channel.id == self.selected_channel);
                if gone {
                    self.selected_channel.clear();
                }
                if self.selected_channel.is_empty()
                    && let Some(first) = self
                        .channels
                        .iter()
                        .find(|channel| channel.kind == ChannelKind::Text)
                {
                    self.selected_channel = first.id.clone();
                }
                let cached: Vec<CachedChannel> =
                    self.channels.iter().map(CachedChannel::from).collect();
                self.pending_cache.push(CacheOp::ReplaceChannels(cached));
            }
            Update::Roles(roles) => {
                self.roles = roles;
                self.busy = false;
            }
            Update::ChannelPermissions {
                channel_id,
                permissions,
            } => {
                if let Some(channel) = self
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    channel.permissions = permissions;
                }
                self.busy = false;
            }
            // Perfis completam a listagem leve de usuários e também são o
            // caminho de convergência para user_join/avatar_update/role_*.
            Update::Profiles(profiles) => {
                let mut members_changed = false;
                for profile in profiles {
                    let id = profile.id.clone();
                    match profile.avatar_blob.clone().filter(|blob| !blob.is_empty()) {
                        Some(blob) => {
                            self.avatars.insert(id.clone(), blob);
                        }
                        None => {
                            self.avatars.remove(&id);
                        }
                    }

                    let mut summaries = profile.roles.clone();
                    summaries.sort_by_key(|role| std::cmp::Reverse(role.position));
                    self.profiles.insert(
                        id.clone(),
                        ProfileDetails {
                            description: profile
                                .description
                                .clone()
                                .filter(|text| !text.trim().is_empty()),
                            banner: profile
                                .banner_media
                                .clone()
                                .filter(|sha| !sha.is_empty()),
                            created_at: profile
                                .created_at
                                .as_deref()
                                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                                .map(|at| at.with_timezone(&Utc)),
                            roles: summaries,
                        },
                    );
                    let roles: Vec<String> =
                        profile.roles.iter().map(|role| role.id.clone()).collect();
                    let color = role_color(&profile.roles);
                    let username = profile.username.clone();
                    let name = profile.display_name().to_owned();
                    let status_message = profile.status_message.clone();
                    let typing_label = profile.typing.clone();
                    if let Some(member) = self.members.iter_mut().find(|member| member.id == id) {
                        member.username = username;
                        member.name = name;
                        member.status_message = status_message;
                        member.typing_label = typing_label;
                        member.roles = roles;
                        member.role_color = color;
                    } else {
                        self.members.push(Member {
                            id,
                            username,
                            name,
                            presence: Presence::Offline,
                            status_message,
                            typing_label,
                            role_color: color,
                            roles,
                        });
                    }
                    members_changed = true;
                }
                if members_changed {
                    self.sort_members();
                    let cached: Vec<CachedMember> =
                        self.members.iter().map(CachedMember::from).collect();
                    self.pending_cache.push(CacheOp::ReplaceMembers(cached));
                }
            }
            Update::OlderMessages {
                channel_id,
                messages,
                has_more,
            } => {
                let me = self.me.clone();
                for message in messages {
                    let message = convert(message, &me);
                    self.apply_mutation(
                        MutationSource::Reconcile,
                        StoreMutation::Timeline {
                            channel_id: Some(channel_id.clone()),
                            mutation: TimelineMutation::MessageUpsert(message),
                        },
                    );
                }
                self.sort_messages();
                self.history_has_more.insert(channel_id.clone(), has_more);
                self.history_loading.remove(&channel_id);
            }
            Update::OlderMessagesFailed { channel_id } => {
                self.history_loading.remove(&channel_id);
            }
            Update::UserSettings(settings) => {
                let mut settings = *settings;
                settings.config = settings.config.normalised();
                self.user_settings = Some(settings);
            }
            Update::Devices(devices) => {
                self.devices = devices;
                self.busy = false;
            }
            Update::AuditLogs { logs, has_more, append } => {
                if append {
                    self.audit_logs.extend(logs);
                } else {
                    self.audit_logs = logs;
                }
                self.audit_has_more = has_more;
                self.busy = false;
            }
            Update::Done => self.busy = false,
            Update::SearchFailed => {
                self.searching = false;
            }
            Update::SearchResults {
                results,
                has_more,
                append,
            } => {
                if append {
                    for result in results {
                        if !self.search_results.iter().any(|existing| existing.id == result.id) {
                            self.search_results.push(result);
                        }
                    }
                } else {
                    self.search_results = results;
                }
                self.search_has_more = has_more;
                self.searching = false;
            }
            Update::ReactionDetailsFailed { message_id } => {
                self.reaction_details_loading.remove(&message_id);
            }
            Update::ReactionDetails {
                message_id,
                reactions,
                has_more,
                append,
            } => {
                let target = self.reaction_details.entry(message_id.clone()).or_default();
                if !append {
                    target.clear();
                }
                for mut incoming in reactions {
                    if let Some(existing) = target.iter_mut().find(|group| {
                        group.emoji_id == incoming.emoji_id && group.unicode == incoming.unicode
                    }) {
                        for user in incoming.users.drain(..) {
                            if !existing.users.iter().any(|known| known.id == user.id) {
                                existing.users.push(user);
                            }
                        }
                        existing.count = existing.users.len() as u32;
                    } else {
                        target.push(incoming);
                    }
                }
                self.reaction_details_has_more.insert(message_id.clone(), has_more);
                self.reaction_details_loading.remove(&message_id);
            }
            // Chega depois da lista nova, então o canal já está lá.
            Update::ChannelCreated(id) => {
                if self.channels.iter().any(|channel| channel.id == id) {
                    self.selected_channel = id;
                }
                self.busy = false;
            }
            Update::Users(users) => {
                let presence: HashMap<String, Presence> = self
                    .members
                    .iter()
                    .map(|member| (member.id.clone(), member.presence))
                    .collect();
                self.members = users
                    .into_iter()
                    .map(|user| {
                        let name = user.display_name().to_owned();
                        let presence = presence
                            .get(&user.id)
                            .copied()
                            .unwrap_or(Presence::Offline);
                        let role_color = role_color(&user.roles);
                        let roles = user.roles.iter().map(|role| role.id.clone()).collect();
                        Member {
                            presence,
                            status_message: user.status_message,
                            typing_label: user.typing,
                            role_color,
                            roles,
                            name,
                            username: user.username,
                            id: user.id,
                        }
                    })
                    .collect();
                self.sort_members();
                let cached: Vec<CachedMember> =
                    self.members.iter().map(CachedMember::from).collect();
                self.pending_cache.push(CacheOp::ReplaceMembers(cached));
            }
            Update::Messages {
                ticket,
                messages,
                has_more,
                pinned_ids,
            } => {
                if self.refresh_ticket_is_current(&ticket) {
                    let channel_id = ticket.channel_id.clone();
                    let barrier_revision = self
                        .loading_channels
                        .get(&channel_id)
                        .map(|refresh| refresh.barrier_revision)
                        .unwrap_or_default();
                    let previous_pins: HashSet<String> = self
                        .messages
                        .iter()
                        .filter(|message| message.channel_id == channel_id && message.pinned)
                        .map(|message| message.id.clone())
                        .collect();
                    // `Some(ids)` é o snapshot autoritativo de fixadas; `None`
                    // (falha ao buscá-las) preserva o que já se sabia.
                    let authoritative_pins = pinned_ids;
                    let pinned: HashSet<String> = authoritative_pins
                        .as_ref()
                        .map(|ids| ids.iter().cloned().collect())
                        .unwrap_or(previous_pins);
                    let me = self.me.clone();
                    let messages = messages
                        .into_iter()
                        .map(|message| {
                            let mut message = convert(message, &me);
                            message.pinned = pinned.contains(&message.id);
                            message
                        })
                        .collect();

                    self.apply_mutation(
                        MutationSource::Reconcile,
                        StoreMutation::ReplaceChannelSnapshot {
                            channel_id: channel_id.clone(),
                            messages,
                            complete: !has_more,
                        },
                    );

                    // O snapshot de fixadas converge o banco mesmo para
                    // mensagens fora da janela carregada: uma fixada antiga que
                    // saiu do conjunto precisa ser desafixada para a retenção
                    // poder removê-la.
                    if let Some(ids) = authoritative_pins {
                        self.pending_cache.push(CacheOp::ReplacePins {
                            channel_id: channel_id.clone(),
                            ids,
                        });
                    }

                    // O snapshot só vira autoridade depois de reaplicar tudo
                    // que chegou pelo WebSocket após o barrier deste ticket.
                    self.replay_timeline_mutations(&channel_id, barrier_revision);
                    self.sort_messages();

                    self.loading_channels.remove(&channel_id);
                    self.history_has_more.insert(channel_id.clone(), has_more);
                    self.history_loading.remove(&channel_id);
                    self.channel_freshness
                        .insert(channel_id.clone(), ticket.generation);
                    self.mutation_journals.remove(&channel_id);
                    log::debug!(
                        "timeline reconciled channel={} generation={} request={} barrier={}",
                        channel_id,
                        ticket.generation,
                        ticket.request_id,
                        barrier_revision
                    );
                } else {
                    log::debug!(
                        "ignored refresh completion channel={} generation={} request={}: ticket is no longer current (store generation={})",
                        ticket.channel_id,
                        ticket.generation,
                        ticket.request_id,
                        self.sync_generation
                    );
                }
            }
            Update::MessagesFailed(ticket) => {
                if self.refresh_ticket_is_current(&ticket) {
                    self.loading_channels.remove(&ticket.channel_id);
                } else {
                    log::debug!(
                        "ignored failed refresh channel={} generation={} request={}: ticket is no longer current (store generation={})",
                        ticket.channel_id,
                        ticket.generation,
                        ticket.request_id,
                        self.sync_generation
                    );
                }
            }
            Update::Outgoing(outgoing) => {
                self.project_outgoing(*outgoing);
            }
            Update::OutgoingRestored(outgoing) => {
                self.messages.retain(|message| !message.pending);
                self.outgoing_states.clear();
                for outgoing in outgoing {
                    self.project_outgoing(outgoing);
                }
            }
            Update::OutgoingRemoved(local_id) => {
                self.messages.retain(|message| message.id != local_id);
                self.outgoing_states.remove(&local_id);
            }
            Update::OutgoingRejected { message, .. } => {
                self.error = Some(message);
            }
            Update::SendConfirmed { local_id, message } => {
                let me = self.me.clone();
                self.apply_mutation(
                    MutationSource::Reconcile,
                    StoreMutation::ConfirmSent {
                        local_id,
                        message: convert(*message, &me),
                    },
                );
            }
            Update::Sent(message) => {
                // Caminho legado de anexos: ele não tem local-id persistente.
                let me = self.me.clone();
                self.apply_mutation(
                    MutationSource::Reconcile,
                    StoreMutation::Timeline {
                        channel_id: Some(message.channel_id.clone()),
                        mutation: TimelineMutation::MessageUpsert(convert(*message, &me)),
                    },
                );
            }
            Update::Edited(message) => {
                let me = self.me.clone();
                let updated = convert(*message, &me);
                self.apply_mutation(
                    MutationSource::Reconcile,
                    StoreMutation::Timeline {
                        channel_id: Some(updated.channel_id.clone()),
                        mutation: TimelineMutation::MessageUpsert(updated),
                    },
                );
            }
            Update::Deleted(id) => {
                let channel_id = self.channel_for_message(&id);
                self.apply_mutation(
                    MutationSource::Reconcile,
                    StoreMutation::Timeline {
                        channel_id,
                        mutation: TimelineMutation::MessageDelete { id },
                    },
                );
            }
            Update::Emojis(emojis) => {
                self.emojis = emojis
                    .into_iter()
                    .map(|emoji| CustomEmoji {
                        id: emoji.id,
                        name: emoji.name,
                        blob: emoji.image_blob,
                    })
                    .collect();
            }
            Update::Pinned { channel_id, ids } => {
                let pinned: HashSet<String> = ids.iter().cloned().collect();
                let changes: Vec<(String, bool)> = self
                    .messages
                    .iter()
                    .filter(|message| message.channel_id == channel_id)
                    .map(|message| (message.id.clone(), pinned.contains(&message.id)))
                    .collect();
                for (message_id, pinned) in changes {
                    self.apply_mutation(
                        MutationSource::Reconcile,
                        StoreMutation::Timeline {
                            channel_id: Some(channel_id.clone()),
                            mutation: TimelineMutation::MessagePinned { message_id, pinned },
                        },
                    );
                }
                // Snapshot autoritativo: converge também linhas que a Store
                // não tem carregadas.
                self.pending_cache.push(CacheOp::ReplacePins { channel_id, ids });
            }
            Update::Notifications(notifications) => {
                for notification in notifications {
                    if notification.read {
                        continue;
                    }
                    let Some(channel_id) = notification.channel_id.clone() else {
                        continue;
                    };
                    if !self.counted_notifications.insert(notification.id.clone()) {
                        continue;
                    }
                    let newer = notification
                        .created_at
                        .zip(self.read_marks.get(&channel_id).copied())
                        .map(|(at, mark)| at > mark)
                        .unwrap_or(true);
                    if !newer {
                        continue;
                    }
                    if let Some(channel) = self
                        .channels
                        .iter_mut()
                        .find(|channel| channel.id == channel_id)
                    {
                        channel.mentions += 1;
                        channel.unread = true;
                    }
                    self.open_notifications
                        .entry(channel_id)
                        .or_default()
                        .push(notification.id);
                }
            }
            Update::Event(event) => self.apply_event(*event),
            // Quem monta a call com isso é a janela (ela tem a thread de
            // mídia); aqui só sabemos que o pedido de entrada saiu.
            Update::VoiceReady { .. } => {}
            Update::VoiceFailed {
                channel_id,
                attempt,
                message,
            } => {
                if self.call.current(&channel_id, attempt) {
                    self.call.error = Some(message.clone());
                    self.call.left();
                }
                self.error = Some(message);
            }
            Update::Connection(connection) => {
                if self.connection == Connection::Online && connection != Connection::Online {
                    self.sync_generation = self.sync_generation.saturating_add(1);
                    self.loading_channels.clear();
                }
                self.connection = connection;
            }
            Update::Error(message) => {
                self.error = Some(message);
                self.busy = false;
            }
            Update::PushDevice { .. } | Update::BackgroundFinished(_) => {}
        }
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Message(message) => {
                let me = self.me.clone();
                let message = convert(*message, &me);
                let channel_id = message.channel_id.clone();
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id: Some(channel_id),
                        mutation: TimelineMutation::MessageUpsert(message),
                    },
                );
            }
            Event::MessageEdited {
                id,
                channel_id,
                content,
            } => {
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id: Some(channel_id),
                        mutation: TimelineMutation::MessageEdit { id, content },
                    },
                );
            }
            Event::MessageDeleted { id, channel_id } => {
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id: Some(channel_id),
                        mutation: TimelineMutation::MessageDelete { id },
                    },
                );
            }
            Event::MessagePinned {
                message_id,
                pinned,
            } => {
                let channel_id = self.channel_for_message(&message_id);
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id,
                        mutation: TimelineMutation::MessagePinned { message_id, pinned },
                    },
                );
            }
            Event::NewPreview { .. } => {
                // O worker de rede resolve new_preview para LinkPreviewUpdated
                // antes de publicar o evento para a Store.
            }
            Event::RemovePreview {
                message_id,
                preview_id,
            } => {
                let channel_id = self.channel_for_message(&message_id);
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id,
                        mutation: TimelineMutation::PreviewRemoved {
                            message_id,
                            preview_id,
                        },
                    },
                );
            }
            Event::LinkPreviewUpdated {
                message_id,
                preview,
            } => {
                let channel_id = self.channel_for_message(&message_id);
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id,
                        mutation: TimelineMutation::PreviewUpsert {
                            message_id,
                            preview,
                        },
                    },
                );
            }
            Event::AttachmentModeration {
                message_id,
                attachment_id,
                status,
            } => {
                let channel_id = self.channel_for_message(&message_id);
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id,
                        mutation: TimelineMutation::AttachmentModeration {
                            message_id,
                            attachment_id,
                            status,
                        },
                    },
                );
            }
            Event::Typing {
                channel_id,
                user_id,
                is_typing,
            } => {
                let users = self.typing.entry(channel_id).or_default();
                if is_typing {
                    users.insert(user_id);
                } else {
                    users.remove(&user_id);
                }
            }
            Event::Presence {
                user_id,
                status,
                status_message,
                typing,
                nickname,
            } => {
                let offline = status == "offline";
                if let Some(member) = self
                    .members
                    .iter_mut()
                    .find(|member| member.id == user_id)
                {
                    member.presence = Presence::parse(&status);
                    if offline {
                        member.status_message = None;
                        member.typing_label = None;
                    } else {
                        member.status_message = status_message;
                        member.typing_label = typing;
                    }
                    if let Some(nickname) = nickname {
                        member.name = nickname;
                    }
                    let cached: Vec<CachedMember> =
                        self.members.iter().map(CachedMember::from).collect();
                    self.pending_cache.push(CacheOp::ReplaceMembers(cached));
                }
                self.sort_members();
            }
            Event::PresenceSync(members) => {
                let online: HashMap<String, crate::api::ws::PresenceMember> = members
                    .into_iter()
                    .map(|member| (member.user_id.clone(), member))
                    .collect();
                for member in &mut self.members {
                    if let Some(live) = online.get(&member.id) {
                        member.presence = Presence::parse(&live.status);
                        member.status_message = live.status_message.clone();
                    } else {
                        member.presence = Presence::Offline;
                        member.status_message = None;
                        member.typing_label = None;
                    }
                }
                self.sort_members();
            }
            Event::ChannelCreated {
                id,
                name,
                kind,
                topic,
            } => {
                // Quem criou o canal já o recebeu pela lista relida logo
                // depois do POST; o evento chega em seguida e duplicaria a
                // linha. Para os outros clientes ele é a única notícia.
                if self.channels.iter().any(|channel| channel.id == id) {
                    return;
                }
                let position = self.channels.len() as i32 + 1;
                self.channels.push(Channel {
                    id,
                    name,
                    kind: ChannelKind::parse(&kind),
                    topic,
                    position,
                    permissions: Vec::new(),
                    notification_settings: "only_mentions".to_owned(),
                    parent_id: None,
                    unread: false,
                    mentions: 0,
                });
            }
            Event::ChannelUpdated { id, name, topic } => {
                if let Some(channel) = self.channels.iter_mut().find(|channel| channel.id == id) {
                    channel.name = name;
                    channel.topic = topic;
                }
            }
            Event::ChannelDeleted { id } => {
                self.channels.retain(|channel| channel.id != id);
                self.messages.retain(|message| message.channel_id != id);
                self.loading_channels.remove(&id);
                self.mutation_journals.remove(&id);
                self.channel_freshness.remove(&id);
                if self.selected_channel == id {
                    self.selected_channel = self
                        .channels
                        .first()
                        .map(|channel| channel.id.clone())
                        .unwrap_or_default();
                }
            }
            Event::UserJoined { .. }
            | Event::AvatarUpdated { .. }
            | Event::RoleAdded { .. }
            | Event::RoleRemoved { .. } => {
                // O worker de rede resolve estes eventos para Update::Profiles.
            }
            Event::VoiceJoined {
                channel_id,
                members,
                active_speakers,
            } => self.call.joined(&channel_id, members, active_speakers),
            Event::VoiceState { channel_id, state } => {
                self.call.update(&channel_id, state);
            }
            Event::VoiceLeft {
                channel_id,
                user_id,
            } => {
                self.call.remove(&channel_id, &user_id);
                // Fomos nós: a call acabou, mesmo que tenha sido o servidor
                // que a encerrou (sala destruída, sessão revogada).
                if user_id == self.me && channel_id == self.call.channel_id {
                    self.call.left();
                }
            }
            Event::ActiveSpeakers {
                channel_id,
                user_ids,
            } => {
                if channel_id == self.call.channel_id {
                    self.call.speakers = user_ids;
                }
            }
            Event::Failure { message, code } => {
                let joining = self.call.phase == Phase::Joining;
                let fatal = code
                    .as_deref()
                    .is_some_and(|code| call::fatal(code, joining));
                if fatal && self.call.active() {
                    self.call.error = Some(message.clone());
                    self.error = Some(message.clone());
                    self.call.left();
                }
                log::warn!("evento de erro do servidor: {message}");
            }
            // A sinalização é da thread da call, não do estado da tela.
            Event::VoiceAnswer { .. } | Event::VoiceOffer { .. } | Event::VoiceCandidate { .. } => {}
            Event::Notification { id, message_id, .. } => {
                // O evento não traz o canal; achamos pela mensagem quando ela
                // já está em memória. O resto vem da listagem REST.
                if !self.counted_notifications.insert(id) {
                    return;
                }
                let Some(channel_id) = message_id
                    .and_then(|id| self.message(&id).map(|message| message.channel_id.clone()))
                else {
                    return;
                };
                if channel_id == self.selected_channel {
                    return;
                }
                if let Some(channel) = self
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    channel.mentions += 1;
                    channel.unread = true;
                }
            }
            Event::Reaction {
                message_id,
                unicode,
                emoji_id,
                count,
            } => {
                let Some(emoji) = Emoji::from_parts(unicode, emoji_id) else {
                    return;
                };
                let channel_id = self.channel_for_message(&message_id);
                self.apply_mutation(
                    MutationSource::Live,
                    StoreMutation::Timeline {
                        channel_id,
                        mutation: TimelineMutation::Reaction {
                            message_id,
                            emoji,
                            count,
                        },
                    },
                );
            }
        }
    }

    /// Aplica a intenção otimista de reação pela mesma projeção canônica.
    pub fn set_reaction_local(&mut self, message_id: &str, emoji: &Emoji, add: bool) {
        let channel_id = self.channel_for_message(message_id);
        self.apply_mutation(
            MutationSource::Local,
            StoreMutation::Timeline {
                channel_id,
                mutation: TimelineMutation::LocalReaction {
                    message_id: message_id.to_owned(),
                    emoji: emoji.clone(),
                    add,
                },
            },
        );
    }

    /// Compatibilidade para chamadores que ainda expressam a ação como toggle.
    pub fn toggle_reaction_local(&mut self, message_id: &str, emoji: &Emoji) -> bool {
        let add = self
            .message(message_id)
            .and_then(|message| {
                message
                    .reactions
                    .iter()
                    .find(|reaction| &reaction.emoji == emoji)
            })
            .is_none_or(|reaction| !reaction.mine);
        self.set_reaction_local(message_id, emoji, add);
        add
    }

    /// Projeta uma linha já durável da fila como eco local. Repetir o mesmo
    /// local_id é idempotente e só atualiza o estado visível.
    pub fn project_outgoing(&mut self, outgoing: CachedOutgoing) {
        let local_id = outgoing.local_id.clone();
        let state = outgoing.state;
        let message = Message {
            id: local_id.clone(),
            channel_id: outgoing.channel_id.clone(),
            author_id: outgoing.owner_user_id,
            content: outgoing.content,
            at: DateTime::from_timestamp_millis(outgoing.created_at)
                .unwrap_or_else(Utc::now)
                .with_timezone(&Local),
            edited: false,
            reply_to: outgoing.reply_to,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: true,
        };
        self.outgoing_states.insert(local_id, state);
        self.apply_mutation(
            MutationSource::Local,
            StoreMutation::Timeline {
                channel_id: Some(outgoing.channel_id),
                mutation: TimelineMutation::MessageUpsert(message),
            },
        );
    }

    pub fn outgoing_state(&self, local_id: &str) -> Option<OutgoingState> {
        self.outgoing_states.get(local_id).copied()
    }

    /// Compatibilidade usada só por testes/fluxos locais antigos. Produz uma
    /// identidade collision-resistant, mas não implica persistência.
    pub fn push_pending(
        &mut self,
        channel_id: &str,
        content: &str,
        reply_to: Option<String>,
    ) -> String {
        let id = crate::cache::new_local_id();
        self.project_outgoing(CachedOutgoing {
            local_id: id.clone(),
            owner_user_id: self.me.clone(),
            channel_id: channel_id.to_owned(),
            content: content.to_owned(),
            reply_to,
            notify_reply: false,
            created_at: now_millis(),
            state: OutgoingState::Queued,
            attempt_count: 0,
            last_attempt_at: None,
            last_error: None,
        });
        id
    }

    pub fn set_message_pending_local(&mut self, message_id: &str, pending: bool) {
        let channel_id = self.channel_for_message(message_id);
        self.apply_mutation(
            MutationSource::Local,
            StoreMutation::Timeline {
                channel_id,
                mutation: TimelineMutation::MessagePending {
                    id: message_id.to_owned(),
                    pending,
                },
            },
        );
    }

    pub fn edit_message_local(&mut self, message_id: &str, content: String) {
        let channel_id = self.channel_for_message(message_id);
        self.apply_mutation(
            MutationSource::Local,
            StoreMutation::Timeline {
                channel_id,
                mutation: TimelineMutation::MessageEdit {
                    id: message_id.to_owned(),
                    content,
                },
            },
        );
    }

    pub fn delete_message_local(&mut self, message_id: &str) {
        let channel_id = self.channel_for_message(message_id);
        self.apply_mutation(
            MutationSource::Local,
            StoreMutation::Timeline {
                channel_id,
                mutation: TimelineMutation::MessageDelete {
                    id: message_id.to_owned(),
                },
            },
        );
    }

    pub fn set_message_pinned_local(&mut self, message_id: &str, pinned: bool) {
        let channel_id = self.channel_for_message(message_id);
        self.apply_mutation(
            MutationSource::Local,
            StoreMutation::Timeline {
                channel_id,
                mutation: TimelineMutation::MessagePinned {
                    message_id: message_id.to_owned(),
                    pinned,
                },
            },
        );
    }

    fn sort_messages(&mut self) {
        self.messages.sort_by_key(|message| message.at);
    }

    fn sort_members(&mut self) {
        self.members
            .sort_by_key(|a| a.name.to_lowercase());
    }
}

fn convert(message: models::Message, me: &str) -> Message {
    let mine: HashSet<Emoji> = message
        .user_reactions
        .iter()
        .filter_map(|reaction| {
            Emoji::from_parts(reaction.unicode.clone(), reaction.emoji_id.clone())
        })
        .collect();
    let _ = me;

    Message {
        id: message.id,
        channel_id: message.channel_id,
        author_id: message.author_id,
        content: message.content.unwrap_or_default(),
        at: message.created_at.with_timezone(&Local),
        edited: message.edited_at.is_some(),
        reply_to: message.reply_to,
        attachments: message.attachments,
        previews: message.previews,
        reactions: message
            .reactions
            .into_iter()
            .filter_map(|reaction| {
                let emoji = Emoji::from_parts(reaction.unicode, reaction.emoji_id)?;
                Some(Reaction {
                    mine: mine.contains(&emoji),
                    emoji,
                    count: reaction.count,
                })
            })
            .collect(),
        pinned: false,
        pending: false,
    }
}

/// Cor do cargo mais alto que define uma.
fn role_color(roles: &[models::RoleSummary]) -> Option<[u8; 3]> {
    roles
        .iter()
        .filter(|role| role.color.is_some())
        .max_by_key(|role| role.position)
        .and_then(|role| role.color.as_deref())
        .and_then(parse_hex_color)
}

#[cfg(test)]
mod channel_layout_tests {
    use super::*;

    fn channel(id: &str, kind: ChannelKind, position: i32, parent: Option<&str>) -> Channel {
        Channel {
            id: id.into(),
            name: id.into(),
            kind,
            topic: None,
            position,
            permissions: Vec::new(),
            notification_settings: "only_mentions".into(),
            parent_id: parent.map(str::to_owned),
            unread: false,
            mentions: 0,
        }
    }

    #[test]
    fn without_parent_ids_a_category_owns_the_channels_after_it() {
        let store = Store {
            channels: vec![
                channel("geral", ChannelKind::Text, 1, None),
                channel("proj", ChannelKind::Category, 2, None),
                channel("dev", ChannelKind::Text, 3, None),
                channel("voz", ChannelKind::Voice, 4, None),
            ],
            ..Store::default()
        };
        let layout: Vec<_> = store
            .channel_layout()
            .into_iter()
            .map(|(index, parent)| (store.channels[index].id.clone(), parent))
            .collect();
        assert_eq!(layout[0], ("geral".into(), None));
        assert_eq!(layout[2], ("dev".into(), Some("proj".into())));
        assert_eq!(layout[3], ("voz".into(), Some("proj".into())));
    }

    #[test]
    fn explicit_parent_ids_win_over_order() {
        let store = Store {
            channels: vec![
                channel("proj", ChannelKind::Category, 1, None),
                channel("geral", ChannelKind::Text, 2, None),
                channel("dev", ChannelKind::Text, 3, Some("proj")),
            ],
            ..Store::default()
        };
        // dev vem logo abaixo da sua categoria, antes de geral.
        let layout: Vec<_> = store
            .channel_layout()
            .into_iter()
            .map(|(index, parent)| (store.channels[index].id.as_str(), parent))
            .collect();
        assert_eq!(
            layout,
            vec![("proj", None), ("dev", Some("proj".into())), ("geral", None)]
        );
    }

    #[test]
    fn children_follow_their_category_wherever_it_is() {
        // A categoria foi para o fim, os filhos ficaram com posições baixas.
        let store = Store {
            channels: vec![
                channel("dev", ChannelKind::Text, 1, Some("proj")),
                channel("design", ChannelKind::Text, 2, Some("proj")),
                channel("voz", ChannelKind::Voice, 3, None),
                channel("proj", ChannelKind::Category, 4, None),
                channel("geral", ChannelKind::Text, 5, None),
            ],
            ..Store::default()
        };
        let order: Vec<_> = store
            .channel_layout()
            .into_iter()
            .map(|(index, _)| store.channels[index].id.as_str())
            .collect();
        assert_eq!(order, vec!["voz", "proj", "dev", "design", "geral"]);
    }

    #[test]
    fn local_move_keeps_positions_contiguous_and_sets_the_parent() {
        let mut store = Store {
            channels: vec![
                channel("a", ChannelKind::Text, 1, None),
                channel("cat", ChannelKind::Category, 2, None),
                channel("b", ChannelKind::Text, 3, None),
            ],
            ..Store::default()
        };
        store.move_channel_local("a", 3, Some("cat".into()));
        let order: Vec<_> = store.channels.iter().map(|c| (c.id.as_str(), c.position)).collect();
        assert_eq!(order, vec![("cat", 1), ("b", 2), ("a", 3)]);
        assert_eq!(store.channel("a").unwrap().parent_id.as_deref(), Some("cat"));
        store.move_channel_local("a", 1, Some(String::new()));
        assert_eq!(store.channel("a").unwrap().parent_id, None);
        assert_eq!(store.channels[0].id, "a");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_project_freshness_refresh_and_barrier_state() {
        let mut store = Store {
            sync_generation: 7,
            selected_channel: "refreshing".to_owned(),
            ..Store::default()
        };
        store.channel_freshness.insert("fresh".to_owned(), 7);
        store.channel_freshness.insert("stale".to_owned(), 6);
        store.loading_channels.insert(
            "refreshing".to_owned(),
            ActiveRefresh {
                ticket: RefreshTicket {
                    channel_id: "refreshing".to_owned(),
                    generation: 7,
                    request_id: 42,
                },
                barrier_revision: 109,
            },
        );
        store.mutation_journals.insert(
            "refreshing".to_owned(),
            ChannelMutationJournal {
                revision: 111,
                entries: VecDeque::from([
                    JournalEntry {
                        revision: 110,
                        mutation: TimelineMutation::MessageDelete { id: "m1".to_owned() },
                    },
                    JournalEntry {
                        revision: 111,
                        mutation: TimelineMutation::MessageDelete { id: "m2".to_owned() },
                    },
                ]),
            },
        );

        let diagnostics = store.diagnostics();
        assert_eq!(diagnostics.sync_generation, 7);
        assert_eq!(diagnostics.selected_channel, "refreshing");

        let fresh = diagnostics.timelines.iter().find(|item| item.channel_id == "fresh").unwrap();
        assert_eq!(fresh.status, TimelineStatus::Fresh);
        assert_eq!(fresh.fresh_generation, Some(7));

        let stale = diagnostics.timelines.iter().find(|item| item.channel_id == "stale").unwrap();
        assert_eq!(stale.status, TimelineStatus::Stale);

        let refreshing = diagnostics.timelines.iter().find(|item| item.channel_id == "refreshing").unwrap();
        assert_eq!(refreshing.status, TimelineStatus::Refreshing);
        assert_eq!(
            refreshing.active_refresh,
            Some(RefreshDiagnostics {
                generation: 7,
                request_id: 42,
                barrier_revision: 109,
            })
        );
        assert_eq!(refreshing.journal_revision, 111);
        assert_eq!(refreshing.journal_entries, 2);
    }

    #[test]
    fn diagnostics_do_not_expose_payloads_or_secrets() {
        let store = Store {
            sync_generation: 3,
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        let diagnostics = store.diagnostics();

        assert_eq!(diagnostics.sync_generation, 3);
        assert_eq!(diagnostics.timelines.len(), 1);
        assert_eq!(diagnostics.timelines[0].channel_id, "geral");
    }


    fn store_com_canal_carregado(channel_id: &str) -> Store {
        let mut store = Store {
            selected_channel: channel_id.to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));
        assert_eq!(
            store.channel_needing_messages(),
            Some(channel_id.to_owned())
        );
        let ticket = store.mark_loading(channel_id);
        store.apply(Update::Messages {
            ticket,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });
        assert_eq!(store.channel_needing_messages(), None);
        store
    }

    fn wire_message(id: &str, channel_id: &str, content: &str) -> models::Message {
        models::Message {
            id: id.to_owned(),
            channel_id: channel_id.to_owned(),
            author_id: "outro".to_owned(),
            content: Some(content.to_owned()),
            created_at: Utc::now(),
            edited_at: None,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            user_reactions: Vec::new(),
        }
    }

    fn finish_snapshot(
        store: &mut Store,
        ticket: RefreshTicket,
        messages: Vec<models::Message>,
    ) {
        store.apply(Update::Messages {
            ticket,
            messages,
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });
    }

    fn store_com_mensagem(channel_id: &str, message_id: &str, content: &str) -> Store {
        let mut store = Store {
            selected_channel: channel_id.to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));
        let ticket = store.mark_loading(channel_id);
        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message(message_id, channel_id, content)],
        );
        store
    }

    fn perder_continuidade(store: &mut Store) {
        store.apply(Update::Connection(Connection::Offline));
        store.apply(Update::Connection(Connection::Connecting));
        store.apply(Update::Connection(Connection::Online));
    }

    #[test]
    fn mencao_vai_para_id_e_volta_ao_nickname_atual() {
        let mut store = Store::default();
        store.members.push(Member {
            id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
            username: "christian".to_owned(),
            name: "Chris".to_owned(),
            presence: Presence::Online,
            status_message: None,
            typing_label: None,
            role_color: None,
            roles: Vec::new(),
        });

        let binding = MentionBinding {
            start: 3,
            label: "Chris".to_owned(),
            user_id: store.members[0].id.clone(),
        };
        let encoded = store.encode_mentions("oi @Chris", &[binding]);
        assert_eq!(
            encoded,
            "oi <@550e8400-e29b-41d4-a716-446655440000>"
        );

        store.members[0].name = "Christian H".to_owned();
        assert_eq!(store.display_mentions(&encoded), "oi @Christian H");
    }

    #[test]
    fn autocomplete_desambigua_nicknames_iguais_por_user_id() {
        let mut store = Store::default();
        for (id, username) in [("id-a", "chris_a"), ("id-b", "chris_b")] {
            store.members.push(Member {
                id: id.to_owned(),
                username: username.to_owned(),
                name: "Chris".to_owned(),
                presence: Presence::Online,
                status_message: None,
                typing_label: None,
                role_color: None,
                roles: Vec::new(),
            });
        }

        // Digitado à mão é ambíguo, portanto fica texto comum.
        assert_eq!(store.encode_mentions("@Chris", &[]), "@Chris");

        // A escolha no autocomplete carrega a identidade exata.
        let binding = MentionBinding {
            start: 0,
            label: "Chris".to_owned(),
            user_id: "id-b".to_owned(),
        };
        assert_eq!(store.encode_mentions("@Chris", &[binding]), "<@id-b>");
    }

    #[test]
    fn email_nao_vira_mencao() {
        let mut store = Store::default();
        store.members.push(Member {
            id: "id-chris".to_owned(),
            username: "chris_real".to_owned(),
            name: "Chris".to_owned(),
            presence: Presence::Online,
            status_message: None,
            typing_label: None,
            role_color: None,
            roles: Vec::new(),
        });

        assert_eq!(
            store.encode_mentions("ana@Chris.com", &[]),
            "ana@Chris.com"
        );
    }

    #[test]
    fn mencao_manual_unica_e_chamados_globais() {
        let mut store = Store::default();
        store.members.push(Member {
            id: "id-ana".to_owned(),
            username: "ana_real".to_owned(),
            name: "Ana Maria".to_owned(),
            presence: Presence::Offline,
            status_message: None,
            typing_label: None,
            role_color: None,
            roles: Vec::new(),
        });

        assert_eq!(
            store.encode_mentions("oi @Ana Maria! @everyone @todos", &[]),
            "oi <@id-ana>! @everyone @todos"
        );
    }

    #[test]
    fn reconexao_invalida_cache_de_mensagens() {
        let mut store = store_com_canal_carregado("geral");

        store.apply(Update::Connection(Connection::Offline));
        assert_eq!(store.channel_needing_messages(), None);

        store.apply(Update::Connection(Connection::Connecting));
        assert_eq!(store.channel_needing_messages(), None);

        store.apply(Update::Connection(Connection::Online));
        assert_eq!(
            store.channel_needing_messages(),
            Some("geral".to_owned())
        );
    }

    #[test]
    fn nao_carrega_historico_enquanto_socket_esta_fora() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };

        assert_eq!(store.channel_needing_messages(), None);

        store.apply(Update::Connection(Connection::Connecting));
        assert_eq!(store.channel_needing_messages(), None);

        store.apply(Update::Connection(Connection::Online));
        assert_eq!(
            store.channel_needing_messages(),
            Some("geral".to_owned())
        );
    }

    #[test]
    fn falha_de_historico_libera_nova_tentativa() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));

        assert_eq!(
            store.channel_needing_messages(),
            Some("geral".to_owned())
        );
        let ticket = store.mark_loading("geral");
        assert_eq!(store.channel_needing_messages(), None);

        store.apply(Update::MessagesFailed(ticket));
        assert_eq!(
            store.channel_needing_messages(),
            Some("geral".to_owned())
        );
    }

    #[test]
    fn resposta_de_geracao_antiga_nao_fica_fresca() {
        let mut store = Store { selected_channel: "geral".to_owned(), ..Store::default() };
        store.apply(Update::Connection(Connection::Online));
        let antiga = store.mark_loading("geral");
        store.apply(Update::Connection(Connection::Offline));
        store.apply(Update::Connection(Connection::Connecting));
        store.apply(Update::Connection(Connection::Online));
        let atual = store.mark_loading("geral");

        store.apply(Update::Messages {
            ticket: antiga,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Refreshing);
        store.apply(Update::Messages {
            ticket: atual,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Fresh);
    }

    #[test]
    fn resposta_antiga_da_mesma_geracao_nao_supersede_tentativa_nova() {
        let mut store = Store { selected_channel: "geral".to_owned(), ..Store::default() };
        store.apply(Update::Connection(Connection::Online));
        let primeira = store.mark_loading("geral");
        let segunda = store.mark_loading("geral");
        assert_ne!(primeira.request_id, segunda.request_id);

        store.apply(Update::Messages {
            ticket: primeira,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Refreshing);
        store.apply(Update::Messages {
            ticket: segunda,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Fresh);
    }

    #[test]
    fn falha_antiga_nao_cancela_tentativa_atual() {
        let mut store = Store { selected_channel: "geral".to_owned(), ..Store::default() };
        store.apply(Update::Connection(Connection::Online));
        let antiga = store.mark_loading("geral");
        let atual = store.mark_loading("geral");
        store.apply(Update::MessagesFailed(antiga));
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Refreshing);
        store.apply(Update::Messages {
            ticket: atual,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Fresh);
    }

    #[test]
    fn geracao_avanca_so_quando_online_perde_continuidade() {
        let mut store = Store::default();
        store.apply(Update::Connection(Connection::Connecting));
        store.apply(Update::Connection(Connection::Online));
        assert_eq!(store.sync_generation(), 0);
        store.apply(Update::Connection(Connection::Offline));
        assert_eq!(store.sync_generation(), 1);
        store.apply(Update::Connection(Connection::Connecting));
        store.apply(Update::Connection(Connection::Online));
        assert_eq!(store.sync_generation(), 1);
    }

    #[test]
    fn mensagem_live_durante_refresh_sobrevive_ao_snapshot() {
        let mut store = store_com_mensagem("geral", "antiga", "antes");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("geral");

        store.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("nova", "geral", "live"),
        )))));
        assert!(store.message("nova").is_none());

        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message("antiga", "geral", "antes")],
        );

        assert_eq!(store.timeline_status("geral"), TimelineStatus::Fresh);
        assert_eq!(
            store.messages.iter().filter(|message| message.id == "nova").count(),
            1
        );
    }

    #[test]
    fn edicao_live_durante_refresh_nao_e_revertida() {
        let mut store = store_com_mensagem("geral", "m1", "velho");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("geral");

        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "novo".to_owned(),
        })));
        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message("m1", "geral", "velho")],
        );

        assert_eq!(store.message("m1").map(|message| message.content.as_str()), Some("novo"));
    }

    #[test]
    fn delete_live_durante_refresh_nao_ressuscita_mensagem() {
        let mut store = store_com_mensagem("geral", "m1", "existe");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("geral");

        store.apply(Update::Event(Box::new(Event::MessageDeleted {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
        })));
        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message("m1", "geral", "existe")],
        );

        assert!(store.message("m1").is_none());
    }

    #[test]
    fn snapshot_e_evento_duplicado_convergem_em_uma_mensagem() {
        let mut store = store_com_mensagem("geral", "antiga", "antes");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("geral");

        store.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("m1", "geral", "live"),
        )))));
        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message("m1", "geral", "snapshot")],
        );

        let found: Vec<&Message> = store
            .messages
            .iter()
            .filter(|message| message.id == "m1")
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "live");
    }

    #[test]
    fn resposta_de_geracao_antiga_nao_toca_barrier_atual() {
        let mut store = store_com_mensagem("geral", "m1", "base");
        perder_continuidade(&mut store);
        let antiga = store.mark_loading("geral");
        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "geracao-antiga".to_owned(),
        })));

        perder_continuidade(&mut store);
        let atual = store.mark_loading("geral");
        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "geracao-atual".to_owned(),
        })));

        finish_snapshot(
            &mut store,
            antiga,
            vec![wire_message("m1", "geral", "snapshot-antigo")],
        );
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Refreshing);

        finish_snapshot(
            &mut store,
            atual,
            vec![wire_message("m1", "geral", "snapshot-atual")],
        );
        assert_eq!(
            store.message("m1").map(|message| message.content.as_str()),
            Some("geracao-atual")
        );
    }

    #[test]
    fn resposta_supersedida_na_mesma_geracao_nao_toca_barrier_novo() {
        let mut store = store_com_mensagem("geral", "m1", "base");
        perder_continuidade(&mut store);
        let primeira = store.mark_loading("geral");
        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "entre-a-e-b".to_owned(),
        })));

        let segunda = store.mark_loading("geral");
        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "depois-de-b".to_owned(),
        })));

        finish_snapshot(
            &mut store,
            primeira,
            vec![wire_message("m1", "geral", "snapshot-a")],
        );
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Refreshing);

        finish_snapshot(
            &mut store,
            segunda,
            vec![wire_message("m1", "geral", "entre-a-e-b")],
        );
        assert_eq!(
            store.message("m1").map(|message| message.content.as_str()),
            Some("depois-de-b")
        );
    }

    #[test]
    fn falha_de_refresh_mantem_cache_stale_e_permite_retry() {
        let mut store = store_com_mensagem("geral", "m1", "base");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("geral");
        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "live".to_owned(),
        })));

        store.apply(Update::MessagesFailed(ticket));

        assert_eq!(store.timeline_status("geral"), TimelineStatus::Stale);
        assert_eq!(
            store.message("m1").map(|message| message.content.as_str()),
            Some("live")
        );
        assert_eq!(store.channel_needing_messages(), Some("geral".to_owned()));
    }

    #[test]
    fn trocar_de_canal_durante_refresh_nao_muda_selecao() {
        let mut store = store_com_mensagem("a", "m1", "base");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("a");
        store.selected_channel = "b".to_owned();

        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message("m1", "a", "reconciliado")],
        );

        assert_eq!(store.selected_channel, "b");
        assert_eq!(store.timeline_status("a"), TimelineStatus::Fresh);
        assert_eq!(store.timeline_status("b"), TimelineStatus::Missing);
    }

    #[test]
    fn journals_de_servidores_diferentes_ficam_isolados() {
        let mut primeiro = store_com_mensagem("a", "m-a", "base-a");
        let segundo = store_com_mensagem("b", "m-b", "base-b");

        perder_continuidade(&mut primeiro);
        let ticket = primeiro.mark_loading("a");
        primeiro.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m-a".to_owned(),
            channel_id: "a".to_owned(),
            content: "live-a".to_owned(),
        })));
        finish_snapshot(
            &mut primeiro,
            ticket,
            vec![wire_message("m-a", "a", "snapshot-a")],
        );

        assert_eq!(
            primeiro.message("m-a").map(|message| message.content.as_str()),
            Some("live-a")
        );
        assert_eq!(
            segundo.message("m-b").map(|message| message.content.as_str()),
            Some("base-b")
        );
        assert_eq!(segundo.timeline_status("b"), TimelineStatus::Fresh);
    }

    #[test]
    fn pin_live_durante_refresh_sobrevive_snapshot_de_pins() {
        let mut store = store_com_mensagem("geral", "m1", "base");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("geral");

        store.apply(Update::Event(Box::new(Event::MessagePinned {
            message_id: "m1".to_owned(),
            pinned: true,
        })));
        store.apply(Update::Messages {
            ticket,
            messages: vec![wire_message("m1", "geral", "base")],
            has_more: false,
            pinned_ids: Some(Vec::new()),
        });

        assert!(store.message("m1").is_some_and(|message| message.pinned));
    }

    #[test]
    fn cache_de_um_servidor_nao_invalida_o_outro() {
        let mut primeiro = store_com_canal_carregado("canal-a");
        let segundo = store_com_canal_carregado("canal-b");

        primeiro.apply(Update::Connection(Connection::Offline));
        primeiro.apply(Update::Connection(Connection::Connecting));
        primeiro.apply(Update::Connection(Connection::Online));

        assert_eq!(
            primeiro.channel_needing_messages(),
            Some("canal-a".to_owned())
        );
        assert_eq!(segundo.channel_needing_messages(), None);
    }

    fn store_para_proveniencia() -> Store {
        let mut store = Store {
            selected_channel: "aberto".to_owned(),
            me: "eu".to_owned(),
            my_name: "Eu".to_owned(),
            ..Store::default()
        };
        store.channels = vec![
            Channel {
                id: "aberto".to_owned(),
                name: "Aberto".to_owned(),
                kind: ChannelKind::Text,
                topic: None,
                position: 0,
                permissions: Vec::new(),
                notification_settings: "only_mentions".to_owned(),
                parent_id: None,
                unread: false,
                mentions: 0,
            },
            Channel {
                id: "outro".to_owned(),
                name: "Outro".to_owned(),
                kind: ChannelKind::Text,
                topic: None,
                position: 1,
                permissions: Vec::new(),
                notification_settings: "only_mentions".to_owned(),
                parent_id: None,
                unread: false,
                mentions: 0,
            },
        ];
        store.apply(Update::Connection(Connection::Online));
        let ticket = store.mark_loading("outro");
        finish_snapshot(&mut store, ticket, Vec::new());
        store
    }

    fn mensagem_convertida(id: &str, channel_id: &str, content: &str) -> Message {
        convert(wire_message(id, channel_id, content), "eu")
    }

    #[test]
    fn live_aplica_unread_e_mencao_uma_vez() {
        let mut store = store_para_proveniencia();
        let message = wire_message("m-live", "outro", "<@eu> oi");

        store.apply(Update::Event(Box::new(Event::Message(Box::new(message.clone())))));
        store.apply(Update::Event(Box::new(Event::Message(Box::new(message)))));

        let channel = store.channel("outro").expect("canal existe");
        assert!(channel.unread);
        assert_eq!(channel.mentions, 1);
        assert_eq!(
            store
                .messages
                .iter()
                .filter(|message| message.id == "m-live")
                .count(),
            1
        );
    }

    #[test]
    fn reconcile_converge_sem_parecer_atividade_nova() {
        let mut store = store_para_proveniencia();
        store.apply_mutation(
            MutationSource::Reconcile,
            StoreMutation::Timeline {
                channel_id: Some("outro".to_owned()),
                mutation: TimelineMutation::MessageUpsert(mensagem_convertida(
                    "m-reconcile",
                    "outro",
                    "<@eu> antigo",
                )),
            },
        );

        assert!(store.message("m-reconcile").is_some());
        let channel = store.channel("outro").expect("canal existe");
        assert!(!channel.unread);
        assert_eq!(channel.mentions, 0);
    }

    #[test]
    fn cache_restore_nao_notifica_nao_journaliza_nem_fica_fresh() {
        let mut store = store_para_proveniencia();
        perder_continuidade(&mut store);
        let _ticket = store.mark_loading("outro");
        assert_eq!(store.timeline_status("outro"), TimelineStatus::Refreshing);
        let journal_before = store
            .mutation_journals
            .get("outro")
            .map(|journal| journal.entries.len())
            .unwrap_or_default();

        store.apply_mutation(
            MutationSource::CacheRestore,
            StoreMutation::Timeline {
                channel_id: Some("outro".to_owned()),
                mutation: TimelineMutation::MessageUpsert(mensagem_convertida(
                    "m-cache",
                    "outro",
                    "<@eu> cache",
                )),
            },
        );

        assert!(store.message("m-cache").is_some());
        let channel = store.channel("outro").expect("canal existe");
        assert!(!channel.unread);
        assert_eq!(channel.mentions, 0);
        assert_eq!(store.timeline_status("outro"), TimelineStatus::Refreshing);
        assert_eq!(
            store
                .mutation_journals
                .get("outro")
                .map(|journal| journal.entries.len())
                .unwrap_or_default(),
            journal_before
        );
    }

    #[test]
    fn local_nao_parece_atividade_remota() {
        let mut store = store_para_proveniencia();
        let pending_id = store.push_pending("outro", "<@eu> local", None);

        assert!(store.message(&pending_id).is_some_and(|message| message.pending));
        let channel = store.channel("outro").expect("canal existe");
        assert!(!channel.unread);
        assert_eq!(channel.mentions, 0);
    }

    #[test]
    fn replay_reconcile_nao_rejournaliza_mutacao_live() {
        let mut store = store_com_mensagem("geral", "m1", "base");
        perder_continuidade(&mut store);
        let ticket = store.mark_loading("geral");
        let barrier = store
            .loading_channels
            .get("geral")
            .expect("refresh ativo")
            .barrier_revision;

        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "live".to_owned(),
        })));
        let before = store
            .mutation_journals
            .get("geral")
            .expect("journal existe")
            .entries
            .len();

        store.replay_timeline_mutations("geral", barrier);

        assert_eq!(
            store
                .mutation_journals
                .get("geral")
                .expect("journal existe")
                .entries
                .len(),
            before
        );
        assert_eq!(
            store.message("m1").map(|message| message.content.as_str()),
            Some("live")
        );

        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message("m1", "geral", "snapshot")],
        );
        assert_eq!(
            store.message("m1").map(|message| message.content.as_str()),
            Some("live")
        );
    }

    #[test]
    fn mutacoes_duplicadas_convergem_sem_deriva() {
        let mut store = store_com_mensagem("geral", "m1", "base");

        for _ in 0..2 {
            store.apply_mutation(
                MutationSource::Reconcile,
                StoreMutation::Timeline {
                    channel_id: Some("geral".to_owned()),
                    mutation: TimelineMutation::MessageEdit {
                        id: "m1".to_owned(),
                        content: "editado".to_owned(),
                    },
                },
            );
            store.apply_mutation(
                MutationSource::Reconcile,
                StoreMutation::Timeline {
                    channel_id: Some("geral".to_owned()),
                    mutation: TimelineMutation::MessagePinned {
                        message_id: "m1".to_owned(),
                        pinned: true,
                    },
                },
            );
            store.apply_mutation(
                MutationSource::Reconcile,
                StoreMutation::Timeline {
                    channel_id: Some("geral".to_owned()),
                    mutation: TimelineMutation::Reaction {
                        message_id: "m1".to_owned(),
                        emoji: Emoji::Unicode("👍".to_owned()),
                        count: 3,
                    },
                },
            );
        }

        let message = store.message("m1").expect("mensagem existe");
        assert_eq!(message.content, "editado");
        assert!(message.edited);
        assert!(message.pinned);
        assert_eq!(message.reactions.len(), 1);
        assert_eq!(message.reactions[0].count, 3);

        for _ in 0..2 {
            store.apply_mutation(
                MutationSource::Reconcile,
                StoreMutation::Timeline {
                    channel_id: Some("geral".to_owned()),
                    mutation: TimelineMutation::MessageDelete {
                        id: "m1".to_owned(),
                    },
                },
            );
        }
        assert!(store.message("m1").is_none());
    }

    #[test]
    fn rest_depois_ws_e_ws_depois_rest_convergem_em_uma_linha() {
        let mut rest_primeiro = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        rest_primeiro.apply(Update::Connection(Connection::Online));
        let ticket = rest_primeiro.mark_loading("geral");
        finish_snapshot(
            &mut rest_primeiro,
            ticket,
            vec![wire_message("m1", "geral", "rest")],
        );
        rest_primeiro.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("m1", "geral", "ws"),
        )))));
        assert_eq!(
            rest_primeiro
                .messages
                .iter()
                .filter(|message| message.id == "m1")
                .count(),
            1
        );
        assert_eq!(
            rest_primeiro.message("m1").map(|message| message.content.as_str()),
            Some("ws")
        );

        let mut ws_primeiro = store_com_mensagem("geral", "base", "base");
        perder_continuidade(&mut ws_primeiro);
        let ticket = ws_primeiro.mark_loading("geral");
        ws_primeiro.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("m1", "geral", "ws"),
        )))));
        finish_snapshot(
            &mut ws_primeiro,
            ticket,
            vec![wire_message("m1", "geral", "rest")],
        );
        assert_eq!(
            ws_primeiro
                .messages
                .iter()
                .filter(|message| message.id == "m1")
                .count(),
            1
        );
        assert_eq!(
            ws_primeiro.message("m1").map(|message| message.content.as_str()),
            Some("ws")
        );
    }

    #[test]
    fn http_confirm_then_ws_event_converges_once() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));
        let ticket = store.mark_loading("geral");
        finish_snapshot(&mut store, ticket, Vec::new());

        let pending = store.push_pending("geral", "oi", None);
        store.apply(Update::SendConfirmed {
            local_id: pending.clone(),
            message: Box::new(wire_message("m1", "geral", "oi")),
        });
        store.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("m1", "geral", "oi"),
        )))));

        assert!(store.message(&pending).is_none());
        assert!(store.messages.iter().all(|message| !message.pending));
        assert_eq!(
            store
                .messages
                .iter()
                .filter(|message| message.id == "m1")
                .count(),
            1
        );
    }

    #[test]
    fn ws_event_then_http_confirm_converges_once() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));
        let ticket = store.mark_loading("geral");
        finish_snapshot(&mut store, ticket, Vec::new());

        let pending = store.push_pending("geral", "oi", None);
        store.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("m1", "geral", "oi"),
        )))));
        store.apply(Update::SendConfirmed {
            local_id: pending.clone(),
            message: Box::new(wire_message("m1", "geral", "oi")),
        });

        assert!(store.message(&pending).is_none());
        assert_eq!(
            store
                .messages
                .iter()
                .filter(|message| message.id == "m1")
                .count(),
            1
        );
    }

    #[test]
    fn dismissing_one_outgoing_removes_only_that_local_echo() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            me: "me".to_owned(),
            ..Store::default()
        };
        let a = store.push_pending("geral", "a", None);
        let b = store.push_pending("geral", "b", None);

        store.apply(Update::OutgoingRemoved(a.clone()));

        assert!(store.message(&a).is_none());
        assert!(store.outgoing_state(&a).is_none());
        assert!(store.message(&b).is_some_and(|message| message.pending));
        assert_eq!(store.outgoing_state(&b), Some(OutgoingState::Queued));
    }

    #[test]
    fn confirming_one_local_id_does_not_clear_another() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            me: "me".to_owned(),
            ..Store::default()
        };
        let a = store.push_pending("geral", "hello", None);
        let b = store.push_pending("geral", "hello", None);
        assert_ne!(a, b);

        store.apply(Update::SendConfirmed {
            local_id: a.clone(),
            message: Box::new(wire_message("server-a", "geral", "hello")),
        });

        assert!(store.message(&a).is_none());
        assert!(store.message(&b).is_some_and(|message| message.pending));
        assert_eq!(store.outgoing_state(&b), Some(OutgoingState::Queued));
        assert_eq!(
            store
                .messages
                .iter()
                .filter(|message| message.id == "server-a")
                .count(),
            1
        );
    }

    #[test]
    fn fontes_de_stores_distintos_nao_vazam_efeitos() {
        let mut a = store_para_proveniencia();
        let mut b = store_para_proveniencia();

        a.apply_mutation(
            MutationSource::Live,
            StoreMutation::Timeline {
                channel_id: Some("outro".to_owned()),
                mutation: TimelineMutation::MessageUpsert(mensagem_convertida(
                    "m-a",
                    "outro",
                    "<@eu> a",
                )),
            },
        );
        b.apply_mutation(
            MutationSource::CacheRestore,
            StoreMutation::Timeline {
                channel_id: Some("outro".to_owned()),
                mutation: TimelineMutation::MessageUpsert(mensagem_convertida(
                    "m-b",
                    "outro",
                    "<@eu> b",
                )),
            },
        );

        assert_eq!(a.channel("outro").expect("canal").mentions, 1);
        assert_eq!(b.channel("outro").expect("canal").mentions, 0);
        assert!(a.message("m-b").is_none());
        assert!(b.message("m-a").is_none());
    }

    fn snapshot_de_cache() -> CachedServerSnapshot {
        CachedServerSnapshot {
            owner_user_id: Some("eu".to_owned()),
            server: Some(CachedServer {
                name: "Papo".to_owned(),
                description: None,
                owner_user_id: Some("eu".to_owned()),
                me_user_id: Some("eu".to_owned()),
                me_display_name: Some("Eu".to_owned()),
                me_username: Some("eu".to_owned()),
                updated_at: 0,
            }),
            channels: vec![CachedChannel {
                id: "geral".to_owned(),
                name: "Geral".to_owned(),
                kind: "text".to_owned(),
                topic: None,
                parent_id: None,
                position: 0,
                unread: false,
                mentions: 0,
            }],
            members: vec![CachedMember {
                id: "outro".to_owned(),
                username: "outro".to_owned(),
                name: "Outro".to_owned(),
                role_color: None,
                roles: Vec::new(),
            }],
            messages: vec![CachedMessage {
                id: "m-cached".to_owned(),
                channel_id: "geral".to_owned(),
                author_id: "outro".to_owned(),
                content: "do disco".to_owned(),
                created_at: 1_000,
                edited: false,
                reply_to: None,
                pinned: false,
                attachments: Vec::new(),
                reactions: Vec::new(),
            }],
            cached_channels: ["geral".to_owned()].into_iter().collect(),
        }
    }

    #[test]
    fn cache_restore_hidrata_sem_frescura_nem_eco() {
        let mut store = Store::default();
        store.restore_cached(snapshot_de_cache());

        assert_eq!(store.message("m-cached").expect("mensagem").content, "do disco");
        assert_eq!(store.server.as_ref().expect("servidor").name, "Papo");
        assert_eq!(store.me, "eu");
        // Cacheado não é fresco.
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Stale);
        assert_eq!(store.sync_generation(), 0);
        // Sem ticket, sem journal, sem operações devolvidas ao banco.
        assert!(!store.loading_channels.contains_key("geral"));
        assert!(store.take_cache_ops().is_empty());
        assert_eq!(store.channel("geral").expect("canal").mentions, 0);
        assert!(!store.channel("geral").expect("canal").unread);
    }

    #[test]
    fn canal_vazio_cacheado_nao_e_missing() {
        let mut snapshot = snapshot_de_cache();
        snapshot.messages.clear();
        snapshot.cached_channels = ["geral".to_owned()].into_iter().collect();
        let mut store = Store::default();
        store.restore_cached(snapshot);

        assert_eq!(store.timeline_status("geral"), TimelineStatus::Stale);
        assert_eq!(store.timeline_status("inexistente"), TimelineStatus::Missing);
    }

    #[test]
    fn restore_seguido_de_reconcile_converge_para_o_servidor() {
        let mut store = Store::default();
        store.restore_cached(snapshot_de_cache());
        store.apply(Update::Connection(Connection::Online));

        let ticket = store.mark_loading("geral");
        finish_snapshot(
            &mut store,
            ticket,
            vec![
                wire_message("m-cached", "geral", "atualizada"),
                wire_message("m-nova", "geral", "nova"),
            ],
        );

        assert_eq!(
            store.message("m-cached").expect("mensagem").content,
            "atualizada"
        );
        assert!(store.message("m-nova").is_some());
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Fresh);
    }

    #[test]
    fn persistencia_vem_so_de_live_e_reconcile() {
        let mut store = store_para_proveniencia();
        let _ = store.take_cache_ops();

        // CacheRestore não gera efeito.
        store.apply_mutation(
            MutationSource::CacheRestore,
            StoreMutation::Timeline {
                channel_id: Some("outro".to_owned()),
                mutation: TimelineMutation::MessageUpsert(mensagem_convertida(
                    "m-cache",
                    "outro",
                    "cache",
                )),
            },
        );
        assert!(store.take_cache_ops().is_empty());

        // Eco local pendente também não.
        store.push_pending("outro", "rascunho", None);
        assert!(store.take_cache_ops().is_empty());

        // Live persiste.
        store.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("m-live", "outro", "viva"),
        )))));
        let ops = store.take_cache_ops();
        assert!(
            ops.iter().any(|op| matches!(
                op,
                CacheOp::UpsertMessage(message) if message.id == "m-live"
            )),
            "live precisa persistir: {ops:?}"
        );
    }

    #[test]
    fn snapshot_reconciliado_gera_substituicao_e_edicao_resulta_em_upsert() {
        let mut store = store_com_mensagem("geral", "m1", "base");
        let _ = store.take_cache_ops();

        let ticket = store.mark_loading("geral");
        finish_snapshot(
            &mut store,
            ticket,
            vec![wire_message("m1", "geral", "reconciliada")],
        );
        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(
            op,
            CacheOp::ReplaceChannelSnapshot { channel_id, .. } if channel_id == "geral"
        )));

        store.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
            content: "editada".to_owned(),
        })));
        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(
            op,
            CacheOp::UpsertMessage(message)
                if message.id == "m1" && message.content == "editada" && message.edited
        )));

        store.apply(Update::Event(Box::new(Event::MessageDeleted {
            id: "m1".to_owned(),
            channel_id: "geral".to_owned(),
        })));
        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(
            op,
            CacheOp::DeleteMessage { message_id } if message_id == "m1"
        )));
    }

    #[test]
    fn sessao_marca_dono_e_expiracao_limpa_so_cache_reconstruivel() {
        let mut store = Store::default();
        store.apply(Update::Session(Some(Box::new(models::Whoami {
            id: "eu".to_owned(),
            username: "eu".to_owned(),
            nickname: None,
            status: None,
            status_message: None,
            roles: Vec::new(),
            settings: models::WhoamiSettings {
                version: 1,
                config: models::UserConfig::default(),
            },
        }))));
        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(
            op,
            CacheOp::SetOwner { owner_user_id, .. } if owner_user_id == "eu"
        )));

        store.apply(Update::Session(None));
        let ops = store.take_cache_ops();
        assert!(
            ops.iter()
                .any(|op| matches!(op, CacheOp::ClearCachedData)),
            "sessão encerrada limpa apenas estado reconstruível"
        );
        assert!(
            !ops.iter().any(|op| matches!(op, CacheOp::ClearServer)),
            "fila/ledger particionados por owner sobrevivem à expiração"
        );
    }

    #[test]
    fn sessao_retém_config_portatil_e_update_substitui_o_snapshot() {
        let mut store = Store::default();
        let initial = models::UserConfig {
            theme: "dark".to_owned(),
            ..Default::default()
        };
        store.apply(Update::Session(Some(Box::new(models::Whoami {
            id: "eu".to_owned(),
            username: "eu".to_owned(),
            nickname: None,
            status: None,
            status_message: None,
            roles: Vec::new(),
            settings: models::WhoamiSettings {
                version: 1,
                config: initial.clone(),
            },
        }))));
        assert_eq!(
            store.user_settings.as_ref().map(|settings| &settings.config),
            Some(&initial)
        );

        let mut updated = initial;
        updated.theme = "light".to_owned();
        updated.notifications.enabled = false;
        store.apply(Update::UserSettings(Box::new(models::UserSettings {
            user_id: "eu".to_owned(),
            version: 1,
            config: updated.clone(),
            updated_at: None,
        })));
        assert_eq!(
            store.user_settings.as_ref().map(|settings| &settings.config),
            Some(&updated)
        );
    }

    #[test]
    fn pins_autoritativos_persistem_mesmo_fora_da_store() {
        let mut store = store_com_mensagem("geral", "m1", "base");
        let _ = store.take_cache_ops();

        // Um id que a Store não carregou ainda assim precisa convergir no
        // banco: é o snapshot autoritativo que manda.
        store.apply(Update::Pinned {
            channel_id: "geral".to_owned(),
            ids: vec!["m1".to_owned(), "fantasma".to_owned()],
        });
        let ops = store.take_cache_ops();
        let pins = ops.iter().find_map(|op| match op {
            CacheOp::ReplacePins { channel_id, ids } => Some((channel_id, ids)),
            _ => None,
        });
        let (channel_id, ids) = pins.expect("snapshot de pins precisa ir para o cache");
        assert_eq!(channel_id, "geral");
        assert!(ids.contains(&"fantasma".to_owned()));
    }

    #[test]
    fn refresh_parcial_preserva_historico_abaixo_da_janela_autoritativa() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));

        let base = Utc::now();
        let mut old = wire_message("old", "geral", "old");
        old.created_at = base - chrono::Duration::minutes(10);
        let mut stale = wire_message("stale", "geral", "stale");
        // Fica dentro da janela autoritativa do head (entre keep e new),
        // portanto sua ausência na resposta significa exclusão.
        stale.created_at = base + chrono::Duration::seconds(30);
        let mut keep = wire_message("keep", "geral", "old value");
        keep.created_at = base;

        let me = store.me.clone();
        store.messages = vec![
            convert(old, &me),
            convert(stale, &me),
            convert(keep, &me),
        ];

        let ticket = store.mark_loading("geral");
        let mut keep_new = wire_message("keep", "geral", "new value");
        keep_new.created_at = base;
        let mut newest = wire_message("new", "geral", "new");
        newest.created_at = base + chrono::Duration::minutes(1);
        store.apply(Update::Messages {
            ticket,
            messages: vec![newest, keep_new],
            has_more: true,
            pinned_ids: Some(Vec::new()),
        });

        assert!(store.message("old").is_some(), "histórico mais antigo deve sobreviver");
        assert!(store.message("stale").is_none(), "linha ausente dentro do head deve sair");
        assert_eq!(
            store.message("keep").map(|message| message.content.as_str()),
            Some("new value")
        );
        assert!(store.message("new").is_some());

        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(
            op,
            CacheOp::MergeChannelHead { channel_id, deleted_ids, .. }
                if channel_id == "geral" && deleted_ids == &vec!["stale".to_owned()]
        )));
        assert!(
            !ops.iter().any(|op| matches!(op, CacheOp::ReplaceChannelSnapshot { .. })),
            "refresh parcial não pode achatar o cache inteiro"
        );
    }

    #[test]
    fn busca_paginada_anexa_sem_duplicar() {
        let mut store = Store::default();
        let at = Utc::now();
        let result = |id: &str| models::SearchResult {
            kind: "message".to_owned(),
            id: id.to_owned(),
            content: id.to_owned(),
            channel_id: "geral".to_owned(),
            channel_name: "Geral".to_owned(),
            author_id: Some("u1".to_owned()),
            author_username: Some("ana".to_owned()),
            created_at: Some(at),
            attachments: Vec::new(),
        };

        store.apply(Update::SearchResults {
            results: vec![result("a"), result("b")],
            has_more: true,
            append: false,
        });
        store.apply(Update::SearchResults {
            results: vec![result("b"), result("c")],
            has_more: false,
            append: true,
        });

        let ids: Vec<_> = store.search_results.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert!(!store.search_has_more);
        assert!(!store.searching);
    }

    #[test]
    fn detalhes_de_reacao_mesclam_paginas_e_cursor_pega_a_mais_antiga() {
        let mut store = Store::default();
        let now = Utc::now();
        let group = |users: Vec<models::ReactionUser>| models::ReactionGroup {
            emoji_id: None,
            unicode: Some("👍".to_owned()),
            count: users.len() as u32,
            users,
        };
        let user = |id: &str, seconds: i64| models::ReactionUser {
            id: id.to_owned(),
            user_id: format!("user-{id}"),
            created_at: now - chrono::Duration::seconds(seconds),
        };

        assert!(store.begin_reaction_details("m1"));
        store.apply(Update::ReactionDetails {
            message_id: "m1".to_owned(),
            reactions: vec![group(vec![user("r1", 1), user("r2", 2)])],
            has_more: true,
            append: false,
        });
        store.apply(Update::ReactionDetails {
            message_id: "m1".to_owned(),
            reactions: vec![group(vec![user("r2", 2), user("r3", 3)])],
            has_more: false,
            append: true,
        });

        let users = &store.reaction_details["m1"][0].users;
        assert_eq!(users.len(), 3);
        let cursor = store.reaction_details_cursor("m1").expect("cursor");
        assert_eq!(cursor.1, "r3");
        assert!(!store.reaction_details_has_more["m1"]);
        assert!(!store.reaction_details_loading.contains("m1"));
    }

    #[test]
    fn pagina_antiga_mescla_sem_substituir_e_fecha_no_fim() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));

        let now = Utc::now();
        let mut newest = wire_message("nova", "geral", "nova");
        newest.created_at = now;
        let mut oldest = wire_message("antiga", "geral", "antiga");
        oldest.created_at = now - chrono::Duration::minutes(1);

        let ticket = store.mark_loading("geral");
        store.apply(Update::Messages {
            ticket,
            messages: vec![newest, oldest],
            has_more: true,
            pinned_ids: Some(Vec::new()),
        });

        assert!(store.can_load_older("geral"));
        let (since, last_id) = store
            .begin_load_older("geral")
            .expect("cursor da mensagem mais antiga");
        assert_eq!(last_id, "antiga");
        assert_eq!(since, now - chrono::Duration::minutes(1));
        assert!(!store.can_load_older("geral"));

        let mut older = wire_message("mais-antiga", "geral", "mais antiga");
        older.created_at = now - chrono::Duration::minutes(2);
        store.apply(Update::OlderMessages {
            channel_id: "geral".to_owned(),
            messages: vec![older],
            has_more: false,
        });

        let ids: Vec<_> = store
            .messages_in("geral")
            .map(|message| message.id.as_str())
            .collect();
        assert_eq!(ids, vec!["mais-antiga", "antiga", "nova"]);
        assert!(!store.can_load_older("geral"));
    }

    #[test]
    fn snapshot_de_mensagens_so_converge_pins_quando_autoritativo() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));

        // `Some` é autoritativo: emite ReplacePins.
        let ticket = store.mark_loading("geral");
        let _ = store.take_cache_ops();
        store.apply(Update::Messages {
            ticket,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: Some(vec!["x".to_owned()]),
        });
        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(
            op,
            CacheOp::ReplacePins { ids, .. } if ids == &vec!["x".to_owned()]
        )));

        // `None` (busca falhou): preserva o que já estava no disco.
        let ticket = store.mark_loading("geral");
        let _ = store.take_cache_ops();
        store.apply(Update::Messages {
            ticket,
            messages: Vec::new(),
            has_more: false,
            pinned_ids: None,
        });
        let ops = store.take_cache_ops();
        assert!(
            !ops.iter().any(|op| matches!(op, CacheOp::ReplacePins { .. })),
            "falha ao buscar pins não pode zerar o cache"
        );
    }
}
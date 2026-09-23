//! Estado da aplicação, alimentado pelas respostas REST e pelos eventos do
//! WebSocket.

pub mod call;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Local, Utc};
use crate::api::models::{self, parse_hex_color, Attachment};
use crate::api::net::{RefreshTicket, Update};
use crate::api::ws::{Connection, Event};
use crate::cache::{
    now_millis, CachedChannel, CachedMember, CachedMessage, CachedServer, CachedServerSnapshot,
    CacheOp,
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
    pub role_color: Option<[u8; 3]>,
    /// Ids dos cargos que a pessoa tem; é o que a tela de cargos marca.
    pub roles: Vec<String>,
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
    },
    ConfirmSent(Message),
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
    pub selected_channel: String,
    /// Geração da continuidade atual do WebSocket.
    sync_generation: u64,
    /// Última geração em que cada canal recebeu uma carga REST autoritativa.
    channel_freshness: HashMap<String, u64>,
    /// Canais com cache em disco ainda não reconciliados nesta geração. Um
    /// snapshot vazio também entra aqui: cacheado e "nunca carregado" são
    /// coisas diferentes.
    cached_channels: HashSet<String>,
    /// Efeitos de cache pendentes, drenados pelo coordenador de persistência.
    pending_cache: Vec<CacheOp>,
    /// Refresh aceito atualmente por canal, incluindo o barrier local.
    loading_channels: HashMap<String, ActiveRefresh>,
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
    /// Sessões abertas da conta neste servidor.
    pub devices: Vec<models::ConnectionInfo>,
    pub audit_logs: Vec<models::AuditLogEntry>,
    /// Última resposta da busca, do servidor na tela.
    pub search_results: Vec<models::SearchResult>,
    /// Uma busca saiu e ainda não voltou.
    pub searching: bool,
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
            selected_channel: String::new(),
            sync_generation: 0,
            channel_freshness: HashMap::new(),
            cached_channels: HashSet::new(),
            pending_cache: Vec::new(),
            loading_channels: HashMap::new(),
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
            devices: Vec::new(),
            audit_logs: Vec::new(),
            search_results: Vec::new(),
            searching: false,
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

    /// Hidrata a Store a partir do cache em disco, antes de a rede começar.
    ///
    /// É deliberadamente sem efeitos colaterais: não soma não lidos nem
    /// menções novas, não notifica, não abre tickets, não mexe na geração e
    /// não devolve operações de persistência — senão o restore viraria eco.
    pub fn restore_cached(&mut self, snapshot: CachedServerSnapshot) {
        if let Some(server) = &snapshot.server {
            self.me = server.me_user_id.clone().unwrap_or_default();
            self.my_name = server.me_display_name.clone().unwrap_or_default();
            self.my_username = server.me_username.clone().unwrap_or_default();
            self.server = Some(Server {
                name: server.name.clone(),
                description: server.description.clone(),
            });
        }

        self.channels = snapshot
            .channels
            .into_iter()
            .map(|channel| Channel {
                id: channel.id,
                name: channel.name,
                kind: ChannelKind::parse(&channel.kind),
                topic: channel.topic,
                position: channel.position,
                unread: channel.unread,
                mentions: channel.mentions,
            })
            .collect();
        self.channels.sort_by_key(|channel| channel.position);

        // Presença lida do disco é sempre velha; ninguém é "online" só por
        // causa dela.
        self.members = snapshot
            .members
            .into_iter()
            .map(|member| Member {
                id: member.id,
                username: member.username,
                name: member.name,
                presence: Presence::Offline,
                role_color: member.role_color,
                roles: member.roles,
            })
            .collect();

        self.messages = snapshot
            .messages
            .into_iter()
            .map(|message| message.to_store())
            .collect();
        self.sort_messages();

        self.cached_channels = snapshot.cached_channels;
        for message in &self.messages {
            self.cached_channels.insert(message.channel_id.clone());
        }

        if self.selected_channel.is_empty()
            && let Some(first) = self
                .channels
                .iter()
                .find(|channel| channel.kind == ChannelKind::Text)
        {
            self.selected_channel = first.id.clone();
        }

        if !self.channels.is_empty() || !self.messages.is_empty() {
            self.screen = Screen::Chat;
            self.connection = Connection::Offline;
        }

        self.pending_cache.clear();
    }

    /// Esvazia o estado vindo do cache. Usado quando a conta verificada não é
    /// a dona do cache — nunca deixar a conversa de um usuário aparecer para
    /// outro.
    pub fn clear_cached_state(&mut self) {
        self.server = None;
        self.channels.clear();
        self.members.clear();
        self.messages.clear();
        self.cached_channels.clear();
        self.pending_cache.clear();
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

        StoreDiagnostics {
            sync_generation: self.sync_generation,
            selected_channel: self.selected_channel.clone(),
            timelines,
        }
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
            } => {
                // O snapshot autoritativo substitui apenas estado confirmado
                // pelo servidor. Ecos locais continuam sendo outra projeção.
                self.messages
                    .retain(|message| message.channel_id != channel_id || message.pending);
                for message in messages {
                    self.upsert_message(message);
                }
                self.sort_messages();

                if source == MutationSource::Reconcile {
                    let confirmed: Vec<CachedMessage> = self
                        .messages
                        .iter()
                        .filter(|message| message.channel_id == channel_id && !message.pending)
                        .map(CachedMessage::from_store)
                        .collect();
                    self.pending_cache
                        .push(CacheOp::ReplaceChannelSnapshot {
                            channel_id,
                            messages: confirmed,
                            cached_at: now_millis(),
                        });
                }
            }
            StoreMutation::ConfirmSent(message) => {
                // O backend não fornece transaction/local-id ainda. Mantemos
                // a semântica atual: uma confirmação limpa ecos pendentes e
                // converge pela mesma implementação de upsert usada no resto.
                self.messages.retain(|existing| !existing.pending);
                let cached = CachedMessage::from_store(&message);
                self.apply_timeline_projection(TimelineMutation::MessageUpsert(message));
                self.pending_cache.push(CacheOp::UpsertMessage(cached));
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
        if message.author_id == self.me {
            return false;
        }
        let content = message.content.to_lowercase();
        (!self.me.is_empty() && content.contains(&format!("<@{}>", self.me.to_lowercase())))
            || (!self.my_name.is_empty()
                && content.contains(&format!("@{}", self.my_name.to_lowercase())))
            || content.contains("@everyone")
            || content.contains("@todos")
    }

    // -- Atualizações ------------------------------------------------------

    /// Aplica uma atualização vinda da rede.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Session(Some(me)) => {
                self.me = me.id.clone();
                self.my_name = me.display_name().to_owned();
                self.my_username = me.username.clone();
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
                // Sessão inválida: o cache deste usuário não deve sobreviver.
                self.pending_cache.push(CacheOp::ClearServer);
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
            // A foto chega em base64 junto com o perfil; guardá-la aqui deixa
            // a lista de pessoas desenhá-la sem pedir de novo.
            Update::Profiles(profiles) => {
                for profile in profiles {
                    match profile.avatar_blob.filter(|blob| !blob.is_empty()) {
                        Some(blob) => {
                            self.avatars.insert(profile.id, blob);
                        }
                        None => {
                            self.avatars.remove(&profile.id);
                        }
                    }
                }
            }
            Update::Devices(devices) => {
                self.devices = devices;
                self.busy = false;
            }
            Update::AuditLogs(logs) => {
                self.audit_logs = logs;
                self.busy = false;
            }
            Update::Done => self.busy = false,
            Update::SearchResults(results) => {
                self.search_results = results;
                self.searching = false;
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
                    .map(|user| Member {
                        presence: presence
                            .get(&user.id)
                            .copied()
                            .unwrap_or(Presence::Offline),
                        role_color: role_color(&user.roles),
                        roles: user.roles.iter().map(|role| role.id.clone()).collect(),
                        name: user.display_name().to_owned(),
                        username: user.username,
                        id: user.id,
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
            Update::Sent(message) => {
                let me = self.me.clone();
                self.apply_mutation(
                    MutationSource::Reconcile,
                    StoreMutation::ConfirmSent(convert(*message, &me)),
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
                nickname,
            } => {
                if let Some(member) = self
                    .members
                    .iter_mut()
                    .find(|member| member.id == user_id)
                {
                    member.presence = Presence::parse(&status);
                    if let Some(nickname) = nickname {
                        member.name = nickname;
                    }
                }
                self.sort_members();
            }
            Event::PresenceSync(members) => {
                let online: HashMap<String, Presence> = members
                    .into_iter()
                    .map(|(id, status)| (id, Presence::parse(&status)))
                    .collect();
                for member in &mut self.members {
                    member.presence = online
                        .get(&member.id)
                        .copied()
                        .unwrap_or(Presence::Offline);
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
            Event::UserJoined { .. } => {}
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

    /// Mensagem otimista: aparece antes da confirmação do servidor.
    pub fn push_pending(
        &mut self,
        channel_id: &str,
        content: &str,
        reply_to: Option<String>,
    ) -> String {
        let id = format!("pending-{}", self.messages.len());
        let message = Message {
            id: id.clone(),
            channel_id: channel_id.to_owned(),
            author_id: self.me.clone(),
            content: content.to_owned(),
            at: Local::now(),
            edited: false,
            reply_to,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: true,
        };
        self.apply_mutation(
            MutationSource::Local,
            StoreMutation::Timeline {
                channel_id: Some(channel_id.to_owned()),
                mutation: TimelineMutation::MessageUpsert(message),
            },
        );
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
        store.apply(Update::Messages { ticket, messages: Vec::new(), pinned_ids: Some(Vec::new()) });
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

        store.apply(Update::Messages { ticket: antiga, messages: Vec::new(), pinned_ids: Some(Vec::new()) });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Refreshing);
        store.apply(Update::Messages { ticket: atual, messages: Vec::new(), pinned_ids: Some(Vec::new()) });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Fresh);
    }

    #[test]
    fn resposta_antiga_da_mesma_geracao_nao_supersede_tentativa_nova() {
        let mut store = Store { selected_channel: "geral".to_owned(), ..Store::default() };
        store.apply(Update::Connection(Connection::Online));
        let primeira = store.mark_loading("geral");
        let segunda = store.mark_loading("geral");
        assert_ne!(primeira.request_id, segunda.request_id);

        store.apply(Update::Messages { ticket: primeira, messages: Vec::new(), pinned_ids: Some(Vec::new()) });
        assert_eq!(store.timeline_status("geral"), TimelineStatus::Refreshing);
        store.apply(Update::Messages { ticket: segunda, messages: Vec::new(), pinned_ids: Some(Vec::new()) });
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
        store.apply(Update::Messages { ticket: atual, messages: Vec::new(), pinned_ids: Some(Vec::new()) });
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
                unread: false,
                mentions: 0,
            },
            Channel {
                id: "outro".to_owned(),
                name: "Outro".to_owned(),
                kind: ChannelKind::Text,
                topic: None,
                position: 1,
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
    fn sent_e_evento_live_da_mesma_mensagem_nao_duplicam() {
        let mut store = Store {
            selected_channel: "geral".to_owned(),
            ..Store::default()
        };
        store.apply(Update::Connection(Connection::Online));
        let ticket = store.mark_loading("geral");
        finish_snapshot(&mut store, ticket, Vec::new());

        let pending = store.push_pending("geral", "oi", None);
        assert!(store.message(&pending).is_some());

        store.apply(Update::Sent(Box::new(wire_message("m1", "geral", "oi"))));
        store.apply(Update::Event(Box::new(Event::Message(Box::new(
            wire_message("m1", "geral", "oi"),
        )))));

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
    fn sessao_marca_dono_e_logout_limpa_cache() {
        let mut store = Store::default();
        store.apply(Update::Session(Some(Box::new(models::Whoami {
            id: "eu".to_owned(),
            username: "eu".to_owned(),
            nickname: None,
            status: None,
            status_message: None,
            roles: Vec::new(),
        }))));
        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(
            op,
            CacheOp::SetOwner { owner_user_id, .. } if owner_user_id == "eu"
        )));

        store.apply(Update::Session(None));
        let ops = store.take_cache_ops();
        assert!(ops.iter().any(|op| matches!(op, CacheOp::ClearServer)));
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
            pinned_ids: None,
        });
        let ops = store.take_cache_ops();
        assert!(
            !ops.iter().any(|op| matches!(op, CacheOp::ReplacePins { .. })),
            "falha ao buscar pins não pode zerar o cache"
        );
    }
}
//! Estado da aplicação, alimentado pelas respostas REST e pelos eventos do
//! WebSocket.

pub mod call;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, Utc};
use crate::api::models::{self, parse_hex_color, Attachment};
use crate::api::net::Update;
use crate::api::ws::{Connection, Event};

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
    /// Canais cuja carga terminou nesta geração contínua da conexão.
    ///
    /// O cache só é válido enquanto o WebSocket permanece na mesma geração:
    /// qualquer transição de conexão invalida esta lista. As mensagens já
    /// renderizadas ficam na memória até a carga nova substituí-las.
    loaded_channels: HashSet<String>,
    /// Cargas REST em voo. Separar "carregando" de "carregado" evita que uma
    /// falha HTTP transforme uma tentativa em cache fresco para sempre.
    loading_channels: HashSet<String>,
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
            loaded_channels: HashSet::new(),
            loading_channels: HashSet::new(),
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

    /// Canal que ainda precisa ter as mensagens buscadas.
    pub fn channel_needing_messages(&self) -> Option<String> {
        // Não marque uma tentativa feita durante a queda como "carregada".
        // Quando o socket voltar para Online a geração é invalidada de novo
        // e o canal aberto será buscado pela fonte REST.
        if self.connection != Connection::Online {
            return None;
        }
        let id = &self.selected_channel;
        if id.is_empty()
            || self.loaded_channels.contains(id)
            || self.loading_channels.contains(id)
        {
            return None;
        }
        // Canal de voz não tem mensagem: pedir a lista dele seria uma
        // chamada por entrada na call, para receber nada.
        if self.channel(id).is_some_and(|channel| channel.kind == ChannelKind::Voice) {
            return None;
        }
        Some(id.clone())
    }

    pub fn mark_loading(&mut self, channel_id: &str) {
        self.loading_channels.insert(channel_id.to_owned());
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

    /// A mensagem cita você? Menção direta ou chamado geral.
    pub fn mentions_me(&self, message: &Message) -> bool {
        if self.my_username.is_empty() || message.author_id == self.me {
            return false;
        }
        let content = message.content.to_lowercase();
        content.contains(&format!("@{}", self.my_username.to_lowercase()))
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
            }
            Update::Session(None) => {
                let was = std::mem::take(self);
                self.screen = Screen::Auth;
                self.error = was.error;
                self.read_marks = was.read_marks;
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
            }
            Update::Messages {
                channel_id,
                messages,
            } => {
                self.messages
                    .retain(|message| message.channel_id != channel_id);
                let me = self.me.clone();
                self.messages
                    .extend(messages.into_iter().map(|message| convert(message, &me)));
                self.sort_messages();
                self.loading_channels.remove(&channel_id);
                self.loaded_channels.insert(channel_id);
            }
            Update::MessagesFailed(channel_id) => {
                self.loading_channels.remove(&channel_id);
            }
            Update::Sent(message) => {
                self.messages.retain(|existing| !existing.pending);
                let me = self.me.clone();
                self.upsert(convert(*message, &me));
            }
            Update::Edited(message) => {
                let me = self.me.clone();
                let updated = convert(*message, &me);
                if let Some(existing) = self
                    .messages
                    .iter_mut()
                    .find(|existing| existing.id == updated.id)
                {
                    let pinned = existing.pinned;
                    *existing = updated;
                    existing.pinned = pinned;
                }
            }
            Update::Deleted(id) => {
                self.messages.retain(|message| message.id != id);
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
                let pinned: HashSet<String> = ids.into_iter().collect();
                for message in self
                    .messages
                    .iter_mut()
                    .filter(|message| message.channel_id == channel_id)
                {
                    message.pinned = pinned.contains(&message.id);
                }
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
                // loaded_channels só vale para uma geração contínua do
                // WebSocket. Se a conexão muda de estado, algum evento pode
                // ter sido perdido; mantemos as mensagens velhas visíveis,
                // mas obrigamos uma carga REST assim que Online voltar.
                if self.connection != connection {
                    self.loaded_channels.clear();
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
                // Só guardamos mensagens de canais já carregados; os outros
                // são buscados por inteiro quando abertos.
                if self.loaded_channels.contains(&message.channel_id) {
                    self.messages.retain(|existing| !existing.pending);
                    self.upsert(message.clone());
                }
                let mention = self.mentions_me(&message);
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
                if let Some(users) = self.typing.get_mut(&message.channel_id) {
                    users.remove(&message.author_id);
                }
            }
            Event::MessageEdited {
                id,
                channel_id: _,
                content,
            } => {
                if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
                    message.content = content;
                    message.edited = true;
                }
            }
            Event::MessageDeleted { id, .. } => {
                self.messages.retain(|message| message.id != id);
            }
            Event::MessagePinned {
                message_id,
                pinned,
            } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    message.pinned = pinned;
                }
            }
            Event::NewPreview { .. } => {
                // O worker de rede resolve new_preview para LinkPreviewUpdated
                // antes de publicar o evento para a Store.
            }
            Event::RemovePreview {
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
            Event::LinkPreviewUpdated {
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
            Event::AttachmentModeration {
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
        }
    }

    /// Reage na hora, sem esperar o servidor: o contador certo chega pelo
    /// evento `react_update`.
    pub fn toggle_reaction_local(&mut self, message_id: &str, emoji: &Emoji) -> bool {
        let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        else {
            return false;
        };
        match message
            .reactions
            .iter_mut()
            .find(|reaction| &reaction.emoji == emoji)
        {
            Some(reaction) if reaction.mine => {
                reaction.mine = false;
                reaction.count = reaction.count.saturating_sub(1);
                message.reactions.retain(|reaction| reaction.count > 0);
                false
            }
            Some(reaction) => {
                reaction.mine = true;
                reaction.count += 1;
                true
            }
            None => {
                message.reactions.push(Reaction {
                    emoji: emoji.clone(),
                    count: 1,
                    mine: true,
                });
                true
            }
        }
    }

    /// Mensagem otimista: aparece antes da confirmação do servidor.
    pub fn push_pending(&mut self, channel_id: &str, content: &str, reply_to: Option<String>) {
        self.messages.push(Message {
            id: format!("pending-{}", self.messages.len()),
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
        });
    }

    fn upsert(&mut self, message: Message) {
        match self
            .messages
            .iter_mut()
            .find(|existing| existing.id == message.id)
        {
            Some(existing) => *existing = message,
            None => {
                self.messages.push(message);
                self.sort_messages();
            }
        }
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

    fn store_com_canal_carregado(channel_id: &str) -> Store {
        let mut store = Store::default();
        store.selected_channel = channel_id.to_owned();
        store.apply(Update::Connection(Connection::Online));
        assert_eq!(
            store.channel_needing_messages(),
            Some(channel_id.to_owned())
        );
        store.mark_loading(channel_id);
        store.apply(Update::Messages {
            channel_id: channel_id.to_owned(),
            messages: Vec::new(),
        });
        assert_eq!(store.channel_needing_messages(), None);
        store
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
        let mut store = Store::default();
        store.selected_channel = "geral".to_owned();

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
        let mut store = Store::default();
        store.selected_channel = "geral".to_owned();
        store.apply(Update::Connection(Connection::Online));

        assert_eq!(
            store.channel_needing_messages(),
            Some("geral".to_owned())
        );
        store.mark_loading("geral");
        assert_eq!(store.channel_needing_messages(), None);

        store.apply(Update::MessagesFailed("geral".to_owned()));
        assert_eq!(
            store.channel_needing_messages(),
            Some("geral".to_owned())
        );
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
}


//! Estado da aplicação, alimentado pelas respostas REST e pelos eventos do
//! WebSocket.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, Utc};
use egui::Color32;

use crate::api::models::{self, parse_hex_color};
use crate::api::net::Update;
use crate::api::ws::{Connection, Event};

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
    pub unread: bool,
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
    pub name: String,
    pub presence: Presence,
    pub role_color: Option<Color32>,
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

#[derive(Clone, Debug)]
pub struct Reaction {
    pub emoji: String,
    pub count: u32,
    pub mine: bool,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub content: String,
    pub at: DateTime<Local>,
    pub edited: bool,
    pub reactions: Vec<Reaction>,
    #[allow(dead_code)] // a tela de fixadas usa isto
    pub pinned: bool,
    /// Mensagem ainda não confirmada pelo servidor.
    pub pending: bool,
}

#[derive(Clone, Debug)]
pub struct Server {
    pub name: String,
    pub description: Option<String>,
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
    pub me: String,
    pub my_name: String,
    pub selected_channel: String,
    /// Canais já carregados, para não repetir a busca a cada troca.
    loaded_channels: HashSet<String>,
    /// Quem está digitando, por canal.
    typing: HashMap<String, HashSet<String>>,
    pub error: Option<String>,
    pub busy: bool,
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
            me: String::new(),
            my_name: String::new(),
            selected_channel: String::new(),
            loaded_channels: HashSet::new(),
            typing: HashMap::new(),
            error: None,
            busy: false,
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
        let id = &self.selected_channel;
        (!id.is_empty() && !self.loaded_channels.contains(id)).then(|| id.clone())
    }

    pub fn mark_loading(&mut self, channel_id: &str) {
        self.loaded_channels.insert(channel_id.to_owned());
    }

    /// Aplica uma atualização vinda da rede.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Session(Some(me)) => {
                self.me = me.id.clone();
                self.my_name = me.display_name().to_owned();
                self.screen = Screen::Chat;
                self.error = None;
                self.busy = false;
            }
            Update::Session(None) => {
                let was = std::mem::take(self);
                self.screen = Screen::Auth;
                self.error = was.error;
            }
            Update::AuthFailed(message) => {
                self.screen = Screen::Auth;
                self.error = Some(message);
                self.busy = false;
            }
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
                self.channels = channels
                    .into_iter()
                    .filter(|channel| channel.kind != "category")
                    .map(|channel| Channel {
                        unread: channel.last_read_message.is_none()
                            && channel.last_message.is_some(),
                        id: channel.id,
                        name: channel.name,
                        kind: ChannelKind::parse(&channel.kind),
                        topic: channel.topic,
                        position: channel.position,
                        mentions: 0,
                    })
                    .collect();
                self.channels.sort_by_key(|channel| channel.position);
                if self.selected_channel.is_empty() {
                    if let Some(first) = self
                        .channels
                        .iter()
                        .find(|channel| channel.kind == ChannelKind::Text)
                    {
                        self.selected_channel = first.id.clone();
                    }
                }
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
                        name: user.display_name().to_owned(),
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
                self.messages.extend(messages.into_iter().map(convert));
                self.sort_messages();
                self.loaded_channels.insert(channel_id);
            }
            Update::Sent(message) => {
                self.messages.retain(|existing| !existing.pending);
                self.upsert(convert(*message));
            }
            Update::Event(event) => self.apply_event(*event),
            Update::Connection(connection) => self.connection = connection,
            Update::Error(message) => {
                self.error = Some(message);
                self.busy = false;
            }
        }
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Message(message) => {
                let message = convert(*message);
                // Só guardamos mensagens de canais já carregados; os outros
                // são buscados por inteiro quando abertos.
                if self.loaded_channels.contains(&message.channel_id) {
                    self.messages.retain(|existing| !existing.pending);
                    self.upsert(message.clone());
                }
                if message.channel_id != self.selected_channel {
                    if let Some(channel) = self
                        .channels
                        .iter_mut()
                        .find(|channel| channel.id == message.channel_id)
                    {
                        channel.unread = true;
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
            Event::UserJoined { .. } | Event::Notification { .. } => {}
            Event::Reaction {
                message_id,
                emoji,
                count,
            } => {
                let Some(emoji) = emoji else { return };
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
                        Some(reaction) if count <= 0 => {
                            reaction.count = 0;
                        }
                        Some(reaction) => reaction.count = count as u32,
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

    /// Mensagem otimista: aparece antes da confirmação do servidor.
    pub fn push_pending(&mut self, channel_id: &str, content: &str) {
        self.messages.push(Message {
            id: format!("pending-{}", self.messages.len()),
            channel_id: channel_id.to_owned(),
            author_id: self.me.clone(),
            content: content.to_owned(),
            at: Local::now(),
            edited: false,
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
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }
}

fn convert(message: models::Message) -> Message {
    Message {
        id: message.id,
        channel_id: message.channel_id,
        author_id: message.author_id,
        content: message.content.unwrap_or_default(),
        at: DateTime::<Utc>::from(message.created_at).with_timezone(&Local),
        edited: message.edited_at.is_some(),
        reactions: message
            .reactions
            .into_iter()
            .map(|reaction| Reaction {
                emoji: reaction.emoji,
                count: reaction.count,
                mine: reaction.me,
            })
            .collect(),
        pinned: false,
        pending: false,
    }
}

/// Cor do cargo mais alto que define uma.
fn role_color(roles: &[models::RoleSummary]) -> Option<Color32> {
    roles
        .iter()
        .filter(|role| role.color.is_some())
        .max_by_key(|role| role.position)
        .and_then(|role| role.color.as_deref())
        .and_then(parse_hex_color)
}

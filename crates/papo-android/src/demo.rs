//! Explicit, ephemeral demo. Uses the real core reducer, creates no sessions,
//! opens no sockets and cannot send anything to a real server.
use super::*;
use papo_core::api::{models, ws::Event};
use papo_core::state::Store;

pub struct Demo {
    pub(crate) store: Store,
    pub(crate) notifications: Vec<models::Notification>,
    events: Vec<Value>,
    media_files: HashMap<String, PathBuf>,
    navigation_serial: u64,
    navigation_target: String,
    archived: models::Message,
    search_query: String,
}
impl Drop for Demo {
    fn drop(&mut self) {
        for path in self.media_files.values() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("valid demo fixture")
}

impl Default for Demo {
    fn default() -> Self {
        Self::new()
    }
}

impl Demo {
    pub fn new() -> Self {
        let mut store = Store::default();
        store.apply(Update::Session(Some(Box::new(decode(json!({
            "id":"me", "username":"christian", "nickname":"Christian", "status":"online",
            "settings":{"version":1,"config":{}}
        }))))));
        store.apply(Update::Server(Some(Box::new(decode(
            json!({"id":"demo","name":"Papo","public":true,"owner_id":"me","icon_blob":base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../../android/compose/src/main/res/mipmap-xxxhdpi/ic_launcher.png"))}),
        )))));
        store.apply(Update::Channels(decode(json!([
            {"id":"general","name":"geral","type":"text","position":0,"topic":"A little bit of everything."},
            {"id":"projects","name":"Projetos","type":"category","position":1},
            {"id":"design","name":"design","type":"text","position":2,"topic":"Small details, better experiences.","parent_id":"projects"},
            {"id":"dev","name":"dev","type":"text","position":3,"topic":"What are you building?","parent_id":"projects"},
            {"id":"music","name":"música","type":"text","position":4,"topic":"Something worth listening to."},
            {"id":"media","name":"mídia","type":"text","position":5,"topic":"Photos, GIFs, video and rich cards."}
        ]))));
        store.apply(Update::Users(decode(json!([
            {"id":"me","username":"christian","nickname":"Christian","status":"online"},
            {"id":"ana","username":"ana","nickname":"Ana","status":"online"},
            {"id":"bruno","username":"bruno","nickname":"Bruno","status":"away"},
            {"id":"dora","username":"dora","nickname":"Dora","status":"busy"}
        ]))));
        use base64::Engine;
        let avatar = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
            "../../../android/compose/src/main/assets/demo/portrait.png"
        ));
        store.apply(Update::Profiles(decode(json!([
            {"id":"ana","username":"ana","nickname":"Ana","avatar_blob":avatar,"banner_media":"demo-banner","description":"Design, café e boas conversas.\nConstruindo pequenos detalhes no Papo.","status_message":"Desenhando ideias","created_at":"2026-01-15T12:00:00Z","roles":[{"id":"designers","name":"Designers","color":"#67B989","position":1}]},
            {"id":"me","username":"christian","nickname":"Christian","avatar_blob":null,"description":"Construindo o Papo para Android.","created_at":"2025-10-02T12:00:00Z","roles":[]},
            {"id":"bruno","username":"bruno","nickname":"Bruno","roles":[]},
            {"id":"dora","username":"dora","nickname":"Dora","roles":[]}
        ]))));
        store.apply(Update::Emojis(vec![models::Emoji {
            id: "demo-wave".into(),
            name: "papo_wave".into(),
            created_at: None,
            format: Some("GIF".into()),
            image_blob: Some(
                base64::engine::general_purpose::STANDARD.encode(include_bytes!(
                    "../../../android/compose/src/main/assets/demo/animation.gif"
                )),
            ),
        }]));
        store.apply(Update::Roles(decode(json!([{"id":"designers","name":"Designers","color":"#67B989","permissions":{"send_attachment":true}}]))));
        let now = now_millis();
        store.apply(Update::DirectMessages(decode(json!([{"id":"dm-ana","user":{"id":"ana","username":"ana","nickname":"Ana","status":"online"},"created_at":at(now),"unread_count":1}]))));
        for (channel, rows) in [
            (
                "general",
                vec![
                    ("ana", "bom dia! ☀️", Some("demo-archived-original")),
                    ("bruno", "bom dia! como ficou a versão nova do Papo?", None),
                    (
                        "me",
                        "agora tem uma casa no Android também :)\nteclado nativo, e as pastilhas de sempre.",
                        None,
                    ),
                    ("dora", "o detalhe do vidro faz toda a diferença", None),
                    (
                        "ana",
                        "sim! mas a conversa tem que continuar sendo o centro de tudo.",
                        None,
                    ),
                    (
                        "bruno",
                        "e o rascunho fica salvo quando troca de canal?",
                        None,
                    ),
                    (
                        "me",
                        "fica sim. pode escrever, sair e voltar.",
                        Some("general-5"),
                    ),
                    (
                        "ana",
                        "boa. vou testar com uma mensagem maior, emoji e umas quebras de linha.\n\nàs vezes o básico é o que mais importa 🌱",
                        None,
                    ),
                    (
                        "dora",
                        "deixei as ideias no #design. depois deem uma olhada!",
                        None,
                    ),
                    ("ana", "o que vocês estão construindo hoje?", None),
                ],
            ),
            (
                "design",
                vec![
                    (
                        "dora",
                        "uma ideia: controles flutuantes, conteúdo livre. o vidro só onde ajuda.",
                        None,
                    ),
                    ("ana", "e nada de uma cor diferente pra cada aviso 🙏", None),
                ],
            ),
            (
                "dev",
                vec![(
                    "bruno",
                    "papo-core continua cuidando da sessão, cache e sincronização. a interface agora é Compose.",
                    None,
                )],
            ),
            ("music", vec![("ana", "trilha sonora do dia? 🎧", None)]),
            ("dm-ana", vec![("ana", "oi! bem-vindo ao Papo 👋", None)]),
        ] {
            let ticket = store.mark_loading(channel);
            let messages = rows
                .into_iter()
                .enumerate()
                .map(|(i, (author, content, reply))| {
                    let mut row=message(
                        &format!("{channel}-{i}"),
                        channel,
                        author,
                        content,
                        reply.map(str::to_owned),
                        now - (12 - i as i64) * 60_000,
                    );
                    if channel=="design" && i==0 {
                        row.external_author=Some(decode(json!({"source":"Wormhole","id":"remote-ari","name":"Ari","avatar_url":"file:///android_asset/demo/portrait.png","profile_url":"https://example.test/ari"})));
                    }
                    row
                })
                .collect();
            store.apply(Update::Messages {
                ticket,
                messages,
                has_more: false,
                pinned_ids: None,
            });
        }
        let fixtures = vec![
            json!({"content":"Uma imagem, sem sair da conversa.","attachments":[{"id":"demo-photo","mime_type":"image/png","original_file_name":"landscape.png","size_bytes":12000,"moderation_status":"pending"}]}),
            json!({"content":"E uma foto em pé.","attachments":[{"id":"demo-portrait","mime_type":"image/png","original_file_name":"portrait.png","moderation_status":"clean"}]}),
            json!({"content":"GIFs também se mexem 🙂","attachments":[{"id":"demo-gif","mime_type":"image/gif","original_file_name":"animation.gif","moderation_status":"clean"}]}),
            json!({"content":"Vídeo com controles nativos.","attachments":[{"id":"demo-video","mime_type":"video/mp4","original_file_name":"clip.mp4","thumbnail_id":"poster","moderation_status":"clean"}]}),
            json!({"content":"Um trecho de áudio.","attachments":[{"id":"demo-audio","mime_type":"video/mp4","original_file_name":"voice-note.m4a","moderation_status":"clean"}]}),
            json!({"content":"Arquivos continuam sendo arquivos.","attachments":[{"id":"demo-file","mime_type":"text/plain","original_file_name":"notes.txt","size_bytes":64,"moderation_status":"clean"}]}),
            json!({"content":"Um card personalizado, com os campos na ordem do servidor.","embeds":[{"id":"embed-custom","source_type":"custom","title":"A place for your people","description":"Rich cards keep their author, fields, footer and image.","site_name":"Papo","color":"#0A84FF","url":"https://github.com/chriexpe/papo-frontend","author":{"name":"Papo","media":{"mime_type":"image/png"}},"image":{"mime_type":"image/png","width":960,"height":540},"fields":[{"position":2,"name":"Platform","value":"Android","inline":true},{"position":1,"name":"Built with","value":"Compose + Rust","inline":true}],"footer":{"text":"Made for good conversations"}}]}),
            json!({"content":"As dimensões chegam antes dos pixels.\nhttps://cdn.example.test/portrait.png?sig=fixture\nhttps://example.test/more","embeds":[{"id":"embed-portrait","url":"https://cdn.example.test/portrait.png?sig=fixture","title":"Portrait preview","image":{"mime_type":"image/png","width":540,"height":960}}]}),
            json!({"content":"Um GIF do resolvedor de embeds.\nhttps://cdn.example.test/animation.gif","embeds":[{"id":"embed-gif","url":"https://cdn.example.test/animation.gif","source_type":"link","fetch_method":"gif","site_name":"GIF preview","title":"A little motion","image":{"mime_type":"image/gif","width":480,"height":320}}]}),
            json!({"content":"Vídeo de embed usa o relay autenticado.\nhttps://example.invalid/clip.mp4","embeds":[{"id":"embed-video","url":"https://example.invalid/clip.mp4","title":"Video preview","thumbnail":{"mime_type":"image/png","width":960,"height":540},"video":{"mime_type":"video/mp4","url":"https://example.invalid/clip.mp4","width":1280,"height":720}}]}),
            json!({"content":"Um player de provedor só abre quando você pede.","embeds":[{"id":"embed-youtube","source_type":"link","site_name":"YouTube","title":"YouTube preview","url":"https://www.youtube.com/watch?v=jfKfPfyJRdk","thumbnail":{"mime_type":"image/png","width":1280,"height":720},"video":{"url":"https://www.youtube.com/embed/jfKfPfyJRdk","width":1280,"height":720}}]}),
            json!({"content":"Às vezes, o texto basta.","embeds":[{"id":"embed-link","title":"Papo on GitHub","description":"A simple link card without media.","url":"https://github.com/chriexpe/papo-frontend","site_name":"GitHub","fields":null}]}),
        ];
        let ticket = store.mark_loading("media");
        let messages = fixtures
            .into_iter()
            .enumerate()
            .map(|(i, mut value)| {
                value["id"] = json!(format!("media-{i}"));
                value["channel_id"] = json!("media");
                value["author_id"] = json!(if i % 2 == 0 { "ana" } else { "dora" });
                value["created_at"] = json!(at(now - (12 - i as i64) * 60_000));
                decode(value)
            })
            .collect();
        store.apply(Update::Messages {
            ticket,
            messages,
            has_more: false,
            pinned_ids: None,
        });
        for member in &mut store.members {
            member.presence = match member.id.as_str() {
                "bruno" => papo_core::state::Presence::Away,
                "dora" => papo_core::state::Presence::Busy,
                _ => papo_core::state::Presence::Online,
            };
        }
        store.selected_channel = "general".into();
        store.screen = Screen::Chat;
        store.connection = Connection::Online;
        if let Some(c) = store.channels.iter_mut().find(|c| c.id == "design") {
            c.unread = true;
            c.mentions = 2;
        }
        let notification = store
            .messages_in("general")
            .filter(|m| m.author_id == "ana")
            .last()
            .map(|m| models::Notification {
                id: "demo-notification".into(),
                message_id: Some(m.id.clone()),
                channel_id: Some("general".into()),
                author_id: Some(m.author_id.clone()),
                message_content: Some(m.content.clone()),
                read: false,
                created_at: Some(m.at.with_timezone(&chrono::Utc)),
            })
            .into_iter()
            .collect::<Vec<_>>();
        store.apply(Update::Notifications(notification.clone()));
        Self {
            notifications: notification,
            store,
            events: vec![],
            media_files: HashMap::new(),
            navigation_serial: 0,
            navigation_target: String::new(),
            search_query: String::new(),
            archived: message(
                "demo-archived-original",
                "general",
                "ana",
                "An older hello from this conversation.",
                None,
                now - 86_400_000,
            ),
        }
    }

    pub fn poll(&mut self) -> Value {
        let mut value = project(&self.store, "demo://papo", "demo");
        value["events"] = json!(std::mem::take(&mut self.events));
        value["more"] = json!(false);
        value["searchQuery"] = json!(self.search_query);
        value["navigationSerial"] = json!(self.navigation_serial);
        value["navigationTarget"] = json!(self.navigation_target);
        value
    }

    pub fn media_request(
        &self,
        resource: media::MediaResource,
    ) -> Result<media::MediaRequest, String> {
        if let media::MediaResource::Attachment { id, .. } = &resource
            && let Some(path) = self.media_files.get(id)
        {
            return Ok(media::MediaRequest {
                url: url::Url::from_file_path(path)
                    .map_err(|_| "Invalid demo attachment")?
                    .to_string(),
                headers: HashMap::new(),
                origin: "demo".into(),
                cache_key: format!("demo/me/{id}"),
            });
        }
        media::demo_request(resource)
    }

    pub fn dispatch(&mut self, action: Action) -> Result<(), String> {
        match action {
            Action::Search { query, more } => {
                let request =
                    papo_core::search::search_request(&query, &self.store, false, false, None)
                        .ok_or("Complete the search filters before searching")?;
                if !more {
                    self.store.reset_search();
                    self.search_query = query;
                }
                let offset = if more {
                    self.store.search_results.len()
                } else {
                    0
                };
                let mut messages: Vec<_> =
                    self.store
                        .messages
                        .iter()
                        .filter(|m| {
                            request.text.as_ref().is_none_or(|t| {
                                m.content.to_lowercase().contains(&t.to_lowercase())
                            }) && request.author.as_ref().is_none_or(|id| &m.author_id == id)
                                && request
                                    .channel_id
                                    .as_ref()
                                    .is_none_or(|id| &m.channel_id == id)
                                && request
                                    .mention
                                    .as_ref()
                                    .is_none_or(|id| m.content.contains(&format!("<@{id}>")))
                                && request.has.as_ref().is_none_or(|_| {
                                    m.content.contains("https://") || !m.previews.is_empty()
                                })
                                && request
                                    .contains_attachment
                                    .is_none_or(|_| !m.attachments.is_empty())
                        })
                        .collect();
                messages.sort_by_key(|m| std::cmp::Reverse((m.at, m.id.clone())));
                let has_more = messages.len() > offset + 20;
                let results = messages
                    .into_iter()
                    .skip(offset)
                    .take(20)
                    .map(|m| models::SearchResult {
                        kind: "message".into(),
                        id: m.id.clone(),
                        content: m.content.clone(),
                        channel_id: m.channel_id.clone(),
                        channel_name: self
                            .store
                            .channel(&m.channel_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default(),
                        author_id: Some(m.author_id.clone()),
                        author_username: self
                            .store
                            .member(&m.author_id)
                            .map(|u| u.username.clone()),
                        created_at: Some(m.at.into()),
                        attachments: m.attachments.clone(),
                    })
                    .collect();
                self.store.apply(Update::SearchResults {
                    generation: self.store.search_generation,
                    results,
                    has_more,
                    append: more,
                });
            }
            Action::ClearSearch => {
                self.store.reset_search();
                self.search_query.clear();
            }
            Action::ChannelNotifications {
                channel_id,
                setting,
            } => {
                discovery::check_notifications(&self.store, &channel_id, &setting)?;
                if let Some(c) = self.store.channels.iter_mut().find(|c| c.id == channel_id) {
                    c.notification_settings = setting;
                }
            }
            Action::MarkServerRead => {
                self.store.mark_all_read();
                for item in &mut self.notifications {
                    item.read = true;
                }
                self.store
                    .apply(Update::Notifications(self.notifications.clone()));
            }
            Action::SaveServer {
                name,
                public,
                password,
            } => {
                discovery::check_server(&self.store, &name, public, password.as_deref())?;
                if let Some(server) = self.store.server.as_mut() {
                    server.name = name.trim().into();
                    server.public = public;
                }
            }
            Action::SaveChannel {
                channel_id,
                name,
                topic,
            } => {
                if let Some(c) = self.store.channels.iter_mut().find(|c| c.id == channel_id) {
                    c.name = name;
                    c.topic = Some(topic);
                }
            }
            Action::SaveRole {
                role_id,
                name,
                color,
                permissions,
            } => {
                if let Some(r) = self.store.roles.iter_mut().find(|r| r.id == role_id) {
                    r.name = name;
                    r.color = Some(color);
                    r.permissions = permissions.into();
                }
            }
            Action::SelectChannel { channel_id } => {
                self.store
                    .end_message_context(&self.store.selected_channel.clone());
                self.navigation_target.clear();
                self.store.reset_read_span(&channel_id);
                self.store.selected_channel = channel_id;
            }
            Action::JumpToMessage {
                channel_id,
                message_id,
            } => {
                if channel_id != self.store.selected_channel {
                    return Err("Open this conversation first".into());
                }
                self.store.reset_read_span(&channel_id);
                if !self
                    .store
                    .messages_in(&channel_id)
                    .any(|m| m.id == message_id)
                {
                    if self
                        .store
                        .messages
                        .iter()
                        .any(|m| m.id == message_id && m.channel_id == channel_id)
                    {
                        self.store.end_message_context(&channel_id);
                    } else if self.archived.id == message_id
                        && self.archived.channel_id == channel_id
                    {
                        let ticket = self.store.begin_message_context(&channel_id);
                        self.store.apply(Update::MessageContext {
                            ticket,
                            message_id: message_id.clone(),
                            context: models::MessageContext {
                                channel_id,
                                messages: vec![self.archived.clone()],
                                has_more_before: false,
                                has_more_after: false,
                            },
                        });
                    } else {
                        return Err("The original message is no longer available.".into());
                    }
                }
                self.navigation_serial += 1;
                self.navigation_target = message_id;
            }
            Action::Latest { channel_id } => {
                if channel_id != self.store.selected_channel {
                    return Ok(());
                }
                self.store.reset_read_span(&channel_id);
                self.store.end_message_context(&channel_id);
                self.navigation_target.clear();
                self.navigation_serial += 1;
            }
            Action::Newer { .. } => {}
            Action::OpenDm { user_id } => {
                let user = self
                    .store
                    .members
                    .iter()
                    .find(|u| u.id == user_id)
                    .ok_or("Person not found")?;
                let dm: models::DirectConversation = decode(
                    json!({"id":format!("dm-{user_id}"),"user":{"id":user.id,"username":user.username,"nickname":user.name},"created_at":at(now_millis())}),
                );
                self.store.apply(Update::DirectMessageOpened(Box::new(dm)));
            }
            Action::LoadProfile { .. } => {}
            Action::Send {
                local_id,
                channel_id,
                content,
                reply_to,
            } => {
                if content.trim().is_empty() {
                    return Err("Write a message first".into());
                }
                self.store.apply(Update::Sent(Box::new(message(
                    &local_id,
                    &channel_id,
                    "me",
                    &content,
                    reply_to,
                    now_millis(),
                ))));
                self.events
                    .push(json!({"type":"accepted","localId":local_id}));
            }
            Action::SendAttachments {
                local_id,
                channel_id,
                content,
                reply_to,
                files,
            } => {
                let files = prepare_uploads(files)?;
                let mut sent = message(
                    &local_id,
                    &channel_id,
                    "me",
                    &content,
                    reply_to,
                    now_millis(),
                );
                for file in files {
                    let id = format!("demo-local-{}", papo_core::cache::new_local_id());
                    let copy = file
                        .path
                        .parent()
                        .ok_or("Attachment folder is unavailable")?
                        .join(&id);
                    std::fs::copy(&file.path, &copy)
                        .map_err(|_| "Could not keep the demo attachment")?;
                    self.media_files.insert(id.clone(), copy);
                    sent.attachments.push(decode(json!({
                        "id":id,"original_file_name":file.wire_name(),"mime_type":file.mime,
                        "size_bytes":file.size,"moderation_status":"clean"
                    })));
                }
                self.store.apply(Update::Sent(Box::new(sent)));
                self.events
                    .push(json!({"type":"accepted","localId":local_id}));
            }
            Action::Edit {
                message_id,
                content,
            } => self
                .store
                .apply(Update::Event(Box::new(Event::MessageEdited {
                    id: message_id,
                    channel_id: self.store.selected_channel.clone(),
                    content,
                }))),
            Action::Delete { message_id } => self.store.apply(Update::Deleted(message_id)),
            Action::React {
                message_id,
                emoji,
                custom,
                add,
                ..
            } => self.store.set_reaction_local(
                &message_id,
                &if custom {
                    Emoji::Custom(emoji)
                } else {
                    Emoji::Unicode(emoji)
                },
                add,
            ),
            Action::ReactionDetails { message_id, .. } => {
                let groups = self
                    .store
                    .message(&message_id)
                    .map(|m| {
                        m.reactions
                            .iter()
                            .map(|r| models::ReactionGroup {
                                emoji_id: if let Emoji::Custom(id) = &r.emoji {
                                    Some(id.clone())
                                } else {
                                    None
                                },
                                unicode: if let Emoji::Unicode(e) = &r.emoji {
                                    Some(e.clone())
                                } else {
                                    None
                                },
                                count: r.count,
                                users: (0..r.count)
                                    .map(|i| models::ReactionUser {
                                        id: format!("{message_id}-{i}"),
                                        user_id: if r.mine && i == 0 { "me" } else { "ana" }.into(),
                                        created_at: chrono::Utc::now(),
                                    })
                                    .collect(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.store.apply(Update::ReactionDetails {
                    message_id,
                    reactions: groups,
                    has_more: false,
                    append: false,
                });
            }
            Action::ClearError => self.store.error = None,
            Action::CancelCatchUp { .. } => {}
            Action::CatchUp {
                channel_id,
                owner_user_id,
            } => {
                if owner_user_id != self.store.me {
                    return Err("Account changed".into());
                }
                if let Some(message_id) = self.store.hourglass_navigation_target(&channel_id) {
                    return self.dispatch(Action::JumpToMessage {
                        channel_id,
                        message_id,
                    });
                }
            }
            Action::BreakReadSpan { channel_id } => self.store.reset_read_span(&channel_id),
            Action::ClearCatchUp {
                channel_id,
                owner_user_id,
            } => {
                if owner_user_id == self.store.me {
                    self.store.mark_read(&channel_id)
                }
            }
            Action::Visible {
                channel_id,
                ids,
                at_latest,
                owner_user_id,
            } => {
                if owner_user_id != self.store.me {
                    return Ok(());
                }

                for notification in &mut self.notifications {
                    if notification.channel_id.as_deref() == Some(&channel_id)
                        && (at_latest
                            || notification
                                .message_id
                                .as_ref()
                                .is_some_and(|id| ids.contains(id)))
                    {
                        notification.read = true;
                    }
                }
                self.store
                    .observe_chat_viewport(&channel_id, &ids, at_latest)
            }
            Action::Refresh | Action::Network { .. } | Action::Typing { .. } | Action::Older => {}
            _ => return Err("This action needs a real server".into()),
        }
        Ok(())
    }
}

fn at(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .unwrap()
        .to_rfc3339()
}
fn message(
    id: &str,
    channel: &str,
    author: &str,
    content: &str,
    reply: Option<String>,
    millis: i64,
) -> models::Message {
    decode(
        json!({"id":id,"channel_id":channel,"author_id":author,"content":content,"reply_to":reply,"created_at":at(millis)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_project_metadata_and_invalidate_removed_avatars() {
        let client = crate::bindings::NativeClient::demo();
        let snapshot = client.poll().unwrap();
        let ana = snapshot.profiles.iter().find(|p| p.id == "ana").unwrap();
        assert!(ana.loaded);
        assert!(!ana.avatar_revision.is_empty());
        assert_eq!(ana.roles[0].name, "Designers");
        assert_eq!(ana.banner, "demo-banner");
        assert!(
            client
                .avatar_image("ana".into())
                .unwrap()
                .starts_with(b"\x89PNG")
        );
        assert!(client.avatar_image("missing".into()).is_err());
        let request = client
            .media_request(media::MediaResource::ProfileBanner {
                user_id: "ana".into(),
            })
            .unwrap();
        assert!(request.url.ends_with("landscape.png"));
        assert!(request.headers.is_empty());
        let mut demo = Demo::new();
        demo.store.apply(Update::Profiles(decode(
            json!([{"id":"ana","username":"ana","avatar_blob":null,"roles":[]}]),
        )));
        let after: crate::bindings::ClientSnapshot = serde_json::from_value(demo.poll()).unwrap();
        let ana = after.profiles.iter().find(|p| p.id == "ana").unwrap();
        assert!(ana.avatar_revision.is_empty());
        assert!(ana.banner.is_empty());
        assert!(ana.roles.is_empty());
    }

    #[test]
    fn attributed_app_identity_takes_precedence_over_the_bridge_account() {
        let client = crate::bindings::NativeClient::demo();
        client
            .dispatch(Action::SelectChannel {
                channel_id: "design".into(),
            })
            .unwrap();
        let snapshot = client.poll().unwrap();
        let message = snapshot
            .messages
            .iter()
            .find(|m| m.id == "design-0")
            .unwrap();
        assert_eq!(message.author, "Ari");
        assert_ne!(message.author_id, "remote-ari");
        let external = message.external_author.as_ref().unwrap();
        assert_eq!(external.source, "Wormhole");
        assert_eq!(external.id, "remote-ari");
        assert!(!external.avatar_url.is_empty());
        assert!(snapshot.members.iter().all(|m| m.id != external.id));
    }
    #[test]
    fn demo_uses_core_reduction_and_never_creates_a_real_session() {
        let mut demo = Demo::new();
        assert_eq!(demo.poll()["messages"].as_array().unwrap().len(), 10);
        demo.dispatch(Action::Send {
            local_id: "test-send".into(),
            channel_id: "general".into(),
            content: "hello 👋".into(),
            reply_to: None,
        })
        .unwrap();
        let snapshot = demo.poll();
        assert_eq!(snapshot["events"][0]["type"], "accepted");
        assert_eq!(
            snapshot["messages"].as_array().unwrap().last().unwrap()["content"],
            "hello 👋"
        );
        demo.dispatch(Action::Delete {
            message_id: "test-send".into(),
        })
        .unwrap();
        assert_eq!(demo.poll()["messages"].as_array().unwrap().len(), 10);
    }

    #[test]
    fn generated_snapshot_contract_contains_real_media_dimensions() {
        let demo = crate::bindings::NativeClient::demo();
        assert_eq!(demo.poll().unwrap().messages.len(), 10);
        demo.dispatch(Action::SelectChannel {
            channel_id: "media".into(),
        })
        .unwrap();
        let snapshot = demo.poll().unwrap();
        assert_eq!(snapshot.messages.len(), 12);
        let portrait = snapshot
            .messages
            .iter()
            .flat_map(|m| &m.embeds)
            .find(|e| e.id == "embed-portrait")
            .unwrap();
        assert_eq!((portrait.width, portrait.height), (540, 960));
        let youtube = snapshot
            .messages
            .iter()
            .flat_map(|m| &m.embeds)
            .find(|e| e.id == "embed-youtube")
            .unwrap();
        assert_eq!(youtube.kind, "iframe");
        demo.shutdown();
        assert!(demo.poll().is_err());
    }
}

#[cfg(test)]
mod reply_navigation_tests {
    use super::*;
    #[test]
    fn reply_jump_loads_original_context_and_latest_restores_the_live_window() {
        let client = crate::bindings::NativeClient::demo();
        let before = client.poll().unwrap();
        let source = before
            .messages
            .iter()
            .find(|m| m.id == "general-0")
            .unwrap();
        assert_eq!(source.reply_to, "demo-archived-original");
        client
            .dispatch(Action::JumpToMessage {
                channel_id: "general".into(),
                message_id: source.reply_to.clone(),
            })
            .unwrap();
        let context = client.poll().unwrap();
        assert!(context.browsing_history);
        assert_eq!(context.messages.len(), 1);
        assert_eq!(context.navigation_target, "demo-archived-original");
        assert_eq!(context.messages[0].id, context.navigation_target);
        assert!(context.navigation_serial > before.navigation_serial);
        // A delayed callback from another conversation must not reset this one.
        client
            .dispatch(Action::Latest {
                channel_id: "design".into(),
            })
            .unwrap();
        assert!(client.poll().unwrap().browsing_history);
        client
            .dispatch(Action::Latest {
                channel_id: "general".into(),
            })
            .unwrap();
        let live = client.poll().unwrap();
        assert!(!live.browsing_history);
        assert_eq!(live.messages.len(), 10);
        assert!(live.navigation_target.is_empty());
        client
            .dispatch(Action::JumpToMessage {
                channel_id: "general".into(),
                message_id: "general-5".into(),
            })
            .unwrap();
        let first = client.poll().unwrap();
        client
            .dispatch(Action::JumpToMessage {
                channel_id: "general".into(),
                message_id: "general-5".into(),
            })
            .unwrap();
        assert!(client.poll().unwrap().navigation_serial > first.navigation_serial);
    }
}

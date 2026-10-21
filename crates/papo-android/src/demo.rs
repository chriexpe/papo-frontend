//! Explicit, ephemeral demo. Uses the real core reducer, creates no sessions,
//! opens no sockets and cannot send anything to a real server.
use super::*;
use papo_core::api::{models, ws::Event};
use papo_core::state::Store;

pub struct Demo {
    store: Store,
    events: Vec<Value>,
    media_files: HashMap<String, PathBuf>,
}
impl Drop for Demo {
    fn drop(&mut self) {
        for path in self.media_files.values() { let _ = std::fs::remove_file(path); }
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
            json!({"id":"demo","name":"Papo","public":true,"owner_id":"me"}),
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
        let now = now_millis();
        store.apply(Update::DirectMessages(decode(json!([{"id":"dm-ana","user":{"id":"ana","username":"ana","nickname":"Ana","status":"online"},"created_at":at(now),"unread_count":1}]))));
        for (channel, rows) in [
            (
                "general",
                vec![
                    ("ana", "bom dia! ☀️", None),
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
                    message(
                        &format!("{channel}-{i}"),
                        channel,
                        author,
                        content,
                        reply.map(str::to_owned),
                        now - (12 - i as i64) * 60_000,
                    )
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
            json!({"content":"Uma imagem, sem sair da conversa.","attachments":[{"id":"demo-photo","mime_type":"image/png","original_file_name":"landscape.png","size_bytes":12000,"moderation_status":"clean"}]}),
            json!({"content":"E uma foto em pé.","attachments":[{"id":"demo-portrait","mime_type":"image/png","original_file_name":"portrait.png","moderation_status":"clean"}]}),
            json!({"content":"GIFs também se mexem 🙂","attachments":[{"id":"demo-gif","mime_type":"image/gif","original_file_name":"animation.gif","moderation_status":"clean"}]}),
            json!({"content":"Vídeo com controles nativos.","attachments":[{"id":"demo-video","mime_type":"video/mp4","original_file_name":"clip.mp4","thumbnail_id":"poster","moderation_status":"clean"}]}),
            json!({"content":"Um trecho de áudio.","attachments":[{"id":"demo-audio","mime_type":"audio/wav","original_file_name":"tone.wav","moderation_status":"clean"}]}),
            json!({"content":"Arquivos continuam sendo arquivos.","attachments":[{"id":"demo-file","mime_type":"text/plain","original_file_name":"notes.txt","size_bytes":64,"moderation_status":"clean"}]}),
            json!({"content":"Um card personalizado, com os campos na ordem do servidor.","embeds":[{"id":"embed-custom","source_type":"custom","title":"A place for your people","description":"Rich cards keep their author, fields, footer and image.","site_name":"Papo","color":"#0A84FF","url":"https://github.com/chriexpe/papo-frontend","author":{"name":"Papo","media":{"mime_type":"image/png"}},"image":{"mime_type":"image/png","width":960,"height":540},"fields":[{"position":2,"name":"Platform","value":"Android","inline":true},{"position":1,"name":"Built with","value":"Compose + Rust","inline":true}],"footer":{"text":"Made for good conversations"}}]}),
            json!({"content":"As dimensões chegam antes dos pixels.","embeds":[{"id":"embed-portrait","title":"Portrait preview","image":{"mime_type":"image/png","width":540,"height":960}}]}),
            json!({"content":"Um GIF do resolvedor de embeds.","embeds":[{"id":"embed-gif","source_type":"link","fetch_method":"gif","site_name":"GIF preview","title":"A little motion","image":{"mime_type":"image/gif","width":480,"height":320}}]}),
            json!({"content":"Vídeo de embed usa o relay autenticado.","embeds":[{"id":"embed-video","title":"Video preview","thumbnail":{"mime_type":"image/png","width":960,"height":540},"video":{"mime_type":"video/mp4","url":"https://example.invalid/clip.mp4","width":1280,"height":720}}]}),
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
        store.selected_channel = "general".into();
        store.screen = Screen::Chat;
        store.connection = Connection::Online;
        if let Some(c) = store.channels.iter_mut().find(|c| c.id == "design") {
            c.unread = true;
            c.mentions = 2;
        }
        Self {
            store,
            events: vec![],
            media_files: HashMap::new(),
        }
    }

    pub fn poll(&mut self) -> Value {
        let mut value = project(&self.store, "demo://papo", "demo");
        value["events"] = json!(std::mem::take(&mut self.events));
        value["more"] = json!(false);
        value
    }

    pub fn media_request(&self, resource: media::MediaResource) -> Result<media::MediaRequest, String> {
        if let media::MediaResource::Attachment { id, .. } = &resource
            && let Some(path) = self.media_files.get(id) {
                return Ok(media::MediaRequest {
                    url: url::Url::from_file_path(path).map_err(|_| "Invalid demo attachment")?.to_string(),
                    headers: HashMap::new(), origin: "demo".into(), cache_key: format!("demo/me/{id}"),
                });
        }
        media::demo_request(resource)
    }

    pub fn dispatch(&mut self, action: Action) -> Result<(), String> {
        match action {
            Action::SelectChannel { channel_id } => self.store.selected_channel = channel_id,
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
            Action::SendAttachments { local_id, channel_id, content, reply_to, files } => {
                let files = prepare_uploads(files)?;
                let mut sent = message(&local_id, &channel_id, "me", &content, reply_to, now_millis());
                for file in files {
                    let id = format!("demo-local-{}", papo_core::cache::new_local_id());
                    let copy = file.path.parent().ok_or("Attachment folder is unavailable")?.join(&id);
                    std::fs::copy(&file.path, &copy).map_err(|_| "Could not keep the demo attachment")?;
                    self.media_files.insert(id.clone(), copy);
                    sent.attachments.push(decode(json!({
                        "id":id,"original_file_name":file.name,"mime_type":file.mime,
                        "size_bytes":file.size,"moderation_status":"clean"
                    })));
                }
                self.store.apply(Update::Sent(Box::new(sent)));
                self.events.push(json!({"type":"accepted","localId":local_id}));
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
                add,
                ..
            } => self
                .store
                .set_reaction_local(&message_id, &Emoji::Unicode(emoji), add),
            Action::ClearError => self.store.error = None,
            Action::Visible { channel_id, ids } => {
                self.store
                    .observe_visible_messages_with_policy(&channel_id, &ids, false)
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

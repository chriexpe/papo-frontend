//! Explicit, ephemeral demo. Uses the real core reducer, creates no sessions,
//! opens no sockets and cannot send anything to a real server.
use super::*;
use papo_core::api::{models, ws::Event};
use papo_core::state::Store;

pub struct Demo {
    store: Store,
    events: Vec<Value>,
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
            {"id":"music","name":"música","type":"text","position":4,"topic":"Something worth listening to."}
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
        }
    }

    pub fn poll(&mut self) -> Value {
        let mut value = project(&self.store, "demo://papo", "demo");
        value["events"] = json!(std::mem::take(&mut self.events));
        value["more"] = json!(false);
        value
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
}

//! Auditoria legível: códigos do backend viram frases, e os filtros da tela
//! viram consulta.
//!
//! O desenho fica em [`super::settings`]; aqui só o que dá para testar sem
//! janela. Tudo que o servidor pode não mandar (alvo, canal, texto apagado)
//! é opcional — servidores antigos continuam com uma frase razoável.

use chrono::{DateTime, Duration, Local, Utc};

use crate::api::models::{AuditLogEntry, AuditQuery};
use crate::i18n::Lang;

/// Famílias de ação, como a ficha "Tipo" as oferece.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    All,
    Deleted,
    Messages,
    Channels,
    Roles,
    Members,
    Server,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::All,
        Kind::Deleted,
        Kind::Messages,
        Kind::Channels,
        Kind::Roles,
        Kind::Members,
        Kind::Server,
    ];

    pub fn matches(self, action: &str) -> bool {
        let family = |prefixes: &[&str]| prefixes.iter().any(|prefix| action.starts_with(prefix));
        match self {
            Kind::All => true,
            Kind::Deleted => action == "message.delete",
            Kind::Messages => family(&["message.", "media."]),
            Kind::Channels => family(&["channel."]),
            Kind::Roles => family(&["role.", "user_role."]),
            Kind::Members => family(&["user.", "auth."]),
            Kind::Server => family(&["server.", "emoji."]),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Period {
    #[default]
    All,
    Today,
    Week,
    Month,
}

impl Period {
    pub const ALL: [Period; 4] = [Period::All, Period::Today, Period::Week, Period::Month];

    pub fn since(self, now: DateTime<Local>) -> Option<DateTime<Utc>> {
        let start = match self {
            Period::All => return None,
            Period::Today => now.date_naive().and_hms_opt(0, 0, 0)?.and_local_timezone(Local).earliest()?,
            Period::Week => now - Duration::days(7),
            Period::Month => now - Duration::days(30),
        };
        Some(start.with_timezone(&Utc))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    /// Apelido, @usuário ou começo do ID; vale para quem fez ou quem sofreu.
    pub person: String,
    pub kind: Kind,
    pub channel: Option<String>,
    pub period: Period,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        *self == Filter::default()
    }

    /// O que dá para pedir ao servidor. Pessoa fica de fora: o texto casa com
    /// apelido, e o servidor só filtra por id. Vai tudo filtrado aqui também,
    /// então servidor que ignore os parâmetros não mostra nada a mais.
    pub fn query(&self, now: DateTime<Local>) -> AuditQuery {
        AuditQuery {
            action: (self.kind == Kind::Deleted).then(|| "message.delete".to_owned()),
            channel_id: self.channel.clone(),
            since: self.period.since(now),
            ..Default::default()
        }
    }

    pub fn keeps(&self, entry: &AuditLogEntry, names: &dyn Fn(&str) -> Option<(String, String)>, now: DateTime<Local>) -> bool {
        if !self.kind.matches(&entry.action) {
            return false;
        }
        if self.channel.as_deref().is_some_and(|channel| entry.channel() != Some(channel)) {
            return false;
        }
        if self.period.since(now).zip(entry.created_at).is_some_and(|(since, at)| at < since) {
            return false;
        }
        let person = self.person.trim().trim_start_matches('@').to_lowercase();
        if person.is_empty() {
            return true;
        }
        let hit = |id: Option<&str>, username: Option<&str>| {
            let known = id.and_then(names);
            id.is_some_and(|id| id.to_lowercase().starts_with(&person))
                || username.is_some_and(|name| name.to_lowercase().contains(&person))
                || known.is_some_and(|(name, username)| {
                    name.to_lowercase().contains(&person) || username.to_lowercase().contains(&person)
                })
        };
        hit(entry.actor_id.as_deref(), Some(&entry.actor_username))
            || hit(entry.target(), entry.target_username.as_deref())
    }
}

/// Pedaço de frase: o desenho pinta pessoas em negrito e canais em destaque.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text(String),
    Person(String),
    Channel(String),
}

pub struct Names<'a> {
    /// id → apelido de quem ainda está no servidor.
    pub person: &'a dyn Fn(&str) -> Option<String>,
    pub channel: &'a dyn Fn(&str) -> Option<String>,
}

fn meta<'a>(entry: &'a AuditLogEntry, key: &str) -> Option<&'a str> {
    entry.metadata.get(key).and_then(|value| value.as_str()).filter(|value| !value.is_empty())
}

/// Texto que o servidor guardou da mensagem apagada, se guardou.
pub fn deleted_text(entry: &AuditLogEntry) -> Option<&str> {
    (entry.action == "message.delete").then(|| meta(entry, "content")).flatten()
}

pub fn is_danger(action: &str) -> bool {
    action.ends_with(".delete") || action == "user.ban" || action == "auth.connection_drop" || action == "message.moderation_blocked"
}

pub fn sentence(entry: &AuditLogEntry, lang: Lang, names: &Names<'_>) -> Vec<Piece> {
    let pt = lang == Lang::PtBr;
    let actor = entry
        .actor_id
        .as_deref()
        .and_then(|id| (names.person)(id))
        .unwrap_or_else(|| entry.actor_username.clone());
    let actor = if actor.is_empty() { (if pt { "Alguém" } else { "Someone" }).to_owned() } else { actor };
    let target_id = entry.target();
    let target = target_id
        .and_then(|id| (names.person)(id))
        .or_else(|| entry.target_username.clone())
        .or_else(|| meta(entry, "author_username").map(str::to_owned));
    let own = target_id.is_some() && target_id == entry.actor_id.as_deref();
    let channel = entry
        .channel()
        .and_then(|id| (names.channel)(id))
        .or_else(|| meta(entry, "channel_name").map(str::to_owned))
        .or_else(|| entry.action.starts_with("channel.").then(|| meta(entry, "name").map(str::to_owned)).flatten());
    let name = meta(entry, "name").or_else(|| meta(entry, "role_name")).map(str::to_owned);

    let mut out = vec![Piece::Person(actor)];
    let text = |out: &mut Vec<Piece>, words: &str| out.push(Piece::Text(words.to_owned()));
    let person = |out: &mut Vec<Piece>, who: &Option<String>, fallback_pt: &str, fallback_en: &str| {
        out.push(match who {
            Some(who) => Piece::Person(who.clone()),
            None => Piece::Text((if pt { fallback_pt } else { fallback_en }).to_owned()),
        })
    };
    let in_channel = |out: &mut Vec<Piece>| {
        if let Some(channel) = &channel {
            text(out, if pt { " em " } else { " in " });
            out.push(Piece::Channel(format!("#{channel}")));
        }
    };
    let the_channel = |out: &mut Vec<Piece>| match &channel {
        Some(channel) => out.push(Piece::Channel(format!("#{channel}"))),
        None => text(out, if pt { "um canal" } else { "a channel" }),
    };
    let named = |out: &mut Vec<Piece>, fallback_pt: &str, fallback_en: &str| match &name {
        Some(name) => out.push(Piece::Person(name.clone())),
        None => text(out, if pt { fallback_pt } else { fallback_en }),
    };

    match entry.action.as_str() {
        "message.delete" => {
            if own {
                text(&mut out, if pt { " apagou a própria mensagem" } else { " deleted their own message" });
            } else if target.is_some() {
                text(&mut out, if pt { " apagou uma mensagem de " } else { " deleted a message by " });
                person(&mut out, &target, "", "");
            } else {
                text(&mut out, if pt { " apagou uma mensagem" } else { " deleted a message" });
            }
            in_channel(&mut out);
        }
        "message.edit" => {
            text(&mut out, if pt { " editou uma mensagem" } else { " edited a message" });
            in_channel(&mut out);
        }
        "message.create" => {
            text(&mut out, if pt { " enviou uma mensagem" } else { " sent a message" });
            in_channel(&mut out);
        }
        "message.pin" => {
            text(&mut out, if pt { " fixou uma mensagem" } else { " pinned a message" });
            in_channel(&mut out);
        }
        "message.unpin" => {
            text(&mut out, if pt { " desafixou uma mensagem" } else { " unpinned a message" });
            in_channel(&mut out);
        }
        "message.moderation_blocked" => {
            text(&mut out, if pt { " teve uma mensagem barrada pela moderação" } else { " had a message blocked by moderation" });
            in_channel(&mut out);
        }
        "media.upload" => {
            text(&mut out, if pt { " enviou um arquivo" } else { " uploaded a file" });
            in_channel(&mut out);
        }
        "channel.create" => {
            text(&mut out, if pt { " criou " } else { " created " });
            the_channel(&mut out);
        }
        "channel.update" => {
            text(&mut out, if pt { " editou " } else { " edited " });
            the_channel(&mut out);
        }
        "channel.move_position" => {
            text(&mut out, if pt { " moveu " } else { " moved " });
            the_channel(&mut out);
        }
        "channel.delete" => {
            text(&mut out, if pt { " apagou " } else { " deleted " });
            the_channel(&mut out);
        }
        "channel.permissions_update" => {
            text(&mut out, if pt { " mudou as permissões de " } else { " changed the permissions of " });
            the_channel(&mut out);
        }
        "role.create" => {
            text(&mut out, if pt { " criou o cargo " } else { " created the role " });
            named(&mut out, "novo", "new");
        }
        "role.update" => {
            text(&mut out, if pt { " editou o cargo " } else { " edited the role " });
            named(&mut out, "—", "—");
        }
        "role.delete" => {
            text(&mut out, if pt { " apagou o cargo " } else { " deleted the role " });
            named(&mut out, "—", "—");
        }
        "user_role.assign" => {
            text(&mut out, if pt { " deu o cargo " } else { " gave the role " });
            named(&mut out, "—", "—");
            text(&mut out, if pt { " a " } else { " to " });
            person(&mut out, &target, "alguém", "someone");
        }
        "user_role.remove" => {
            text(&mut out, if pt { " tirou o cargo " } else { " removed the role " });
            named(&mut out, "—", "—");
            text(&mut out, if pt { " de " } else { " from " });
            person(&mut out, &target, "alguém", "someone");
        }
        "user.ban" => {
            text(&mut out, if pt { " baniu " } else { " banned " });
            person(&mut out, &target, "alguém", "someone");
        }
        "user.unban" => {
            text(&mut out, if pt { " desbaniu " } else { " unbanned " });
            person(&mut out, &target, "alguém", "someone");
        }
        "user.reset_password" => {
            text(&mut out, if pt { " redefiniu a senha de " } else { " reset the password of " });
            person(&mut out, &target, "alguém", "someone");
        }
        "user.register" => text(&mut out, if pt { " entrou no servidor" } else { " joined the server" }),
        "user.update_profile" => text(&mut out, if pt { " atualizou o perfil" } else { " updated their profile" }),
        "user.update_avatar" => text(&mut out, if pt { " trocou a foto" } else { " changed their picture" }),
        "user.update_banner" => text(&mut out, if pt { " trocou o banner" } else { " changed their banner" }),
        "user.update_status" => text(&mut out, if pt { " mudou o status" } else { " changed their status" }),
        "user.update_settings" => text(&mut out, if pt { " mudou os próprios ajustes" } else { " changed their settings" }),
        "user.change_password" => text(&mut out, if pt { " trocou a senha" } else { " changed their password" }),
        "auth.connection_drop" => text(&mut out, if pt { " derrubou uma conexão" } else { " dropped a connection" }),
        "server.create" => text(&mut out, if pt { " criou o servidor" } else { " created the server" }),
        "server.update" => text(&mut out, if pt { " editou o servidor" } else { " edited the server" }),
        "emoji.create" => {
            text(&mut out, if pt { " adicionou a figurinha " } else { " added the sticker " });
            named(&mut out, "nova", "new");
        }
        "emoji.delete" => {
            text(&mut out, if pt { " removeu a figurinha " } else { " removed the sticker " });
            named(&mut out, "—", "—");
        }
        other => text(&mut out, &format!(" · {other}")),
    }
    out
}

/// "Hoje", "Ontem" ou a data, para o cabeçalho do grupo.
pub fn day_label(at: DateTime<Utc>, now: DateTime<Local>, lang: Lang) -> String {
    let day = at.with_timezone(&Local).date_naive();
    let today = now.date_naive();
    let pt = lang == Lang::PtBr;
    if day == today {
        (if pt { "Hoje" } else { "Today" }).to_owned()
    } else if Some(day) == today.pred_opt() {
        (if pt { "Ontem" } else { "Yesterday" }).to_owned()
    } else if pt {
        day.format("%d/%m/%Y").to_string()
    } else {
        day.format("%b %-d, %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(action: &str, meta: serde_json::Value) -> AuditLogEntry {
        AuditLogEntry {
            id: "1".into(),
            actor_username: "ana".into(),
            action: action.into(),
            entity_type: String::new(),
            created_at: None,
            actor_id: Some("u-ana".into()),
            entity_id: None,
            target_user_id: None,
            target_username: None,
            channel_id: None,
            metadata: meta.as_object().cloned().unwrap_or_default(),
        }
    }

    fn render(pieces: &[Piece]) -> String {
        pieces
            .iter()
            .map(|piece| match piece {
                Piece::Text(text) | Piece::Person(text) | Piece::Channel(text) => text.as_str(),
            })
            .collect()
    }

    fn names() -> (impl Fn(&str) -> Option<String>, impl Fn(&str) -> Option<String>) {
        (
            |id: &str| match id {
                "u-ana" => Some("Ana".to_owned()),
                "u-bruno" => Some("Bruno".to_owned()),
                _ => None,
            },
            |id: &str| (id == "c-geral").then(|| "geral".to_owned()),
        )
    }

    #[test]
    fn deleted_message_names_author_and_channel() {
        let (person, channel) = names();
        let names = Names { person: &person, channel: &channel };
        let e = entry("message.delete", serde_json::json!({"author_id": "u-bruno", "channel_id": "c-geral", "content": "oi"}));
        assert_eq!(render(&sentence(&e, Lang::PtBr, &names)), "Ana apagou uma mensagem de Bruno em #geral");
        assert_eq!(deleted_text(&e), Some("oi"));
    }

    #[test]
    fn own_deletion_reads_as_own() {
        let (person, channel) = names();
        let names = Names { person: &person, channel: &channel };
        let e = entry("message.delete", serde_json::json!({"author_id": "u-ana"}));
        assert_eq!(render(&sentence(&e, Lang::En, &names)), "Ana deleted their own message");
    }

    #[test]
    fn old_servers_still_get_a_sentence() {
        let (person, channel) = names();
        let names = Names { person: &person, channel: &channel };
        let mut e = entry("channel.create", serde_json::json!({}));
        e.actor_id = None;
        assert_eq!(render(&sentence(&e, Lang::PtBr, &names)), "ana criou um canal");
        let e = entry("something.new", serde_json::json!({}));
        assert_eq!(render(&sentence(&e, Lang::PtBr, &names)), "Ana · something.new");
    }

    #[test]
    fn person_filter_matches_actor_or_target() {
        let lookup = |id: &str| (id == "u-bruno").then(|| ("Bruno".to_owned(), "bruno".to_owned()));
        let now = Local::now();
        let e = entry("message.delete", serde_json::json!({"author_id": "u-bruno"}));
        let filter = |person: &str| Filter { person: person.into(), ..Default::default() };
        assert!(filter("@ana").keeps(&e, &lookup, now));
        assert!(filter("brU").keeps(&e, &lookup, now));
        assert!(filter("u-bru").keeps(&e, &lookup, now));
        assert!(!filter("dora").keeps(&e, &lookup, now));
    }

    #[test]
    fn kind_and_channel_filter_locally() {
        let lookup = |_: &str| None;
        let now = Local::now();
        let e = entry("role.update", serde_json::json!({}));
        assert!(Filter { kind: Kind::Roles, ..Default::default() }.keeps(&e, &lookup, now));
        assert!(!Filter { kind: Kind::Deleted, ..Default::default() }.keeps(&e, &lookup, now));
        assert!(!Filter { channel: Some("c-geral".into()), ..Default::default() }.keeps(&e, &lookup, now));
    }
}

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDb {
    dir: PathBuf,
}

impl TempDb {
    fn new(label: &str) -> Self {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "papo-cache-{label}-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temporary cache dir");
        Self { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("papo-cache.db")
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn open(temp: &TempDb) -> ClientDb {
    ClientDb::open(Some(temp.path()))
}

fn message(id: &str, channel: &str, content: &str, created_at: i64) -> CachedMessage {
    CachedMessage {
        id: id.to_owned(),
        channel_id: channel.to_owned(),
        author_id: "author".to_owned(),
        content: content.to_owned(),
        created_at,
        edited: false,
        reply_to: None,
        pinned: false,
        attachments: Vec::new(),
        reactions: Vec::new(),
    }
}

fn channel(id: &str, position: i32) -> CachedChannel {
    CachedChannel {
        id: id.to_owned(),
        name: format!("canal-{id}"),
        kind: "text".to_owned(),
        topic: None,
        position,
        unread: false,
        mentions: 0,
    }
}

fn outgoing(local_id: &str, owner: &str, content: &str, state: OutgoingState) -> CachedOutgoing {
    CachedOutgoing {
        local_id: local_id.to_owned(),
        owner_user_id: owner.to_owned(),
        channel_id: "geral".to_owned(),
        content: content.to_owned(),
        reply_to: None,
        notify_reply: false,
        created_at: now_millis(),
        state,
        attempt_count: 0,
        last_attempt_at: None,
        last_error: None,
    }
}

fn draft(owner: &str, channel: &str, text: &str) -> CachedDraft {
    CachedDraft {
        owner_user_id: owner.to_owned(),
        channel_id: channel.to_owned(),
        text: text.to_owned(),
        mentions: Vec::new(),
        reply_to: None,
        notify_reply: true,
        updated_at: now_millis(),
    }
}


#[test]
fn open_apply_reopen_round_trip() {
    let temp = TempDb::new("roundtrip");
    {
        let db = open(&temp);
        assert!(db.is_enabled(), "cache deve abrir num diretório temporário");
        db.submit(
            "srv-a",
            vec![
                CacheOp::SetOwner {
                    owner_user_id: "user-1".to_owned(),
                    me_name: "Ana".to_owned(),
                    me_username: "ana".to_owned(),
                },
                CacheOp::UpsertServer(CachedServer {
                    name: "Papo".to_owned(),
                    description: Some("de ana".to_owned()),
                    owner_user_id: Some("user-1".to_owned()),
                    me_user_id: Some("user-1".to_owned()),
                    me_display_name: Some("Ana".to_owned()),
                    me_username: Some("ana".to_owned()),
                    updated_at: now_millis(),
                }),
                CacheOp::ReplaceChannels(vec![channel("geral", 0)]),
                CacheOp::ReplaceMembers(vec![CachedMember {
                    id: "user-2".to_owned(),
                    username: "bia".to_owned(),
                    name: "Bia".to_owned(),
                    role_color: Some([1, 2, 3]),
                    roles: vec!["mod".to_owned()],
                }]),
                CacheOp::ReplaceChannelSnapshot {
                    channel_id: "geral".to_owned(),
                    cached_at: now_millis(),
                    messages: vec![message("m1", "geral", "oi", 1_000)],
                },
            ],
        );
        db.flush();
        assert!(temp.path().exists(), "o arquivo do banco deve existir");
    }

    let reopened = open(&temp);
    let snapshot = reopened.load_snapshot("srv-a").expect("snapshot");
    assert_eq!(snapshot.owner_user_id.as_deref(), Some("user-1"));
    assert_eq!(snapshot.server.expect("server").name, "Papo");
    assert_eq!(snapshot.channels.len(), 1);
    assert_eq!(snapshot.members.len(), 1);
    assert_eq!(snapshot.members[0].roles, vec!["mod".to_owned()]);
    assert_eq!(snapshot.members[0].role_color, Some([1, 2, 3]));
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].content, "oi");
    assert!(snapshot.cached_channels.contains("geral"));
}

#[test]
fn migration_is_idempotent_across_reopens() {
    let temp = TempDb::new("migration");
    for _ in 0..3 {
        let db = open(&temp);
        assert!(db.is_enabled());
        db.submit("srv", vec![CacheOp::ReplaceChannels(vec![channel("c", 0)])]);
        db.flush();
    }
    let db = open(&temp);
    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert_eq!(snapshot.channels.len(), 1, "não pode duplicar por reabrir");
}

#[test]
fn servers_are_partitioned() {
    let temp = TempDb::new("partition");
    let db = open(&temp);
    db.submit(
        "srv-a",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![message("123", "geral", "do a", 1_000)],
        }],
    );
    db.submit(
        "srv-b",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![message("123", "geral", "do b", 2_000)],
        }],
    );
    db.flush();

    let a = db.load_snapshot("srv-a").expect("a");
    let b = db.load_snapshot("srv-b").expect("b");
    assert_eq!(a.messages.len(), 1);
    assert_eq!(a.messages[0].content, "do a");
    assert_eq!(b.messages.len(), 1);
    assert_eq!(b.messages[0].content, "do b");
}

#[test]
fn clear_server_only_touches_its_own_rows() {
    let temp = TempDb::new("clear");
    let db = open(&temp);
    for server in ["srv-a", "srv-b"] {
        db.submit(
            server,
            vec![
                CacheOp::ReplaceChannels(vec![channel("geral", 0)]),
                CacheOp::ReplaceChannelSnapshot {
                    channel_id: "geral".to_owned(),
                    cached_at: now_millis(),
                    messages: vec![message("m1", "geral", server, 1_000)],
                },
            ],
        );
    }
    db.flush();

    db.clear_server("srv-a");
    db.flush();

    let a = db.load_snapshot("srv-a").expect("a");
    assert!(a.is_empty(), "srv-a deve sumir inteiro: {a:?}");
    let b = db.load_snapshot("srv-b").expect("b");
    assert_eq!(b.messages.len(), 1);
    assert_eq!(b.channels.len(), 1);
    assert_eq!(b.messages[0].content, "srv-b");
}

#[test]
fn empty_snapshot_is_distinguishable_from_missing() {
    let temp = TempDb::new("empty");
    let db = open(&temp);
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "vazio".to_owned(),
            cached_at: now_millis(),
            messages: Vec::new(),
        }],
    );
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert!(snapshot.messages.is_empty());
    assert!(snapshot.cached_channels.contains("vazio"));
    assert!(!snapshot.is_empty());
}

#[test]
fn upsert_edit_delete_pin_reaction_round_trip() {
    let temp = TempDb::new("mutations");
    let db = open(&temp);
    let mut msg = message("m1", "geral", "primeiro", 1_000);
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![msg.clone()],
        }],
    );
    db.flush();

    msg.content = "editado".to_owned();
    msg.edited = true;
    msg.pinned = true;
    msg.reactions.push(CachedReaction {
        emoji_unicode: Some("👍".to_owned()),
        emoji_custom: None,
        count: 3,
        mine: true,
    });
    db.submit("srv", vec![CacheOp::UpsertMessage(msg.clone())]);
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    let stored = &snapshot.messages[0];
    assert_eq!(stored.content, "editado");
    assert!(stored.edited);
    assert!(stored.pinned);
    assert_eq!(stored.reactions.len(), 1);
    assert_eq!(stored.reactions[0].count, 3);
    assert!(stored.reactions[0].mine);

    db.submit(
        "srv",
        vec![CacheOp::DeleteMessage {
            message_id: "m1".to_owned(),
        }],
    );
    db.flush();
    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert!(snapshot.messages.is_empty());
    // O marcador de cache do canal sobrevive à remoção de uma mensagem.
    assert!(snapshot.cached_channels.contains("geral"));
}

#[test]
fn snapshot_replacement_removes_missing_messages() {
    let temp = TempDb::new("replace");
    let db = open(&temp);
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![
                message("m1", "geral", "antiga", 1_000),
                message("m2", "geral", "fica", 2_000),
            ],
        }],
    );
    db.flush();
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![message("m2", "geral", "fica", 2_000)],
        }],
    );
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    let ids: Vec<&str> = snapshot.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["m2"]);
}

#[test]
fn retention_is_hard_bounded() {
    let temp = TempDb::new("retention");
    let db = open(&temp);
    let messages: Vec<CachedMessage> = (0..MESSAGE_RETENTION + 250)
        .map(|index| message(&format!("m{index}"), "geral", "x", index))
        .collect();
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages,
        }],
    );
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert_eq!(
        snapshot.messages.len() as i64,
        MESSAGE_RETENTION,
        "retenção deve manter só a janela recente"
    );
    // A janela mantém as mais recentes.
    assert!(snapshot.messages.iter().any(|m| m.id == format!("m{}", MESSAGE_RETENTION + 249)));
}

#[test]
fn attachment_metadata_survives() {
    let temp = TempDb::new("attachment");
    let db = open(&temp);
    let mut msg = message("m1", "geral", "com anexo", 1_000);
    msg.attachments.push(CachedAttachment {
        id: "att-1".to_owned(),
        mime_type: Some("image/png".to_owned()),
        original_file_name: Some("foto.png".to_owned()),
        size_bytes: 1234,
        thumbnail_id: Some("thumb".to_owned()),
        created_at: Some(999),
        moderation_status: Some("clean".to_owned()),
    });
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![msg],
        }],
    );
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert_eq!(snapshot.messages[0].attachments.len(), 1);
    assert_eq!(snapshot.messages[0].attachments[0].size_bytes, 1234);
}

#[test]
fn missing_directory_means_disabled_not_dead() {
    let path = Path::new("/proc/definitely-not-writable/papo-cache.db").to_path_buf();
    let db = ClientDb::open(Some(path));
    assert!(!db.is_enabled());
    db.submit("srv", vec![CacheOp::ClearServer]);
    db.flush();
    assert!(db.load_snapshot("srv").is_none());
}

#[test]
fn no_path_disables_cache() {
    let db = ClientDb::open(None);
    assert!(!db.is_enabled());
    assert!(db.load_snapshot("srv").is_none());
}

#[test]
fn live_upserts_stay_within_retention() {
    let temp = TempDb::new("live-retention");
    let db = open(&temp);
    // Começa exatamente no teto.
    let initial: Vec<CachedMessage> = (0..MESSAGE_RETENTION)
        .map(|index| message(&format!("base{index}"), "geral", "x", index))
        .collect();
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: initial,
        }],
    );
    db.flush();

    // Um WebSocket saudável só acrescenta mensagens novas, sem snapshot.
    for index in 0..300 {
        db.submit(
            "srv",
            vec![CacheOp::UpsertMessage(message(
                &format!("live{index}"),
                "geral",
                "y",
                10_000 + index,
            ))],
        );
    }
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert_eq!(
        snapshot.messages.len() as i64,
        MESSAGE_RETENTION,
        "o crescimento live precisa continuar limitado"
    );
    assert!(
        snapshot.messages.iter().any(|m| m.id == "live299"),
        "as mais recentes ficam"
    );
    assert!(
        snapshot.messages.iter().all(|m| m.id != "base0"),
        "as mais antigas saem"
    );
}

#[test]
fn pinned_upserts_stay_within_pinned_retention() {
    let temp = TempDb::new("pinned-retention");
    let db = open(&temp);
    for index in 0..(PINNED_RETENTION + 80) {
        let mut pinned = message(&format!("pin{index}"), "geral", "p", index);
        pinned.pinned = true;
        db.submit("srv", vec![CacheOp::UpsertMessage(pinned)]);
    }
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    let pinned = snapshot.messages.iter().filter(|m| m.pinned).count() as i64;
    assert_eq!(pinned, PINNED_RETENTION, "o teto de fixadas vale para live");
}

#[test]
fn clear_server_survives_saturated_data_queue() {
    use std::time::Duration;

    let temp = TempDb::new("clear-saturated");
    // Fila minúscula para saturar de forma determinística.
    let db = ClientDb::open_with_limit(Some(temp.path()), 1);
    assert!(db.is_enabled());

    // Semeia com a fila vazia e confirma cada um antes de saturar.
    db.submit(
        "srv-a",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![message("m1", "geral", "a", 1_000)],
        }],
    );
    db.flush();
    db.submit(
        "srv-b",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![message("m1", "geral", "b", 1_000)],
        }],
    );
    db.flush();

    let (entered, resume) = db.pause_worker();
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("worker precisa pausar");

    // Com o worker parado e a fila de capacidade 1, quase tudo é descartado.
    for index in 0..32 {
        db.submit(
            "srv-a",
            vec![CacheOp::UpsertMessage(message(
                &format!("late{index}"),
                "geral",
                "late",
                2_000 + index,
            ))],
        );
    }
    assert!(
        db.stats().dropped > 0,
        "a fila de dados precisa ter saturado"
    );

    // Controle não pode ser descartado, mesmo com a fila de dados cheia.
    db.clear_server("srv-a");

    resume.send(()).expect("retomar o worker");
    db.flush();

    let a = db.load_snapshot("srv-a").expect("a");
    assert!(
        a.is_empty(),
        "o clear precisa ter apagado A mesmo sob pressão: {a:?}"
    );
    let b = db.load_snapshot("srv-b").expect("b");
    assert_eq!(b.messages.len(), 1, "B não pode ser tocado");
}

#[test]
fn owner_transition_survives_saturated_data_queue() {
    use std::time::Duration;

    let temp = TempDb::new("owner-saturated");
    let db = ClientDb::open_with_limit(Some(temp.path()), 1);
    assert!(db.is_enabled());

    db.submit(
        "srv-a",
        vec![CacheOp::SetOwner {
            owner_user_id: "user-1".to_owned(),
            me_name: "Ana".to_owned(),
            me_username: "ana".to_owned(),
        }],
    );
    db.flush();

    let (entered, resume) = db.pause_worker();
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("worker precisa pausar");

    for index in 0..32 {
        db.submit(
            "srv-a",
            vec![CacheOp::UpsertMessage(message(
                &format!("m{index}"),
                "geral",
                "x",
                1_000 + index,
            ))],
        );
    }
    assert!(db.stats().dropped > 0, "fila de dados saturada");

    // `SetOwner` viaja na fila confiável, então não pode cair.
    db.submit(
        "srv-a",
        vec![CacheOp::SetOwner {
            owner_user_id: "user-2".to_owned(),
            me_name: "Bia".to_owned(),
            me_username: "bia".to_owned(),
        }],
    );

    resume.send(()).expect("retomar o worker");
    db.flush();

    let snapshot = db.load_snapshot("srv-a").expect("snapshot");
    assert_eq!(
        snapshot.owner_user_id.as_deref(),
        Some("user-2"),
        "a troca de dono não pode ser descartada"
    );
}

#[test]
fn clear_server_orders_after_large_backlog() {
    use std::time::Duration;

    let temp = TempDb::new("clear-backlog");
    let db = open(&temp);

    db.submit(
        "srv-a",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![message("seed", "geral", "seed", 0)],
        }],
    );
    db.submit(
        "srv-b",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![message("b", "geral", "b", 0)],
        }],
    );
    db.flush();

    let (entered, resume) = db.pause_worker();
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("worker precisa pausar");

    // Backlog maior que o BATCH_LIMIT do worker: um lote local não pode
    // reordenar o clear para o meio dele.
    let backlog = BATCH_LIMIT + 300;
    for index in 0..backlog {
        db.submit(
            "srv-a",
            vec![CacheOp::UpsertMessage(message(
                &format!("pre{index}"),
                "geral",
                "pre",
                1_000 + index as i64,
            ))],
        );
    }
    assert_eq!(
        db.stats().dropped,
        0,
        "todo o backlog precisa ter sido aceito para o teste valer"
    );
    db.clear_server("srv-a");
    db.submit(
        "srv-a",
        vec![CacheOp::UpsertMessage(message("post", "geral", "post", 100_000))],
    );

    resume.send(()).expect("retomar o worker");
    db.flush();

    let a = db.load_snapshot("srv-a").expect("a");
    let ids: Vec<&str> = a.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["post"],
        "o clear precede todo o backlog aceito antes dele: {ids:?}"
    );
    let b = db.load_snapshot("srv-b").expect("b");
    assert_eq!(b.messages.len(), 1, "o outro servidor fica intacto");
}

#[test]
fn flush_is_a_barrier_behind_large_backlog() {
    let temp = TempDb::new("flush-barrier");
    let db = open(&temp);

    let backlog = BATCH_LIMIT + 100;
    for index in 0..backlog {
        db.submit(
            "srv",
            vec![CacheOp::UpsertMessage(message(
                &format!("m{index}"),
                "geral",
                "x",
                index as i64,
            ))],
        );
    }
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert_eq!(
        snapshot.messages.len(),
        backlog,
        "depois do flush tudo que foi aceito já precisa estar visível"
    );
}

#[test]
fn load_is_a_barrier_behind_previously_accepted_writes() {
    let temp = TempDb::new("load-barrier");
    let db = open(&temp);

    let backlog = BATCH_LIMIT + 100;
    for index in 0..backlog {
        db.submit(
            "srv",
            vec![CacheOp::UpsertMessage(message(
                &format!("m{index}"),
                "geral",
                "x",
                index as i64,
            ))],
        );
    }

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert_eq!(
        snapshot.messages.len(),
        backlog,
        "o load observa tudo que foi aceito antes dele"
    );
}

#[test]
fn replace_channels_prunes_orphaned_rows() {
    let temp = TempDb::new("prune-channels");
    let db = open(&temp);
    db.submit(
        "srv",
        vec![
            CacheOp::ReplaceChannels(vec![channel("A", 0), channel("B", 1)]),
            CacheOp::ReplaceChannelSnapshot {
                channel_id: "A".to_owned(),
                cached_at: now_millis(),
                messages: vec![message("a1", "A", "no a", 1_000)],
            },
            CacheOp::ReplaceChannelSnapshot {
                channel_id: "B".to_owned(),
                cached_at: now_millis(),
                messages: vec![message("b1", "B", "no b", 1_000)],
            },
        ],
    );
    db.flush();

    // Autoritativo: B foi apagado no servidor.
    db.submit("srv", vec![CacheOp::ReplaceChannels(vec![channel("A", 0)])]);
    db.flush();

    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert_eq!(snapshot.channels.len(), 1);
    assert_eq!(snapshot.channels[0].id, "A");
    assert!(
        snapshot.messages.iter().all(|m| m.channel_id != "B"),
        "mensagens de B precisam sumir: {:?}",
        snapshot.messages
    );
    assert!(
        !snapshot.cached_channels.contains("B"),
        "o marcador de cache de B precisa sumir"
    );
    assert!(snapshot.messages.iter().any(|m| m.id == "a1"), "A intacto");
}

#[test]
fn authoritative_pins_drop_stale_row_after_reopen() {
    let temp = TempDb::new("pins-drop");
    let db = open(&temp);

    // P antiga e fixada, fora da janela recente.
    let mut old_pin = message("P", "geral", "pin antigo", 1);
    old_pin.pinned = true;
    let mut snapshot_messages = vec![old_pin];
    for index in 0..(MESSAGE_RETENTION + 50) {
        snapshot_messages.push(message(
            &format!("recent{index}"),
            "geral",
            "r",
            10_000 + index,
        ));
    }
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: snapshot_messages,
        }],
    );
    db.flush();
    assert!(
        db.load_snapshot("srv")
            .expect("snapshot")
            .messages
            .iter()
            .any(|m| m.id == "P")
    );

    // O servidor não fixa mais P.
    db.submit(
        "srv",
        vec![CacheOp::ReplacePins {
            channel_id: "geral".to_owned(),
            ids: Vec::new(),
        }],
    );
    db.flush();
    drop(db);

    let reopened = open(&temp);
    let snapshot = reopened.load_snapshot("srv").expect("snapshot");
    assert!(
        snapshot.messages.iter().all(|m| m.id != "P"),
        "P desafixada e velha não pode voltar: {:?}",
        snapshot.messages.iter().map(|m| &m.id).collect::<Vec<_>>()
    );
}

#[test]
fn authoritative_pins_keep_old_pinned_row_after_reopen() {
    let temp = TempDb::new("pins-keep");
    let db = open(&temp);

    let mut old_pin = message("P", "geral", "pin antigo", 1);
    old_pin.pinned = true;
    let mut snapshot_messages = vec![old_pin];
    for index in 0..(MESSAGE_RETENTION + 50) {
        snapshot_messages.push(message(
            &format!("recent{index}"),
            "geral",
            "r",
            10_000 + index,
        ));
    }
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: snapshot_messages,
        }],
    );
    db.flush();

    // O servidor continua fixando P.
    db.submit(
        "srv",
        vec![CacheOp::ReplacePins {
            channel_id: "geral".to_owned(),
            ids: vec!["P".to_owned()],
        }],
    );
    db.flush();
    drop(db);

    let reopened = open(&temp);
    let snapshot = reopened.load_snapshot("srv").expect("snapshot");
    let pin = snapshot.messages.iter().find(|m| m.id == "P");
    assert!(pin.is_some_and(|m| m.pinned), "P segue fixada e presente");
}

#[test]
fn missing_pin_snapshot_preserves_cached_pins() {
    let temp = TempDb::new("pins-preserve");
    let db = open(&temp);
    let mut pinned = message("P", "geral", "pin", 1_000);
    pinned.pinned = true;
    db.submit(
        "srv",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "geral".to_owned(),
            cached_at: now_millis(),
            messages: vec![pinned],
        }],
    );
    db.flush();
    drop(db);

    // Reabre sem nenhum ReplacePins (a busca de pins falhou): preserva.
    let reopened = open(&temp);
    let snapshot = reopened.load_snapshot("srv").expect("snapshot");
    assert!(
        snapshot
            .messages
            .iter()
            .find(|m| m.id == "P")
            .is_some_and(|m| m.pinned),
        "fixada antiga sobrevive quando o snapshot de pins falha"
    );
}

#[test]
fn restart_produces_deterministic_store_projection() {
    use crate::state::{Store, TimelineStatus};

    let temp = TempDb::new("restart-store");
    {
        // "Primeiro processo": a Store produziu estes efeitos.
        let db = open(&temp);
        db.submit(
            "srv",
            vec![
                CacheOp::SetOwner {
                    owner_user_id: "user-1".to_owned(),
                    me_name: "Ana".to_owned(),
                    me_username: "ana".to_owned(),
                },
                CacheOp::ReplaceChannels(vec![channel("geral", 0)]),
                CacheOp::ReplaceChannelSnapshot {
                    channel_id: "geral".to_owned(),
                    cached_at: now_millis(),
                    messages: vec![
                        message("m1", "geral", "do disco", 1_000),
                        message("m2", "geral", "também", 2_000),
                    ],
                },
            ],
        );
        db.flush();
    }

    // "Segundo processo": abre o mesmo arquivo e hidrata uma Store vazia.
    let db = open(&temp);
    let snapshot = db.load_snapshot("srv").expect("snapshot");
    let mut store = Store::default();
    store.restore_cached(snapshot);

    assert_eq!(store.messages.len(), 2);
    assert_eq!(store.message("m1").expect("m1").content, "do disco");
    assert_eq!(store.channels.len(), 1);
    assert_eq!(store.me, "user-1");
    assert_eq!(store.timeline_status("geral"), TimelineStatus::Stale);
}


#[test]
fn local_ids_are_unique() {
    let mut ids = std::collections::HashSet::new();
    for _ in 0..1_000 {
        assert!(ids.insert(new_local_id()), "local id duplicado");
    }
}

#[test]
fn queued_outgoing_survives_reopen() {
    let temp = TempDb::new("outgoing-reopen");
    {
        let db = open(&temp);
        db.enqueue_outgoing(
            "srv",
            outgoing("local-a", "user-a", "oi", OutgoingState::Queued),
        )
        .expect("enqueue");
    }

    let db = open(&temp);
    let rows = db.load_outgoing("srv", "user-a").expect("load");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_id, "local-a");
    assert_eq!(rows[0].state, OutgoingState::Queued);
}

#[test]
fn sending_becomes_unknown_after_process_reopen() {
    let temp = TempDb::new("outgoing-sending-reopen");
    {
        let db = open(&temp);
        db.enqueue_outgoing(
            "srv",
            outgoing("local-a", "user-a", "oi", OutgoingState::Queued),
        )
        .expect("enqueue");
        db.transition_outgoing(
            "srv",
            "user-a",
            "local-a",
            OutgoingState::Sending,
            None,
        )
        .expect("sending");
    }

    let db = open(&temp);
    let rows = db.load_outgoing("srv", "user-a").expect("load");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, OutgoingState::UnknownOutcome);
    assert!(!rows[0].state.may_auto_send());
}

#[test]
fn unknown_outcome_never_restores_as_retryable() {
    let temp = TempDb::new("outgoing-unknown-reopen");
    {
        let db = open(&temp);
        db.enqueue_outgoing(
            "srv",
            outgoing("local-a", "user-a", "oi", OutgoingState::Queued),
        )
        .expect("enqueue");
        db.transition_outgoing(
            "srv",
            "user-a",
            "local-a",
            OutgoingState::UnknownOutcome,
            Some("ambiguous".to_owned()),
        )
        .expect("unknown");
    }

    let db = open(&temp);
    let rows = db.load_outgoing("srv", "user-a").expect("load");
    assert_eq!(rows[0].state, OutgoingState::UnknownOutcome);
    assert!(!rows[0].state.may_auto_send());
}

#[test]
fn identical_outgoing_messages_keep_distinct_local_ids() {
    let temp = TempDb::new("outgoing-identical");
    let db = open(&temp);
    for id in ["local-a", "local-b"] {
        db.enqueue_outgoing(
            "srv",
            outgoing(id, "user-a", "hello", OutgoingState::Queued),
        )
        .expect("enqueue");
    }
    let rows = db.load_outgoing("srv", "user-a").expect("load");
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].local_id, rows[1].local_id);
    assert!(rows.iter().all(|row| row.content == "hello"));
}

#[test]
fn outgoing_rows_are_partitioned_by_owner_and_server() {
    let temp = TempDb::new("outgoing-partition");
    let db = open(&temp);
    db.enqueue_outgoing(
        "srv-a",
        outgoing("local-a", "user-a", "a", OutgoingState::Queued),
    )
    .expect("a");
    db.enqueue_outgoing(
        "srv-a",
        outgoing("local-b", "user-b", "b", OutgoingState::Queued),
    )
    .expect("b");
    db.enqueue_outgoing(
        "srv-b",
        outgoing("local-c", "user-a", "c", OutgoingState::Queued),
    )
    .expect("c");

    let a = db.load_outgoing("srv-a", "user-a").expect("a");
    let b = db.load_outgoing("srv-a", "user-b").expect("b");
    let other_server = db.load_outgoing("srv-b", "user-a").expect("other");
    assert_eq!(a.iter().map(|row| row.local_id.as_str()).collect::<Vec<_>>(), vec!["local-a"]);
    assert_eq!(b.iter().map(|row| row.local_id.as_str()).collect::<Vec<_>>(), vec!["local-b"]);
    assert_eq!(
        other_server
            .iter()
            .map(|row| row.local_id.as_str())
            .collect::<Vec<_>>(),
        vec!["local-c"]
    );
}

#[test]
fn confirming_one_outgoing_keeps_the_other_and_caches_server_message() {
    let temp = TempDb::new("outgoing-confirm-one");
    let db = open(&temp);
    for id in ["local-a", "local-b"] {
        db.enqueue_outgoing(
            "srv",
            outgoing(id, "user-a", "hello", OutgoingState::Queued),
        )
        .expect("enqueue");
    }

    db.confirm_outgoing(
        "srv",
        "local-a",
        message("server-a", "geral", "hello", now_millis()),
    )
    .expect("confirm");

    let rows = db.load_outgoing("srv", "user-a").expect("load");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_id, "local-b");
    let snapshot = db.load_snapshot("srv").expect("snapshot");
    assert!(snapshot.messages.iter().any(|row| row.id == "server-a"));
}

#[test]
fn rebuildable_cache_clear_preserves_outgoing_but_server_clear_removes_it() {
    let temp = TempDb::new("outgoing-clear-policy");
    let db = open(&temp);
    db.enqueue_outgoing(
        "srv",
        outgoing("local-a", "user-a", "oi", OutgoingState::Queued),
    )
    .expect("enqueue");
    db.clear_cached_data("srv");
    db.flush();
    assert_eq!(
        db.load_outgoing("srv", "user-a").expect("preserved").len(),
        1
    );

    db.clear_server("srv");
    db.flush();
    assert!(
        db.load_outgoing("srv", "user-a")
            .expect("cleared")
            .is_empty()
    );
}

#[test]
fn disabled_clientdb_rejects_durable_enqueue() {
    let db = ClientDb::open(None);
    let result = db.enqueue_outgoing(
        "srv",
        outgoing("local-a", "user-a", "oi", OutgoingState::Queued),
    );
    assert!(result.is_err(), "sem ClientDb não pode existir envio indurável");
}

#[test]
fn outgoing_queue_hard_bound_rejects_excess_without_discarding_old_rows() {
    let temp = TempDb::new("outgoing-bound");
    let db = open(&temp);
    for index in 0..OUTGOING_LIMIT {
        db.enqueue_outgoing(
            "srv",
            outgoing(
                &format!("local-{index}"),
                "user-a",
                "x",
                OutgoingState::Queued,
            ),
        )
        .expect("dentro do limite");
    }
    let excess = db.enqueue_outgoing(
        "srv",
        outgoing("local-excess", "user-a", "x", OutgoingState::Queued),
    );
    assert!(excess.is_err(), "o item 501 precisa ser rejeitado");
    let rows = db.load_outgoing("srv", "user-a").expect("load");
    assert_eq!(rows.len() as i64, OUTGOING_LIMIT);
    assert!(rows.iter().all(|row| row.local_id != "local-excess"));
}


#[test]
fn deliberate_retry_is_a_durable_explicit_transition_back_to_queued() {
    let temp = TempDb::new("outgoing-explicit-retry");
    let db = open(&temp);
    db.enqueue_outgoing(
        "srv",
        outgoing("local-a", "user-a", "hello", OutgoingState::Queued),
    )
    .expect("enqueue");
    db.transition_outgoing(
        "srv",
        "user-a",
        "local-a",
        OutgoingState::UnknownOutcome,
        Some("ambiguous".to_owned()),
    )
    .expect("unknown");
    assert_eq!(
        db.load_outgoing("srv", "user-a").expect("load")[0].state,
        OutgoingState::UnknownOutcome
    );

    db.transition_outgoing(
        "srv",
        "user-a",
        "local-a",
        OutgoingState::Queued,
        None,
    )
    .expect("explicit retry");
    assert_eq!(
        db.load_outgoing("srv", "user-a").expect("load")[0].state,
        OutgoingState::Queued
    );
}

#[test]
fn dismiss_removes_only_the_selected_outgoing_row_durably() {
    let temp = TempDb::new("outgoing-dismiss");
    let db = open(&temp);
    for id in ["local-a", "local-b"] {
        db.enqueue_outgoing(
            "srv",
            outgoing(id, "user-a", "hello", OutgoingState::UnknownOutcome),
        )
        .expect("enqueue");
    }

    db.remove_outgoing("srv", "user-a", "local-a")
        .expect("dismiss");
    let rows = db.load_outgoing("srv", "user-a").expect("load");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_id, "local-b");
}


fn cached_preview(url: &str, title: &str, used: i64) -> CachedPreview {
    CachedPreview {
        url_key: url.to_owned(),
        source_url: url.to_owned(),
        state: PreviewCacheState::Ready,
        kind: Some("link".to_owned()),
        media_url: None,
        image_url: Some(format!("{url}/cover.png")),
        embed_url: None,
        title: Some(title.to_owned()),
        description: Some("rich metadata".to_owned()),
        provider_name: Some("example".to_owned()),
        resolved_at: used,
        retry_after: None,
        failure_class: None,
        last_used_at: used,
    }
}

#[test]
fn preview_metadata_survives_clientdb_reopen() {
    let temp = TempDb::new("preview-reopen");
    let url = "https://example.com/project";
    {
        let db = open(&temp);
        db.store_preview(cached_preview(url, "Papo", 1_000))
            .expect("store preview");
    }

    let db = open(&temp);
    let row = db
        .load_preview(url)
        .expect("load preview")
        .expect("persisted preview");
    assert_eq!(row.title.as_deref(), Some("Papo"));
    assert_eq!(row.image_url.as_deref(), Some("https://example.com/project/cover.png"));
    assert_eq!(row.state, PreviewCacheState::Ready);
}

#[test]
fn preview_negative_and_retry_states_survive_reopen() {
    let temp = TempDb::new("preview-states");
    let negative_url = "https://example.com/no-media";
    let retry_url = "https://example.com/temporary";

    {
        let db = open(&temp);
        let mut negative = cached_preview(negative_url, "negative", 1_000);
        negative.state = PreviewCacheState::Negative;
        negative.kind = None;
        negative.image_url = None;
        negative.failure_class = Some("negative".to_owned());
        db.store_preview(negative).expect("store negative");

        let mut retry = cached_preview(retry_url, "retry", 2_000);
        retry.state = PreviewCacheState::RetryAfter;
        retry.kind = None;
        retry.image_url = None;
        retry.retry_after = Some(9_000);
        retry.failure_class = Some("transient".to_owned());
        db.store_preview(retry).expect("store retry");
    }

    let db = open(&temp);
    assert_eq!(
        db.load_preview(negative_url)
            .expect("negative load")
            .expect("negative row")
            .state,
        PreviewCacheState::Negative
    );
    let retry = db
        .load_preview(retry_url)
        .expect("retry load")
        .expect("retry row");
    assert_eq!(retry.state, PreviewCacheState::RetryAfter);
    assert_eq!(retry.retry_after, Some(9_000));
}

#[tokio::test]
async fn preview_retention_is_global_and_hard_bounded() {
    let temp = TempDb::new("preview-bound");
    let path = temp.path().to_string_lossy().into_owned();
    let mut cache = TursoCache::open(&path).await.expect("open cache");

    for (url, used) in [
        ("https://one.example/a", 1_i64),
        ("https://two.example/b", 2_i64),
        ("https://three.example/c", 3_i64),
    ] {
        cache
            .store_preview_for_test(&cached_preview(url, url, used), 2)
            .await
            .expect("store preview");
    }

    assert!(
        cache
            .load_preview("https://one.example/a")
            .await
            .expect("load oldest")
            .is_none(),
        "oldest metadata must be pruned"
    );
    assert!(
        cache
            .load_preview("https://two.example/b")
            .await
            .expect("load second")
            .is_some()
    );
    assert!(
        cache
            .load_preview("https://three.example/c")
            .await
            .expect("load newest")
            .is_some()
    );
}


#[test]
fn drafts_round_trip_replace_and_partition_by_owner_server_channel() {
    let temp = TempDb::new("draft-partition");
    let db = open(&temp);

    let mut general = draft("owner-a", "general", "oi @Alex");
    general.mentions.push(CachedMentionBinding {
        start: 3,
        label: "Alex".to_owned(),
        user_id: "stable-a".to_owned(),
    });
    general.reply_to = Some("m-1".to_owned());
    general.notify_reply = false;

    db.submit(
        "srv-a",
        vec![
            CacheOp::UpsertDraft(general.clone()),
            CacheOp::UpsertDraft(draft("owner-a", "rust", "cargo")),
            CacheOp::UpsertDraft(draft("owner-b", "general", "outra conta")),
        ],
    );
    db.submit(
        "srv-b",
        vec![CacheOp::UpsertDraft(draft("owner-a", "general", "outro servidor"))],
    );
    db.flush();

    let a = db.load_drafts("srv-a", "owner-a").expect("drafts owner a");
    assert_eq!(a.len(), 2);
    let restored = a.iter().find(|d| d.channel_id == "general").unwrap();
    assert_eq!(restored.text, "oi @Alex");
    assert_eq!(restored.mentions, general.mentions);
    assert_eq!(restored.reply_to.as_deref(), Some("m-1"));
    assert!(!restored.notify_reply);

    let b = db.load_drafts("srv-a", "owner-b").expect("drafts owner b");
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].text, "outra conta");
    let other_server = db.load_drafts("srv-b", "owner-a").expect("other server");
    assert_eq!(other_server.len(), 1);
    assert_eq!(other_server[0].text, "outro servidor");

    let mut replacement = draft("owner-a", "general", "substituído");
    replacement.notify_reply = true;
    db.submit("srv-a", vec![CacheOp::UpsertDraft(replacement)]);
    db.flush();
    let a = db.load_drafts("srv-a", "owner-a").unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(
        a.iter().find(|d| d.channel_id == "general").unwrap().text,
        "substituído"
    );
}

#[test]
fn drafts_survive_reconstructible_clear_but_not_delete_or_server_removal() {
    let temp = TempDb::new("draft-clear");
    let db = open(&temp);

    db.submit(
        "srv",
        vec![CacheOp::UpsertDraft(draft("owner", "general", "fica"))],
    );
    db.clear_cached_data("srv");
    db.flush();
    assert_eq!(db.load_drafts("srv", "owner").unwrap().len(), 1);

    db.submit(
        "srv",
        vec![CacheOp::DeleteDraft {
            owner_user_id: "owner".to_owned(),
            channel_id: "general".to_owned(),
        }],
    );
    db.flush();
    assert!(db.load_drafts("srv", "owner").unwrap().is_empty());

    db.submit(
        "srv",
        vec![CacheOp::UpsertDraft(draft("owner", "general", "some"))],
    );
    db.clear_server("srv");
    db.flush();
    assert!(db.load_drafts("srv", "owner").unwrap().is_empty());
}

#[test]
fn v4_database_migrates_to_v5_without_reset() {
    let temp = TempDb::new("draft-v4-migration");
    let path = temp.path();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let path = path.to_string_lossy().into_owned();
        let db = turso::Builder::new_local(&path)
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('schema_version', '4')",
            (),
        )
        .await
        .unwrap();
    });

    let db = open(&temp);
    assert!(db.is_enabled());
    db.submit(
        "srv",
        vec![CacheOp::UpsertDraft(draft("owner", "general", "migrado"))],
    );
    db.flush();
    let rows = db.load_drafts("srv", "owner").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text, "migrado");
}

//! Implementação SQL do cache sobre Turso.
//!
//! Nada aqui é autoridade: o servidor continua mandando. Este módulo só
//! grava e lê a projeção delimitada que a Store produz.

use std::collections::HashSet;

use turso::{Builder, Connection, Value};

use super::schema::apply_migrations;
use super::types::{
    CachedAttachment, CachedChannel, CachedMember, CachedMessage, CachedOutgoing, CachedReaction,
    CachedServer, CachedServerSnapshot, CacheOp, OutgoingState, MESSAGE_RETENTION,
    OUTGOING_LIMIT, PINNED_RETENTION,
};

/// Conexão de trabalho do cache. Uma só por processo, dona de um worker.
pub struct TursoCache {
    conn: Connection,
}

struct Stmt {
    sql: &'static str,
    params: Vec<Value>,
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn opt_text(value: Option<&str>) -> Value {
    value.map(|v| Value::Text(v.to_owned())).unwrap_or(Value::Null)
}

fn integer(value: i64) -> Value {
    Value::Integer(value)
}

fn boolean(value: bool) -> Value {
    Value::Integer(value as i64)
}

fn encode_color(color: Option<[u8; 3]>) -> Value {
    match color {
        Some([r, g, b]) => Value::Text(format!("#{r:02x}{g:02x}{b:02x}")),
        None => Value::Null,
    }
}

fn encode_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[]".to_owned())
}

const UPSERT_MESSAGE: &str = "INSERT INTO messages (
        server_key, message_id, channel_id, author_id, content, created_at,
        edited, reply_to, pinned, attachments, reactions
    ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
    ON CONFLICT(server_key, message_id) DO UPDATE SET
        channel_id = excluded.channel_id,
        author_id = excluded.author_id,
        content = excluded.content,
        created_at = excluded.created_at,
        edited = excluded.edited,
        reply_to = excluded.reply_to,
        pinned = excluded.pinned,
        attachments = excluded.attachments,
        reactions = excluded.reactions";

fn message_params(server_key: &str, message: &CachedMessage) -> Vec<Value> {
    vec![
        text(server_key),
        text(&message.id),
        text(&message.channel_id),
        text(&message.author_id),
        text(&message.content),
        integer(message.created_at),
        boolean(message.edited),
        opt_text(message.reply_to.as_deref()),
        boolean(message.pinned),
        text(&encode_json(&message.attachments)),
        text(&encode_json(&message.reactions)),
    ]
}

/// Mantém só as mensagens mais recentes por canal, com tetos duros também
/// para as fixadas.
fn retention_statements(server_key: &str, channel_id: &str) -> [Stmt; 2] {
    [
        Stmt {
            sql: "DELETE FROM messages
                  WHERE server_key = ?1 AND channel_id = ?2 AND pinned = 0
                    AND message_id NOT IN (
                        SELECT message_id FROM messages
                        WHERE server_key = ?1 AND channel_id = ?2 AND pinned = 0
                        ORDER BY created_at DESC LIMIT ?3
                    )",
            params: vec![
                text(server_key),
                text(channel_id),
                integer(MESSAGE_RETENTION),
            ],
        },
        Stmt {
            sql: "DELETE FROM messages
                  WHERE server_key = ?1 AND channel_id = ?2 AND pinned = 1
                    AND message_id NOT IN (
                        SELECT message_id FROM messages
                        WHERE server_key = ?1 AND channel_id = ?2 AND pinned = 1
                        ORDER BY created_at DESC LIMIT ?3
                    )",
            params: vec![
                text(server_key),
                text(channel_id),
                integer(PINNED_RETENTION),
            ],
        },
    ]
}

fn statements_for(server_key: &str, op: &CacheOp) -> Vec<Stmt> {
    match op {
        CacheOp::UpsertServer(server) => vec![Stmt {
            sql: "INSERT INTO server_cache (
                      server_key, owner_user_id, server_name, server_description,
                      me_user_id, me_display_name, me_username, updated_at
                  ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                  ON CONFLICT(server_key) DO UPDATE SET
                      server_name = excluded.server_name,
                      server_description = excluded.server_description,
                      owner_user_id = COALESCE(excluded.owner_user_id, owner_user_id),
                      me_user_id = COALESCE(excluded.me_user_id, me_user_id),
                      me_display_name = COALESCE(excluded.me_display_name, me_display_name),
                      me_username = COALESCE(excluded.me_username, me_username),
                      updated_at = excluded.updated_at",
            params: vec![
                text(server_key),
                opt_text(server.owner_user_id.as_deref()),
                text(&server.name),
                opt_text(server.description.as_deref()),
                opt_text(server.me_user_id.as_deref()),
                opt_text(server.me_display_name.as_deref()),
                opt_text(server.me_username.as_deref()),
                integer(server.updated_at),
            ],
        }],
        CacheOp::SetOwner {
            owner_user_id,
            me_name,
            me_username,
        } => vec![Stmt {
            sql: "INSERT INTO server_cache (
                      server_key, owner_user_id, me_user_id, me_display_name,
                      me_username, updated_at
                  ) VALUES (?1,?2,?2,?3,?4,?5)
                  ON CONFLICT(server_key) DO UPDATE SET
                      owner_user_id = excluded.owner_user_id,
                      me_user_id = excluded.me_user_id,
                      me_display_name = excluded.me_display_name,
                      me_username = excluded.me_username,
                      updated_at = excluded.updated_at",
            params: vec![
                text(server_key),
                text(owner_user_id),
                text(me_name),
                text(me_username),
                integer(super::types::now_millis()),
            ],
        }],
        CacheOp::ReplaceChannels(channels) => {
            let mut statements = vec![Stmt {
                sql: "DELETE FROM channels WHERE server_key = ?1",
                params: vec![text(server_key)],
            }];
            for channel in channels {
                statements.push(Stmt {
                    sql: "INSERT INTO channels (
                              server_key, channel_id, name, kind, topic, position,
                              unread, mentions, updated_at
                          ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
                          ON CONFLICT(server_key, channel_id) DO UPDATE SET
                              name = excluded.name,
                              kind = excluded.kind,
                              topic = excluded.topic,
                              position = excluded.position,
                              unread = excluded.unread,
                              mentions = excluded.mentions,
                              updated_at = excluded.updated_at",
                    params: vec![
                        text(server_key),
                        text(&channel.id),
                        text(&channel.name),
                        text(&channel.kind),
                        opt_text(channel.topic.as_deref()),
                        integer(channel.position as i64),
                        boolean(channel.unread),
                        integer(channel.mentions as i64),
                        integer(super::types::now_millis()),
                    ],
                });
            }
            // Só agora a subconsulta enxerga a lista autoritativa nova; canais
            // que sumiram perdem mensagens e marcador de cache junto.
            statements.push(Stmt {
                sql: "DELETE FROM messages
                      WHERE server_key = ?1
                        AND channel_id NOT IN (
                            SELECT channel_id FROM channels WHERE server_key = ?1
                        )",
                params: vec![text(server_key)],
            });
            statements.push(Stmt {
                sql: "DELETE FROM channel_cache_state
                      WHERE server_key = ?1
                        AND channel_id NOT IN (
                            SELECT channel_id FROM channels WHERE server_key = ?1
                        )",
                params: vec![text(server_key)],
            });
            statements
        }
        CacheOp::ReplaceMembers(members) => {
            let mut statements = vec![Stmt {
                sql: "DELETE FROM members WHERE server_key = ?1",
                params: vec![text(server_key)],
            }];
            for member in members {
                statements.push(Stmt {
                    sql: "INSERT INTO members (
                              server_key, user_id, username, name, role_color,
                              roles, updated_at
                          ) VALUES (?1,?2,?3,?4,?5,?6,?7)
                          ON CONFLICT(server_key, user_id) DO UPDATE SET
                              username = excluded.username,
                              name = excluded.name,
                              role_color = excluded.role_color,
                              roles = excluded.roles,
                              updated_at = excluded.updated_at",
                    params: vec![
                        text(server_key),
                        text(&member.id),
                        text(&member.username),
                        text(&member.name),
                        encode_color(member.role_color),
                        text(&encode_json(&member.roles)),
                        integer(super::types::now_millis()),
                    ],
                });
            }
            statements
        }
        CacheOp::ReplaceChannelSnapshot {
            channel_id,
            messages,
            cached_at,
        } => {
            let mut statements = vec![
                Stmt {
                    // Mensagens confirmadas somem; as fixadas ficam até o
                    // snapshot ou um evento de exclusão dizerem o contrário.
                    sql: "DELETE FROM messages
                          WHERE server_key = ?1 AND channel_id = ?2 AND pinned = 0",
                    params: vec![text(server_key), text(channel_id)],
                },
                Stmt {
                    sql: "INSERT INTO channel_cache_state (server_key, channel_id, cached_at)
                          VALUES (?1,?2,?3)
                          ON CONFLICT(server_key, channel_id) DO UPDATE SET
                              cached_at = excluded.cached_at",
                    params: vec![text(server_key), text(channel_id), integer(*cached_at)],
                },
            ];
            for message in messages {
                statements.push(Stmt {
                    sql: UPSERT_MESSAGE,
                    params: message_params(server_key, message),
                });
            }
            // A retenção deste canal roda uma vez no fim do lote.
            statements
        }
        CacheOp::UpsertMessage(message) => vec![Stmt {
            sql: UPSERT_MESSAGE,
            params: message_params(server_key, message),
        }],
        CacheOp::DeleteMessage { message_id } => vec![Stmt {
            sql: "DELETE FROM messages WHERE server_key = ?1 AND message_id = ?2",
            params: vec![text(server_key), text(message_id)],
        }],
        CacheOp::ReplacePins { channel_id, ids } => {
            // Limpa o estado local de fixadas do canal e reaplica só o que o
            // servidor confirmou. Fixadas fora da janela recente que saíram do
            // conjunto ficam desafixadas e a retenção pode removê-las.
            let mut statements = vec![Stmt {
                sql: "UPDATE messages SET pinned = 0
                      WHERE server_key = ?1 AND channel_id = ?2 AND pinned = 1",
                params: vec![text(server_key), text(channel_id)],
            }];
            for message_id in ids {
                statements.push(Stmt {
                    sql: "UPDATE messages SET pinned = 1
                          WHERE server_key = ?1 AND message_id = ?2",
                    params: vec![text(server_key), text(message_id)],
                });
            }
            statements
        }
        CacheOp::ClearCachedData => vec![
            Stmt {
                sql: "DELETE FROM messages WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM channels WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM members WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM channel_cache_state WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM server_cache WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
        ],
        CacheOp::ClearServer => vec![
            Stmt {
                sql: "DELETE FROM send_queue WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM messages WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM channels WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM members WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM channel_cache_state WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
            Stmt {
                sql: "DELETE FROM server_cache WHERE server_key = ?1",
                params: vec![text(server_key)],
            },
        ],
    }
}

impl TursoCache {
    /// Abre o banco e aplica as migrações pendentes.
    pub async fn open(path: &str) -> Result<Self, turso::Error> {
        let db = Builder::new_local(path).build().await?;
        let mut conn = db.connect()?;
        let version = apply_migrations(&mut conn).await?;
        log::debug!("cache: opened {path} schema_version={version}");
        Ok(Self { conn })
    }

    /// Aplica um lote de operações numa transação só.
    pub async fn apply_batch(
        &mut self,
        server_key: &str,
        ops: &[CacheOp],
    ) -> Result<(), turso::Error> {
        let mut statements: Vec<Stmt> = Vec::new();
        let mut retention_channels: Vec<String> = Vec::new();
        for op in ops {
            // Toda operação que pode aumentar (ou desafixar) mensagens de um
            // canal passa pela retenção, uma vez por canal no fim do lote.
            match op {
                CacheOp::UpsertMessage(message) => {
                    retention_channels.push(message.channel_id.clone());
                }
                CacheOp::ReplaceChannelSnapshot { channel_id, .. }
                | CacheOp::ReplacePins { channel_id, .. } => {
                    retention_channels.push(channel_id.clone());
                }
                _ => {}
            }
            statements.extend(statements_for(server_key, op));
        }

        let mut seen: HashSet<&str> = HashSet::new();
        for channel_id in &retention_channels {
            if seen.insert(channel_id.as_str()) {
                statements.extend(retention_statements(server_key, channel_id));
            }
        }

        if statements.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction().await?;
        for statement in &statements {
            tx.execute(statement.sql, statement.params.clone()).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Insere uma intenção de envio de forma transacional e aplica o limite
    /// duro por conta/servidor. O worker é o único escritor.
    pub async fn enqueue_outgoing(
        &mut self,
        server_key: &str,
        outgoing: &CachedOutgoing,
    ) -> Result<(), turso::Error> {
        let tx = self.conn.transaction().await?;
        let mut rows = tx
            .query(
                "SELECT COUNT(*) FROM send_queue
                 WHERE server_key = ?1 AND owner_user_id = ?2",
                [server_key, outgoing.owner_user_id.as_str()],
            )
            .await?;
        let count = rows
            .next()
            .await?
            .map(|row| row.get::<i64>(0))
            .transpose()?
            .unwrap_or(0);
        drop(rows);
        if count >= OUTGOING_LIMIT {
            return Err(turso::Error::Misuse(format!(
                "outgoing queue full ({OUTGOING_LIMIT})"
            )));
        }
        tx.execute(
            "INSERT INTO send_queue (
                 server_key, local_id, owner_user_id, channel_id, content,
                 reply_to, notify_reply, created_at, state, attempt_count,
                 last_attempt_at, last_error
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            vec![
                text(server_key),
                text(&outgoing.local_id),
                text(&outgoing.owner_user_id),
                text(&outgoing.channel_id),
                text(&outgoing.content),
                opt_text(outgoing.reply_to.as_deref()),
                boolean(outgoing.notify_reply),
                integer(outgoing.created_at),
                text(outgoing.state.as_db()),
                integer(outgoing.attempt_count as i64),
                outgoing.last_attempt_at.map(Value::Integer).unwrap_or(Value::Null),
                opt_text(outgoing.last_error.as_deref()),
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Carrega somente a fila da conta informada. Qualquer envio que estava
    /// em Sending quando o processo morreu vira UnknownOutcome antes da
    /// leitura; reinício nunca transforma uma transmissão possivelmente
    /// concluída em retry automático.
    pub async fn load_outgoing(
        &mut self,
        server_key: &str,
        owner_user_id: &str,
    ) -> Result<Vec<CachedOutgoing>, turso::Error> {
        self.conn
            .execute(
                "UPDATE send_queue
                 SET state = 'unknown_outcome',
                     last_error = COALESCE(last_error, 'process interrupted while sending')
                 WHERE server_key = ?1 AND owner_user_id = ?2 AND state = 'sending'",
                [server_key, owner_user_id],
            )
            .await?;

        let mut rows = self
            .conn
            .query(
                "SELECT local_id, owner_user_id, channel_id, content, reply_to,
                        notify_reply, created_at, state, attempt_count,
                        last_attempt_at, last_error
                 FROM send_queue
                 WHERE server_key = ?1 AND owner_user_id = ?2
                 ORDER BY created_at, local_id",
                [server_key, owner_user_id],
            )
            .await?;
        let mut outgoing = Vec::new();
        while let Some(row) = rows.next().await? {
            let raw_state: String = row.get(7)?;
            let Some(state) = OutgoingState::from_db(&raw_state) else {
                return Err(turso::Error::Misuse(format!(
                    "unknown outgoing state {raw_state:?}"
                )));
            };
            outgoing.push(CachedOutgoing {
                local_id: row.get(0)?,
                owner_user_id: row.get(1)?,
                channel_id: row.get(2)?,
                content: row.get(3)?,
                reply_to: row.get(4)?,
                notify_reply: row.get(5)?,
                created_at: row.get(6)?,
                state,
                attempt_count: row.get::<i64>(8)?.max(0) as u32,
                last_attempt_at: row.get(9)?,
                last_error: row.get(10)?,
            });
        }
        Ok(outgoing)
    }

    pub async fn transition_outgoing(
        &mut self,
        server_key: &str,
        owner_user_id: &str,
        local_id: &str,
        state: OutgoingState,
        last_error: Option<&str>,
    ) -> Result<(), turso::Error> {
        let now = super::types::now_millis();
        if state == OutgoingState::Sending {
            self.conn
                .execute(
                    "UPDATE send_queue
                     SET state = ?1,
                         attempt_count = attempt_count + 1,
                         last_attempt_at = ?2,
                         last_error = NULL
                     WHERE server_key = ?3 AND owner_user_id = ?4 AND local_id = ?5",
                    vec![
                        text(state.as_db()),
                        integer(now),
                        text(server_key),
                        text(owner_user_id),
                        text(local_id),
                    ],
                )
                .await?;
        } else {
            self.conn
                .execute(
                    "UPDATE send_queue
                     SET state = ?1, last_error = ?2
                     WHERE server_key = ?3 AND owner_user_id = ?4 AND local_id = ?5",
                    vec![
                        text(state.as_db()),
                        opt_text(last_error),
                        text(server_key),
                        text(owner_user_id),
                        text(local_id),
                    ],
                )
                .await?;
        }
        Ok(())
    }

    /// Confirma uma única intenção. A mensagem de servidor entra no cache e a
    /// linha de saída some na mesma transação.
    pub async fn confirm_outgoing(
        &mut self,
        server_key: &str,
        local_id: &str,
        message: &CachedMessage,
    ) -> Result<(), turso::Error> {
        let tx = self.conn.transaction().await?;
        tx.execute(UPSERT_MESSAGE, message_params(server_key, message))
            .await?;
        for statement in retention_statements(server_key, &message.channel_id) {
            tx.execute(statement.sql, statement.params).await?;
        }
        tx.execute(
            "DELETE FROM send_queue WHERE server_key = ?1 AND local_id = ?2",
            [server_key, local_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_outgoing(
        &mut self,
        server_key: &str,
        owner_user_id: &str,
        local_id: &str,
    ) -> Result<(), turso::Error> {
        self.conn
            .execute(
                "DELETE FROM send_queue
                 WHERE server_key = ?1 AND owner_user_id = ?2 AND local_id = ?3",
                [server_key, owner_user_id, local_id],
            )
            .await?;
        Ok(())
    }

    /// Lê a projeção persistida de um servidor. Nunca é autoridade.
    pub async fn load_snapshot(
        &self,
        server_key: &str,
    ) -> Result<CachedServerSnapshot, turso::Error> {
        let mut snapshot = CachedServerSnapshot::default();

        let mut rows = self
            .conn
            .query(
                "SELECT server_name, server_description, owner_user_id, me_user_id,
                        me_display_name, me_username, updated_at
                 FROM server_cache WHERE server_key = ?1",
                [server_key],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            snapshot.owner_user_id = row.get::<Option<String>>(2)?;
            snapshot.server = Some(CachedServer {
                name: row.get::<Option<String>>(0)?.unwrap_or_default(),
                description: row.get(1)?,
                owner_user_id: row.get(2)?,
                me_user_id: row.get(3)?,
                me_display_name: row.get(4)?,
                me_username: row.get(5)?,
                updated_at: row.get(6)?,
            });
        }
        drop(rows);

        let mut rows = self
            .conn
            .query(
                "SELECT channel_id, name, kind, topic, position, unread, mentions
                 FROM channels WHERE server_key = ?1 ORDER BY position",
                [server_key],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            snapshot.channels.push(CachedChannel {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                topic: row.get(3)?,
                position: row.get::<i64>(4)? as i32,
                unread: row.get(5)?,
                mentions: row.get::<i64>(6)? as u32,
            });
        }
        drop(rows);

        let mut rows = self
            .conn
            .query(
                "SELECT user_id, username, name, role_color, roles
                 FROM members WHERE server_key = ?1",
                [server_key],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let color: Option<String> = row.get(3)?;
            let roles: String = row.get(4)?;
            snapshot.members.push(CachedMember {
                id: row.get(0)?,
                username: row.get(1)?,
                name: row.get(2)?,
                role_color: color
                    .as_deref()
                    .and_then(crate::api::models::parse_hex_color),
                roles: serde_json::from_str(&roles).unwrap_or_default(),
            });
        }
        drop(rows);

        let mut rows = self
            .conn
            .query(
                "SELECT message_id, channel_id, author_id, content, created_at,
                        edited, reply_to, pinned, attachments, reactions
                 FROM messages WHERE server_key = ?1 ORDER BY created_at",
                [server_key],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            let attachments: String = row.get(8)?;
            let reactions: String = row.get(9)?;
            snapshot.messages.push(CachedMessage {
                id: row.get(0)?,
                channel_id: row.get(1)?,
                author_id: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                edited: row.get(5)?,
                reply_to: row.get(6)?,
                pinned: row.get(7)?,
                attachments: serde_json::from_str::<Vec<CachedAttachment>>(&attachments)
                    .unwrap_or_default(),
                reactions: serde_json::from_str::<Vec<CachedReaction>>(&reactions)
                    .unwrap_or_default(),
            });
        }
        drop(rows);

        let mut rows = self
            .conn
            .query(
                "SELECT channel_id FROM channel_cache_state WHERE server_key = ?1",
                [server_key],
            )
            .await?;
        let mut cached: HashSet<String> = HashSet::new();
        while let Some(row) = rows.next().await? {
            cached.insert(row.get(0)?);
        }
        drop(rows);

        // Qualquer canal com mensagem guardada também conta como cacheado.
        for message in &snapshot.messages {
            cached.insert(message.channel_id.clone());
        }
        snapshot.cached_channels = cached;

        Ok(snapshot)
    }
}

//! Migrações versionadas do banco de cache.
//!
//! Cada migração é uma lista de instruções aplicadas numa transação só. A
//! versão fica em `meta.schema_version`; se qualquer passo falhar, nada
//! avança e o cache é tratado como indisponível.

use turso::Connection;

/// Versão que o código espera encontrar depois de migrar.
pub const SCHEMA_VERSION: i64 = 11;

/// Uma migração: versão alvo e as instruções SQL que a compõem.
struct Migration {
    version: i64,
    statements: &'static [&'static str],
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: &[
        "CREATE TABLE server_cache (
             server_key TEXT PRIMARY KEY,
             owner_user_id TEXT,
             server_name TEXT,
             server_description TEXT,
             me_user_id TEXT,
             me_display_name TEXT,
             me_username TEXT,
             updated_at INTEGER NOT NULL
         )",
        "CREATE TABLE channels (
             server_key TEXT NOT NULL,
             channel_id TEXT NOT NULL,
             name TEXT NOT NULL,
             kind TEXT NOT NULL,
             topic TEXT,
             position INTEGER NOT NULL,
             unread INTEGER NOT NULL DEFAULT 0,
             mentions INTEGER NOT NULL DEFAULT 0,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (server_key, channel_id)
         )",
        "CREATE TABLE members (
             server_key TEXT NOT NULL,
             user_id TEXT NOT NULL,
             username TEXT NOT NULL,
             name TEXT NOT NULL,
             role_color TEXT,
             roles TEXT NOT NULL,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (server_key, user_id)
         )",
        "CREATE TABLE messages (
             server_key TEXT NOT NULL,
             message_id TEXT NOT NULL,
             channel_id TEXT NOT NULL,
             author_id TEXT NOT NULL,
             content TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             edited INTEGER NOT NULL DEFAULT 0,
             reply_to TEXT,
             pinned INTEGER NOT NULL DEFAULT 0,
             attachments TEXT NOT NULL DEFAULT '[]',
             reactions TEXT NOT NULL DEFAULT '[]',
             PRIMARY KEY (server_key, message_id)
         )",
        "CREATE INDEX messages_channel_idx ON messages(server_key, channel_id, created_at)",
        "CREATE TABLE channel_cache_state (
             server_key TEXT NOT NULL,
             channel_id TEXT NOT NULL,
             cached_at INTEGER NOT NULL,
             PRIMARY KEY (server_key, channel_id)
         )",
    ],
}, Migration {
    version: 2,
    statements: &[
        "CREATE TABLE send_queue (
             server_key TEXT NOT NULL,
             local_id TEXT NOT NULL,
             owner_user_id TEXT NOT NULL,
             channel_id TEXT NOT NULL,
             content TEXT NOT NULL,
             reply_to TEXT,
             notify_reply INTEGER NOT NULL,
             created_at INTEGER NOT NULL,
             state TEXT NOT NULL,
             attempt_count INTEGER NOT NULL DEFAULT 0,
             last_attempt_at INTEGER,
             last_error TEXT,
             PRIMARY KEY (server_key, local_id)
         )",
        "CREATE INDEX send_queue_owner_idx
             ON send_queue(server_key, owner_user_id, created_at)",
    ],
}, Migration {
    version: 3,
    statements: &[
        "CREATE TABLE notification_ledger (
             server_key TEXT NOT NULL,
             owner_user_id TEXT NOT NULL,
             message_id TEXT NOT NULL,
             channel_id TEXT NOT NULL,
             notification_id TEXT,
             decision TEXT NOT NULL,
             reason TEXT,
             handled_at INTEGER NOT NULL,
             PRIMARY KEY (server_key, owner_user_id, message_id)
         )",
        "CREATE INDEX notification_ledger_retention_idx
             ON notification_ledger(server_key, owner_user_id, handled_at DESC)",
    ],
}, Migration {
    version: 4,
    statements: &[
        "CREATE TABLE preview_cache (
             url_key TEXT PRIMARY KEY,
             source_url TEXT NOT NULL,
             state TEXT NOT NULL,
             kind TEXT,
             media_url TEXT,
             image_url TEXT,
             embed_url TEXT,
             title TEXT,
             description TEXT,
             provider_name TEXT,
             resolved_at INTEGER NOT NULL,
             retry_after INTEGER,
             failure_class TEXT,
             last_used_at INTEGER NOT NULL
         )",
        "CREATE INDEX preview_cache_retention_idx
             ON preview_cache(last_used_at DESC, url_key DESC)",
    ],
}, Migration {
    version: 5,
    statements: &[
        "CREATE TABLE drafts (
             server_key TEXT NOT NULL,
             owner_user_id TEXT NOT NULL,
             channel_id TEXT NOT NULL,
             text TEXT NOT NULL,
             mentions TEXT NOT NULL DEFAULT '[]',
             reply_to TEXT,
             notify_reply INTEGER NOT NULL DEFAULT 1,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (server_key, owner_user_id, channel_id)
         )",
        "CREATE INDEX drafts_owner_updated_idx
             ON drafts(server_key, owner_user_id, updated_at DESC)",
    ],
}, Migration {
    version: 6,
    statements: &[
        "ALTER TABLE channels ADD COLUMN parent_id TEXT",
        "INSERT OR IGNORE INTO channel_cache_state (server_key, channel_id, cached_at)
             SELECT server_key, channel_id, MAX(created_at)
             FROM messages
             GROUP BY server_key, channel_id",
        "DROP INDEX messages_channel_idx",
        "CREATE INDEX messages_timeline_idx
             ON messages(server_key, channel_id, pinned, created_at DESC, message_id DESC)",
    ],
}, Migration {
    version: 7,
    statements: &["ALTER TABLE preview_cache ADD COLUMN post_meta TEXT"],
}, Migration {
    // O resolvedor passou a achar vídeo direto, autor e reações onde antes
    // gravava a página como embed: o cache antigo só atrapalharia.
    version: 8,
    statements: &["DELETE FROM preview_cache"],
}, Migration {
    version: 9,
    statements: &[
        "CREATE TABLE channel_read_state (
             server_key TEXT NOT NULL,
             owner_user_id TEXT NOT NULL,
             channel_id TEXT NOT NULL,
             read_at INTEGER,
             read_message_id TEXT,
             seen_out_of_order TEXT NOT NULL DEFAULT '[]',
             unread_mentions TEXT NOT NULL DEFAULT '[]',
             jump_back TEXT,
             jump_forward TEXT,
             updated_at INTEGER NOT NULL,
             PRIMARY KEY (server_key, owner_user_id, channel_id)
         )",
        "CREATE INDEX channel_read_state_owner_idx
             ON channel_read_state(server_key, owner_user_id, updated_at DESC)",
    ],
}, Migration {
    // O modelo de leitura passou a ser fronteira + marca d'água: o que o
    // usuário viu de mais adiante. Fica derivado daí se há lacuna não lida,
    // então `seen_out_of_order`, `jump_back` e `jump_forward` deixam de ser
    // lidos/escritos (as colunas ficam para não reescrever a tabela).
    version: 10,
    statements: &[
        "ALTER TABLE channel_read_state ADD COLUMN seen_to_at INTEGER",
        "ALTER TABLE channel_read_state ADD COLUMN seen_to_message_id TEXT",
    ],
}, Migration {
    version: 11,
    statements: &["ALTER TABLE messages ADD COLUMN embeds TEXT NOT NULL DEFAULT '[]'"],
}];

/// Lê a versão atual do esquema (0 quando ainda não há nada).
async fn read_version(conn: &Connection) -> Result<i64, turso::Error> {
    let mut rows = conn
        .query("SELECT value FROM meta WHERE key = 'schema_version'", ())
        .await?;
    match rows.next().await? {
        Some(row) => {
            let raw: String = row.get(0)?;
            Ok(raw.parse().unwrap_or(0))
        }
        None => Ok(0),
    }
}

/// Garante que `meta` exista antes de qualquer leitura de versão.
async fn ensure_meta(conn: &Connection) -> Result<(), turso::Error> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        (),
    )
    .await?;
    Ok(())
}

/// Aplica migrações até `target`. O helper também permite aos testes
/// construir um banco antigo real usando exatamente as migrações de produção.
async fn apply_migrations_through(
    conn: &mut Connection,
    target: i64,
) -> Result<i64, turso::Error> {
    ensure_meta(conn).await?;
    let mut current = read_version(conn).await?;
    if current > target {
        return Err(turso::Error::Misuse(format!(
            "cache schema version {current} is newer than supported {target}"
        )));
    }

    for migration in MIGRATIONS {
        if migration.version <= current || migration.version > target {
            continue;
        }
        let tx = conn.transaction().await?;
        for statement in migration.statements {
            tx.execute(*statement, ()).await?;
        }
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [migration.version.to_string()],
        )
        .await?;
        tx.commit().await?;
        current = migration.version;
    }

    Ok(current)
}

/// Aplica as migrações pendentes. Idempotente: reabrir na mesma versão não
/// executa nada.
pub async fn apply_migrations(conn: &mut Connection) -> Result<i64, turso::Error> {
    apply_migrations_through(conn, SCHEMA_VERSION).await
}

#[cfg(test)]
pub(super) async fn apply_migrations_for_test(
    conn: &mut Connection,
    target: i64,
) -> Result<i64, turso::Error> {
    apply_migrations_through(conn, target).await
}

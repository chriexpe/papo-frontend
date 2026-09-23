//! Migrações versionadas do banco de cache.
//!
//! Cada migração é uma lista de instruções aplicadas numa transação só. A
//! versão fica em `meta.schema_version`; se qualquer passo falhar, nada
//! avança e o cache é tratado como indisponível.

use turso::Connection;

/// Versão que o código espera encontrar depois de migrar.
pub const SCHEMA_VERSION: i64 = 1;

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

/// Aplica as migrações pendentes. Idempotente: reabrir na mesma versão não
/// executa nada.
pub async fn apply_migrations(conn: &mut Connection) -> Result<i64, turso::Error> {
    ensure_meta(conn).await?;
    let mut current = read_version(conn).await?;
    if current > SCHEMA_VERSION {
        return Err(turso::Error::Misuse(format!(
            "cache schema version {current} is newer than supported {SCHEMA_VERSION}"
        )));
    }

    for migration in MIGRATIONS {
        if migration.version <= current {
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

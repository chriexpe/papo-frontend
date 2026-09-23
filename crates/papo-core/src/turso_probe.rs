//! PR7 Turso feasibility probe.
//!
//! This module is deliberately not the production ClientDb. It exercises the
//! boring local-database subset Papo intends to depend on and keeps all probe
//! state in a dedicated database path supplied by the caller.

use std::path::Path;

use turso::{Builder, Connection, IntoParams, Value};

pub type ProbeError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type ProbeResult<T> = Result<T, ProbeError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartProbe {
    pub previous_generation: i64,
    pub committed_generation: i64,
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

async fn scalar_i64<P: IntoParams>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> ProbeResult<i64> {
    let mut rows = conn.query(sql, params).await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| "Turso probe query returned no rows".to_owned())?;
    Ok(row.get::<i64>(0)?)
}

/// Open a persistent probe DB, observe the previous committed generation,
/// commit the next one, then physically drop/reopen the database and verify it.
///
/// Running this once at each Android process start gives the real-device
/// force-stop/relaunch test a stable sentinel without mixing probe data with
/// Papo credentials or the future production cache.
pub async fn run_restart_probe(path: &Path) -> ProbeResult<RestartProbe> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path = path_text(path);

    let db = Builder::new_local(&path).build().await?;
    let mut conn = db.connect()?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS papo_turso_restart_probe (
             singleton INTEGER PRIMARY KEY,
             generation INTEGER NOT NULL
         )",
        (),
    )
    .await?;

    let previous_generation = {
        let mut rows = conn
            .query(
                "SELECT generation FROM papo_turso_restart_probe WHERE singleton = 1",
                (),
            )
            .await?;
        match rows.next().await? {
            Some(row) => row.get::<i64>(0)?,
            None => 0,
        }
    };
    let committed_generation = previous_generation.saturating_add(1);

    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO papo_turso_restart_probe(singleton, generation)
         VALUES (1, ?1)
         ON CONFLICT(singleton) DO UPDATE SET generation = excluded.generation",
        [committed_generation],
    )
    .await?;
    tx.commit().await?;

    drop(conn);
    drop(db);

    let reopened = Builder::new_local(&path).build().await?;
    let reopened_conn = reopened.connect()?;
    let persisted = scalar_i64(
        &reopened_conn,
        "SELECT generation FROM papo_turso_restart_probe WHERE singleton = 1",
        (),
    )
    .await?;
    if persisted != committed_generation {
        return Err(format!(
            "Turso reopen mismatch: committed {committed_generation}, read {persisted}"
        )
        .into());
    }

    Ok(RestartProbe {
        previous_generation,
        committed_generation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDb {
        dir: PathBuf,
        path: PathBuf,
    }

    impl TempDb {
        fn new(label: &str) -> Self {
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "papo-turso-{label}-{}-{serial}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("temporary Turso directory must be created");
            let path = dir.join("probe.db");
            Self { dir, path }
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn open(path: &Path) -> ProbeResult<(turso::Database, Connection)> {
        let text = path_text(path);
        let db = Builder::new_local(&text).build().await?;
        let conn = db.connect()?;
        Ok((db, conn))
    }

    #[tokio::test]
    async fn persistent_crud_binding_transactions_and_reopen() -> ProbeResult<()> {
        let temp = TempDb::new("crud");
        let (db, mut conn) = open(&temp.path).await?;

        conn.execute(
            "CREATE TABLE probe (
                 id TEXT PRIMARY KEY,
                 value TEXT NOT NULL,
                 counter INTEGER NOT NULL,
                 blob_value BLOB,
                 nullable_value TEXT
             )",
            (),
        )
        .await?;
        conn.execute("CREATE INDEX probe_counter_idx ON probe(counter)", ())
            .await?;

        conn.execute(
            "INSERT INTO probe(id, value, counter, blob_value, nullable_value)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            vec![
                Value::Text("alpha".to_owned()),
                Value::Text("A".to_owned()),
                Value::Integer(1),
                Value::Blob(vec![0, 1, 2, 255]),
                Value::Null,
            ],
        )
        .await?;

        let mut rows = conn
            .query(
                "SELECT value, counter, blob_value, nullable_value
                 FROM probe WHERE id = ?1",
                ["alpha"],
            )
            .await?;
        let row = rows.next().await?.ok_or("inserted probe row is missing")?;
        assert_eq!(row.get::<String>(0)?, "A");
        assert_eq!(row.get::<i64>(1)?, 1);
        assert_eq!(row.get_value(2)?, Value::Blob(vec![0, 1, 2, 255]));
        assert_eq!(row.get_value(3)?, Value::Null);
        drop(rows);

        conn.execute(
            "UPDATE probe SET value = ?1, counter = ?2 WHERE id = ?3",
            vec![
                Value::Text("B".to_owned()),
                Value::Integer(2),
                Value::Text("alpha".to_owned()),
            ],
        )
        .await?;

        {
            let tx = conn.transaction().await?;
            tx.execute(
                "INSERT INTO probe(id, value, counter) VALUES (?1, ?2, ?3)",
                ("commit-a", "committed", 10_i64),
            )
            .await?;
            tx.execute(
                "INSERT INTO probe(id, value, counter) VALUES (?1, ?2, ?3)",
                ("commit-b", "committed", 11_i64),
            )
            .await?;
            tx.commit().await?;
        }
        assert_eq!(
            scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM probe WHERE id IN ('commit-a', 'commit-b')",
                (),
            )
            .await?,
            2
        );

        {
            let tx = conn.transaction().await?;
            tx.execute(
                "UPDATE probe SET value = 'must-rollback' WHERE id = 'alpha'",
                (),
            )
            .await?;
            tx.rollback().await?;
        }
        let mut rows = conn
            .query("SELECT value FROM probe WHERE id = 'alpha'", ())
            .await?;
        assert_eq!(
            rows.next()
                .await?
                .ok_or("alpha disappeared after rollback")?
                .get::<String>(0)?,
            "B"
        );
        drop(rows);

        conn.execute(
            "INSERT INTO probe(id, value, counter) VALUES ('alpha', 'upserted', 3)
             ON CONFLICT(id) DO UPDATE SET
                 value = excluded.value,
                 counter = excluded.counter",
            (),
        )
        .await?;

        conn.execute("DELETE FROM probe WHERE id = ?1", ["commit-b"])
            .await?;
        assert_eq!(
            scalar_i64(
                &conn,
                "SELECT COUNT(*) FROM probe WHERE id = 'commit-b'",
                (),
            )
            .await?,
            0
        );

        drop(conn);
        drop(db);

        let (_reopened_db, reopened) = open(&temp.path).await?;
        let mut rows = reopened
            .query(
                "SELECT value, counter FROM probe WHERE id = ?1",
                ["alpha"],
            )
            .await?;
        let row = rows.next().await?.ok_or("reopened alpha row is missing")?;
        assert_eq!(row.get::<String>(0)?, "upserted");
        assert_eq!(row.get::<i64>(1)?, 3);
        assert!(temp.path.exists(), "Turso did not create the database file");

        Ok(())
    }

    #[tokio::test]
    async fn multiple_connections_preserve_transaction_visibility() -> ProbeResult<()> {
        let temp = TempDb::new("connections");
        let (_db, mut writer) = open(&temp.path).await?;
        writer
            .execute(
                "CREATE TABLE concurrency_probe (
                     id INTEGER PRIMARY KEY,
                     value INTEGER NOT NULL
                 )",
                (),
            )
            .await?;
        writer
            .execute("INSERT INTO concurrency_probe VALUES (1, 1)", ())
            .await?;

        let path = path_text(&temp.path);
        let shared_db = Builder::new_local(&path).build().await?;
        let reader = shared_db.connect()?;
        let second_writer = shared_db.connect()?;

        {
            let tx = writer.transaction().await?;
            tx.execute(
                "UPDATE concurrency_probe SET value = 2 WHERE id = 1",
                (),
            )
            .await?;

            // A distinct connection must not observe the uncommitted write.
            assert_eq!(
                scalar_i64(
                    &reader,
                    "SELECT value FROM concurrency_probe WHERE id = 1",
                    (),
                )
                .await?,
                1
            );
            tx.commit().await?;
        }
        assert_eq!(
            scalar_i64(
                &reader,
                "SELECT value FROM concurrency_probe WHERE id = 1",
                (),
            )
            .await?,
            2
        );

        // Ordinary writers can take turns without any experimental MVCC mode.
        writer
            .execute("INSERT INTO concurrency_probe VALUES (2, 20)", ())
            .await?;
        second_writer
            .execute("INSERT INTO concurrency_probe VALUES (3, 30)", ())
            .await?;

        // Independent async readers may execute concurrently on the same DB.
        let read_a = scalar_i64(
            &reader,
            "SELECT COUNT(*) FROM concurrency_probe",
            (),
        );
        let read_b = scalar_i64(
            &second_writer,
            "SELECT SUM(value) FROM concurrency_probe",
            (),
        );
        let (count, sum) = tokio::join!(read_a, read_b);
        assert_eq!(count?, 3);
        assert_eq!(sum?, 52);

        Ok(())
    }

    #[tokio::test]
    async fn restart_probe_advances_only_committed_persistent_state() -> ProbeResult<()> {
        let temp = TempDb::new("restart");

        let first = run_restart_probe(&temp.path).await?;
        assert_eq!(
            first,
            RestartProbe {
                previous_generation: 0,
                committed_generation: 1,
            }
        );

        let second = run_restart_probe(&temp.path).await?;
        assert_eq!(
            second,
            RestartProbe {
                previous_generation: 1,
                committed_generation: 2,
            }
        );

        Ok(())
    }
}

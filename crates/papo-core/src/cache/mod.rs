//! Cache persistente e descartável do cliente.
//!
//! Um `ClientDb` por processo, com um único worker assíncrono dono do banco.
//! A Store continua sendo a projeção quente; o banco só sobrevive reinícios.
//! Nada aqui é autoridade — o servidor manda — e nada aqui guarda segredos.

mod schema;
mod store;
mod types;

#[cfg(test)]
mod tests;

pub use store::TursoCache;
pub use types::{
    now_millis, CachedAttachment, CachedChannel, CachedMember, CachedMessage, CachedReaction,
    CachedServer, CachedServerSnapshot, CacheOp, MESSAGE_RETENTION, PINNED_RETENTION,
};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Fila limitada: sob pressão extrema o cache descarta em vez de crescer sem
/// limite. A próxima reconciliação autoritativa recompõe o que faltar.
const QUEUE_CAPACITY: usize = 4096;
/// Quantas mensagens o worker drena de uma vez antes de gravar.
const BATCH_LIMIT: usize = 256;
/// Espera máxima por open/load síncronos no arranque.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Contadores observáveis do subsistema de cache. Sem conteúdo privado.
#[derive(Debug, Default)]
pub struct CacheStats {
    dropped: AtomicU64,
    written: AtomicU64,
    write_failures: AtomicU64,
    restores: AtomicU64,
    last_error: Mutex<Option<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheStatsSnapshot {
    pub dropped: u64,
    pub written: u64,
    pub write_failures: u64,
    pub restores: u64,
    pub last_error: Option<String>,
}

impl CacheStats {
    fn snapshot(&self) -> CacheStatsSnapshot {
        CacheStatsSnapshot {
            dropped: self.dropped.load(Ordering::Relaxed),
            written: self.written.load(Ordering::Relaxed),
            write_failures: self.write_failures.load(Ordering::Relaxed),
            restores: self.restores.load(Ordering::Relaxed),
            last_error: self
                .last_error
                .lock()
                .ok()
                .and_then(|error| error.clone()),
        }
    }
}

enum WorkerMsg {
    Write {
        server_key: String,
        ops: Vec<CacheOp>,
    },
    Load {
        server_key: String,
        reply: std::sync::mpsc::Sender<Result<CachedServerSnapshot, String>>,
    },
    Flush(std::sync::mpsc::Sender<()>),
}

/// Dono do banco de cache. Clonável por fora apenas por `Arc` no frontend;
/// aqui a posse real é do worker, alcançado por canal.
pub struct ClientDb {
    sender: Option<SyncSender<WorkerMsg>>,
    stats: Arc<CacheStats>,
    path: Option<PathBuf>,
}

impl ClientDb {
    /// Abre o banco (se houver caminho) e espera o worker ficar pronto.
    /// Qualquer falha vira um cache desabilitado — nunca impede o Papo.
    pub fn open(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            log::warn!("cache: sem pasta de dados; seguindo sem cache");
            return Self {
                sender: None,
                stats: Arc::new(CacheStats::default()),
                path: None,
            };
        };

        let stats = Arc::new(CacheStats::default());
        let (sender, receiver) = std::sync::mpsc::sync_channel(QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker_stats = Arc::clone(&stats);
        let worker_path = path.clone();

        let spawned = std::thread::Builder::new()
            .name("papo-cache".to_owned())
            .spawn(move || worker(receiver, ready_tx, worker_path, worker_stats));

        if let Err(error) = spawned {
            log::warn!("cache: worker não abriu: {error}");
            return Self {
                sender: None,
                stats,
                path: Some(path),
            };
        }

        match ready_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(())) => {
                log::info!("cache: habilitado em {}", path.display());
                Self {
                    sender: Some(sender),
                    stats,
                    path: Some(path),
                }
            }
            Ok(Err(error)) => {
                log::warn!("cache: indisponível ({error}); seguindo sem cache");
                Self {
                    sender: None,
                    stats,
                    path: Some(path),
                }
            }
            Err(_) => {
                log::warn!("cache: worker não respondeu a tempo; seguindo sem cache");
                Self {
                    sender: None,
                    stats,
                    path: Some(path),
                }
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.sender.is_some()
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }

    /// Enfileira operações de um servidor. Nunca bloqueia: se a fila enche, o
    /// lote é descartado e contabilizado.
    pub fn submit(&self, server_key: &str, ops: Vec<CacheOp>) {
        if ops.is_empty() {
            return;
        }
        let Some(sender) = &self.sender else {
            return;
        };
        match sender.try_send(WorkerMsg::Write {
            server_key: server_key.to_owned(),
            ops,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                log::warn!("cache: fila cheia; lote de {server_key} descartado");
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stats.write_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn clear_server(&self, server_key: &str) {
        self.submit(server_key, vec![CacheOp::ClearServer]);
    }

    /// Espera tudo que já foi enfileirado ser aplicado. Usado por testes.
    pub fn flush(&self) {
        let Some(sender) = &self.sender else {
            return;
        };
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if sender.send(WorkerMsg::Flush(reply_tx)).is_err() {
            return;
        }
        let _ = reply_rx.recv_timeout(LOAD_TIMEOUT);
    }

    /// Leitura síncrona no arranque. Não é caminho de frame.
    pub fn load_snapshot(&self, server_key: &str) -> Option<CachedServerSnapshot> {
        let sender = self.sender.as_ref()?;
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        sender
            .send(WorkerMsg::Load {
                server_key: server_key.to_owned(),
                reply: reply_tx,
            })
            .ok()?;
        match reply_rx.recv_timeout(LOAD_TIMEOUT) {
            Ok(Ok(snapshot)) => {
                self.stats.restores.fetch_add(1, Ordering::Relaxed);
                log::debug!(
                    "cache: restored server={server_key} channels={} members={} messages={} cached_channels={}",
                    snapshot.channels.len(),
                    snapshot.members.len(),
                    snapshot.messages.len(),
                    snapshot.cached_channels.len()
                );
                Some(snapshot)
            }
            Ok(Err(error)) => {
                self.stats.write_failures.fetch_add(1, Ordering::Relaxed);
                self.set_last_error(error.clone());
                log::warn!("cache: restore falhou para {server_key}: {error}");
                None
            }
            Err(_) => None,
        }
    }

    fn set_last_error(&self, error: String) {
        if let Ok(mut slot) = self.stats.last_error.lock() {
            *slot = Some(error);
        }
    }
}

fn worker(
    receiver: Receiver<WorkerMsg>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
    path: PathBuf,
    stats: Arc<CacheStats>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };

    runtime.block_on(async move {
        let path = path.to_string_lossy().into_owned();
        let mut cache = match TursoCache::open(&path).await {
            Ok(cache) => {
                let _ = ready.send(Ok(()));
                cache
            }
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };

        loop {
            let first = match receiver.recv() {
                Ok(message) => message,
                Err(_) => break,
            };
            let mut batch = vec![first];
            while batch.len() < BATCH_LIMIT {
                match receiver.try_recv() {
                    Ok(message) => batch.push(message),
                    Err(_) => break,
                }
            }

            for message in batch {
                match message {
                    WorkerMsg::Write { server_key, ops } => {
                        let count = ops.len() as u64;
                        match cache.apply_batch(&server_key, &ops).await {
                            Ok(()) => {
                                stats.written.fetch_add(count, Ordering::Relaxed);
                            }
                            Err(error) => {
                                stats.write_failures.fetch_add(1, Ordering::Relaxed);
                                if let Ok(mut slot) = stats.last_error.lock() {
                                    *slot = Some(error.to_string());
                                }
                                log::warn!("cache: escrita falhou em {server_key}: {error}");
                            }
                        }
                    }
                    WorkerMsg::Load { server_key, reply } => {
                        let result = cache
                            .load_snapshot(&server_key)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                    }
                    WorkerMsg::Flush(reply) => {
                        let _ = reply.send(());
                    }
                }
            }
        }
    });
}

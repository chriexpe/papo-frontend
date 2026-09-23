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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

/// Fila limitada de dados: sob pressão extrema o cache descarta em vez de
/// crescer sem limite. A próxima reconciliação autoritativa recompõe o que
/// faltar.
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
    /// Só em testes: prende o worker até o teste liberar, para saturar a fila
    /// de dados de forma determinística.
    #[cfg(test)]
    Pause {
        entered: std::sync::mpsc::Sender<()>,
        resume: std::sync::mpsc::Receiver<()>,
    },
}

/// Mensagem com ordem global. As duas filas (dados e controle) são fundidas
/// por `seq` antes de aplicar, então um clear nunca "passa na frente" de um
/// dado enfileirado antes dele.
struct Queued {
    seq: u64,
    msg: WorkerMsg,
}

/// Dono do banco de cache. Clonável por fora apenas por `Arc` no frontend;
/// aqui a posse real é do worker, alcançado por canal.
///
/// Há duas filas: uma limitada para dados reconstruíveis (best-effort) e uma
/// ilimitada para posse/controle (`ClearServer`, `SetOwner`), que nunca pode
/// ser descartada. Nenhuma das duas bloqueia quem chama.
pub struct ClientDb {
    data: Option<mpsc::Sender<Queued>>,
    control: Option<mpsc::UnboundedSender<Queued>>,
    next_seq: Arc<AtomicU64>,
    stats: Arc<CacheStats>,
    path: Option<PathBuf>,
}

impl ClientDb {
    /// Abre o banco (se houver caminho) e espera o worker ficar pronto.
    /// Qualquer falha vira um cache desabilitado — nunca impede o Papo.
    pub fn open(path: Option<PathBuf>) -> Self {
        Self::open_with_capacity(path, QUEUE_CAPACITY)
    }

    fn open_with_capacity(path: Option<PathBuf>, capacity: usize) -> Self {
        let Some(path) = path else {
            log::warn!("cache: sem pasta de dados; seguindo sem cache");
            return Self::disabled(None);
        };

        let stats = Arc::new(CacheStats::default());
        let (data_tx, data_rx) = mpsc::channel(capacity.max(1));
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker_stats = Arc::clone(&stats);
        let worker_path = path.clone();

        let spawned = std::thread::Builder::new()
            .name("papo-cache".to_owned())
            .spawn(move || worker(data_rx, control_rx, ready_tx, worker_path, worker_stats));

        if let Err(error) = spawned {
            log::warn!("cache: worker não abriu: {error}");
            return Self {
                data: None,
                control: None,
                next_seq: Arc::new(AtomicU64::new(0)),
                stats,
                path: Some(path),
            };
        }

        let ready = ready_rx.recv_timeout(OPEN_TIMEOUT);
        match ready {
            Ok(Ok(())) => {
                log::info!("cache: habilitado em {}", path.display());
                Self {
                    data: Some(data_tx),
                    control: Some(control_tx),
                    next_seq: Arc::new(AtomicU64::new(0)),
                    stats,
                    path: Some(path),
                }
            }
            Ok(Err(error)) => {
                log::warn!("cache: indisponível ({error}); seguindo sem cache");
                Self::disabled(Some(path))
            }
            Err(_) => {
                log::warn!("cache: worker não respondeu a tempo; seguindo sem cache");
                Self::disabled(Some(path))
            }
        }
    }

    fn disabled(path: Option<PathBuf>) -> Self {
        Self {
            data: None,
            control: None,
            next_seq: Arc::new(AtomicU64::new(0)),
            stats: Arc::new(CacheStats::default()),
            path,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.data.is_some()
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }

    fn next_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Operações de posse/controle vão pela fila ilimitada: nunca bloqueiam e
    /// nunca são descartadas.
    fn send_control(&self, msg: WorkerMsg) -> bool {
        let Some(control) = &self.control else {
            return false;
        };
        control
            .send(Queued {
                seq: self.next_seq(),
                msg,
            })
            .is_ok()
    }

    /// Dados vão pela fila limitada: sob pressão, o lote é descartado.
    fn send_data(&self, msg: WorkerMsg) {
        let Some(data) = &self.data else {
            return;
        };
        match data.try_send(Queued {
            seq: self.next_seq(),
            msg,
        }) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                log::warn!("cache: fila cheia; lote descartado");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.stats.write_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Enfileira operações de um servidor. Lotes que contêm controle usam a
    /// fila confiável; o resto é best-effort.
    pub fn submit(&self, server_key: &str, ops: Vec<CacheOp>) {
        if ops.is_empty() {
            return;
        }
        let msg = WorkerMsg::Write {
            server_key: server_key.to_owned(),
            ops,
        };
        match &msg {
            WorkerMsg::Write { ops, .. } if ops.iter().any(CacheOp::is_control) => {
                if !self.send_control(msg) {
                    self.stats.write_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ => self.send_data(msg),
        }
    }

    pub fn clear_server(&self, server_key: &str) {
        let sent = self.send_control(WorkerMsg::Write {
            server_key: server_key.to_owned(),
            ops: vec![CacheOp::ClearServer],
        });
        if !sent {
            self.stats.write_failures.fetch_add(1, Ordering::Relaxed);
            log::warn!("cache: não foi possível enfileirar o clear de {server_key}");
        }
    }

    /// Espera tudo que já foi enfileirado ser aplicado. Usado por testes.
    pub fn flush(&self) {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if !self.send_control(WorkerMsg::Flush(reply_tx)) {
            return;
        }
        let _ = reply_rx.recv_timeout(LOAD_TIMEOUT);
    }

    /// Leitura síncrona no arranque. Não é caminho de frame.
    pub fn load_snapshot(&self, server_key: &str) -> Option<CachedServerSnapshot> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if !self.send_control(WorkerMsg::Load {
            server_key: server_key.to_owned(),
            reply: reply_tx,
        }) {
            return None;
        }
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

    /// Só em testes: prende o worker e devolve o gatilho para soltá-lo.
    #[cfg(test)]
    fn pause_worker(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        assert!(
            self.send_control(WorkerMsg::Pause {
                entered: entered_tx,
                resume: resume_rx,
            }),
            "worker precisa aceitar o pause"
        );
        (entered_rx, resume_tx)
    }
}

fn worker(
    mut data: mpsc::Receiver<Queued>,
    mut control: mpsc::UnboundedReceiver<Queued>,
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
            // Controle tem prioridade para acordar o worker, mas a ordem real
            // vem do `seq` depois de fundir as duas filas.
            let first = tokio::select! {
                biased;
                Some(queued) = control.recv() => queued,
                Some(queued) = data.recv() => queued,
                else => break,
            };

            let mut batch = vec![first];
            while batch.len() < BATCH_LIMIT {
                let mut progressed = false;
                if let Ok(queued) = data.try_recv() {
                    batch.push(queued);
                    progressed = true;
                }
                if batch.len() >= BATCH_LIMIT {
                    break;
                }
                if let Ok(queued) = control.try_recv() {
                    batch.push(queued);
                    progressed = true;
                }
                if !progressed {
                    break;
                }
            }
            batch.sort_by_key(|queued| queued.seq);

            // Funde escritas consecutivas do mesmo servidor num só lote de
            // banco: a retenção de cada canal roda uma vez no fim do lote.
            let mut queue: std::collections::VecDeque<Queued> = batch.into();
            while let Some(queued) = queue.pop_front() {
                match queued.msg {
                    WorkerMsg::Write {
                        server_key,
                        mut ops,
                    } => {
                        while let Some(front) = queue.front() {
                            match &front.msg {
                                WorkerMsg::Write { server_key: next, .. }
                                    if *next == server_key => {}
                                _ => break,
                            }
                            let Some(Queued {
                                msg: WorkerMsg::Write { ops: mut more, .. },
                                ..
                            }) = queue.pop_front()
                            else {
                                break;
                            };
                            ops.append(&mut more);
                        }
                        apply_write(&mut cache, &stats, &server_key, &ops).await;
                    }
                    other => apply(&mut cache, &stats, other).await,
                }
            }
        }
    });
}

async fn apply_write(
    cache: &mut TursoCache,
    stats: &Arc<CacheStats>,
    server_key: &str,
    ops: &[CacheOp],
) {
    let count = ops.len() as u64;
    match cache.apply_batch(server_key, ops).await {
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

async fn apply(cache: &mut TursoCache, stats: &Arc<CacheStats>, message: WorkerMsg) {
    match message {
        WorkerMsg::Write { server_key, ops } => {
            apply_write(cache, stats, &server_key, &ops).await;
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
        #[cfg(test)]
        WorkerMsg::Pause { entered, resume } => {
            let _ = entered.send(());
            let _ = resume.recv();
        }
    }
}

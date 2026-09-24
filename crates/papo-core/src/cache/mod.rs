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
    new_local_id, now_millis, CachedAttachment, CachedChannel, CachedMember, CachedMessage,
    CachedOutgoing, CachedReaction, CachedServer, CachedServerSnapshot, CacheOp, ClaimResult,
    NotificationDecision, NotificationLedgerEntry, NotificationLedgerStats, OutgoingState,
    MESSAGE_RETENTION, NOTIFICATION_LEDGER_LIMIT, OUTGOING_LIMIT, PINNED_RETENTION,
};

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

/// Teto de dados pendentes: dados são best-effort e o cache descarta em vez
/// de crescer sem limite. A próxima reconciliação autoritativa recompõe o que
/// faltar. Controle (clear, dono, load, flush) não consome esta cota.
const MAX_PENDING_DATA: usize = 4096;
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
    EnqueueOutgoing {
        server_key: String,
        outgoing: CachedOutgoing,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    LoadOutgoing {
        server_key: String,
        owner_user_id: String,
        reply: std::sync::mpsc::Sender<Result<Vec<CachedOutgoing>, String>>,
    },
    TransitionOutgoing {
        server_key: String,
        owner_user_id: String,
        local_id: String,
        state: OutgoingState,
        last_error: Option<String>,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    ConfirmOutgoing {
        server_key: String,
        local_id: String,
        message: CachedMessage,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    RemoveOutgoing {
        server_key: String,
        owner_user_id: String,
        local_id: String,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    ClaimNotification {
        server_key: String,
        entry: NotificationLedgerEntry,
        reply: std::sync::mpsc::Sender<
            Result<(ClaimResult, NotificationLedgerStats), String>,
        >,
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

/// Uma fila FIFO só para tudo, com uma cota separada de dados pendentes.
///
/// Controlar a cota dentro do mesmo lock que enfileira dá um ponto de
/// linearização único: se `A` foi aceito antes de `B`, `A` entra na fila
/// antes de `B`, sem depender de qual classe cada um é. O worker nunca
/// reordena — só funde escritas de dados adjacentes do mesmo servidor.
pub struct ClientDb {
    queue: Option<mpsc::UnboundedSender<WorkerMsg>>,
    pending_data: Arc<Mutex<usize>>,
    max_pending_data: usize,
    stats: Arc<CacheStats>,
    path: Option<PathBuf>,
}

impl ClientDb {
    /// Abre o banco (se houver caminho) e espera o worker ficar pronto.
    /// Qualquer falha vira um cache desabilitado — nunca impede o Papo.
    pub fn open(path: Option<PathBuf>) -> Self {
        Self::open_with_limit(path, MAX_PENDING_DATA)
    }

    fn open_with_limit(path: Option<PathBuf>, max_pending_data: usize) -> Self {
        let Some(path) = path else {
            log::warn!("cache: sem pasta de dados; seguindo sem cache");
            return Self::disabled(None, max_pending_data);
        };

        let stats = Arc::new(CacheStats::default());
        let pending_data = Arc::new(Mutex::new(0usize));
        let (queue_tx, queue_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker_stats = Arc::clone(&stats);
        let worker_pending = Arc::clone(&pending_data);
        let worker_path = path.clone();

        let spawned = std::thread::Builder::new()
            .name("papo-cache".to_owned())
            .spawn(move || worker(queue_rx, worker_pending, ready_tx, worker_path, worker_stats));

        if let Err(error) = spawned {
            log::warn!("cache: worker não abriu: {error}");
            return Self {
                queue: None,
                pending_data,
                max_pending_data,
                stats,
                path: Some(path),
            };
        }

        match ready_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(())) => {
                log::info!("cache: habilitado em {}", path.display());
                Self {
                    queue: Some(queue_tx),
                    pending_data,
                    max_pending_data,
                    stats,
                    path: Some(path),
                }
            }
            Ok(Err(error)) => {
                log::warn!("cache: indisponível ({error}); seguindo sem cache");
                Self::disabled(Some(path), max_pending_data)
            }
            Err(_) => {
                log::warn!("cache: worker não respondeu a tempo; seguindo sem cache");
                Self::disabled(Some(path), max_pending_data)
            }
        }
    }

    fn disabled(path: Option<PathBuf>, max_pending_data: usize) -> Self {
        Self {
            queue: None,
            pending_data: Arc::new(Mutex::new(0)),
            max_pending_data,
            stats: Arc::new(CacheStats::default()),
            path,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.queue.is_some()
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }

    /// Enfileira uma mensagem com um ponto de linearização único. Dados
    /// consomem a cota limitada; controle nunca. Nunca bloqueia no banco.
    fn enqueue(&self, msg: WorkerMsg, data: bool) -> bool {
        let Some(queue) = &self.queue else {
            return false;
        };
        let Ok(mut pending) = self.pending_data.lock() else {
            return false;
        };
        if data {
            if *pending >= self.max_pending_data {
                drop(pending);
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                log::warn!("cache: fila de dados cheia; lote descartado");
                return false;
            }
            *pending += 1;
        }
        match queue.send(msg) {
            Ok(()) => true,
            Err(_) => {
                if data {
                    *pending = pending.saturating_sub(1);
                }
                self.stats.write_failures.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Enfileira operações de um servidor. Lotes que contêm controle são
    /// confiáveis e não consomem a cota de dados; o resto é best-effort.
    pub fn submit(&self, server_key: &str, ops: Vec<CacheOp>) {
        if ops.is_empty() {
            return;
        }
        let data = !is_control_batch(&ops);
        let msg = WorkerMsg::Write {
            server_key: server_key.to_owned(),
            ops,
        };
        if !self.enqueue(msg, data) && !data {
            log::warn!("cache: não foi possível enfileirar o controle de {server_key}");
        }
    }

    pub fn clear_cached_data(&self, server_key: &str) {
        let msg = WorkerMsg::Write {
            server_key: server_key.to_owned(),
            ops: vec![CacheOp::ClearCachedData],
        };
        if !self.enqueue(msg, false) {
            log::warn!("cache: não foi possível limpar dados reconstruíveis de {server_key}");
        }
    }

    pub fn clear_server(&self, server_key: &str) {
        let msg = WorkerMsg::Write {
            server_key: server_key.to_owned(),
            ops: vec![CacheOp::ClearServer],
        };
        if !self.enqueue(msg, false) {
            log::warn!("cache: não foi possível enfileirar o clear de {server_key}");
        }
    }

    fn wait_result<T>(
        &self,
        msg: WorkerMsg,
        reply_rx: std::sync::mpsc::Receiver<Result<T, String>>,
    ) -> Result<T, String> {
        if !self.enqueue(msg, false) {
            return Err("ClientDb indisponível".to_owned());
        }
        reply_rx
            .recv_timeout(LOAD_TIMEOUT)
            .map_err(|_| "ClientDb não respondeu a tempo".to_owned())?
    }

    /// Escritas da fila de saída não podem transformar "timeout esperando o
    /// ack" em "a escrita falhou": o worker poderia confirmar a transação um
    /// instante depois. O chamador é o worker de rede, nunca a thread egui, e
    /// portanto espera o resultado definitivo antes de permitir um POST.
    fn wait_durable_result<T>(
        &self,
        msg: WorkerMsg,
        reply_rx: std::sync::mpsc::Receiver<Result<T, String>>,
    ) -> Result<T, String> {
        if !self.enqueue(msg, false) {
            return Err("ClientDb indisponível".to_owned());
        }
        reply_rx
            .recv()
            .map_err(|_| "worker do ClientDb encerrou antes do ack".to_owned())?
    }

    pub fn enqueue_outgoing(
        &self,
        server_key: &str,
        outgoing: CachedOutgoing,
    ) -> Result<(), String> {
        let (reply, recv) = std::sync::mpsc::channel();
        self.wait_durable_result(
            WorkerMsg::EnqueueOutgoing {
                server_key: server_key.to_owned(),
                outgoing,
                reply,
            },
            recv,
        )
    }

    pub fn load_outgoing(
        &self,
        server_key: &str,
        owner_user_id: &str,
    ) -> Result<Vec<CachedOutgoing>, String> {
        let (reply, recv) = std::sync::mpsc::channel();
        self.wait_result(
            WorkerMsg::LoadOutgoing {
                server_key: server_key.to_owned(),
                owner_user_id: owner_user_id.to_owned(),
                reply,
            },
            recv,
        )
    }

    pub fn transition_outgoing(
        &self,
        server_key: &str,
        owner_user_id: &str,
        local_id: &str,
        state: OutgoingState,
        last_error: Option<String>,
    ) -> Result<(), String> {
        let (reply, recv) = std::sync::mpsc::channel();
        self.wait_durable_result(
            WorkerMsg::TransitionOutgoing {
                server_key: server_key.to_owned(),
                owner_user_id: owner_user_id.to_owned(),
                local_id: local_id.to_owned(),
                state,
                last_error,
                reply,
            },
            recv,
        )
    }

    pub fn confirm_outgoing(
        &self,
        server_key: &str,
        local_id: &str,
        message: CachedMessage,
    ) -> Result<(), String> {
        let (reply, recv) = std::sync::mpsc::channel();
        self.wait_durable_result(
            WorkerMsg::ConfirmOutgoing {
                server_key: server_key.to_owned(),
                local_id: local_id.to_owned(),
                message,
                reply,
            },
            recv,
        )
    }

    pub fn remove_outgoing(
        &self,
        server_key: &str,
        owner_user_id: &str,
        local_id: &str,
    ) -> Result<(), String> {
        let (reply, recv) = std::sync::mpsc::channel();
        self.wait_durable_result(
            WorkerMsg::RemoveOutgoing {
                server_key: server_key.to_owned(),
                owner_user_id: owner_user_id.to_owned(),
                local_id: local_id.to_owned(),
                reply,
            },
            recv,
        )
    }

    /// Claim durável usado pelo NotificationCoordinator. Assim como a fila
    /// de saída, não aceita timeout ambíguo: ou o worker confirmou o commit ou
    /// o chamador não publica a notificação.
    pub fn claim_notification(
        &self,
        server_key: &str,
        entry: NotificationLedgerEntry,
    ) -> Result<(ClaimResult, NotificationLedgerStats), String> {
        let (reply, recv) = std::sync::mpsc::channel();
        self.wait_durable_result(
            WorkerMsg::ClaimNotification {
                server_key: server_key.to_owned(),
                entry,
                reply,
            },
            recv,
        )
    }

    /// Espera tudo que já foi enfileirado ser aplicado. É barreira: como só há
    /// uma fila, tudo que foi aceito antes entra antes e é aplicado antes.
    pub fn flush(&self) {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if !self.enqueue(WorkerMsg::Flush(reply_tx), false) {
            return;
        }
        let _ = reply_rx.recv_timeout(LOAD_TIMEOUT);
    }

    /// Leitura síncrona no arranque; também é barreira atrás do que já foi
    /// aceito. Não é caminho de frame.
    pub fn load_snapshot(&self, server_key: &str) -> Option<CachedServerSnapshot> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if !self.enqueue(
            WorkerMsg::Load {
                server_key: server_key.to_owned(),
                reply: reply_tx,
            },
            false,
        ) {
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
            self.enqueue(
                WorkerMsg::Pause {
                    entered: entered_tx,
                    resume: resume_rx,
                },
                false
            ),
            "worker precisa aceitar o pause"
        );
        (entered_rx, resume_tx)
    }
}

fn is_control_batch(ops: &[CacheOp]) -> bool {
    ops.iter().any(CacheOp::is_control)
}

fn release_data(pending: &Mutex<usize>, count: usize) {
    if let Ok(mut pending) = pending.lock() {
        *pending = pending.saturating_sub(count);
    }
}

fn worker(
    mut queue: mpsc::UnboundedReceiver<WorkerMsg>,
    pending_data: Arc<Mutex<usize>>,
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
            // Uma fila só: a ordem de chegada é a ordem de aplicação. O dreno
            // é limitado para não monopolizar, mas nunca reordena nem pula o
            // que ficou para o próximo ciclo.
            let Some(first) = queue.recv().await else {
                break;
            };
            let mut batch: VecDeque<WorkerMsg> = VecDeque::with_capacity(BATCH_LIMIT);
            batch.push_back(first);
            while batch.len() < BATCH_LIMIT {
                match queue.try_recv() {
                    Ok(message) => batch.push_back(message),
                    Err(_) => break,
                }
            }

            while let Some(message) = batch.pop_front() {
                match message {
                    // Funde escritas de dados adjacentes do mesmo servidor num
                    // só lote de banco. Controle nunca entra na fusão: ele
                    // muda a semântica de ordenação.
                    WorkerMsg::Write { server_key, ops } if !is_control_batch(&ops) => {
                        let mut merged = ops;
                        let mut merged_count = 1usize;
                        while let Some(WorkerMsg::Write {
                            server_key: next,
                            ops: more,
                        }) = batch.front()
                        {
                            if *next != server_key || is_control_batch(more) {
                                break;
                            }
                            let Some(WorkerMsg::Write { ops: mut more, .. }) = batch.pop_front()
                            else {
                                break;
                            };
                            merged.append(&mut more);
                            merged_count += 1;
                        }
                        apply_write(&mut cache, &stats, &server_key, &merged).await;
                        release_data(&pending_data, merged_count);
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
        WorkerMsg::EnqueueOutgoing {
            server_key,
            outgoing,
            reply,
        } => {
            let result = cache
                .enqueue_outgoing(&server_key, &outgoing)
                .await
                .map_err(|error| error.to_string());
            let _ = reply.send(result);
        }
        WorkerMsg::LoadOutgoing {
            server_key,
            owner_user_id,
            reply,
        } => {
            let result = cache
                .load_outgoing(&server_key, &owner_user_id)
                .await
                .map_err(|error| error.to_string());
            let _ = reply.send(result);
        }
        WorkerMsg::TransitionOutgoing {
            server_key,
            owner_user_id,
            local_id,
            state,
            last_error,
            reply,
        } => {
            let result = cache
                .transition_outgoing(
                    &server_key,
                    &owner_user_id,
                    &local_id,
                    state,
                    last_error.as_deref(),
                )
                .await
                .map_err(|error| error.to_string());
            let _ = reply.send(result);
        }
        WorkerMsg::ConfirmOutgoing {
            server_key,
            local_id,
            message,
            reply,
        } => {
            let result = cache
                .confirm_outgoing(&server_key, &local_id, &message)
                .await
                .map_err(|error| error.to_string());
            let _ = reply.send(result);
        }
        WorkerMsg::RemoveOutgoing {
            server_key,
            owner_user_id,
            local_id,
            reply,
        } => {
            let result = cache
                .remove_outgoing(&server_key, &owner_user_id, &local_id)
                .await
                .map_err(|error| error.to_string());
            let _ = reply.send(result);
        }
        WorkerMsg::ClaimNotification {
            server_key,
            entry,
            reply,
        } => {
            let result = cache
                .claim_notification(&server_key, &entry)
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

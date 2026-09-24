//! Política de notificações do cliente, compartilhada por todas as plataformas.
//!
//! Somente candidatos live entram aqui. Cache restore e reconciliação podem
//! reconstruir estado/unread, mas nunca devem replayar efeitos do sistema.
//! A deduplicação durável converge Message + Notification pelo message_id.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::api::models::{Message, Notification};
use crate::cache::{
    ClaimResult, ClientDb, NotificationDecision, NotificationLedgerEntry,
    NotificationLedgerStats, now_millis,
};

pub type NotificationSink = Arc<dyn Fn(NotificationEnvelope) + Send + Sync>;
pub type ForegroundProbe = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateSource {
    Live,
    CacheRestore,
    Reconcile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressionReason {
    NotificationsDisabled,
    OwnMessage,
    ForegroundVisible,
    PlatformUnavailable,
}

impl SuppressionReason {
    fn as_db(self) -> &'static str {
        match self {
            Self::NotificationsDisabled => "notifications_disabled",
            Self::OwnMessage => "own_message",
            Self::ForegroundVisible => "foreground_visible",
            Self::PlatformUnavailable => "platform_unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationOutcome {
    Ignored,
    Delivered,
    Suppressed(SuppressionReason),
    Duplicate,
    LedgerUnavailable,
}

#[derive(Clone, Debug, Default)]
pub struct NotificationContext {
    /// Chave estável e segura; também é o escopo dos logs/ledger.
    pub server_key: String,
    /// Identidade de navegação em memória. Nunca é persistida no ledger.
    pub navigation_server: String,
    pub server_label: String,
    pub owner_user_id: String,
    pub owner_name: String,
    pub selected_channel: String,
    pub visible_server: bool,
    pub notifications_enabled: bool,
    pub channels: HashMap<String, String>,
    pub members: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationEnvelope {
    pub server_key: String,
    pub navigation_server: String,
    pub channel_id: String,
    pub message_id: String,
    /// ID da Notification do backend, quando este candidato veio dela.
    /// Não é a identidade de dedupe e não é fabricado para Message.
    pub notification_id: Option<String>,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationDiagnostics {
    pub ledger_rows: u64,
    pub delivered: u64,
    pub suppressed: u64,
    pub duplicate_claims: u64,
    pub claim_failures: u64,
}

#[derive(Clone)]
struct Candidate {
    channel_id: String,
    message_id: String,
    notification_id: Option<String>,
    author_id: Option<String>,
    body: String,
}

type DiagnosticKey = (String, String);

/// Um coordenador por processo. Contextos são pequenos snapshots por servidor;
/// política e dedupe são únicos para Linux/Android e futuros produtores push.
pub struct NotificationCoordinator {
    cache: Arc<ClientDb>,
    contexts: RwLock<HashMap<String, NotificationContext>>,
    sink: Option<NotificationSink>,
    foreground_probe: Option<ForegroundProbe>,
    foreground: AtomicBool,
    diagnostics: Mutex<HashMap<DiagnosticKey, NotificationDiagnostics>>,
}

impl NotificationCoordinator {
    pub fn new(cache: Arc<ClientDb>, sink: Option<NotificationSink>) -> Self {
        Self {
            cache,
            contexts: RwLock::new(HashMap::new()),
            sink,
            foreground_probe: None,
            foreground: AtomicBool::new(false),
            diagnostics: Mutex::new(HashMap::new()),
        }
    }

    /// Android usa o lifecycle nativo diretamente: ele continua correto mesmo
    /// quando egui não está pintando quadros.
    pub fn with_foreground_probe(
        cache: Arc<ClientDb>,
        sink: Option<NotificationSink>,
        probe: ForegroundProbe,
    ) -> Self {
        let mut coordinator = Self::new(cache, sink);
        coordinator.foreground_probe = Some(probe);
        coordinator
    }

    /// Usado por desktops, onde o estado de foco/minimização pertence à janela.
    pub fn set_foreground(&self, foreground: bool) {
        self.foreground.store(foreground, Ordering::Release);
    }

    pub fn sync_context(&self, context: NotificationContext) {
        if context.server_key.is_empty() {
            return;
        }
        if let Ok(mut contexts) = self.contexts.write() {
            contexts.insert(context.server_key.clone(), context);
        }
    }

    pub fn remove_context(&self, server_key: &str) {
        if let Ok(mut contexts) = self.contexts.write() {
            contexts.remove(server_key);
        }
    }

    pub fn diagnostics(&self, server_key: &str) -> NotificationDiagnostics {
        let owner = self
            .context(server_key)
            .map(|context| context.owner_user_id)
            .unwrap_or_default();
        self.diagnostics
            .lock()
            .ok()
            .and_then(|diagnostics| {
                diagnostics
                    .get(&(server_key.to_owned(), owner))
                    .cloned()
            })
            .unwrap_or_default()
    }

    pub fn handle_message(
        &self,
        server_key: &str,
        message: &Message,
        source: CandidateSource,
    ) -> NotificationOutcome {
        if source != CandidateSource::Live {
            return NotificationOutcome::Ignored;
        }
        let Some(context) = self.context(server_key) else {
            return NotificationOutcome::Ignored;
        };
        if context.owner_user_id.is_empty() {
            return NotificationOutcome::Ignored;
        }

        let body = message.content.as_deref().unwrap_or("");
        if !mentions_user(
            body,
            &context.owner_user_id,
            &context.owner_name,
            &message.author_id,
        ) {
            // Importante: Message sem menção não consome o ledger. Um
            // Notification posterior ainda pode representar reply/all.
            return NotificationOutcome::Ignored;
        }

        self.process(
            context,
            Candidate {
                channel_id: message.channel_id.clone(),
                message_id: message.id.clone(),
                notification_id: None,
                author_id: Some(message.author_id.clone()),
                body: body.to_owned(),
            },
        )
    }

    pub fn handle_notification(
        &self,
        server_key: &str,
        notification: &Notification,
        source: CandidateSource,
    ) -> NotificationOutcome {
        if source != CandidateSource::Live || notification.read {
            return NotificationOutcome::Ignored;
        }
        let (Some(channel_id), Some(message_id)) = (
            notification.channel_id.as_deref(),
            notification.message_id.as_deref(),
        ) else {
            return NotificationOutcome::Ignored;
        };
        let Some(context) = self.context(server_key) else {
            return NotificationOutcome::Ignored;
        };
        if context.owner_user_id.is_empty() {
            return NotificationOutcome::Ignored;
        }

        self.process(
            context,
            Candidate {
                channel_id: channel_id.to_owned(),
                message_id: message_id.to_owned(),
                notification_id: Some(notification.id.clone()),
                author_id: notification.author_id.clone(),
                body: notification.message_content.clone().unwrap_or_default(),
            },
        )
    }

    fn context(&self, server_key: &str) -> Option<NotificationContext> {
        self.contexts
            .read()
            .ok()
            .and_then(|contexts| contexts.get(server_key).cloned())
    }

    fn is_foreground(&self) -> bool {
        self.foreground_probe
            .as_ref()
            .map(|probe| probe())
            .unwrap_or_else(|| self.foreground.load(Ordering::Acquire))
    }

    fn process(
        &self,
        context: NotificationContext,
        candidate: Candidate,
    ) -> NotificationOutcome {
        let suppression = if !context.notifications_enabled {
            Some(SuppressionReason::NotificationsDisabled)
        } else if candidate.author_id.as_deref() == Some(context.owner_user_id.as_str()) {
            Some(SuppressionReason::OwnMessage)
        } else if self.is_foreground()
            && context.visible_server
            && context.selected_channel == candidate.channel_id
        {
            Some(SuppressionReason::ForegroundVisible)
        } else if self.sink.is_none() {
            Some(SuppressionReason::PlatformUnavailable)
        } else {
            None
        };

        let decision = if suppression.is_some() {
            NotificationDecision::Suppressed
        } else {
            NotificationDecision::Delivered
        };
        let entry = NotificationLedgerEntry {
            owner_user_id: context.owner_user_id.clone(),
            message_id: candidate.message_id.clone(),
            channel_id: candidate.channel_id.clone(),
            notification_id: candidate.notification_id.clone(),
            decision,
            reason: suppression.map(|reason| reason.as_db().to_owned()),
            handled_at: now_millis(),
        };

        let claim = self.cache.claim_notification(&context.server_key, entry);
        let key = (
            context.server_key.clone(),
            context.owner_user_id.clone(),
        );

        let (claim, stats) = match claim {
            Ok(result) => result,
            Err(error) => {
                if let Ok(mut diagnostics) = self.diagnostics.lock() {
                    diagnostics.entry(key).or_default().claim_failures += 1;
                }
                log::warn!(
                    "notification {}: ledger claim falhou message={}: {error}",
                    context.server_key,
                    candidate.message_id
                );
                // Fail closed: sem claim durável, entregar criaria uma via de
                // duplicação descontrolada após restart/race.
                return NotificationOutcome::LedgerUnavailable;
            }
        };
        self.update_persistent_stats(&key, stats);

        if claim == ClaimResult::AlreadyHandled {
            if let Ok(mut diagnostics) = self.diagnostics.lock() {
                diagnostics.entry(key).or_default().duplicate_claims += 1;
            }
            log::debug!(
                "notification {}: duplicate message={}",
                context.server_key,
                candidate.message_id
            );
            return NotificationOutcome::Duplicate;
        }

        if let Some(reason) = suppression {
            log::debug!(
                "notification {}: suppressed {} message={}",
                context.server_key,
                reason.as_db(),
                candidate.message_id
            );
            return NotificationOutcome::Suppressed(reason);
        }

        let envelope_title =
            title(&context, candidate.author_id.as_deref(), &candidate.channel_id);
        let envelope_body = display_mentions(&context.members, &candidate.body);
        let envelope = NotificationEnvelope {
            server_key: context.server_key.clone(),
            navigation_server: context.navigation_server,
            channel_id: candidate.channel_id,
            message_id: candidate.message_id.clone(),
            notification_id: candidate.notification_id,
            title: envelope_title,
            body: envelope_body,
        };

        // O claim já foi commitado. Um crash exatamente daqui até o show()
        // pode perder uma notificação; esta janela é deliberada para manter
        // at-most-once em vez de duplicar depois de reiniciar.
        if let Some(sink) = &self.sink {
            sink(envelope);
        }
        log::debug!(
            "notification {}: delivered message={}",
            context.server_key,
            candidate.message_id
        );
        NotificationOutcome::Delivered
    }

    fn update_persistent_stats(&self, key: &DiagnosticKey, stats: NotificationLedgerStats) {
        if let Ok(mut diagnostics) = self.diagnostics.lock() {
            let diagnostic = diagnostics.entry(key.clone()).or_default();
            diagnostic.ledger_rows = stats.rows.max(0) as u64;
            diagnostic.delivered = stats.delivered.max(0) as u64;
            diagnostic.suppressed = stats.suppressed.max(0) as u64;
        }
    }
}

/// A semântica histórica de menção do Papo, compartilhada pela Store e pelo
/// coordenador para não nascerem duas implementações divergentes.
pub fn mentions_user(text: &str, owner_user_id: &str, owner_name: &str, author_id: &str) -> bool {
    if author_id == owner_user_id {
        return false;
    }
    let lower = text.to_lowercase();
    (!owner_user_id.is_empty()
        && lower.contains(&format!("<@{}>", owner_user_id.to_lowercase())))
        || (!owner_name.is_empty()
            && lower.contains(&format!("@{}", owner_name.to_lowercase())))
        || lower.contains("@everyone")
        || lower.contains("@todos")
}

fn display_mentions(members: &HashMap<String, String>, text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i + 3 < chars.len() && chars[i + 1] == '@' {
            let mut end = i + 2;
            while end < chars.len() && chars[end] != '>' {
                end += 1;
            }
            if end < chars.len() {
                let id: String = chars[i + 2..end].iter().collect();
                if let Some(name) = members.get(&id) {
                    out.push('@');
                    out.push_str(name);
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn title(
    context: &NotificationContext,
    author_id: Option<&str>,
    channel_id: &str,
) -> String {
    let author = author_id
        .and_then(|id| context.members.get(id))
        .cloned()
        .or_else(|| author_id.map(str::to_owned))
        .unwrap_or_else(|| "Papo".to_owned());
    let channel = context
        .channels
        .get(channel_id)
        .map(|name| format!("#{name}"))
        .unwrap_or_default();

    match (channel.is_empty(), context.server_label.is_empty()) {
        (true, true) => author,
        (true, false) => format!("{author} · {}", context.server_label),
        (false, true) => format!("{author} · {channel}"),
        (false, false) => format!("{author} · {channel} · {}", context.server_label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::path::PathBuf;

    struct TempDb {
        dir: PathBuf,
    }

    impl TempDb {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "papo-notification-{name}-{}-{}",
                std::process::id(),
                now_millis()
            ));
            std::fs::create_dir_all(&dir).expect("temporary cache dir");
            Self { dir }
        }

        fn db(&self) -> Arc<ClientDb> {
            Arc::new(ClientDb::open(Some(self.dir.join("papo-cache.db"))))
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn context(server: &str, owner: &str, enabled: bool, visible: bool) -> NotificationContext {
        NotificationContext {
            server_key: server.to_owned(),
            navigation_server: format!("https://{server}.example"),
            server_label: server.to_owned(),
            owner_user_id: owner.to_owned(),
            owner_name: "ana".to_owned(),
            selected_channel: "geral".to_owned(),
            visible_server: visible,
            notifications_enabled: enabled,
            channels: [("geral".to_owned(), "Geral".to_owned()), ("outro".to_owned(), "Outro".to_owned())]
                .into_iter()
                .collect(),
            members: [
                (owner.to_owned(), "Ana".to_owned()),
                ("bia".to_owned(), "Bia".to_owned()),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn message(id: &str, channel: &str, author: &str, content: &str) -> Message {
        Message {
            id: id.to_owned(),
            channel_id: channel.to_owned(),
            author_id: author.to_owned(),
            content: Some(content.to_owned()),
            created_at: Utc::now(),
            edited_at: None,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            user_reactions: Vec::new(),
        }
    }

    fn notification(id: &str, message_id: &str, channel: &str, author: &str) -> Notification {
        Notification {
            id: id.to_owned(),
            message_id: Some(message_id.to_owned()),
            channel_id: Some(channel.to_owned()),
            author_id: Some(author.to_owned()),
            message_content: Some("reply".to_owned()),
            read: false,
            created_at: None,
        }
    }

    fn coordinator(
        db: Arc<ClientDb>,
        foreground: bool,
    ) -> (NotificationCoordinator, Arc<AtomicUsize>) {
        let deliveries = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&deliveries);
        let coordinator = NotificationCoordinator::new(
            db,
            Some(Arc::new(move |_| {
                seen.fetch_add(1, AtomicOrdering::SeqCst);
            })),
        );
        coordinator.set_foreground(foreground);
        (coordinator, deliveries)
    }

    #[test]
    fn message_mention_claims_and_delivers_once() {
        let temp = TempDb::new("message-once");
        let (coordinator, deliveries) = coordinator(temp.db(), false);
        coordinator.sync_context(context("srv", "me", true, true));
        let msg = message("m1", "geral", "bia", "oi <@me>");

        assert_eq!(
            coordinator.handle_message("srv", &msg, CandidateSource::Live),
            NotificationOutcome::Delivered
        );
        assert_eq!(
            coordinator.handle_message("srv", &msg, CandidateSource::Live),
            NotificationOutcome::Duplicate
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn non_mention_message_does_not_consume_ledger() {
        let temp = TempDb::new("non-mention");
        let db = temp.db();
        let (coordinator, deliveries) = coordinator(Arc::clone(&db), false);
        coordinator.sync_context(context("srv", "me", true, true));

        assert_eq!(
            coordinator.handle_message(
                "srv",
                &message("m1", "geral", "bia", "normal"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Ignored
        );
        assert_eq!(
            coordinator.handle_notification(
                "srv",
                &notification("n1", "m1", "geral", "bia"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Delivered
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn backend_notification_delivers_once() {
        let temp = TempDb::new("notification-once");
        let (coordinator, deliveries) = coordinator(temp.db(), false);
        coordinator.sync_context(context("srv", "me", true, true));
        let item = notification("n1", "m1", "geral", "bia");

        assert_eq!(
            coordinator.handle_notification("srv", &item, CandidateSource::Live),
            NotificationOutcome::Delivered
        );
        assert_eq!(
            coordinator.handle_notification("srv", &item, CandidateSource::Live),
            NotificationOutcome::Duplicate
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn message_then_notification_converge_to_one_delivery() {
        let temp = TempDb::new("message-notification");
        let (coordinator, deliveries) = coordinator(temp.db(), false);
        coordinator.sync_context(context("srv", "me", true, true));
        assert_eq!(
            coordinator.handle_message(
                "srv",
                &message("m1", "geral", "bia", "@everyone"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Delivered
        );
        assert_eq!(
            coordinator.handle_notification(
                "srv",
                &notification("n1", "m1", "geral", "bia"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Duplicate
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn notification_then_message_converge_to_one_delivery() {
        let temp = TempDb::new("notification-message");
        let (coordinator, deliveries) = coordinator(temp.db(), false);
        coordinator.sync_context(context("srv", "me", true, true));
        assert_eq!(
            coordinator.handle_notification(
                "srv",
                &notification("n1", "m1", "geral", "bia"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Delivered
        );
        assert_eq!(
            coordinator.handle_message(
                "srv",
                &message("m1", "geral", "bia", "@todos"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Duplicate
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn disabled_worthy_candidate_is_persistently_suppressed() {
        let temp = TempDb::new("disabled");
        let (coordinator, deliveries) = coordinator(temp.db(), false);
        coordinator.sync_context(context("srv", "me", false, true));
        let msg = message("m1", "geral", "bia", "<@me>");

        assert_eq!(
            coordinator.handle_message("srv", &msg, CandidateSource::Live),
            NotificationOutcome::Suppressed(SuppressionReason::NotificationsDisabled)
        );
        coordinator.sync_context(context("srv", "me", true, true));
        assert_eq!(
            coordinator.handle_message("srv", &msg, CandidateSource::Live),
            NotificationOutcome::Duplicate
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn same_visible_channel_is_persistently_suppressed() {
        let temp = TempDb::new("visible");
        let (coordinator, deliveries) = coordinator(temp.db(), true);
        coordinator.sync_context(context("srv", "me", true, true));
        let msg = message("m1", "geral", "bia", "<@me>");

        assert_eq!(
            coordinator.handle_message("srv", &msg, CandidateSource::Live),
            NotificationOutcome::Suppressed(SuppressionReason::ForegroundVisible)
        );
        coordinator.set_foreground(false);
        assert_eq!(
            coordinator.handle_message("srv", &msg, CandidateSource::Live),
            NotificationOutcome::Duplicate
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn another_channel_or_server_still_delivers_in_foreground() {
        let temp = TempDb::new("other-visible");
        let (coordinator, deliveries) = coordinator(temp.db(), true);
        coordinator.sync_context(context("srv-a", "me", true, true));
        coordinator.sync_context(context("srv-b", "me", true, false));

        assert_eq!(
            coordinator.handle_message(
                "srv-a",
                &message("a", "outro", "bia", "<@me>"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Delivered
        );
        assert_eq!(
            coordinator.handle_message(
                "srv-b",
                &message("b", "geral", "bia", "<@me>"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Delivered
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 2);
    }

    #[test]
    fn own_message_never_delivers() {
        let temp = TempDb::new("own");
        let (coordinator, deliveries) = coordinator(temp.db(), false);
        coordinator.sync_context(context("srv", "me", true, true));

        assert_eq!(
            coordinator.handle_message(
                "srv",
                &message("m1", "geral", "me", "<@me>"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Ignored
        );
        assert_eq!(
            coordinator.handle_notification(
                "srv",
                &notification("n2", "m2", "geral", "me"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Suppressed(SuppressionReason::OwnMessage)
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn restore_and_reconcile_sources_cannot_notify_or_claim() {
        for source in [CandidateSource::CacheRestore, CandidateSource::Reconcile] {
            let temp = TempDb::new(match source {
                CandidateSource::CacheRestore => "restore",
                CandidateSource::Reconcile => "reconcile",
                CandidateSource::Live => unreachable!(),
            });
            let (coordinator, deliveries) = coordinator(temp.db(), false);
            coordinator.sync_context(context("srv", "me", true, true));
            let msg = message("m1", "geral", "bia", "<@me>");
            assert_eq!(
                coordinator.handle_message("srv", &msg, source),
                NotificationOutcome::Ignored
            );
            assert_eq!(
                coordinator.handle_message("srv", &msg, CandidateSource::Live),
                NotificationOutcome::Delivered
            );
            assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
        }
    }

    #[test]
    fn account_and_server_partitions_are_independent() {
        let temp = TempDb::new("partitions");
        let (coordinator, deliveries) = coordinator(temp.db(), false);
        let msg = message("same", "geral", "bia", "<@me>");

        coordinator.sync_context(context("srv-a", "me", true, true));
        assert_eq!(
            coordinator.handle_message("srv-a", &msg, CandidateSource::Live),
            NotificationOutcome::Delivered
        );

        coordinator.sync_context(context("srv-a", "other", true, true));
        let other_msg = message("same", "geral", "bia", "<@other>");
        assert_eq!(
            coordinator.handle_message("srv-a", &other_msg, CandidateSource::Live),
            NotificationOutcome::Delivered
        );

        coordinator.sync_context(context("srv-b", "me", true, false));
        assert_eq!(
            coordinator.handle_message("srv-b", &msg, CandidateSource::Live),
            NotificationOutcome::Delivered
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 3);
    }

    #[test]
    fn db_failure_fails_closed_without_uncontrolled_delivery() {
        let db = Arc::new(ClientDb::open(None));
        let (coordinator, deliveries) = coordinator(db, false);
        coordinator.sync_context(context("srv", "me", true, true));

        assert_eq!(
            coordinator.handle_message(
                "srv",
                &message("m1", "geral", "bia", "<@me>"),
                CandidateSource::Live,
            ),
            NotificationOutcome::LedgerUnavailable
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn one_coordinator_produces_the_platform_neutral_envelope() {
        let temp = TempDb::new("envelope");
        let captured = Arc::new(Mutex::new(Vec::<NotificationEnvelope>::new()));
        let sink_captured = Arc::clone(&captured);
        let coordinator = NotificationCoordinator::new(
            temp.db(),
            Some(Arc::new(move |envelope| {
                sink_captured.lock().unwrap().push(envelope);
            })),
        );
        coordinator.sync_context(context("srv", "me", true, true));

        assert_eq!(
            coordinator.handle_message(
                "srv",
                &message("m1", "geral", "bia", "oi <@me>"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Delivered
        );
        let envelopes = captured.lock().unwrap();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].message_id, "m1");
        assert_eq!(envelopes[0].notification_id, None);
        assert_eq!(envelopes[0].body, "oi @Ana");
    }

    #[test]
    fn duplicate_remains_suppressed_after_clientdb_reopen() {
        let temp = TempDb::new("reopen");
        {
            let (coordinator, deliveries) = coordinator(temp.db(), false);
            coordinator.sync_context(context("srv", "me", true, true));
            assert_eq!(
                coordinator.handle_message(
                    "srv",
                    &message("m1", "geral", "bia", "<@me>"),
                    CandidateSource::Live,
                ),
                NotificationOutcome::Delivered
            );
            assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
        }

        let (coordinator, deliveries) = coordinator(temp.db(), false);
        coordinator.sync_context(context("srv", "me", true, true));
        assert_eq!(
            coordinator.handle_message(
                "srv",
                &message("m1", "geral", "bia", "<@me>"),
                CandidateSource::Live,
            ),
            NotificationOutcome::Duplicate
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn atomic_concurrent_candidates_deliver_exactly_once() {
        let temp = TempDb::new("concurrent");
        let deliveries = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&deliveries);
        let coordinator = Arc::new(NotificationCoordinator::new(
            temp.db(),
            Some(Arc::new(move |_| {
                seen.fetch_add(1, AtomicOrdering::SeqCst);
            })),
        ));
        coordinator.sync_context(context("srv", "me", true, true));

        let mut threads = Vec::new();
        for _ in 0..16 {
            let coordinator = Arc::clone(&coordinator);
            threads.push(std::thread::spawn(move || {
                coordinator.handle_message(
                    "srv",
                    &message("m1", "geral", "bia", "<@me>"),
                    CandidateSource::Live,
                )
            }));
        }
        let outcomes: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().expect("notification thread"))
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == NotificationOutcome::Delivered)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == NotificationOutcome::Duplicate)
                .count(),
            15
        );
        assert_eq!(deliveries.load(AtomicOrdering::SeqCst), 1);
    }

}

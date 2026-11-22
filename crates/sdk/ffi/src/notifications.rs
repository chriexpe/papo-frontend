//! Shared notification policy, inbox, push device ownership, and live FULL delivery handoffs.
//! Platform notification banners, channels, and Firebase tokens remain in Android app code.

use std::{collections::BTreeSet, sync::Arc};

use futures_util::stream;
use papo_sdk::{NotificationRecord, NotificationStatus};
use papo_sdk_ui::{AccountInbox, NotificationClient};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid_list},
    notification_records::{
        FfiAccountInbox, FfiInboxCursor, FfiInboxPage, FfiNotificationDeliveryListener,
        FfiNotificationDiagnostics, FfiNotificationRecord, FfiNotificationSettings,
        FfiNotificationStatusListener, FfiNotificationTarget, map_delivery, map_settings_from_sdk,
        map_settings_to_sdk, map_status,
    },
    operation_errors::FfiNotificationError,
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

#[uniffi::export]
impl FfiClient {
    /// Updates visible foreground channels for notification suppression (`empty` means none suppressed).
    ///
    /// # Errors
    /// Returns invalid UUID or closed session errors.
    pub fn set_visible_notification_channels(
        &self,
        account: FfiAccountId,
        channel_ids: Vec<String>,
    ) -> Result<(), FfiNotificationError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiNotificationError::Session { error: err })?;
        let parsed: BTreeSet<_> = parse_uuid_list(channel_ids)
            .map_err(|_| FfiNotificationError::InvalidData)?
            .into_iter()
            .collect();
        session
            .notifications()
            .set_visible_channels(parsed)
            .map_err(Into::into)
    }

    /// Revalidates an account-scoped notification target before posting or navigating on tap.
    ///
    /// # Errors
    /// Returns `InvalidData` / session errors if account replaced, channel gone, or denied.
    pub fn validate_notification_target(
        &self,
        target: FfiNotificationTarget,
    ) -> Result<(), FfiNotificationError> {
        if self.is_closed() {
            return Err(FfiNotificationError::Session {
                error: FfiClientError::Closed,
            });
        }
        let sdk_target = target.into_sdk()?;
        let session = self
            .handle
            .session(sdk_target.account)
            .map_err(|err| FfiNotificationError::Session { error: err.into() })?;
        session
            .notifications()
            .validate_target(&sdk_target)
            .map_err(Into::into)
    }

    /// Reads notification queue overflow diagnostics without exposing IDs or content.
    ///
    /// # Errors
    /// Returns [`FfiClientError`] if session missing.
    pub fn notification_diagnostics(
        &self,
        account: FfiAccountId,
    ) -> Result<FfiNotificationDiagnostics, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let d = session.notifications().diagnostics();
        Ok(FfiNotificationDiagnostics {
            deferred_evictions: d.deferred_evictions,
            lookup_evictions: d.lookup_evictions,
        })
    }

    /// Subscribes to notification producer status (`Ready` / `Fault` / `Closed`).
    ///
    /// # Errors
    /// Returns missing session or worker startup failures.
    pub fn subscribe_notification_status(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiNotificationStatusListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let mut rx = session.notifications().subscribe_status();
        let baseline = *rx.borrow_and_update();
        if matches!(baseline, NotificationStatus::Closed) {
            return Err(FfiClientError::Closed);
        }
        let stream = Box::pin(stream::unfold(Some(rx), |opt| async move {
            let mut receiver = opt?;
            receiver.changed().await.ok()?;
            let next = *receiver.borrow_and_update();
            let remaining = (!matches!(next, NotificationStatus::Closed)).then_some(receiver);
            Some((next, remaining))
        }));
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            stream,
            move |s| {
                l_snap.on_snapshot(map_status(s));
                true
            },
            move |s| {
                l_upd.on_update(map_status(s));
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Subscribes to live FULL-committed platform notification deliveries for `account`.
    ///
    /// Multiple consumers on one account compete cleanly without duplicating deliveries.
    ///
    /// # Errors
    /// Returns missing session or worker failures.
    pub fn subscribe_notification_deliveries(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiNotificationDeliveryListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let stream = Box::pin(stream::unfold(session.notifications(), |svc| async move {
            let delivery = svc.next_delivery().await.ok()?;
            Some((delivery, svc))
        }));
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            (),
            stream,
            move |()| true,
            move |delivery| {
                l_upd.on_delivery(map_delivery(delivery));
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Loads durable notification policy settings for `account`.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError`].
    pub async fn notification_settings(
        &self,
        account: FfiAccountId,
    ) -> Result<FfiNotificationSettings, FfiNotificationError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiNotificationError::Session { error: err })?;
        self.run_scoped(
            || FfiNotificationError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let s = session.notifications().settings().await?;
                Ok(map_settings_from_sdk(&s))
            },
        )
        .await
    }

    /// Persists durable device notification policy settings with FULL commit.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError`].
    pub async fn set_notification_settings(
        &self,
        account: FfiAccountId,
        settings: FfiNotificationSettings,
    ) -> Result<(), FfiNotificationError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiNotificationError::Session { error: err })?;
        let sdk = map_settings_to_sdk(settings)?;
        self.run_scoped(
            || FfiNotificationError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .notifications()
                    .set_settings(sdk)
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Fetches one authoritative notification inbox page for `account`.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError`].
    pub async fn fetch_inbox(
        &self,
        account: FfiAccountId,
        cursor: Option<FfiInboxCursor>,
    ) -> Result<FfiInboxPage, FfiNotificationError> {
        let sdk_account = account
            .into_sdk()
            .map_err(|_| FfiNotificationError::InvalidData)?;
        let handle = self.handle.clone();
        self.run_scoped(
            || FfiNotificationError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let page = NotificationClient::new(handle)
                    .inbox(sdk_account, cursor.map(Into::into))
                    .await?;
                Ok(FfiInboxPage {
                    records: page.records.iter().map(Into::into).collect(),
                    next_cursor: page.next.as_ref().map(Into::into),
                })
            },
        )
        .await
    }

    /// Fetches all signed-in accounts' first inbox pages concurrently without failing one another.
    ///
    /// # Errors
    /// Returns [`FfiClientError::Closed`] if shut down.
    pub async fn fetch_all_inboxes(&self) -> Result<Vec<FfiAccountInbox>, FfiClientError> {
        let handle = self.handle.clone();
        self.run_scoped(|| FfiClientError::Closed, async move {
            let items = NotificationClient::new(handle).inboxes().await;
            Ok(items
                .into_iter()
                .map(|item| match item {
                    AccountInbox::Ready { account, page } => FfiAccountInbox::Ready {
                        account: FfiAccountId::from_sdk(account),
                        page: FfiInboxPage {
                            records: page.records.iter().map(Into::into).collect(),
                            next_cursor: page.next.as_ref().map(Into::into),
                        },
                    },
                    AccountInbox::Retry(account) => FfiAccountInbox::Retry {
                        account: FfiAccountId::from_sdk(account),
                    },
                    _ => FfiAccountInbox::Retry {
                        account: FfiAccountId {
                            server_id: String::new(),
                            user_id: String::new(),
                        },
                    },
                })
                .collect())
        })
        .await
    }

    /// Marks specific backend notification rows read on the server.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError`].
    pub async fn mark_notifications_read(
        &self,
        records: Vec<FfiNotificationRecord>,
    ) -> Result<(), FfiNotificationError> {
        if self.is_closed() {
            return Err(FfiNotificationError::Session {
                error: FfiClientError::Closed,
            });
        }
        if records.is_empty() {
            return Ok(());
        }
        let first_account = records[0]
            .account
            .clone()
            .into_sdk()
            .map_err(|_| FfiNotificationError::InvalidData)?;
        let session = self
            .handle
            .session(first_account)
            .map_err(|err| FfiNotificationError::Session { error: err.into() })?;
        let mut sdk_records = Vec::with_capacity(records.len());
        for row in records {
            let acc = row
                .account
                .into_sdk()
                .map_err(|_| FfiNotificationError::InvalidData)?;
            if acc != first_account {
                return Err(FfiNotificationError::InvalidData);
            }
            sdk_records.push(NotificationRecord {
                account: acc,
                generation: papo_sdk::Generation::from_raw(row.generation),
                record: Arc::new(papo_sdk::Notification {
                    id: row.id,
                    message_id: row.message_id,
                    channel_id: row.channel_id,
                    author_id: row.author_id,
                    webhook_id: row.webhook_id,
                    external_author: None,
                    message_content: row.message_content,
                    read: row.read,
                    created_at: row.created_at,
                }),
                actual_mention: row.actual_mention,
            });
        }
        self.run_scoped(
            || FfiNotificationError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .notifications()
                    .mark_read(&sdk_records)
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }
}

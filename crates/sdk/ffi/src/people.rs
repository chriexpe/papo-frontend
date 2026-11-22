//! Member, block, role, role-definition, and on-demand profile projections across FFI.
//! Preserves bounded per-caller profile loading and separates external bridge authors.

use std::sync::Arc;

use papo_sdk_ui::{MemberItem, PeopleListSubscription, PeopleService};

use crate::{
    client::FfiClient,
    diffs::translate_diffs,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid_list},
    people_records::{
        FfiLoadedProfilesResult, FfiMemberDiff, FfiMemberListListener, FfiMemberSnapshot,
        FfiMemberUpdate, FfiProfileDiff, FfiProfileListListener, FfiProfileSnapshot,
        FfiProfileUpdate, FfiRoleItemDiff, FfiRoleItemListListener, FfiRoleItemSnapshot,
        FfiRoleItemUpdate, FfiRoleLabelDiff, FfiRoleLabelListListener, FfiRoleLabelSnapshot,
        FfiRoleLabelUpdate,
    },
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

/// Shared adapter hooking any `PeopleListSubscription<MemberItem>` to FFI listeners with all 11 diffs.
fn bridge_member_list(
    client: &FfiClient,
    sub: PeopleListSubscription<MemberItem>,
    listener: Arc<dyn FfiMemberListListener>,
) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
    let baseline = FfiMemberSnapshot {
        version: sub.version,
        generation: sub.generation.get(),
        items: sub.snapshot.into_iter().map(Into::into).collect(),
    };
    let l_snap = Arc::clone(&listener);
    let l_upd = Arc::clone(&listener);
    let l_end = listener;
    spawn_stream_subscription(
        &client.runtime,
        &client.tasks,
        &client.shutdown,
        client.callbacks.clone(),
        baseline,
        sub.stream,
        move |snap| {
            l_snap.on_snapshot(snap);
            true
        },
        move |upd| {
            let Ok(diffs) = translate_diffs(upd.diffs, |d| FfiMemberDiff::from_sdk(d, Into::into))
            else {
                return false;
            };
            l_upd.on_update(FfiMemberUpdate {
                version: upd.version,
                generation: upd.generation.get(),
                diffs,
            });
            true
        },
        move || l_end.on_closed(),
    )
}

#[uniffi::export]
impl FfiClient {
    /// Subscribes to admitted lightweight member list items (without triggering profile fetches).
    ///
    /// # Errors
    /// Returns missing session or worker startup failures.
    pub fn subscribe_members(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiMemberListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        bridge_member_list(self, PeopleService::new(session).members(), listener)
    }

    /// Subscribes to backend blocked user summaries.
    ///
    /// # Errors
    /// Returns missing session or worker startup failures.
    pub fn subscribe_blocks(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiMemberListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        bridge_member_list(self, PeopleService::new(session).blocks(), listener)
    }

    /// Subscribes to role badge labels for member pills.
    ///
    /// # Errors
    /// Returns missing session or worker startup failures.
    pub fn subscribe_roles(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiRoleLabelListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let sub = PeopleService::new(session).roles();
        let baseline = FfiRoleLabelSnapshot {
            version: sub.version,
            generation: sub.generation.get(),
            items: sub
                .snapshot
                .into_iter()
                .map(|r| r.as_ref().into())
                .collect(),
        };
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |upd| {
                let Ok(diffs) = translate_diffs(upd.diffs, |d| {
                    FfiRoleLabelDiff::from_sdk(d, |r| r.as_ref().into())
                }) else {
                    return false;
                };
                l_upd.on_update(FfiRoleLabelUpdate {
                    version: upd.version,
                    generation: upd.generation.get(),
                    diffs,
                });
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Subscribes to full role definitions including backend permission bits.
    ///
    /// # Errors
    /// Returns missing session or worker startup failures.
    pub fn subscribe_role_definitions(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiRoleItemListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let sub = PeopleService::new(session).role_definitions();
        let baseline = FfiRoleItemSnapshot {
            version: sub.version,
            generation: sub.generation.get(),
            items: sub.snapshot.into_iter().map(Into::into).collect(),
        };
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |upd| {
                let Ok(diffs) =
                    translate_diffs(upd.diffs, |d| FfiRoleItemDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                l_upd.on_update(FfiRoleItemUpdate {
                    version: upd.version,
                    generation: upd.generation.get(),
                    diffs,
                });
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Subscribes to cached requested profiles for one account.
    ///
    /// # Errors
    /// Returns missing session or worker startup failures.
    pub fn subscribe_profiles(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiProfileListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let sub = PeopleService::new(session).profiles();
        let baseline = FfiProfileSnapshot {
            version: sub.version,
            generation: sub.generation.get(),
            items: sub.snapshot.into_iter().map(Into::into).collect(),
        };
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |upd| {
                let Ok(diffs) =
                    translate_diffs(upd.diffs, |d| FfiProfileDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                l_upd.on_update(FfiProfileUpdate {
                    version: upd.version,
                    generation: upd.generation.get(),
                    diffs,
                });
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// On-demand bounded batch load of requested user profiles (up to 50 IDs per request).
    ///
    /// # Errors
    /// Returns [`FfiClientError::InvalidArgument`], [`FfiClientError::ProfileLimit`], or session/transport errors.
    pub async fn load_profiles(
        &self,
        account: FfiAccountId,
        user_ids: Vec<String>,
    ) -> Result<FfiLoadedProfilesResult, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let ids = parse_uuid_list(user_ids)?;
        let svc = PeopleService::new(session);
        self.run_scoped(|| FfiClientError::Closed, async move {
            let generated = svc.load_profiles(&ids).await?;
            Ok(FfiLoadedProfilesResult {
                generation: generated.generation.get(),
                profiles: generated.value.into_iter().map(Into::into).collect(),
            })
        })
        .await
    }
}

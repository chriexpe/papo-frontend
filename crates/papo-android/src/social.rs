//! Account-scoped social operations. Success means the server accepted the
//! change; rail updates do not depend on an unrelated follow-up GET succeeding.
use crate::{
    account,
    bindings::{Backend, NativeClient},
};
use papo_core::api::{client::ApiError, net::Update};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SocialError {
    #[error("account changed")]
    AccountChanged,
    #[error("invalid person or conversation")]
    InvalidTarget,
    #[error("direct messages blocked")]
    Blocked,
    #[error("session expired")]
    SignedOut,
    #[error("request unavailable")]
    Unavailable,
    #[error("request rejected")]
    Rejected,
}
impl From<ApiError> for SocialError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::DirectMessageBlocked(_) => Self::Blocked,
            ApiError::Unauthorized | ApiError::ServerLocked => Self::SignedOut,
            ApiError::NotFound => Self::InvalidTarget,
            ApiError::Network(_) | ApiError::TooManyRequests(_) => Self::Unavailable,
            _ => Self::Rejected,
        }
    }
}

#[derive(Debug, uniffi::Record)]
pub struct BlockedPerson {
    pub id: String,
    pub name: String,
    pub username: String,
}

#[uniffi::export]
impl NativeClient {
    pub fn blocked_people(&self, owner: String) -> Result<Vec<BlockedPerson>, SocialError> {
        {
            let backend = self.backend.lock().map_err(|_| SocialError::Unavailable)?;
            if let Some(Backend::Demo(client)) = backend.as_ref() {
                if client.store.me != owner {
                    return Err(SocialError::AccountChanged);
                }
                return Ok(client
                    .store
                    .members
                    .iter()
                    .filter(|user| client.store.is_user_blocked(&user.id))
                    .map(|user| BlockedPerson {
                        id: user.id.clone(),
                        name: user.name.clone(),
                        username: user.username.clone(),
                    })
                    .collect());
            }
        }
        let request = self
            .scoped_api(&owner)
            .map_err(|_| SocialError::AccountChanged)?;
        let users = account::run_api(request.api.blocked_users())?;
        let people = users
            .iter()
            .map(|user| BlockedPerson {
                id: user.id.clone(),
                name: user.display_name().into(),
                username: user.username.clone(),
            })
            .collect();
        self.finish_request(&request, vec![Update::BlockedUsers(users)])
            .map_err(|_| SocialError::AccountChanged)?;
        Ok(people)
    }

    pub fn set_person_blocked(
        &self,
        owner: String,
        user_id: String,
        blocked: bool,
    ) -> Result<(), SocialError> {
        if user_id.is_empty() || user_id == owner {
            return Err(SocialError::InvalidTarget);
        }
        let update = Update::UserBlockChanged {
            user_id: user_id.clone(),
            blocked,
        };
        {
            let mut backend = self.backend.lock().map_err(|_| SocialError::Unavailable)?;
            if let Some(Backend::Demo(client)) = backend.as_mut() {
                if client.store.me != owner {
                    return Err(SocialError::AccountChanged);
                }
                if client.store.member(&user_id).is_none() {
                    return Err(SocialError::InvalidTarget);
                }
                client.store.apply(update);
                return Ok(());
            }
        }
        let request = self
            .scoped_api(&owner)
            .map_err(|_| SocialError::AccountChanged)?;
        account::run_api(async {
            if blocked {
                request.api.block_user(&user_id).await
            } else {
                request.api.unblock_user(&user_id).await
            }
        })?;
        // Unblocking does not reopen the DM; only POST /dms does that.
        self.finish_request(&request, vec![update])
            .map_err(|_| SocialError::AccountChanged)
    }

    pub fn hide_direct_message(&self, owner: String, dm_id: String) -> Result<(), SocialError> {
        {
            let mut backend = self.backend.lock().map_err(|_| SocialError::Unavailable)?;
            let store = match backend.as_mut() {
                Some(Backend::Live(c)) => &mut c.runtime.store,
                Some(Backend::Demo(c)) => &mut c.store,
                None => return Err(SocialError::AccountChanged),
            };
            if store.me != owner {
                return Err(SocialError::AccountChanged);
            }
            if store.direct_message(&dm_id).is_none() {
                return Err(SocialError::InvalidTarget);
            }
            if let Some(Backend::Demo(client)) = backend.as_mut() {
                client.store.apply(Update::DirectMessageHidden(dm_id));
                return Ok(());
            }
        }
        let request = self
            .scoped_api(&owner)
            .map_err(|_| SocialError::AccountChanged)?;
        account::run_api(request.api.hide_direct_message(&dm_id))?;
        self.finish_request(&request, vec![Update::DirectMessageHidden(dm_id)])
            .map_err(|_| SocialError::AccountChanged)
    }

    pub fn open_direct_message(&self, owner: String, user_id: String) -> Result<(), SocialError> {
        if user_id.is_empty() || user_id == owner {
            return Err(SocialError::InvalidTarget);
        }
        let existing_dm = {
            let mut backend = self.backend.lock().map_err(|_| SocialError::Unavailable)?;
            if let Some(Backend::Demo(client)) = backend.as_mut() {
                if client.store.me != owner {
                    return Err(SocialError::AccountChanged);
                }
                if client.store.is_user_blocked(&user_id) {
                    return Err(SocialError::Blocked);
                }
                return client
                    .dispatch(crate::Action::OpenDm { user_id })
                    .map_err(|_| SocialError::InvalidTarget);
            }
            match backend.as_ref() {
                Some(Backend::Live(client)) => client
                    .runtime
                    .store
                    .direct_messages
                    .iter()
                    .find(|dm| dm.user.id == user_id)
                    .map(|dm| dm.id.clone()),
                _ => return Err(SocialError::AccountChanged),
            }
        };
        let request = self
            .scoped_api(&owner)
            .map_err(|_| SocialError::AccountChanged)?;
        let dm = match account::run_api(request.api.open_direct_message(&user_id)) {
            Ok(dm) => dm,
            Err(ApiError::DirectMessageBlocked(_)) => {
                // A block by the other participant is not our own block. Evict
                // only this inaccessible rail entry, leaving the block list intact.
                if let Some(id) = existing_dm {
                    self.finish_request(&request, vec![Update::DirectMessageHidden(id)])
                        .map_err(|_| SocialError::AccountChanged)?;
                }
                return Err(SocialError::Blocked);
            }
            Err(error) => return Err(error.into()),
        };
        self.finish_request(&request, vec![Update::DirectMessageOpened(Box::new(dm))])
            .map_err(|_| SocialError::AccountChanged)
    }
}

#[cfg(test)]
#[path = "social_tests.rs"]
mod tests;

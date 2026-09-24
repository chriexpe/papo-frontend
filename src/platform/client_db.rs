//! ClientDb construction policy.
//!
//! Android needs a process-wide owner because WorkManager may create the
//! process before GameActivity and the Activity can start later in that same
//! process. Desktop retains the normal PapoApp-owned lifetime.

use std::sync::Arc;

use papo_core::cache::ClientDb;

#[cfg(target_os = "android")]
pub fn get() -> Arc<ClientDb> {
    use std::sync::OnceLock;

    static CLIENT_DB: OnceLock<Arc<ClientDb>> = OnceLock::new();
    Arc::clone(CLIENT_DB.get_or_init(|| {
        Arc::new(ClientDb::open(super::dirs::cache_db()))
    }))
}

#[cfg(not(target_os = "android"))]
pub fn get() -> Arc<ClientDb> {
    Arc::new(ClientDb::open(super::dirs::cache_db()))
}

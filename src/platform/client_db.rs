//! Process-wide ClientDb owner shared by the UI runtime and Android headless work.
//!
//! Android WorkManager can create the process before GameActivity. Whichever
//! entry point arrives first opens the single cache worker; later callers clone
//! the same Arc instead of opening another connection to papo-cache.db.

use std::sync::{Arc, OnceLock};

use papo_core::cache::ClientDb;

static CLIENT_DB: OnceLock<Arc<ClientDb>> = OnceLock::new();

pub fn get() -> Arc<ClientDb> {
    Arc::clone(CLIENT_DB.get_or_init(|| {
        Arc::new(ClientDb::open(super::dirs::cache_db()))
    }))
}

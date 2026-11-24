//! The account-scoped handle to one staged attachment blob; staging and collection are `files`'.

use crate::AccountId;

/// Account-scoped reference to an immutable, fsynced attachment blob in outgoing staging.
///
/// Created by [`crate::OutgoingService::stage_attachment`] or attached when loading a committed
/// draft or operation. While a freshly staged handle is alive in memory, background orphan
/// collection will not delete its blob; once dropped without a durable draft or operation
/// reference (or after process restart), the file remains eligible for collection. Once committed
/// in a draft or queued operation, its immutable bytes stay pinned until `Sent`, explicit discard,
/// draft replacement or account removal. Never retains a caller path or handle.
#[derive(Clone)]
pub struct StagedAttachment {
    account: AccountId,
    digest: [u8; 32],
    size_bytes: u64,
    _pin: Option<std::sync::Arc<super::super::StagedPin>>,
}

impl std::fmt::Debug for StagedAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagedAttachment")
            .field("account", &self.account)
            .field("digest", &self.digest_hex())
            .field("size_bytes", &self.size_bytes)
            .finish_non_exhaustive()
    }
}

impl PartialEq for StagedAttachment {
    fn eq(&self, other: &Self) -> bool {
        self.account == other.account
            && self.digest == other.digest
            && self.size_bytes == other.size_bytes
    }
}

impl Eq for StagedAttachment {}

impl StagedAttachment {
    pub(in crate::outgoing) const fn new(
        account: AccountId,
        digest: [u8; 32],
        size_bytes: u64,
    ) -> Self {
        Self {
            account,
            digest,
            size_bytes,
            _pin: None,
        }
    }

    pub(in crate::outgoing) fn with_pin(
        account: AccountId,
        digest: [u8; 32],
        size_bytes: u64,
        pin: std::sync::Arc<super::super::StagedPin>,
    ) -> Self {
        Self {
            account,
            digest,
            size_bytes,
            _pin: Some(pin),
        }
    }

    /// Account whose private staging directory owns this immutable blob.
    #[must_use]
    pub const fn account(&self) -> AccountId {
        self.account
    }

    /// SHA-256 digest of the staged file contents.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Lowercase hexadecimal SHA-256 digest naming the blob inside the account's staging directory.
    #[must_use]
    pub fn digest_hex(&self) -> String {
        super::super::files::digest_hex(&self.digest)
    }

    /// Exact byte length of the staged file.
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

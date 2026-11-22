//! `UniFFI` bindings bridging Android Kotlin adapters to `papo-sdk` and `papo-sdk-ui`.
//!
//! # Single-image native media architecture
//! Session 16 links `papo-ffi` (`rlib`) + `papo-call` + narrow JNI `ANativeWindow` surface code
//! into a single Android `cdylib` (`.so`). Separate `cdylib`s each receive distinct copies of Rust
//! `static` variables (`OnceLock`), so dual-`cdylib` static sharing is forbidden; instead,
//! callers either invoke [`register_media_engine_factory_once`] before [`FfiClient::open`] or
//! construct via [`FfiClient::open_with_media_engine`] inside that unified native library.
//! Meanwhile, `papo-ffi`'s standalone `cdylib` remains media-free for SDK CI, `:sdk-proof`, and
//! host JVM tests (`--features test-support`).

#![forbid(unsafe_code)]
// UniFFI `#[uniffi::export]` `Lift` requires owned `String`/`Record`/`Vec` values across FFI.
#![allow(clippy::needless_pass_by_value)]

mod auth;
mod auth_records;
mod call_enums;
mod call_snapshot;
mod call_view;
mod calls;
mod capabilities;
mod channels;
mod client;
mod composer;
mod composer_tokens;
mod diffs;
mod direct_messages;
mod disk;
mod errors;
mod expressions;
mod ids;
mod media;
mod media_disk_errors;
mod media_part_file;
mod media_records;
mod message_content;
mod messages;
mod notification_records;
mod notifications;
mod notifications_push;
mod operation_errors;
mod operations_admin;
mod operations_audit;
mod operations_channel_records;
mod operations_profile;
mod operations_profile_records;
mod operations_reactions_pins;
mod operations_records;
mod operations_role_records;
mod operations_roles;
mod operations_social;
mod operations_webhooks;
mod outgoing;
mod outgoing_payload;
mod outgoing_state;
mod people;
mod people_records;
mod reading_pins;
mod search;
mod search_records;
mod secret;
#[cfg(test)]
mod secret_tests;
mod server_badges_records;
mod server_records;
mod servers;
mod shutdown_report;
mod subscription;
mod sync_connection;
#[cfg(feature = "test-support")]
mod test_support;
mod timeline;
mod timeline_errors;
mod timeline_items;
#[cfg(feature = "integration-proof")]
pub mod timeline_proof;
mod timeline_seek;

pub use auth::FfiAuthHandle;
pub use auth_records::{
    FfiAuthState, FfiAuthStateListener, FfiConnectedDevice, FfiConnectedDevicesResult,
    FfiDropConnectionResult, FfiRegisterResponse,
};
pub use call_enums::{
    FfiCallEndReason, FfiCallPhase, FfiCallWarning, FfiCameraFacing, FfiCaptureState,
    FfiMediaLeaseError, FfiMediaTransportState, FfiSpeakingState, FfiVideoCodec, FfiVideoKind,
    FfiVoiceCapability,
};
pub use call_snapshot::{
    FfiActiveAccountCall, FfiAudioTrackRoute, FfiCallParticipant, FfiCallSettings, FfiCallSnapshot,
    FfiPlaybackSettings, FfiVideoBinding, FfiVideoTarget, FfiVoiceDeviceState,
};
pub use call_view::{
    FfiActiveCallView, FfiCallParticipantItem, FfiCallView, FfiParticipantVideoBinding,
    FfiRoomParticipantItem, FfiVoiceRoomView,
};
pub use calls::{FfiCallAttemptListener, FfiCallHandle, FfiCallViewListener};
pub use capabilities::{
    FfiActivityListener, FfiActivityView, FfiCapabilityListener, FfiCapabilityView,
    FfiChannelCapabilities, FfiPresenceItem, FfiPresenceStatus, FfiServerCapabilities,
    FfiTypingItem,
};
pub use channels::{
    FfiChannelBadges, FfiChannelDiff, FfiChannelKind, FfiChannelListItem, FfiChannelListListener,
    FfiChannelListSnapshot, FfiChannelListUpdate, FfiMessageKey, FfiReadState,
};
pub use client::{FfiClient, FfiClientConfig, register_media_engine_factory_once};
pub use composer::{
    FfiPairClosingState, FfiPairEdit, FfiSelectionFormat, FfiTextChangeSource,
    FfiTextSelectionEdit, apply_composer_selection_format, composer_on_pair_change,
    composer_skip_pair_closer, composer_track_pair_change, is_spoiler_attachment_filename,
    normalize_composer_fenced_code_blocks, with_spoiler_attachment_prefix,
    without_spoiler_attachment_prefix,
};
pub use composer_tokens::{
    FfiComposerMentionBinding, FfiDecodedComposerMentions, FfiMentionDisplayName, FfiPreviewSpan,
    FfiPreviewSpanKind, composer_preview_spans_utf16, decode_composer_mentions_utf16,
    encode_composer_mentions_utf16,
};
pub use direct_messages::{
    FfiDirectMessageDiff, FfiDirectMessageItem, FfiDirectMessageListener, FfiDirectMessageSnapshot,
    FfiDirectMessageUpdate, FfiServerInfoListener, FfiServerInfoView,
};
pub use disk::{FfiDiskFault, FfiDiskReport, FfiDiskStatus, FfiDiskStatusListener};
pub use errors::{
    FfiAuthError, FfiAuthFailureKind, FfiClientError, FfiDiscoveryFailureKind, FfiProblemKind,
    FfiSecretError, FfiSecretFailure, MediaFactoryRegistrationError,
};
pub use expressions::{
    FfiCatalogPage, FfiExpressionFault, FfiExpressionId, FfiExpressionItem,
    FfiExpressionLibraryEvent, FfiExpressionLibraryListener, FfiExpressionLibraryView, FfiGif,
    FfiReactionUsageItem, FfiSticker,
};
pub use ids::{FfiAccountId, FfiBackendMode, FfiHistoryCursor, FfiHistoryOrder, FfiNullableText};
pub use media_disk_errors::{FfiDiskError, FfiMediaError};
pub use media_records::{
    FfiEmojiMetadata, FfiEmojiPage, FfiMediaData, FfiMediaExportResult, FfiMediaRange,
    FfiMediaSource, FfiRangeMediaBase,
};
pub use message_content::{
    FfiBlock, FfiCustomEmojiRef, FfiInline, FfiListItem, FfiMentionTarget, FfiMessageContent,
    FfiTableAlignment, FfiTableRow, FfiTextStyle, is_plain_message_text, is_safe_message_href,
    parse_message_content,
};
pub use messages::{
    FfiAttachment, FfiEmbed, FfiEmbedAuthor, FfiEmbedField, FfiEmbedFooter, FfiEmbedMedia,
    FfiExternalAuthor, FfiMessage, FfiMessageReactionSummary, FfiMessageUserReaction,
};
pub use notification_records::{
    FfiAccountInbox, FfiChannelNotificationMode, FfiChannelNotificationOverride, FfiInboxCursor,
    FfiInboxPage, FfiNotificationDelivery, FfiNotificationDeliveryListener,
    FfiNotificationDiagnostics, FfiNotificationRecord, FfiNotificationSettings,
    FfiNotificationStatus, FfiNotificationStatusListener, FfiNotificationTarget, FfiPushDevice,
    FfiPushRegistration,
};
pub use operation_errors::{
    FfiCallError, FfiExpressionError, FfiNotificationError, FfiOperationError, FfiReactionError,
    FfiRefreshFault, FfiRefreshStatus, FfiSearchError,
};
pub use operations_audit::{
    FfiAuditCursor, FfiAuditLogEntry, FfiAuditMetadataField, FfiAuditMetadataValue, FfiAuditPage,
    FfiAuditRequest,
};
pub use operations_channel_records::{
    FfiChangeChannelPositionRequest, FfiChannelPermissionEntry, FfiChannelPermissions,
    FfiChannelPermissionsResult, FfiChannelReceipt, FfiChannelWire, FfiCreateChannelRequest,
    FfiUpdateChannelPermissionsReceipt, FfiUpdateChannelRequest, FfiUserSummaryWire,
};
pub use operations_profile_records::{
    FfiChannelSettingReceipt, FfiEmojiReceipt, FfiEmojiWire, FfiPasswordResetLink,
    FfiPasswordResetPreparation, FfiPasswordResetReceipt, FfiUserConfig, FfiUserDisplayConfig,
    FfiUserNotificationsConfig, FfiUserSettingsReceipt,
};
pub use operations_reactions_pins::{
    FfiPinReceipt, FfiPinnedMessageWire, FfiReactionAdmission, FfiReactionEmoji,
    FfiReactionGroupPageItem, FfiReactionIntent, FfiReactionUserWire, FfiReactorCursor,
    FfiReactorPage,
};
pub use operations_records::{
    FfiBatchGenericOutcome, FfiBatchUnitOutcome, FfiGenericReceipt, FfiMetadataScope,
    FfiPatchServerRequest, FfiRefreshOutcome, FfiReplaceServerRequest, FfiServerReceipt,
    FfiServerWire, FfiUnitReceipt,
};
pub use operations_role_records::{
    FfiBanChangeInput, FfiBatchUserRoleOutcome, FfiCreateRoleRequest, FfiRoleAssignmentInput,
    FfiRoleGrants, FfiRoleListResult, FfiRoleReceipt, FfiRoleUpdate, FfiRoleWire, FfiUserRole,
    FfiUserRoleReceipt,
};
pub use operations_roles::build_role_rename_update;
pub use operations_social::FfiDirectConversationWire;
pub use operations_webhooks::{
    FfiCreatedWebhook, FfiCreatedWebhookReceipt, FfiWebhookAttachmentInput, FfiWebhookMessageInput,
    FfiWebhookMessageReceipt, FfiWebhookWire,
};
pub use outgoing_payload::{
    FfiDraft, FfiDraftBodyInput, FfiEmbedInput, FfiEmbedInputAuthor, FfiEmbedInputField,
    FfiEmbedMediaInput, FfiMessageTarget, FfiOutgoingAttachmentInput, FfiOutgoingAttachmentView,
    FfiOutgoingMessageInput, FfiStagedAttachment,
};
pub use outgoing_state::{
    FfiOperation, FfiOperationFailure, FfiOperationKindInput, FfiOperationKindView,
    FfiOperationState, FfiOutgoingFault, FfiOutgoingListener, FfiOutgoingSnapshot,
    FfiRetryDecision, FfiUploadProgress,
};
pub use people_records::{
    FfiLoadedProfile, FfiLoadedProfilesResult, FfiMemberDiff, FfiMemberItem, FfiMemberListListener,
    FfiMemberSnapshot, FfiMemberUpdate, FfiProfileDiff, FfiProfileItem, FfiProfileListListener,
    FfiProfileSnapshot, FfiProfileUpdate, FfiRoleFlags, FfiRoleItem, FfiRoleItemDiff,
    FfiRoleItemListListener, FfiRoleItemSnapshot, FfiRoleItemUpdate, FfiRoleLabel,
    FfiRoleLabelDiff, FfiRoleLabelListListener, FfiRoleLabelSnapshot, FfiRoleLabelUpdate,
};
pub use reading_pins::{
    FfiChannelReadingPosition, FfiPinnedMessages, FfiReadingPosition, FfiReadingPositionsListener,
    FfiReadingPositionsSnapshot,
};
pub use search::{FfiDetachedMessageRequest, FfiSearchSurface};
pub use search_records::{
    FfiDetachedMessageUpdate, FfiSearchCursor, FfiSearchHit, FfiSearchListener, FfiSearchMatches,
    FfiSearchPage, FfiSearchQuery, FfiSearchState, FfiSearchTarget,
};
pub use secret::{FfiSecretKey, FfiSecretRead, FfiSecretStore, FfiSecretStoreError};
pub use server_badges_records::{
    FfiAccountBadgesEntry, FfiBadgeAggregate, FfiServerBadgesEntry, FfiServerBadgesSnapshot,
    FfiUnreadCount,
};
pub use server_records::{
    FfiAccountDiff, FfiAccountListSnapshot, FfiAccountListUpdate, FfiAddedServer, FfiRemovalDiff,
    FfiRemovalFailureKind, FfiRemovalJob, FfiRemovalStep, FfiRemovalTarget, FfiSavedAccount,
    FfiSavedServer, FfiServerDiff, FfiServerDiscovery, FfiServerIcon, FfiServerListSnapshot,
    FfiServerListUpdate,
};
pub use servers::{FfiAccountListListener, FfiServerBadgesListener, FfiServerListListener};
pub use shutdown_report::{
    FfiDatabaseFailure, FfiShutdownReport, FfiUnpersistedWrite, FfiWorkerShutdownReport,
    FfiWriteDisposition,
};
pub use subscription::{FfiCallbackDrainStatus, FfiSubscriptionHandle};
pub use sync_connection::{
    FfiBackgroundAccountResult, FfiBackgroundOutcome, FfiBackgroundReport, FfiConnectionListener,
    FfiConnectionState, FfiConnectionStatus, FfiInteractiveSyncGuard,
};
#[cfg(feature = "test-support")]
pub use test_support::{FfiFakeMediaFactory, open_ffi_client_with_fake_media};
pub use timeline::FfiTimeline;
pub use timeline_errors::{
    FfiHistoryClosedReason, FfiHistoryError, FfiOutgoingError, FfiReadingError, FfiTimelineError,
};
pub use timeline_items::{
    FfiHistoryGap, FfiOperationStub, FfiTimelineArrivals, FfiTimelineCatchUp, FfiTimelineConfig,
    FfiTimelineDiff, FfiTimelineDirection, FfiTimelineItem, FfiTimelineListener, FfiTimelinePage,
    FfiTimelineReadingListener, FfiTimelineReadingState, FfiTimelineSnapshot, FfiTimelineUpdate,
    FfiUtcOffsetProvider, FfiVisibilityContinuity,
};
pub use timeline_seek::FfiCatchUpSeek;

uniffi::setup_scaffolding!();

//! Client, account sessions, authentication, reconnect policy and durable device state.
//! Wire transport belongs to `papo-api`; UI projections and platform integration live above this crate.

mod activity;
pub use activity::{ActivitySnapshot, Presence, PresenceStatus, Typing};
mod account;
mod account_cache;
mod auth;
mod badges;
pub use badges::{BadgesSnapshot, ChannelBadges};
mod background;
pub use background::{
    BackgroundAccountResult, BackgroundCancellation, BackgroundOutcome, BackgroundReport,
    InteractiveSyncGuard,
};
mod calls;
pub use calls::{
    AudioRoute, CallAttemptId, CallConfig, CallEndReason, CallError, CallHandle, CallParticipant,
    CallPhase, CallSettings, CallSnapshot, CallWarning, Calls, CameraFacing, CaptureState,
    IceServerConfig, InitialMediaSettings, MAX_PLAYBACK_GAIN_PERCENT,
    MEDIA_REPORT_CHANNEL_CAPACITY, MediaCommand, MediaEngineFactory, MediaFatalError, MediaLease,
    MediaLeaseError, MediaReport, MediaReportKind, MediaReportSink, MediaReportSinkError,
    MediaStartError, MediaStartFuture, MediaTransportState, MediaWarning, PlaybackSettings,
    RoomMember, RoomsSnapshot, SpeakingState, VideoBinding, VideoCodec, VideoDestination,
    VideoDestinationId, VideoKind, VideoRouteFence, VideoSlotBinding, VideoTarget,
    VoiceDeviceState, VoiceErrorCode, VoiceErrorDisposition, VoiceRoom, classify_voice_error,
    is_fatal_voice_code,
};
#[cfg(any(test, feature = "test-support"))]
pub use calls::{FakeMediaFactory, FakeMediaLeaseHandle, FakeStartBehavior, FakeVideoDestination};
mod client;
mod client_error;
mod client_handle;
mod connection;
mod connection_driver;
mod discovery;
mod disk_budget;
mod generation;
pub use discovery::{AddedServer, DiscoveryFailure, MAX_ICON_BYTES, ServerDiscovery, ServerIcon};
pub use disk_budget::{DiskBudget, DiskError, DiskFault, DiskReport, DiskStatus};
mod expressions;
pub use expressions::{
    CatalogPage, ExpressionError, ExpressionService, ExpressionSnapshot, Gif, GiphyConfig, Sticker,
};
pub use expressions::{ExpressionFault, ExpressionUpdate};
mod media;
pub use media::{
    EmojiMetadata, EmojiPage, MediaData, MediaError, MediaRange, MediaService, MediaSource,
};
mod history;
mod images;
pub use history::{
    HistoryBudget, HistoryChunk, HistoryClosedReason, HistoryCoverage, HistoryDiagnostics,
    HistoryError, HistoryGap, HistoryHandle, HistoryPage, HistoryRange, HistorySnapshot,
    HistorySource, HistorySubscription, HistoryUpdate, HistoryWork, InvalidMessageKey,
    LocatedMessage, MessageKey, MessageLocator, ResolvedKey, authored_by_other,
    parse_message_timestamp_us,
};
pub use papo_api::wire::Message;
pub use papo_api::{HistoryCursor, HistoryOrder, HistoryQuery, Uuid};
mod observable;
pub mod operations;
pub use operations::{OperationError, OperationReceipt, Operations, RefreshFault, RefreshStatus};
mod outgoing;
pub use images::{
    Anonymize, AttachmentEdit, CropRect, FixedImage, ImageError, ImageFormat, ImageInfo,
    NAME_LENGTH, UnsupportedFormat, random_attachment_name,
};
pub use outgoing::OutgoingFault;
pub use outgoing::{
    AttachmentProgress, Draft, DraftBody, MessageTarget, Operation, OperationFailure,
    OperationKind, OperationState, OutgoingAttachment, OutgoingAttachmentContent, OutgoingError,
    OutgoingMessage, OutgoingService, OutgoingSnapshot, PendingAttachment, RetryDecision,
    StagedAttachment, StagingFailure, UploadProgress,
};
pub use outgoing::{
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS_PER_MESSAGE, MAX_MESSAGE_ATTACHMENT_BYTES,
};
pub use papo_api::wire::{EmbedInput, EmbedMediaInput};
mod catch_up;
mod catch_up_cleanup;
mod catch_up_history;
mod catch_up_position_store;
mod catch_up_spans;
mod catch_up_store;
pub mod mentions;
pub use mentions::{
    DecodedMentions, MentionBinding, MentionTarget, MentionToken, contains_everyone_mention,
    contains_user_mention, decode_mentions_for_editing, encode_mentions_with_bindings,
    encode_user_mention, extract_user_mentions, parse_user_mention, scan_mentions,
};
mod inline_avatar;
pub mod permissions;
mod private_fs;
pub use inline_avatar::{InlineAvatarError, inline_avatar};
mod profiles;
mod read_points;
mod reading;
pub use catch_up::{
    CatchUpSnapshot, CatchUpView, ReadingPosition, ReadingPositionsSnapshot, VisibilityContinuity,
};
pub use reading::{ReadState, ReadingError, ReadingObserver, ReadingSnapshot, UnreadCount};
mod reconcile;
mod reconcile_event;
mod reconcile_fetch;
mod resource_admission;
mod resource_commit;
pub use resource_admission::{AdmissionDiagnostics, AdmissionFault, DroppedRow, ResourceRejection};
mod resources;
pub use account_cache::{AccountCache, AccountCaches, CacheError, CacheResource};
pub use papo_api::wire::{
    ChangePasswordRequest, Channel, DirectConversation, ExternalAuthor, LoginRequest,
    LoginServerRequest, RegisterRequest, Role, RoleSummary, SearchResult, UserProfile, UserSummary,
};
pub use read_points::ServerReadPoint;
pub use resources::{
    BackendMode, ChannelKind, ChannelRecord, DirectRecord, EntityTable, MessageHead, Overwrite,
    OverwriteKey, ProfileRecord, ResourceIds, ResourceSnapshot, ResourceTable, RoleFlags,
    RoleRecord, ServerRecord, SnapshotRows, UserRecord,
};
mod notifications;
pub use notifications::{
    ChannelNotificationMode, InboxCursor, InboxPage, NotificationDelivery, NotificationDiagnostics,
    NotificationError, NotificationRecord, NotificationService, NotificationSettings,
    NotificationStatus, NotificationTarget, PushDevice, PushRegistration,
};
mod scoped_request;
mod search;
pub use search::{
    DateOrder, DateWords, DetachedMessageRequest, DetachedMessageUpdate, FilterValue, HasKind,
    LocalDays, MAX_SEARCH_BYTES, MAX_SEARCH_CANDIDATES, SearchCalendar, SearchCursor, SearchDate,
    SearchDraft, SearchError, SearchFilter, SearchHit, SearchKeyword, SearchNames, SearchPage,
    SearchProblem, SearchQuery, SearchRequestHandle, SearchScope, SearchTarget, parse_search_date,
};
mod session;
mod session_pool;

pub use auth::{AuthFailure, AuthService, AuthState};
pub use client::{Client, ClientConfig};
pub use client_error::ClientError;
pub use client_handle::{ClientHandle, RegistrySnapshot};
pub use connection::{
    Backoff, ConnectionState, ConnectionStatus, Jitter, RoundTrip, RuntimeJitter,
};
pub use generation::{Generated, Generation};
pub use session::{Session, SessionEventError, SessionEvents};
mod data_root;
mod registry;
mod registry_record;
pub mod removal;
pub mod secret;
mod server;
mod state_schema;
mod storage;
mod storage_error;

pub use storage::{Storage, StorageConfig, StorageShutdown};
pub mod storage_worker;

#[cfg(test)]
mod client_lifecycle_tests;
#[cfg(all(test, target_os = "linux"))]
mod crash_tests;
#[cfg(test)]
mod registry_tests;
#[cfg(test)]
mod removal_credential_tests;
#[cfg(test)]
mod removal_state_tests;
#[cfg(test)]
mod removal_tests;
#[cfg(test)]
mod storage_lifecycle_tests;
#[cfg(test)]
mod storage_tests;

pub use account::AccountId;
pub use data_root::DataRoot;
pub use registry::Registry;
pub use registry_record::{SavedAccount, SavedServer, UnixMicros};
pub use server::{ServerAddress, ServerAddressError, ServerId};
pub use storage_error::StorageError;

#[cfg(feature = "test-support")]
pub use outgoing::{OutgoingDispatchPhase, OutgoingDispatchProbe};

//! Typed account operation access for UI adapters; all authorization, dispatch and refresh remain in the SDK.
//! Native editors own unsaved input and explicit confirmation. A refresh warning never reopens a confirmed mutation.

pub use papo_sdk::operations::{
    MetadataScope, OperationError, OperationReceipt, Operations as OperationService, RefreshFault,
    RefreshStatus, administration, audit, channels, emojis, interactions, moderation, profile,
    reactions, roles, settings, social, webhooks,
};

pub use papo_sdk::operations::reactions::{
    ReactionAdmission, ReactionEmoji, ReactionError, ReactionIntent, ReactionService,
};

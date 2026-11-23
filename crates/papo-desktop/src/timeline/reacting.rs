//! What a timeline's rows read to draw their reactions and hover bars,
//! beyond the message: the account's outgoing queue (a chip waits on it),
//! its quick reactions, what it may do in the channel, and the hover bar
//! Add reaction stretched. The shell fills it once per view.

use std::sync::Arc;

use papo_sdk::{OutgoingSnapshot, Uuid};
use papo_sdk_ui::ChannelCapabilities;

/// What a row's reactions and hover bar read beyond the message.
#[derive(Clone)]
pub(crate) struct Reacting {
    /// The account's outgoing queue: a chip whose change is in it waits.
    pub(crate) outgoing: Arc<OutgoingSnapshot>,
    /// The account's quick reactions, most used first.
    pub(crate) quick: Arc<[String]>,
    /// What the account may do in the channel; none while unknown.
    pub(crate) capabilities: Option<ChannelCapabilities>,
    /// The message whose bar Add reaction stretched, and the emoji ← and →
    /// are on.
    pub(crate) stretched: Option<(Uuid, usize)>,
}

impl Reacting {
    /// Nothing known yet: no quick reactions, an empty queue.
    pub(crate) fn unknown() -> Reacting {
        Reacting {
            outgoing: Arc::new(OutgoingSnapshot {
                account: None,
                version: 0,
                operations: Vec::new(),
                upload_progress: None,
                fault: None,
            }),
            quick: Arc::from([]),
            capabilities: None,
            stretched: None,
        }
    }
}

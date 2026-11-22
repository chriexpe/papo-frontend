//! The timeline's local echoes (`TimelineItem::LocalEcho`): each unsent or
//! just-sent message of yours, as a message its row can draw and group like
//! any other, with its parsed body and where its send stands.
//!
//! Built when the items change, never in a view: an echo's body is parsed
//! once per operation image the SDK publishes. A confirmed send draws the
//! server's own message until history takes its place.

use std::collections::HashMap;
use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Message, MessageKey, Operation, OperationKind, OperationState, Uuid};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;

/// Where an unsent message's send stands, as its row shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SendState {
    /// Queued or on its way.
    Pending,
    /// The server refused it.
    Failed,
    /// The send was cut off: it may have arrived.
    Unknown,
}

impl SendState {
    /// The state of `state`; none once sent.
    fn of(state: OperationState) -> Option<SendState> {
        match state {
            OperationState::Sent => None,
            OperationState::Failed => Some(SendState::Failed),
            OperationState::Unknown => Some(SendState::Unknown),
            // Queued and Sending, and any state this client does not know
            // yet: not sent.
            _ => Some(SendState::Pending),
        }
    }
}

/// One echo, drawn as a message.
pub(crate) struct Echo {
    /// The SDK's image it was built from: a new one rebuilds it.
    operation: Arc<Operation>,
    pub(crate) message: Arc<Message>,
    pub(crate) content: Arc<MessageContent>,
    /// None once sent: the row then looks like any message.
    pub(crate) state: Option<SendState>,
}

/// The echoes among a timeline's items, by local identity.
#[derive(Default)]
pub(crate) struct Echoes(HashMap<Uuid, Echo>);

impl Echoes {
    /// The echo with local identity `id`.
    pub(crate) fn get(&self, id: Uuid) -> Option<&Echo> {
        self.0.get(&id)
    }

    /// Follows `items`: builds the echoes whose operation changed, keeps the
    /// rest, forgets those gone. `me` is the account's user, the author of
    /// every echo.
    pub(crate) fn refresh(&mut self, items: &Vector<Arc<TimelineItem>>, me: Uuid) {
        let mut next = HashMap::new();
        for item in items {
            let TimelineItem::LocalEcho { id, operation, .. } = item.as_ref() else {
                continue;
            };
            let kept = self
                .0
                .remove(id)
                .filter(|echo| Arc::ptr_eq(&echo.operation, operation));
            if let Some(echo) = kept.or_else(|| build(operation, me)) {
                next.insert(*id, echo);
            }
        }
        self.0 = next;
    }
}

/// The echo of `operation`, when it creates a message.
fn build(operation: &Arc<Operation>, me: Uuid) -> Option<Echo> {
    let OperationKind::Message(outgoing) = operation.kind.as_ref() else {
        return None;
    };
    let message = operation.message.clone().unwrap_or_else(|| {
        // Until the server answers, the echo stands at the time it was
        // written, under its local identity.
        let key = MessageKey {
            created_at_us: operation.created_at_us,
            message_id: operation.id,
        };
        Arc::new(Message {
            id: Some(operation.id.to_string()),
            author_id: Some(me.to_string()),
            content: Some(outgoing.content.clone()),
            reply_to: outgoing.reply_to.map(|target| match target {
                papo_sdk::MessageTarget::Remote(id) | papo_sdk::MessageTarget::Local(id) => {
                    id.to_string()
                }
            }),
            created_at: key.to_cursor().ok().map(|cursor| cursor.created_at),
            ..Message::default()
        })
    });
    let content = Arc::new(MessageContent::parse(
        message.content.as_deref().unwrap_or_default(),
    ));
    Some(Echo {
        operation: Arc::clone(operation),
        message,
        content,
        state: SendState::of(operation.state),
    })
}

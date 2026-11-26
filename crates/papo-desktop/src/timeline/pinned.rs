//! The channel's pins as the timeline holds them: the SDK's pin list
//! (`ChannelPins`, newest pin first), each pin's content parsed once when
//! the list arrives (the timeline's own parse when it holds the same
//! message), and the pinned IDs for the menus' Pin and Unpin. Whether a
//! fetch of the list failed, so the panel can say so.
//!
//! Owns the parsed copy; the list, its fetch and its live events are the
//! SDK's.

use std::collections::HashSet;
use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Message, Uuid};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::{CustomEmojis, MessageContent};

/// One pin, ready to draw.
pub(crate) struct Pin {
    pub(crate) id: Uuid,
    /// The SDK's payload: a new one means the pin changed.
    pub(crate) message: Arc<Message>,
    pub(crate) content: Arc<MessageContent>,
}

/// A timeline's pins.
#[derive(Default)]
pub(crate) struct Pins {
    /// Newest pin first; none while not known.
    list: Option<Vec<Pin>>,
    ids: HashSet<Uuid>,
    /// The last fetch of the list failed.
    pub(crate) failed: bool,
    /// The custom emoji revision the pins were parsed against.
    emojis: u64,
}

impl Pins {
    /// Takes the SDK's pins (none while not known). A pin whose payload is
    /// the one `items` holds shares the timeline's parse; one whose payload
    /// is unchanged keeps its own.
    pub(crate) fn set(
        &mut self,
        pins: Option<Vector<Arc<Message>>>,
        items: &Vector<Arc<TimelineItem>>,
        emojis: &CustomEmojis,
    ) {
        // Another emoji catalog parses every pin again; the timeline's items already follow it.
        if self.emojis != emojis.revision() {
            self.list = None;
            self.emojis = emojis.revision();
        }
        let Some(pins) = pins else {
            self.list = None;
            self.ids.clear();
            return;
        };
        self.failed = false;
        let old = self.list.take().unwrap_or_default();
        let list: Vec<Pin> = pins
            .iter()
            .filter_map(|message| {
                let id = message
                    .id
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok())?;
                let content = old
                    .iter()
                    .find(|pin| Arc::ptr_eq(&pin.message, message))
                    .map(|pin| Arc::clone(&pin.content))
                    .or_else(|| held(items, message))
                    .unwrap_or_else(|| {
                        Arc::new(emojis.parse(message.content.as_deref().unwrap_or_default()))
                    });
                Some(Pin {
                    id,
                    message: Arc::clone(message),
                    content,
                })
            })
            .collect();
        self.ids = list.iter().map(|pin| pin.id).collect();
        self.list = Some(list);
    }

    /// Parses the pins again against `emojis`, a new catalog.
    pub(crate) fn resolve(&mut self, items: &Vector<Arc<TimelineItem>>, emojis: &CustomEmojis) {
        let messages = self
            .list
            .as_ref()
            .map(|list| list.iter().map(|pin| Arc::clone(&pin.message)).collect());
        if messages.is_some() {
            self.set(messages, items, emojis);
        }
    }

    /// The pins, newest first; none while not known.
    pub(crate) fn list(&self) -> Option<&[Pin]> {
        self.list.as_deref()
    }

    /// Pin `id`, when pinned.
    pub(crate) fn get(&self, id: Uuid) -> Option<&Pin> {
        self.list.as_ref()?.iter().find(|pin| pin.id == id)
    }

    /// Whether message `id` is pinned; none while the pins are not known.
    pub(crate) fn pinned(&self, id: Uuid) -> Option<bool> {
        self.list.as_ref().map(|_| self.ids.contains(&id))
    }
}

/// The timeline's parse of `message`, when it holds that same payload.
fn held(items: &Vector<Arc<TimelineItem>>, message: &Arc<Message>) -> Option<Arc<MessageContent>> {
    items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            message: held,
            content,
            ..
        } if Arc::ptr_eq(held, message) => Some(Arc::clone(content)),
        _ => None,
    })
}

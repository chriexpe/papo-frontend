//! Which audio (`audio_store`'s key): a received attachment, or one of your unsent message's,
//! read from its staged file until the server has it. Once the message is sent its audio goes
//! on under the server's attachment (`AudioStore::sent`), so a player keeps playing.

use papo_sdk::{AccountId, Operation, OperationKind, OperationState, Uuid};
use papo_sdk_ui::AttachmentKind;

use crate::media_store::{MediaKey, attachment_id};

/// One audio as one account sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AudioKey {
    /// An attachment the server has.
    Received(MediaKey),
    /// Attachment `index` of your unsent message `local`.
    Unsent {
        account: AccountId,
        local: Uuid,
        index: usize,
    },
}

impl AudioKey {
    /// Whether it is a file of your unsent message `local`.
    pub(crate) fn is_unsent_of(self, local: Uuid) -> bool {
        matches!(self, AudioKey::Unsent { local: unsent, .. } if unsent == local)
    }

    /// Attachment `index` of `account`'s message `operation`: the server's attachment once it
    /// was sent, else the unsent file.
    pub(crate) fn of_operation(account: AccountId, operation: &Operation, index: usize) -> Self {
        match sent_attachment(operation, index) {
            Some(attachment) => AudioKey::Received(MediaKey::attachment(account, attachment)),
            None => AudioKey::Unsent {
                account,
                local: operation.id,
                index,
            },
        }
    }
}

/// Attachment `index`'s ID on the server, once `operation` (a message) was sent: the server
/// lists the message's files in its order, and it is audio there too.
fn sent_attachment(operation: &Operation, index: usize) -> Option<Uuid> {
    if operation.state != OperationState::Sent {
        return None;
    }
    let OperationKind::Message(message) = operation.kind.as_ref() else {
        return None;
    };
    let sent = operation.message.as_deref()?.attachments.as_deref()?;
    if sent.len() != message.attachments.len() {
        return None;
    }
    let attachment = sent.get(index)?;
    if AttachmentKind::of(attachment) != AttachmentKind::Audio {
        return None;
    }
    attachment_id(attachment)
}

//! "Quit while uploading?" (`quit_ask`), headless: closing or quitting
//! asks while a message's files are preparing or uploading, never while
//! nothing uploads, and Cancel keeps the app.

use std::sync::Arc;

use papo_sdk::{
    Operation, OperationKind, OperationState, OutgoingAttachment, OutgoingMessage,
    OutgoingSnapshot, Uuid,
};
use rune::{Key, KeyEvent, Mods, Runtime};

use super::composer::{shown, writing};
use super::{FRAME_S, Shell, tree};
use crate::quit_ask::{Leaving, QUIT_ASK_KEY, uploading};

/// A queue holding one message with a file, in `state`.
fn queue_with(state: OperationState) -> OutgoingSnapshot {
    let message = OutgoingMessage {
        attachments: vec![OutgoingAttachment::from_bytes("dump.bin", vec![0; 4])],
        ..OutgoingMessage::default()
    };
    OutgoingSnapshot {
        account: None,
        version: 1,
        operations: vec![Arc::new(Operation {
            id: Uuid::from_u128(7),
            channel_id: Uuid::from_u128(1),
            kind: Arc::new(OperationKind::Message(message)),
            state,
            attempts: 1,
            depends_on: None,
            failure: None,
            message_id: None,
            history_projected: false,
            message: None,
            created_at_us: 0,
        })],
        upload_progress: None,
        fault: None,
    }
}

/// #general, its account's queue `queue`.
fn with_queue(queue: OutgoingSnapshot) -> Runtime<Shell> {
    let mut rt = writing("");
    rt.update(move |shell, _tx| {
        let account = shown(shell).account;
        shell.activity.entry(account).or_default().outgoing = Some(Arc::new(queue));
    });
    rt.frame(FRAME_S);
    rt
}

fn ctrl_q(rt: &mut Runtime<Shell>) {
    rt.key(KeyEvent {
        key: Key::Command('q'),
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
}

#[test]
fn only_preparing_or_uploading_files_count() {
    assert!(uploading(&queue_with(OperationState::Sending)));
    assert!(uploading(&queue_with(OperationState::Preparing)));
    // Queued survives a relaunch and is sent then; Failed waits for you.
    assert!(!uploading(&queue_with(OperationState::Queued)));
    assert!(!uploading(&queue_with(OperationState::Failed)));
}

#[test]
fn quitting_while_uploading_asks_first_and_cancel_stays() {
    let mut rt = with_queue(queue_with(OperationState::Sending));
    rt.update(|shell, tx| shell.leave_or_ask(Leaving::Quit, tx));
    rt.frame(FRAME_S);
    assert_eq!(rt.state.leaving, Some(Leaving::Quit));
    assert!(tree(&rt).contains("Quit while uploading?"), "{}", tree(&rt));
    rt.update(|shell, _tx| shell.stay());
    rt.frame(FRAME_S);
    assert_eq!(rt.state.leaving, None);
    assert!(!tree(&rt).contains(QUIT_ASK_KEY));
}

#[test]
fn ctrl_q_reaches_the_question() {
    let mut rt = with_queue(queue_with(OperationState::Sending));
    ctrl_q(&mut rt);
    assert_eq!(rt.state.leaving, Some(Leaving::Quit));
}

#[test]
fn the_close_button_is_held_only_while_uploading() {
    let mut busy = with_queue(queue_with(OperationState::Preparing));
    let mut held = false;
    busy.update(|shell, tx| held = shell.close_requested(tx));
    assert!(held);
    assert_eq!(busy.state.leaving, Some(Leaving::Close));
    let mut idle = with_queue(queue_with(OperationState::Queued));
    let mut held = true;
    idle.update(|shell, tx| held = shell.close_requested(tx));
    assert!(!held);
    assert_eq!(idle.state.leaving, None);
}

#[test]
fn the_question_leaves_as_asked_once_the_uploads_end() {
    let mut rt = with_queue(queue_with(OperationState::Sending));
    ctrl_q(&mut rt);
    assert_eq!(rt.state.leaving, Some(Leaving::Quit));
    let sent = queue_with(OperationState::Sent);
    rt.update(move |shell, tx| {
        let account = shown(shell).account;
        shell.activity.entry(account).or_default().outgoing = Some(Arc::new(sent));
        assert!(shell.uploads_ended(tx));
    });
    rt.frame(FRAME_S);
    assert_eq!(rt.state.leaving, None);
    assert_eq!(rt.shell_request, Some(rune::Shell::Quit));
}

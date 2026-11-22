//! Integration tests covering Amendment 2 (`.part` atomic publication, error/revocation zero
//! partial leak, off-Tokio file I/O, >4 MiB inline cap) and outgoing staged upload / `Unknown` retry.

mod ffi_fixture;

use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use ffi_fixture::FfiTestHarness;
use papo_ffi::{
    FfiAccountId, FfiMediaError, FfiMediaSource, FfiOperationKindInput, FfiOperationState,
    FfiOutgoingAttachmentInput, FfiOutgoingError, FfiOutgoingListener, FfiOutgoingMessageInput,
    FfiOutgoingSnapshot, FfiRetryDecision,
};
use papo_test_server::{Config, EncodedMedia};

struct OutListener(Arc<Mutex<mpsc::Sender<FfiOutgoingSnapshot>>>);

impl FfiOutgoingListener for OutListener {
    fn on_snapshot(&self, s: FfiOutgoingSnapshot) {
        let _ = self.0.lock().unwrap().send(s);
    }
    fn on_update(&self, s: FfiOutgoingSnapshot) {
        let _ = self.0.lock().unwrap().send(s);
    }
    fn on_closed(&self) {}
}

fn list_part_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("part"))
                .then_some(name)
        })
        .collect()
}

fn assert_no_part_files(dir: &Path) {
    assert_eq!(list_part_files(dir), Vec::<String>::new());
}

fn install_media(h: &FfiTestHarness, user_id: &str, id: &str, bytes: Vec<u8>) {
    h.on_server(h.server().set_encoded_media(EncodedMedia {
        path: format!("/attachments/{id}"),
        bytes,
        users: std::iter::once(user_id.to_owned()).collect(),
        etag: None,
        status: 200,
        content_range: None,
    }))
    .unwrap();
}

fn verify_revoked_export(h: &FfiTestHarness, account: FfiAccountId, out_dir: &Path) {
    let att_slow = "71000000-0000-4000-8000-000000000003".to_owned();
    let revoked_dest = out_dir.join("revoked.bin");
    install_media(h, &account.user_id, &att_slow, vec![0xEE_u8; 4096]);
    let mut rest_hold = h
        .on_server(
            h.server()
                .hold_next_rest("GET", &format!("/attachments/{att_slow}")),
        )
        .unwrap();

    let client = Arc::clone(h.client());
    let acct_dl = account.clone();
    let revoked_str = revoked_dest.to_str().unwrap().to_owned();
    let dl_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(client.fetch_media_to_file(
            acct_dl,
            FfiMediaSource::Attachment {
                attachment_id: att_slow,
            },
            true,
            revoked_str,
        ))
    });

    h.on_server(rest_hold.wait_started()).unwrap();
    let auth = h.client().account_auth(account).unwrap();
    h.block_on(auth.logout()).unwrap();
    rest_hold.release();

    let dl_err = dl_thread.join().unwrap().unwrap_err();
    assert!(matches!(
        dl_err,
        FfiMediaError::Session { .. } | FfiMediaError::Unavailable
    ));
    assert!(!revoked_dest.exists());
    assert_no_part_files(out_dir);
}

#[test]
fn amendment2_media_part_atomic_publish_cleanup_and_inline_limit() {
    let h = FfiTestHarness::open(Config::default(), None);
    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "dana");

    let att_small = "71000000-0000-4000-8000-000000000001".to_owned();
    let payload_64k = vec![0xAB_u8; 64 * 1024];
    install_media(&h, &account.user_id, &att_small, payload_64k.clone());

    let att_big = "71000000-0000-4000-8000-000000000002".to_owned();
    let payload_5m = vec![0xCD_u8; 5 * 1024 * 1024];
    install_media(&h, &account.user_id, &att_big, payload_5m.clone());

    let out_dir = tempfile::tempdir().unwrap();
    let small_dest = out_dir.path().join("small.bin");
    let big_dest = out_dir.path().join("big.bin");
    let missing_dest = out_dir.path().join("missing.bin");

    let res = h
        .block_on(h.client().fetch_media_to_file(
            account.clone(),
            FfiMediaSource::Attachment {
                attachment_id: att_small.clone(),
            },
            false,
            small_dest.to_str().unwrap().into(),
        ))
        .unwrap();
    assert_eq!(res.size_bytes, payload_64k.len() as u64);
    assert_eq!(fs::read(&small_dest).unwrap(), payload_64k);
    assert_no_part_files(out_dir.path());

    let cached = h
        .block_on(h.client().cached_media_bytes(
            account.clone(),
            FfiMediaSource::Attachment {
                attachment_id: att_small,
            },
        ))
        .unwrap();
    assert_eq!(cached.bytes, payload_64k);

    let err = h
        .block_on(h.client().fetch_media_bytes(
            account.clone(),
            FfiMediaSource::Attachment {
                attachment_id: att_big.clone(),
            },
            false,
        ))
        .unwrap_err();
    assert_eq!(err, FfiMediaError::ExceedsInlineLimit);

    let big_res = h
        .block_on(h.client().fetch_media_to_file(
            account.clone(),
            FfiMediaSource::Attachment {
                attachment_id: att_big,
            },
            false,
            big_dest.to_str().unwrap().into(),
        ))
        .unwrap();
    assert_eq!(big_res.size_bytes, payload_5m.len() as u64);
    assert_eq!(fs::read(&big_dest).unwrap().len(), payload_5m.len());
    assert_no_part_files(out_dir.path());

    let fail = h
        .block_on(h.client().fetch_media_to_file(
            account.clone(),
            FfiMediaSource::Attachment {
                attachment_id: "71000000-0000-4000-8000-000000000009".into(),
            },
            false,
            missing_dest.to_str().unwrap().into(),
        ))
        .unwrap_err();
    assert!(!matches!(fail, FfiMediaError::ExceedsInlineLimit));
    assert!(!missing_dest.exists());
    assert_no_part_files(out_dir.path());

    verify_revoked_export(&h, account, out_dir.path());
    assert!(h.shutdown(1_000).reads_drained);
}

fn wait_op_state(
    rx: &mpsc::Receiver<FfiOutgoingSnapshot>,
    local_id: &str,
    expected: FfiOperationState,
) -> bool {
    while let Ok(snap) = rx.recv_timeout(Duration::from_secs(5)) {
        if snap
            .operations
            .iter()
            .any(|op| op.id == local_id && op.state == expected)
        {
            return true;
        }
    }
    false
}

fn assert_unknown_retry_duplicate_guard(
    h: &mut FfiTestHarness,
    account: FfiAccountId,
    ch_id: String,
) {
    assert!(
        h.client
            .take()
            .unwrap()
            .shutdown(500)
            .unwrap()
            .reads_drained
    );

    let server_guard = h.server.take().unwrap();
    let server_rt = h.server_rt.take().unwrap();
    let msg_hold = server_rt
        .block_on(server_guard.hold_next_rest("POST", "/messages"))
        .unwrap();

    let mut cfg = ffi_fixture::client_config(h.profile.path().to_str().unwrap());
    cfg.request_timeout_ms = 150;
    let store = Arc::clone(&h.store);
    let client2 = papo_ffi::FfiClient::open_with_media_engine(cfg, store, None).unwrap();
    let (tx2, rx2) = mpsc::channel();
    let sub2 = h
        .call_rt
        .block_on(client2.subscribe_outgoing(
            account.clone(),
            Arc::new(OutListener(Arc::new(Mutex::new(tx2)))),
        ))
        .unwrap();
    ffi_fixture::wait_account_online(&client2, account.clone());
    let local2 = "40000000-0000-4000-8000-000000000002".to_owned();
    h.call_rt
        .block_on(client2.enqueue_operation(
            account.clone(),
            ch_id,
            local2.clone(),
            FfiOperationKindInput::Message {
                message: FfiOutgoingMessageInput {
                    content: "ambiguous".into(),
                    reply_to: None,
                    embeds: Vec::new(),
                    attachments: Vec::new(),
                    gif: None,
                },
            },
            2000,
        ))
        .unwrap();

    assert!(wait_op_state(&rx2, &local2, FfiOperationState::Unknown));
    msg_hold.release();

    let err = h
        .call_rt
        .block_on(client2.retry_outgoing(
            account.clone(),
            local2.clone(),
            FfiRetryDecision::DefiniteFailure,
        ))
        .unwrap_err();
    assert_eq!(err, FfiOutgoingError::DuplicateRisk);

    h.call_rt
        .block_on(client2.retry_outgoing(
            account,
            local2,
            FfiRetryDecision::AcceptPossibleDuplicate,
        ))
        .unwrap();

    sub2.dispose();
    assert!(client2.shutdown(500).unwrap().reads_drained);
    server_rt.block_on(server_guard.shutdown()).unwrap();
}

#[test]
fn staged_attachment_and_unknown_retry_duplicate_guard() {
    let mut h = FfiTestHarness::open(Config::default(), None);
    let ch = h.on_server(h.server().create_channel("outbox")).unwrap();
    let ch_id = ch.id.unwrap();
    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "erin");

    let att_dir = tempfile::tempdir().unwrap();
    let att_path = att_dir.path().join("note.bin");
    fs::write(&att_path, b"attachment-payload").unwrap();

    let (sent_tx, sent_rx) = mpsc::channel::<FfiOutgoingSnapshot>();
    let sub = h
        .block_on(h.client().subscribe_outgoing(
            account.clone(),
            Arc::new(OutListener(Arc::new(Mutex::new(sent_tx)))),
        ))
        .unwrap();
    let staged = h
        .block_on(
            h.client()
                .stage_attachment_file(account.clone(), att_path.to_str().unwrap().into()),
        )
        .unwrap();
    assert_eq!(staged.size_bytes(), 18);

    let local1 = "40000000-0000-4000-8000-000000000001".to_owned();
    h.block_on(h.client().enqueue_operation(
        account.clone(),
        ch_id.clone(),
        local1.clone(),
        FfiOperationKindInput::Message {
            message: FfiOutgoingMessageInput {
                content: "hello".into(),
                reply_to: None,
                embeds: Vec::new(),
                attachments: vec![FfiOutgoingAttachmentInput::Staged {
                    filename: "note.bin".into(),
                    staged,
                }],
                gif: None,
            },
        },
        1000,
    ))
    .unwrap();
    assert!(wait_op_state(&sent_rx, &local1, FfiOperationState::Sent));
    sub.dispose();
    assert_unknown_retry_duplicate_guard(&mut h, account, ch_id);
}

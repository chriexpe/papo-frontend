//! Deterministic chunk-reader integrity and cancellation-termination tests; no sockets or clocks.

use super::*;
use crate::{AccountId, DataRoot, OutgoingSnapshot, ServerAddress, Uuid, observable::StateCell};
use std::{io::Write, task::Waker};

fn reader_for(contents: &[u8], expected_size: u64, expected_digest: [u8; 32]) -> ReaderState {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(contents).unwrap();
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(0)).unwrap();
    ReaderState {
        file,
        expected_digest,
        expected_size,
        bytes_read: 0,
        hasher: Sha256::new(),
    }
}

#[test]
fn chunk_reader_detects_truncation_growth_and_digest_mismatch() {
    let valid = b"hello-stream-world";
    let digest: [u8; 32] = Sha256::digest(valid).into();

    // Valid payload succeeds on final chunk.
    let step = read_next_chunk(reader_for(valid, valid.len() as u64, digest)).unwrap();
    assert!(matches!(step, ReadStep::LastChunk(bytes) if bytes.as_ref() == valid));

    // Zero-byte file succeeds with Eof.
    let empty_digest: [u8; 32] = Sha256::digest(b"").into();
    let step = read_next_chunk(reader_for(b"", 0, empty_digest)).unwrap();
    assert!(matches!(step, ReadStep::Eof));

    // Truncated file returns UnexpectedEof once EOF arrives before expected_size.
    let truncated = reader_for(&valid[..5], valid.len() as u64, digest);
    let first = read_next_chunk(truncated).unwrap();
    let ReadStep::Chunk(_, next_state) = first else {
        panic!("expected partial chunk before EOF");
    };
    let eof_err = read_next_chunk(next_state).err().unwrap();
    assert_eq!(eof_err.kind(), ErrorKind::UnexpectedEof);

    // File that grew past expected_size returns InvalidData.
    let mut grown = valid.to_vec();
    grown.push(0xff);
    let err = read_next_chunk(reader_for(&grown, valid.len() as u64, digest))
        .err()
        .unwrap();
    assert_eq!(err.kind(), ErrorKind::InvalidData);

    // Tampered bytes with same length return InvalidData before emitting final chunk.
    let mut tampered = valid.to_vec();
    tampered[0] ^= 0xff;
    let err = read_next_chunk(reader_for(&tampered, valid.len() as u64, digest))
        .err()
        .unwrap();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[test]
fn cancelled_stream_emits_terminal_error_once_then_terminates_with_none() {
    let payload = b"cancelled-payload";
    let digest: [u8; 32] = Sha256::digest(payload).into();
    let dir = tempfile::tempdir().unwrap();
    let lease = DataRoot::new(dir.path()).acquire().unwrap();
    let account = AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(1),
    );
    let cell = StateCell::new(Arc::new(OutgoingSnapshot {
        account: Some(account),
        version: 1,
        operations: Vec::new(),
        upload_progress: None,
        fault: None,
    }));

    for cancel_root in [true, false] {
        let root_cancel = CancellationToken::new();
        let epoch_cancel = CancellationToken::new();
        if cancel_root {
            root_cancel.cancel();
        } else {
            epoch_cancel.cancel();
        }
        let (progress, _guard) = ProgressReporter::start(
            account,
            Uuid::from_u128(2),
            1,
            payload.len() as u64,
            root_cancel.clone(),
            Arc::clone(&cell),
        );
        let mut stream = StagedFileStream {
            slot: ActiveUploadSlot::new(reader_for(payload, payload.len() as u64, digest)),
            state: StreamState::Idle,
            root_cancel,
            epoch_cancel,
            tasks: TaskTracker::new(),
            lease: Arc::clone(&lease),
            progress,
        };
        let mut cx = Context::from_waker(Waker::noop());

        // First poll returns the terminal Interrupted error once and closes the file slot.
        let first = Pin::new(&mut stream).poll_next(&mut cx);
        match first {
            Poll::Ready(Some(Err(err))) => assert_eq!(err.kind(), ErrorKind::Interrupted),
            other => panic!("expected terminal Interrupted error, got {other:?}"),
        }
        assert!(stream.slot.take().is_none());

        // Subsequent polls on the already-cancelled stream must cleanly return Ready(None).
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(None)
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(None)
        ));
    }
}

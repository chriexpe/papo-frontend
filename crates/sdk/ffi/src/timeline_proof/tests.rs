//! Checks worker completion and translation errors; Android owns main-thread application.

use std::sync::{Arc, Mutex};

use super::*;

#[test]
fn callbacks_arrive_in_order_on_one_worker_before_returning() {
    #[derive(Default)]
    struct Received {
        snapshot: Vec<ProofItem>,
        versions: Vec<u64>,
        threads: Vec<thread::ThreadId>,
    }
    struct Listener(Arc<Mutex<Received>>);
    impl ProofListener for Listener {
        fn on_snapshot(&self, version: u64, items: Vec<ProofItem>) {
            let mut received = self.0.lock().unwrap();
            received.snapshot = items;
            received.versions.push(version);
            received.threads.push(thread::current().id());
        }
        fn on_batch(&self, batch: ProofBatch) {
            let mut received = self.0.lock().unwrap();
            received.versions.push(batch.version);
            received.threads.push(thread::current().id());
        }
    }
    let received = Arc::new(Mutex::new(Received::default()));
    run_timeline_proof(Box::new(Listener(Arc::clone(&received)))).unwrap();
    let received = received.lock().unwrap();
    assert_eq!(received.snapshot, vec![item(30, 0), item(40, 0)]);
    assert_eq!(received.versions, (40..=47).collect::<Vec<_>>());
    assert!(received.threads.iter().all(|id| *id == received.threads[0]));
    assert_ne!(received.threads[0], thread::current().id());
}

#[test]
fn a_worker_panic_is_returned_instead_of_silently_passing() {
    assert_eq!(
        run_worker(|| panic!("intentional proof failure")),
        Err(ProofError::WorkerPanicked)
    );
}

#[test]
fn a_listener_panic_fails_the_proof_even_after_snapshot_delivery() {
    struct Listener;
    impl ProofListener for Listener {
        fn on_snapshot(&self, _: u64, _: Vec<ProofItem>) {}
        fn on_batch(&self, _: ProofBatch) {
            panic!("intentional listener failure");
        }
    }
    assert_eq!(
        run_timeline_proof(Box::new(Listener)),
        Err(ProofError::WorkerPanicked)
    );
}

#[test]
fn indices_up_to_kotlins_signed_maximum_translate_exactly() {
    assert_eq!(list_index(0), Ok(0));
    assert_eq!(list_index(2_147_483_647), Ok(2_147_483_647));
}

#[test]
fn overflowing_set_and_insert_indices_return_typed_errors() {
    for diff in [
        VectorDiff::Set {
            index: 2_147_483_648,
            value: item(1, 0),
        },
        VectorDiff::Insert {
            index: usize::MAX,
            value: item(1, 0),
        },
    ] {
        assert_eq!(map_diff(diff), Err(ProofError::IndexOutOfRange));
    }
}

#[test]
fn operations_outside_the_proof_return_a_typed_error() {
    assert_eq!(
        map_diff(VectorDiff::Clear),
        Err(ProofError::UnsupportedDiff)
    );
}

#[test]
fn every_fixture_diff_translates_without_changing_order_or_values() {
    let batches = fixture_batches()
        .into_iter()
        .map(|(version, diffs)| ProofBatch {
            version,
            diffs: diffs
                .into_iter()
                .map(|diff| map_diff(diff).unwrap())
                .collect(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        batches[0].diffs,
        vec![ProofDiff::PushBack { value: item(50, 0) }]
    );
    assert_eq!(
        batches[1].diffs,
        vec![ProofDiff::Set {
            index: 0,
            value: item(30, 1)
        }]
    );
    assert_eq!(
        batches[2].diffs,
        vec![
            ProofDiff::Insert {
                index: 0,
                value: item(20, 0)
            },
            ProofDiff::Insert {
                index: 0,
                value: item(10, 0)
            },
        ]
    );
    assert_eq!(
        batches[5].diffs,
        vec![ProofDiff::Reset {
            values: vec![item(20, 0), item(30, 2), item(40, 0), item(50, 0)],
        }]
    );
}

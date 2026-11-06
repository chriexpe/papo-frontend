//! Cache-page callbacks wake consumers without a blocking receipt waiter.

use super::*;

#[test]
fn page_callback_completes_on_the_cache_worker_without_polling_a_receiver() {
    let temp = TempDb::new("page-callback");
    let cache = open(&temp);
    cache.submit(
        "fixture",
        vec![CacheOp::ReplaceChannelSnapshot {
            channel_id: "channel".into(),
            messages: vec![message("message", "channel", "", 1)],
            cached_at: 1,
        }],
    );
    let (completed, receipt) = std::sync::mpsc::channel();
    let caller = std::thread::current().id();
    assert!(
        cache.load_channel_page_with_callback("fixture", "channel", None, move |result| {
            completed
                .send((std::thread::current().id(), result))
                .unwrap();
        })
    );
    let (worker, page) = receipt.recv().unwrap();
    assert_ne!(worker, caller);
    let page = page.unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].id, "message");
}

#[test]
fn disabled_cache_rejects_a_callback_without_waiting_or_invoking_it() {
    let cache = ClientDb::open(None);
    let (completed, receipt) = std::sync::mpsc::channel();
    assert!(
        !cache.load_channel_page_with_callback("fixture", "channel", None, move |_| {
            completed.send(()).unwrap();
        })
    );
    assert!(matches!(
        receipt.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
}

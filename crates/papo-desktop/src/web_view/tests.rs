//! The WebView against the system's WPE WebKit: ignored, since CI has none. Run with
//! `cargo test -p papo-desktop web_view -- --ignored` on a machine that has it.

use std::sync::mpsc;
use std::time::Duration;

use super::*;

fn start(root: &std::path::Path) -> (WebViews, mpsc::Receiver<WebEvent>) {
    let (sender, events) = mpsc::channel();
    let sender = std::sync::Mutex::new(sender);
    let sink: Events = Arc::new(move |event| {
        let _ = sender.lock().map(|sender| sender.send(event));
    });
    (WebViews::start(root.to_owned(), None, sink), events)
}

fn wait_for(events: &mpsc::Receiver<WebEvent>, wanted: &WebEvent) -> Vec<WebEvent> {
    let mut seen = Vec::new();
    while let Ok(event) = events.recv_timeout(Duration::from_secs(20)) {
        let done = &event == wanted;
        seen.push(event);
        if done {
            return seen;
        }
    }
    panic!("{wanted:?} never came: {seen:?}");
}

#[test]
#[ignore = "needs WPE WebKit and a GPU"]
fn a_page_draws_its_first_frame_into_its_surface_at_the_boxs_pixels() {
    let root = tempfile::tempdir().unwrap();
    let (views, events) = start(root.path());
    wait_for(&events, &WebEvent::Ready);
    let surface = NativeSurface::new();
    views.open(
        1,
        PageSpec {
            address: "data:text/html,<body style='background:%23c00'>papo</body>".into(),
            keeps: Arc::new(|_| true),
            site: "test".into(),
            size_pt: [320.0, 180.0],
            scale: 2.0,
            surface: surface.clone(),
            muted: true,
        },
    );
    wait_for(&events, &WebEvent::Page(1, PageEvent::Painted));
    let frame = surface.latest().expect("a frame in the surface");
    assert_eq!(frame.size, [640, 360]);
    views.close(1);
    drop(views);
}

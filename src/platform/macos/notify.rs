//! macOS Notification Center adapter for the shared notification coordinator.
//! No Apple framework API is called on a background Rust thread.
#![cfg(target_os = "macos")]
use std::{ffi::{c_char, CString}, sync::mpsc};

unsafe extern "C" {
    fn papo_notify_initialize();
    fn papo_notify_show(title: *const c_char, body: *const c_char, tag: *const c_char);
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub summary: String,
    pub body: String,
    pub tag: Option<String>,
}

#[derive(Clone)]
pub struct Notifier {
    tx: mpsc::Sender<Notification>,
}

impl Notifier {
    pub fn spawn() -> Option<Self> {
        // Async bridge sends AppKit work to dispatch_get_main_queue.
        unsafe { papo_notify_initialize(); }
        let (tx, rx) = mpsc::channel::<Notification>();
        std::thread::Builder::new()
            .name("papo-macos-notify".into())
            .spawn(move || {
                while let Ok(notification) = rx.recv() {
                    let (Ok(title), Ok(body)) =
                        (CString::new(notification.summary), CString::new(notification.body))
                    else { continue; };
                    let tag = notification.tag.and_then(|s| CString::new(s).ok());
                    unsafe {
                        papo_notify_show(title.as_ptr(), body.as_ptr(),
                            tag.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()));
                    }
                }
            }).ok()?;
        Some(Self { tx })
    }

    pub fn show(&self, notification: Notification) {
        let _ = self.tx.send(notification);
    }
}

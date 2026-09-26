//! In-process exclusion between the normal Papo runtime and WorkManager.
//!
//! A headless worker may never run beside PapoApp. Foreground startup marks
//! itself as waiting before it waits for existing workers, which prevents a new
//! worker from slipping into the handoff window.

use std::collections::HashSet;
use std::sync::{Condvar, Mutex, OnceLock};

#[derive(Default)]
struct LeaseState {
    foreground: bool,
    foreground_waiting: bool,
    headless: HashSet<String>,
}

fn state() -> &'static (Mutex<LeaseState>, Condvar) {
    static STATE: OnceLock<(Mutex<LeaseState>, Condvar)> = OnceLock::new();
    STATE.get_or_init(|| (Mutex::new(LeaseState::default()), Condvar::new()))
}

pub struct ForegroundLease;

impl ForegroundLease {
    pub fn acquire() -> Self {
        let (lock, cv) = state();
        let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.foreground_waiting = true;
        while !guard.headless.is_empty() {
            guard = cv.wait(guard).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        guard.foreground_waiting = false;
        guard.foreground = true;
        Self
    }
}

impl Drop for ForegroundLease {
    fn drop(&mut self) {
        let (lock, cv) = state();
        let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.foreground = false;
        cv.notify_all();
    }
}

pub struct HeadlessLease {
    server_key: String,
}

pub fn try_acquire_headless(server_key: &str) -> Option<HeadlessLease> {
    let (lock, _) = state();
    let mut guard = lock.lock().ok()?;
    if guard.foreground || guard.foreground_waiting || guard.headless.contains(server_key) {
        return None;
    }
    guard.headless.insert(server_key.to_owned());
    Some(HeadlessLease {
        server_key: server_key.to_owned(),
    })
}

/// A foreground runtime is active or waiting for current workers to leave.
/// Headless JNI loops use this to cancel promptly during Activity startup.
pub fn foreground_requested() -> bool {
    let (lock, _) = state();
    lock.lock()
        .map(|guard| guard.foreground || guard.foreground_waiting)
        .unwrap_or(true)
}

impl Drop for HeadlessLease {
    fn drop(&mut self) {
        let (lock, cv) = state();
        if let Ok(mut guard) = lock.lock() {
            guard.headless.remove(&self.server_key);
            cv.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_is_exclusive_in_both_directions() {
        let foreground = ForegroundLease::acquire();
        assert!(try_acquire_headless("a").is_none());
        drop(foreground);

        let first = try_acquire_headless("server-b").expect("first worker");
        assert!(try_acquire_headless("server-b").is_none());
        drop(first);
        assert!(try_acquire_headless("server-b").is_some());
    }
}

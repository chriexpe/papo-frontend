//! Native macOS system appearance provider. Called on the egui/AppKit UI thread.
#![cfg(target_os = "macos")]

use egui::Color32;

unsafe extern "C" {
    fn papo_macos_system_appearance(red: *mut u8, green: *mut u8, blue: *mut u8) -> i32;
}

pub fn read() -> (Option<bool>, Option<Color32>) {
    let (mut r, mut g, mut b) = (0, 0, 0);
    // SAFETY: all pointers reference initialized stack bytes. The caller
    // runs on Papo's AppKit UI thread, including initial application setup.
    let dark = unsafe { papo_macos_system_appearance(&mut r, &mut g, &mut b) };
    if dark < 0 {
        return (None, None);
    }
    (Some(dark == 1), Some(Color32::from_rgb(r, g, b)))
}

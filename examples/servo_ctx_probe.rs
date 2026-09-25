//! Diagnostic probe: does Servo's offscreen `SoftwareRenderingContext` survive
//! create -> make_current -> drop on this machine, with **no Servo engine and
//! no WebView**? If this panics, the crash is unrelated to ClientStorage.
//!
//!   RUST_BACKTRACE=full RUST_LOG=surfman=trace,servo=debug \
//!     cargo run --example servo_ctx_probe

fn main() {
    #[cfg(target_os = "linux")]
    {
        use servo::RenderingContext;
        let size = dpi::PhysicalSize {
            width: 640,
            height: 360,
        };
        println!("probe: SoftwareRenderingContext::new({size:?})");
        let ctx =
            servo::SoftwareRenderingContext::new(size).expect("SoftwareRenderingContext::new");
        println!("probe: make_current()");
        ctx.make_current().expect("make_current");
        println!("probe: drop()");
        drop(ctx);
        println!("probe: survived create -> make_current -> drop");
    }
    #[cfg(not(target_os = "linux"))]
    println!("probe: linux-only");
}

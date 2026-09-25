//! Diagnostic probe #3: the working engine path, but with the things the app
//! does that probe #2 did not — install the rustls provider, load a real embed,
//! and drive resize + hidpi + paint + readback like `present()` does.
//!
//!   RUST_BACKTRACE=full RUST_LOG=surfman=trace,servo=debug \
//!     cargo run --example servo_page_probe

#[cfg(target_os = "linux")]
fn main() {
    use std::rc::Rc;
    use euclid::Scale;
    use servo::{
        DeviceIndependentPixel, DeviceIntPoint, DeviceIntRect, DeviceIntSize, DevicePixel,
        EventLoopWaker, RenderingContext, ServoBuilder, WebViewBuilder, WebViewDelegate,
    };

    struct Waker;
    impl EventLoopWaker for Waker {
        fn clone_box(&self) -> Box<dyn EventLoopWaker> {
            Box::new(Waker)
        }
        fn wake(&self) {}
    }
    struct Delegate;
    impl WebViewDelegate for Delegate {}

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    println!("1. context 320x180");
    let context = Rc::new(
        servo::SoftwareRenderingContext::new(dpi::PhysicalSize {
            width: 320,
            height: 180,
        })
        .expect("context"),
    );

    println!("2. Servo");
    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(Waker))
        .build();

    println!("3. WebView youtube embed");
    let url = url::Url::parse("https://www.youtube.com/embed/dQw4w9WgXcQ").expect("url");
    let rendering: Rc<dyn RenderingContext> = context.clone();
    let webview = WebViewBuilder::new(&servo, rendering)
        .url(url)
        .delegate(Rc::new(Delegate))
        .build();

    for step in 0..4 {
        let width = 320 + step * 160;
        let height = 180 + step * 90;
        println!("4.{step}. resize {width}x{height} + hidpi 2.0 + spin");
        let size = dpi::PhysicalSize { width, height };
        context.resize(size);
        webview.resize(size);
        webview.set_hidpi_scale_factor(Scale::<f32, DeviceIndependentPixel, DevicePixel>::new(2.0));
        for _ in 0..30 {
            servo.spin_event_loop();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        println!("5.{step}. paint + read");
        webview.paint();
        let rect = DeviceIntRect::from_origin_and_size(
            DeviceIntPoint::new(0, 0),
            DeviceIntSize::new(width as i32, height as i32),
        );
        println!("6.{step}. image = {:?}", context.read_to_image(rect).map(|i| i.dimensions()));
    }

    println!("7. drop");
    drop(webview);
    drop(servo);
    drop(context);
    println!("probe: survived real page + resize + hidpi");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("probe: linux-only");
}

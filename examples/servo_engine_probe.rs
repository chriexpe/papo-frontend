//! Diagnostic probe #2: bisect the Servo offscreen path one step at a time, so
//! the exact transition that trips the surfman `EGLBackedContext::drop` assert
//! is obvious. Steps: context -> Servo -> about:blank WebView -> spin ->
//! paint -> read_to_image -> drop.
//!
//!   RUST_BACKTRACE=full RUST_LOG=surfman=trace,servo=debug \
//!     cargo run --example servo_engine_probe

#[cfg(target_os = "linux")]
fn main() {
    use std::rc::Rc;
    use servo::{
        DeviceIntPoint, DeviceIntRect, DeviceIntSize, EventLoopWaker, RenderingContext,
        ServoBuilder, WebViewBuilder, WebViewDelegate,
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

    let size = dpi::PhysicalSize {
        width: 640,
        height: 360,
    };
    println!("1. SoftwareRenderingContext::new");
    let context = Rc::new(servo::SoftwareRenderingContext::new(size).expect("context"));

    println!("2. ServoBuilder::build");
    let servo = ServoBuilder::default()
        .event_loop_waker(Box::new(Waker))
        .build();

    println!("3. about:blank WebView");
    let url = url::Url::parse("about:blank").expect("url");
    let rendering: Rc<dyn RenderingContext> = context.clone();
    let webview = WebViewBuilder::new(&servo, rendering)
        .url(url)
        .delegate(Rc::new(Delegate))
        .build();

    println!("4. spin_event_loop x10");
    for _ in 0..10 {
        servo.spin_event_loop();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    println!("5. paint");
    webview.paint();

    println!("6. read_to_image");
    let rect = DeviceIntRect::from_origin_and_size(
        DeviceIntPoint::new(0, 0),
        DeviceIntSize::new(640, 360),
    );
    let image = context.read_to_image(rect);
    println!("7. image = {:?}", image.map(|image| image.dimensions()));

    println!("8. drop webview");
    drop(webview);
    println!("9. drop servo");
    drop(servo);
    println!("10. drop context");
    drop(context);
    println!("probe: survived the whole engine path");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("probe: linux-only");
}

//! Memory probe: open/play/drop many players like scrolling through a chat full of videos.
use std::time::{Duration, Instant};
fn anon_mb() -> usize {
    let s = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap();
    s.lines().find(|l| l.starts_with("Anonymous:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0) / 1024
}
fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("path"));
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(40);
    let play_ms: u64 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(1500);
    let ctx = egui::Context::default();
    papo::media::player::init();
    println!("start anon={}MB", anon_mb());
    for i in 0..n {
        let mut p = papo::media::player::Player::open(&path, true, ctx.clone()).expect("open");
        p.play();
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(play_ms) {
            p.frame(&ctx);
            let _ = ctx.run_ui(Default::default(), |_| {}); // free dropped textures like a frame would
            std::thread::sleep(Duration::from_millis(16));
        }
        drop(p);
        std::thread::sleep(Duration::from_millis(300));
        if i % 5 == 4 { println!("after {:>3} players anon={}MB", i + 1, anon_mb()); }
    }
    std::thread::sleep(Duration::from_secs(3));
    println!("end anon={}MB", anon_mb());
}

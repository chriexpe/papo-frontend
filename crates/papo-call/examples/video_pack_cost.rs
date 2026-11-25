//! Isolates CPU pixel packing; it does not decode, submit or present media.

use papo_call::{
    ColourMatrix, ColourRange, ColourTransfer, CpuVideoFrame, VideoColour, VideoRotation,
};
use std::{error::Error, hint::black_box, time::Instant};

const WARM_BATCHES: usize = 20;
const MEASURED_BATCHES: usize = 120;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let profile = args
        .next()
        .ok_or("profile required: 360p, 720p, 1080p or four")?;
    let path = args.next().ok_or("pixel path required: nv12 or bgra")?;
    let (width_px, height_px, tiles) = match profile.as_str() {
        "360p" => (640, 360, 1),
        "720p" => (1280, 720, 1),
        "1080p" => (1920, 1080, 1),
        "four" => (1280, 720, 4),
        _ => return Err("unknown profile".into()),
    };
    let bgra = match path.as_str() {
        "bgra" => true,
        "nv12" => false,
        _ => return Err("unknown pixel path".into()),
    };
    let stride = usize::try_from(width_px)? + 32;
    let y_len = stride * usize::try_from(height_px)?;
    let colour = VideoColour {
        matrix: ColourMatrix::Bt709,
        range: ColourRange::Limited,
        transfer: ColourTransfer::Bt709,
    };
    let frames: Vec<_> = (0..tiles)
        .map(|tile| {
            let mut bytes = vec![128; y_len + y_len / 2];
            bytes[..y_len].fill(81 + tile);
            Ok::<_, std::num::TryFromIntError>(CpuVideoFrame {
                width_px,
                height_px,
                offsets: [0, y_len],
                strides: [i32::try_from(stride)?; 2],
                colour,
                pts_ns: None,
                bytes,
            })
        })
        .collect::<Result<_, _>>()?;
    let mut observations = [0_u128; MEASURED_BATCHES];
    for batch in 0..WARM_BATCHES + MEASURED_BATCHES {
        let start = Instant::now();
        for frame in &frames {
            if bgra {
                black_box(frame.pack_bgra(VideoRotation::Upright)?);
            } else {
                black_box(frame.pack_nv12()?);
            }
        }
        if batch >= WARM_BATCHES {
            observations[batch - WARM_BATCHES] = start.elapsed().as_micros();
        }
    }
    observations.sort_unstable();
    let at = |percent: usize| observations[(observations.len() * percent).div_ceil(100) - 1];
    println!(
        "{profile} {path} {tiles} tiles, {} packed frames: p50={}us p95={}us per batch",
        MEASURED_BATCHES * usize::from(tiles),
        at(50),
        at(95),
    );
    Ok(())
}

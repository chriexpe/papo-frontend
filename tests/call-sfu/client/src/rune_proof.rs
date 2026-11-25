//! Presents the backend fixture's decoded RTP in Rune; signaling stays in Endpoint.

use super::{Endpoint, Result, backend_room, drive};
use papo_call::{
    ColourMatrix, ColourRange, ColourTransfer, CpuVideoDestination, CpuVideoFrame,
    CpuVideoFrameLease, VideoColour, VideoRotation,
};
use papo_sdk::VideoKind;
use rune::native_surface::{ByteFrame, ColorInfo, Format, Matrix, Transfer};
use rune::{App, AppSpec, Cx, El, NativeSurface, SurfaceFit, Tx, col, el, text};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{oneshot, watch};

const SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const SAMPLE_INTERVAL: Duration = Duration::from_millis(83);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn run(address: String, password: String) -> Result {
    let frame_limit = std::env::var("PAPO_CALL_PROOF_RUNE_FRAMES")
        .ok()
        .map(|value| value.parse::<std::num::NonZeroU64>())
        .transpose()?;
    let (destination, input) = watch::channel(None);
    let (stop, stopping) = oneshot::channel();
    let runtime = tokio::runtime::Handle::current();
    let worker = std::thread::Builder::new()
        .name("call-proof-rtp".into())
        .spawn(move || {
            runtime
                .block_on(receive(&address, &password, destination, stopping))
                .map_err(|_| "Rune proof signaling or media session failed")
        })?;
    let lifecycle = Arc::new(Mutex::new(Lifecycle {
        worker: Some((stop, worker)),
        frame_limit,
        ..Lifecycle::default()
    }));
    let state_lifecycle = Arc::clone(&lifecycle);
    let window = App::new(AppSpec {
        poll: Some(State::poll),
        ..AppSpec::new("RTP video proof", (640.0, 480.0), State::default, view)
    })
    .init(move || State {
        input: Some(input),
        lifecycle: state_lifecycle,
        frame_limit,
        ..State::default()
    })
    .on_close(State::close)
    .on_quit(State::close)
    .try_run();
    let result = lifecycle
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .finish();
    window?;
    result?;
    Ok(())
}

type Worker = std::thread::JoinHandle<std::result::Result<(), &'static str>>;

#[derive(Default)]
struct Lifecycle {
    worker: Option<(oneshot::Sender<()>, Worker)>,
    frame_limit: Option<std::num::NonZeroU64>,
    completed_sources: u64,
    error: Option<&'static str>,
}

impl Lifecycle {
    fn finish(&mut self) -> std::result::Result<(), &'static str> {
        if let Some((stop, worker)) = self.worker.take() {
            let _ = stop.send(());
            self.error = match worker.join() {
                Ok(result) => result.err(),
                Err(_) => Some("Rune proof RTP worker panicked"),
            };
            let count = self.completed_sources;
            if self.error.is_none() && self.frame_limit.is_some_and(|limit| count < limit.get()) {
                self.error = Some("Rune proof stopped before the requested number of draws");
            }
            if let Some(error) = self.error {
                eprintln!("{error}");
            } else {
                println!(
                    "Rune proof: {count} distinct decoded RTP uploads completed GPU drawing; call drained"
                );
            }
        }
        self.error.map_or(Ok(()), Err)
    }
}

async fn receive(
    address: &str,
    password: &str,
    destination: watch::Sender<Option<Arc<CpuVideoDestination>>>,
    mut stopping: oneshot::Receiver<()>,
) -> Result {
    let setup = async {
        let (a, b, owner, channel) = backend_room(address, password).await?;
        let mut publisher = Endpoint::join(&a, &channel, 520.0, "red").await?;
        let mut receiver = Endpoint::join(&b, &channel, 760.0, "green").await?;
        while !publisher.connected || !receiver.connected {
            drive(&mut publisher, &mut receiver).await?;
        }
        publisher.camera_on().await?;
        while publisher.answer_count < 2 {
            drive(&mut publisher, &mut receiver).await?;
        }
        Ok::<_, Box<dyn std::error::Error>>((publisher, receiver, owner))
    };
    let (mut publisher, mut receiver, owner) = tokio::select! {
        _ = &mut stopping => return Ok(()),
        result = tokio::time::timeout(SETUP_TIMEOUT, setup) => result??,
    };
    let key = (owner, VideoKind::Camera);
    let stream = async {
        let mut retry_at = tokio::time::Instant::now();
        loop {
            let now = tokio::time::Instant::now();
            if now >= retry_at && !receiver.publisher_tracks.contains_key(&key) {
                receiver.interest(owner, key.1, true).await?;
                if let Some(target) = receiver.video.get(&key) {
                    destination.send_if_modified(|current| {
                        if current.is_some() {
                            return false;
                        }
                        *current = Some(target.clone());
                        true
                    });
                }
                retry_at = now + Duration::from_millis(500);
            }
            drive(&mut publisher, &mut receiver).await?;
        }
    };
    let result: Result = tokio::select! {
        _ = stopping => Ok(()),
        result = stream => result,
    };
    destination.send_replace(None);
    let (publisher_left, receiver_left) = tokio::join!(
        tokio::time::timeout(DRAIN_TIMEOUT, publisher.leave()),
        tokio::time::timeout(DRAIN_TIMEOUT, receiver.leave()),
    );
    result?;
    publisher_left??;
    receiver_left??;
    Ok(())
}

struct Upload {
    surface: NativeSurface,
    lease: CpuVideoFrameLease,
    visible: bool,
}

#[derive(Default)]
struct State {
    input: Option<watch::Receiver<Option<Arc<CpuVideoDestination>>>>,
    upload: Option<Upload>,
    lifecycle: Arc<Mutex<Lifecycle>>,
    completed_sources: u64,
    renderer_failed: bool,
    worker_stopped: bool,
    frame_limit: Option<std::num::NonZeroU64>,
}

impl State {
    fn close(&mut self, _: &mut Tx) {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lifecycle.completed_sources = self.completed_sources;
        let _ = lifecycle.finish();
    }

    fn poll(&mut self, _: f64, tx: &mut Tx) -> bool {
        let Some(input) = &self.input else {
            return false;
        };
        if self.worker_stopped || self.renderer_failed {
            return false;
        }
        if input.has_changed().is_err() {
            self.worker_stopped = true;
            if let Some(upload) = self.upload.take() {
                upload.surface.clear();
            }
            return true;
        }
        tx.wake_after(SAMPLE_INTERVAL);
        let destination = input.borrow().clone();
        let mut changed = false;
        if let Some(upload) = &mut self.upload {
            if upload.surface.last_error().is_some() {
                upload.surface.clear();
                self.upload = None;
                self.renderer_failed = true;
                return true;
            }
            let current = destination.is_some() && upload.lease.is_current();
            if !current && upload.visible {
                upload.surface.clear();
                upload.visible = false;
                changed = true;
            }
            // This key only ever sees one source frame. Counts observe its draws without
            // enabling the optional GPU timing queries. Repeated resizes can redraw it.
            let (submitted, completed) = upload.surface.counts();
            if submitted != completed || (submitted == 0 && current) {
                return changed;
            }
            if current && completed != 0 {
                self.completed_sources = self.completed_sources.saturating_add(1);
                if self.frame_limit.map(std::num::NonZeroU64::get) == Some(self.completed_sources) {
                    self.upload = None;
                    tx.close_window();
                    return true;
                }
            }
            self.upload = None; // release the destination's single checkout before taking again
            changed = true;
        }
        let Some(lease) = destination.and_then(|target| target.take()) else {
            return changed;
        };
        let epoch = lease.format_epoch();
        if !lease.is_current() {
            return changed;
        }
        let Some(bytes) = pack(&lease) else {
            return changed;
        };
        if !lease.is_current() {
            return changed;
        }
        // A new key makes Rune evict the previous key and its prepared bind sets (R-006).
        let surface = NativeSurface::new();
        if surface.push_bytes(bytes, epoch).is_err() {
            return changed;
        }
        self.upload = Some(Upload {
            surface,
            lease,
            visible: true,
        });
        true
    }
}

fn view(state: &State, cx: &mut Cx<State>) -> El {
    let theme = rune::tok::theme(cx.system_dark());
    let picture = el().w_full().grow(1.0).min_h(0.0).center();
    let picture = match &state.upload {
        Some(upload) if upload.visible && upload.lease.is_current() => {
            picture.native_surface(&upload.surface, SurfaceFit::Fit)
        }
        _ => picture.child(text("P").color(theme.text)),
    };
    let status = if state.renderer_failed {
        "Rune could not render the received video"
    } else if state.worker_stopped {
        "Fixture media stopped"
    } else {
        "Synthetic fixture publisher (decoded RTP)"
    };
    col()
        .w_full()
        .h_full()
        .fill(theme.bg)
        .child(text(status).color(theme.text))
        .child(picture)
}

pub(super) fn pack(frame: &CpuVideoFrame) -> Option<ByteFrame> {
    let transfer = match frame.colour.transfer {
        ColourTransfer::Bt709 => Transfer::Bt709,
        ColourTransfer::Srgb => Transfer::Srgb,
        ColourTransfer::Other => return None,
    };
    if frame.width_px.is_multiple_of(2)
        && frame.height_px.is_multiple_of(2)
        && let Some((format, color)) = nv12_color(frame.colour, transfer)
        && let Ok(nv12) = frame.pack_nv12()
    {
        return Some(ByteFrame {
            width: nv12.size_px[0],
            height: nv12.size_px[1],
            format,
            color,
            planes: nv12.planes,
        });
    }
    let bgra = frame.pack_bgra(VideoRotation::Upright).ok()?;
    Some(ByteFrame {
        width: bgra.size_px[0],
        height: bgra.size_px[1],
        format: Format::Bgra,
        color: ColorInfo {
            transfer,
            ..ColorInfo::default()
        },
        planes: [bgra.bytes, Vec::new()],
    })
}

fn nv12_color(colour: VideoColour, transfer: Transfer) -> Option<(Format, ColorInfo)> {
    let matrix = match colour.matrix {
        ColourMatrix::Bt601 => Matrix::Bt601,
        ColourMatrix::Bt709 => Matrix::Bt709,
        ColourMatrix::Bt2020 | ColourMatrix::Unknown => return None,
    };
    let format = match colour.range {
        ColourRange::Full => Format::Nv12Full,
        ColourRange::Limited => Format::Nv12Video,
        ColourRange::Unknown => return None,
    };
    Some((format, ColorInfo { matrix, transfer }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width_px: u32) -> CpuVideoFrame {
        CpuVideoFrame {
            width_px,
            height_px: 2,
            offsets: [1, 8],
            strides: [3, 2],
            colour: VideoColour {
                matrix: ColourMatrix::Bt601,
                range: ColourRange::Limited,
                transfer: ColourTransfer::Bt709,
            },
            pts_ns: None,
            bytes: vec![0, 16, 16, 0, 16, 16, 0, 0, 128, 128],
        }
    }

    #[test]
    fn even_frame_preserves_nv12_range_matrix_and_packed_planes() {
        let image = pack(&frame(2)).unwrap();
        assert_eq!(image.format, Format::Nv12Video);
        assert_eq!(image.color.matrix, Matrix::Bt601);
        assert_eq!(image.color.transfer, Transfer::Bt709);
        assert_eq!(image.planes, [vec![16; 4], vec![128; 2]]);
    }

    #[test]
    fn odd_frame_uses_checked_bgra_instead_of_invalid_nv12() {
        let image = pack(&frame(1)).unwrap();
        assert_eq!(image.format, Format::Bgra);
        assert_eq!(image.color.transfer, Transfer::Bt709);
        assert_eq!([image.width, image.height], [1, 2]);
        assert_eq!(image.planes, [vec![0, 0, 0, 255, 0, 0, 0, 255], vec![]]);
    }
}

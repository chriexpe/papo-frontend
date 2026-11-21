//! Narrow JNI surface for the debug-only Android video proof; never holds locks across FFI.

mod fixture;
mod plugins;
mod session;
mod surface;
mod verify;
mod writer;

use jni::{
    JNIEnv,
    objects::{JClass, JObject},
    sys::{jint, jlong, jstring},
};
use papo_call::CpuVideoFrameLease;
use session::{FAILED, REFUSED_CAPACITY, SessionHandle, SessionRegistry, WAITING};
use std::{
    ptr::null_mut,
    sync::{Arc, Mutex, OnceLock},
    thread,
};
use surface::Window;
use tokio::sync::oneshot;
use verify::ExpectedColor;
use writer::{TileWriter, lock};

static REGISTRY: Mutex<SessionRegistry<CpuVideoFrameLease>> = Mutex::new(SessionRegistry::new());
static PLUGINS_READY: OnceLock<bool> = OnceLock::new();

struct AttachGuard {
    committed: bool,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        if !self.committed {
            lock(&REGISTRY).abort_attach();
        }
    }
}

struct MediaDrainGuard(Arc<SessionHandle<CpuVideoFrameLease>>);

impl Drop for MediaDrainGuard {
    fn drop(&mut self) {
        self.0.mark_media_drained();
    }
}

fn begin_attach() -> Result<(u64, AttachGuard), jlong> {
    lock(&REGISTRY)
        .begin_attach()
        .map(|generation| (generation, AttachGuard { committed: false }))
        .map_err(|_| jlong::from(REFUSED_CAPACITY))
}

fn attach_windows(
    mut guard: AttachGuard,
    generation: u64,
    windows: Vec<(Window, ExpectedColor)>,
) -> jlong {
    let mut writers = Vec::with_capacity(windows.len());
    let mut writer_pairs = Vec::with_capacity(windows.len());
    for (window, expected) in windows {
        let writer = Arc::new(TileWriter::new(expected));
        writers.push(Arc::clone(&writer));
        writer_pairs.push((writer, window));
    }

    let (cancel_tx, cancel_rx) = oneshot::channel();
    let session = Arc::new(SessionHandle::new(generation, writers.clone(), cancel_tx));

    let mut pairs = writer_pairs.into_iter().enumerate();
    let mut spawned_writers = Vec::with_capacity(writers.len());
    while let Some((idx, (writer, window))) = pairs.next() {
        let name = format!("video-writer-{generation}-{idx}");
        let worker_writer = Arc::clone(&writer);
        let worker = thread::Builder::new()
            .name(name)
            .spawn(move || worker_writer.run_with_window(window));
        match worker {
            Ok(_) => spawned_writers.push(writer),
            Err(_) => {
                session.cancel();
                drop(pairs);
                for active in &spawned_writers {
                    active.wait_drained();
                }
                session.mark_media_drained();
                drop(guard);
                return jlong::from(FAILED);
            }
        }
    }

    let media_session = Arc::clone(&session);
    let media_writers = writers;
    let media_worker = thread::Builder::new()
        .name(format!("video-media-{generation}"))
        .spawn(move || {
            let _media_drain = MediaDrainGuard(Arc::clone(&media_session));
            let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            else {
                for writer in &media_writers {
                    writer.fail();
                }
                return;
            };
            let outcome = runtime.block_on(async {
                if !*PLUGINS_READY.get_or_init(plugins::initialize) {
                    return Err("gstreamer plugins unavailable".into());
                }
                fixture::run(&media_writers, cancel_rx).await
            });
            if outcome.is_err() && !media_session.is_canceled() {
                for writer in &media_writers {
                    writer.fail();
                }
            }
            // Drop Tokio runtime naturally after explicit media drain; no shutdown_timeout.
            drop(runtime);
        });
    if media_worker.is_err() {
        session.cancel();
        session.wait_writers_drained();
        session.mark_media_drained();
        drop(guard);
        return jlong::from(FAILED);
    }

    lock(&REGISTRY).finish_attach(Arc::clone(&session));
    guard.committed = true;
    i64::try_from(generation).unwrap_or(i64::from(FAILED))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeAttach(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    surface: JObject<'_>,
) -> jlong {
    let (generation, guard) = match begin_attach() {
        Ok(reserved) => reserved,
        Err(refused) => return refused,
    };
    let Some(window) = Window::from_surface(&env, &surface) else {
        return jlong::from(FAILED);
    };
    attach_windows(guard, generation, vec![(window, ExpectedColor::Red)])
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeAttachDual(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    surface0: JObject<'_>,
    surface1: JObject<'_>,
) -> jlong {
    let (generation, guard) = match begin_attach() {
        Ok(reserved) => reserved,
        Err(refused) => return refused,
    };
    let (Some(w0), Some(w1)) = (
        Window::from_surface(&env, &surface0),
        Window::from_surface(&env, &surface1),
    ) else {
        return jlong::from(FAILED);
    };
    attach_windows(
        guard,
        generation,
        vec![(w0, ExpectedColor::Red), (w1, ExpectedColor::Green)],
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeDetachGeneration(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    generation: jlong,
) {
    if let Ok(gen_id) = u64::try_from(generation) {
        lock(&REGISTRY).detach(Some(gen_id));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeAwaitSurfaceRelease(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    generation: jlong,
) {
    let Ok(gen_id) = u64::try_from(generation) else {
        return;
    };
    let session = lock(&REGISTRY).find_generation(gen_id);
    if let Some(session) = session {
        session.wait_writers_drained();
        lock(&REGISTRY).reap();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeState(
    _: JNIEnv<'_>,
    _: JClass<'_>,
) -> jint {
    lock(&REGISTRY).overall_status()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeTileState(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    index: jint,
) -> jint {
    let Ok(idx) = usize::try_from(index) else {
        return WAITING;
    };
    lock(&REGISTRY).tile_status(idx)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeArmStall(
    _: JNIEnv<'_>,
    _: JClass<'_>,
) -> jint {
    i32::from(lock(&REGISTRY).arm_stall_live())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeReleaseStall(
    _: JNIEnv<'_>,
    _: JClass<'_>,
) {
    lock(&REGISTRY).release_all_stalls();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_videoproof_ProofActivity_nativeTelemetryJson(
    env: JNIEnv<'_>,
    _: JClass<'_>,
) -> jstring {
    let snap = papo_call::telemetry::take_snapshot();
    let json =
        lock(&REGISTRY).telemetry_json(snap.fence_accepted_packets, snap.fence_rejected_packets);
    env.new_string(json)
        .map_or(null_mut(), jni::objects::JString::into_raw)
}

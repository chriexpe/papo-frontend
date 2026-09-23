//! Integração da call com o ciclo de vida do Android.
//!
//! O transporte continua em Rust/GStreamer. O Android só mantém a execução
//! autorizada em segundo plano (foreground service), apresenta os controles
//! nativos e informa quando a Activity virou Picture-in-Picture.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};

use crate::api::net::{Command as NetCommand, NetSender};
use crate::voice::{Command as CallCommand, Frame};

use std::ffi::c_void;

#[repr(C)]
struct ANativeWindow {
    _private: [u8; 0],
}

#[repr(C)]
struct ANativeWindowBuffer {
    width: i32,
    height: i32,
    stride: i32,
    format: i32,
    bits: *mut c_void,
    reserved: [u32; 6],
}

unsafe extern "C" {
    fn ANativeWindow_fromSurface(
        env: *mut jni::sys::JNIEnv,
        surface: jni::sys::jobject,
    ) -> *mut ANativeWindow;
    fn ANativeWindow_release(window: *mut ANativeWindow);
    fn ANativeWindow_setBuffersGeometry(
        window: *mut ANativeWindow,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;
    fn ANativeWindow_lock(
        window: *mut ANativeWindow,
        out_buffer: *mut ANativeWindowBuffer,
        dirty_bounds: *mut c_void,
    ) -> i32;
    fn ANativeWindow_unlockAndPost(window: *mut ANativeWindow) -> i32;
}

const WINDOW_FORMAT_RGBA_8888: i32 = 1;
const PIP_WIDTH: usize = 640;
const PIP_HEIGHT: usize = 360;

#[derive(Debug, Clone, Copy)]
pub enum UiAction {
    Muted(bool),
    Camera(bool),
    Hangup,
}

struct Control {
    commands: mpsc::Sender<CallCommand>,
    net: NetSender,
    channel_id: String,
    muted: bool,
    camera: bool,
}

static CONTROL: Mutex<Option<Control>> = Mutex::new(None);
static ACTIONS: Mutex<Vec<UiAction>> = Mutex::new(Vec::new());
static IN_PIP: AtomicBool = AtomicBool::new(false);
static FOREGROUND: AtomicBool = AtomicBool::new(true);
static LAST_SERVICE_STATE: Mutex<Option<String>> = Mutex::new(None);
static LAST_PRESENTATION: Mutex<Option<String>> = Mutex::new(None);
/// Endereço do ANativeWindow do SurfaceView de PiP. Guardado como usize
/// para que o mutex seja Send/Sync; o ponteiro só é usado enquanto o mutex
/// está tomado, então surfaceDestroyed não consegue liberá-lo no meio de um
/// quadro.
static PIP_WINDOW: Mutex<usize> = Mutex::new(0);

pub fn bind(
    commands: mpsc::Sender<CallCommand>,
    net: NetSender,
    channel_id: String,
    muted: bool,
    camera: bool,
) {
    if let Ok(mut slot) = CONTROL.lock() {
        *slot = Some(Control {
            commands,
            net,
            channel_id,
            muted,
            camera,
        });
    }
}

pub fn clear() {
    if let Ok(mut slot) = CONTROL.lock() {
        *slot = None;
    }
}

pub fn start_service(title: &str) {
    let _ = super::jvm::call_activity(
        "startCallService",
        "(Ljava/lang/String;)V",
        Some(title),
    );
}

pub fn stop_service() {
    let _ = super::jvm::call_activity("stopCallService", "()V", None);
    if let Ok(mut state) = LAST_SERVICE_STATE.lock() {
        *state = None;
    }
    if let Ok(mut state) = LAST_PRESENTATION.lock() {
        *state = None;
    }
    clear();
}

/// Mantém a notificação nativa em sincronia com a pastilha da call sem fazer
/// JNI em todo frame. O nome do speaker vai em JSON para não depender de
/// separadores que também podem existir num nome de usuário.
pub fn sync_service(
    muted: bool,
    camera: bool,
    members: usize,
    speaker: Option<&str>,
) {
    let state = serde_json::json!({
        "muted": muted,
        "camera": camera,
        "members": members,
        "speaker": speaker.unwrap_or(""),
    })
    .to_string();

    let changed = LAST_SERVICE_STATE
        .lock()
        .map(|mut last| {
            if last.as_deref() == Some(state.as_str()) {
                false
            } else {
                *last = Some(state.clone());
                true
            }
        })
        .unwrap_or(true);

    if changed {
        let _ = super::jvm::call_activity(
            "updateCallService",
            "(Ljava/lang/String;)V",
            Some(&state),
        );
    }

    if let Ok(mut slot) = CONTROL.lock()
        && let Some(control) = slot.as_mut()
    {
        control.muted = muted;
        control.camera = camera;
    }
}

pub fn set_presentation(active: bool, video: bool, muted: bool, camera: bool) {
    let state = format!(
        "{};muted={};camera={}",
        if !active {
            "off"
        } else if video {
            "video"
        } else {
            "voice"
        },
        if muted { 1 } else { 0 },
        if camera { 1 } else { 0 },
    );

    let changed = LAST_PRESENTATION
        .lock()
        .map(|mut last| {
            if last.as_deref() == Some(state.as_str()) {
                false
            } else {
                *last = Some(state.clone());
                true
            }
        })
        .unwrap_or(true);

    if changed {
        let _ = super::jvm::call_activity(
            "setCallPresentation",
            "(Ljava/lang/String;)V",
            Some(&state),
        );
    }
}

pub fn is_in_pip() -> bool {
    IN_PIP.load(Ordering::Relaxed)
}

pub fn is_foreground() -> bool {
    FOREGROUND.load(Ordering::Relaxed)
}

/// Nome mostrado sobre o vídeo nativo de PiP. Pode ser chamado da thread de
/// rede: a Activity publica a mudança na UI thread.
pub fn set_pip_speaker(name: &str) {
    let _ = super::jvm::call_activity(
        "setPipSpeaker",
        "(Ljava/lang/String;)V",
        Some(name),
    );
}

/// Fecha imediatamente a apresentação Android da call. Diferente da Store,
/// isto não espera um frame egui — é usado quando o servidor encerra a call
/// enquanto a Activity está pausada dentro do PiP.
pub fn call_ended_from_network() {
    let _ = super::jvm::call_activity("closeCallPictureInPicture", "()V", None);
    stop_service();
}

/// Escreve o quadro decodificado diretamente no SurfaceView do PiP.
///
/// O buffer do Surface é 16:9 porque essa é a janela que pedimos ao Android.
/// A imagem inteira é encaixada dentro dele, sem crop: câmera vertical ganha
/// barras laterais; vídeo 16:9 ocupa tudo.
pub fn present_pip_frame(frame: &Frame) {
    if !is_in_pip() || frame.width == 0 || frame.height == 0 {
        return;
    }

    let Ok(window_guard) = PIP_WINDOW.lock() else {
        return;
    };
    let window = *window_guard as *mut ANativeWindow;
    if window.is_null() {
        return;
    }

    unsafe {
        if ANativeWindow_setBuffersGeometry(
            window,
            PIP_WIDTH as i32,
            PIP_HEIGHT as i32,
            WINDOW_FORMAT_RGBA_8888,
        ) != 0
        {
            return;
        }

        let mut buffer = std::mem::zeroed::<ANativeWindowBuffer>();
        if ANativeWindow_lock(window, &mut buffer, std::ptr::null_mut()) != 0
            || buffer.bits.is_null()
            || buffer.stride <= 0
        {
            return;
        }

        let dst = buffer.bits.cast::<u8>();
        let stride = buffer.stride as usize;

        // Fundo preto, inclusive nas barras de letterbox.
        for y in 0..PIP_HEIGHT {
            std::ptr::write_bytes(dst.add(y * stride * 4), 0, PIP_WIDTH * 4);
        }

        let source_ratio = frame.width as f32 / frame.height as f32;
        let target_ratio = PIP_WIDTH as f32 / PIP_HEIGHT as f32;
        let (draw_w, draw_h) = if source_ratio > target_ratio {
            (PIP_WIDTH, ((PIP_WIDTH as f32) / source_ratio).round() as usize)
        } else {
            (((PIP_HEIGHT as f32) * source_ratio).round() as usize, PIP_HEIGHT)
        };
        let draw_w = draw_w.max(1).min(PIP_WIDTH);
        let draw_h = draw_h.max(1).min(PIP_HEIGHT);
        let x0 = (PIP_WIDTH - draw_w) / 2;
        let y0 = (PIP_HEIGHT - draw_h) / 2;

        // Nearest-neighbour é suficiente para uma janela PiP pequena e evita
        // criar outro buffer/Bitmap em cada quadro.
        for y in 0..draw_h {
            let sy = y * frame.height / draw_h;
            for x in 0..draw_w {
                let sx = x * frame.width / draw_w;
                let [r, g, b, a] = frame.pixels[sy * frame.width + sx].to_array();
                let at = dst.add(((y0 + y) * stride + x0 + x) * 4);
                *at = r;
                *at.add(1) = g;
                *at.add(2) = b;
                *at.add(3) = a;
            }
        }

        let _ = ANativeWindow_unlockAndPost(window);
    }
}

/// SurfaceView criado pela Activity para o vídeo de PiP.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeSetPipSurface(
    env: jni::JNIEnv,
    _class: jni::objects::JClass,
    surface: jni::objects::JObject,
) {
    let Ok(mut guard) = PIP_WINDOW.lock() else {
        return;
    };

    let old = *guard as *mut ANativeWindow;
    if !old.is_null() {
        unsafe { ANativeWindow_release(old) };
        *guard = 0;
    }

    if surface.is_null() {
        return;
    }

    let window = unsafe {
        ANativeWindow_fromSurface(env.get_native_interface(), surface.as_raw())
    };
    *guard = window as usize;
}

pub fn take_actions() -> Vec<UiAction> {
    ACTIONS
        .lock()
        .map(|mut actions| std::mem::take(&mut *actions))
        .unwrap_or_default()
}

fn push_action(action: UiAction) {
    if let Ok(mut actions) = ACTIONS.lock() {
        actions.push(action);
    }
    super::wake::request();
}

/// Ação de uma notificação de call. Mute e hangup são executados aqui mesmo:
/// não dependem de um novo frame da Activity.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_CallService_nativeCallAction(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    action: jni::objects::JString,
) {
    let Ok(action) = env.get_string(&action) else {
        return;
    };
    let action: String = action.into();

    let Ok(mut slot) = CONTROL.lock() else {
        return;
    };

    if action == "hangup" {
        if let Some(control) = slot.as_mut() {
            control.net.send(NetCommand::VoiceSignal(format!(
                r#"{{"type":"voice_leave","channel_id":"{}"}}"#,
                control.channel_id
            )));
            let _ = control.commands.send(CallCommand::Stop);
        }
        // Mesmo antes de VoiceReady/Call::start, a Store já está em Joining.
        // A UI acordada abaixo cancela essa tentativa e manda voice_leave se
        // o join tiver alcançado o socket no meio da corrida.
        drop(slot);
        push_action(UiAction::Hangup);
        return;
    }

    if let Some(value) = action.strip_prefix("mute=") {
        let muted = value == "1";
        if let Some(control) = slot.as_mut() {
            control.muted = muted;
            let _ = control.commands.send(CallCommand::Muted(muted));
        }
        // Antes de a thread da call existir, guardar a escolha na Store basta:
        // pump_call a reaplica assim que o servidor confirmar a entrada.
        drop(slot);
        push_action(UiAction::Muted(muted));
        return;
    }

    if let Some(value) = action.strip_prefix("camera=") {
        let camera = value == "1";
        if let Some(control) = slot.as_mut() {
            control.camera = camera;
            let _ = control.commands.send(CallCommand::Camera(camera));
        }
        drop(slot);
        push_action(UiAction::Camera(camera));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeSetPictureInPictureMode(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    enabled: bool,
) {
    IN_PIP.store(enabled, Ordering::Relaxed);
    super::wake::request();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeLifecycleChanged(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    foreground: bool,
) {
    FOREGROUND.store(foreground, Ordering::Relaxed);
    super::wake::request();
}

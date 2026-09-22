//! Integração da call com o ciclo de vida do Android.
//!
//! O transporte continua em Rust/GStreamer. O Android só mantém a execução
//! autorizada em segundo plano (foreground service), apresenta os controles
//! nativos e informa quando a Activity virou Picture-in-Picture.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};

use crate::api::net::{Command as NetCommand, NetSender};
use crate::voice::Command as CallCommand;

#[derive(Debug, Clone, Copy)]
pub enum UiAction {
    Muted(bool),
    Hangup,
}

struct Control {
    commands: mpsc::Sender<CallCommand>,
    net: NetSender,
    channel_id: String,
    muted: bool,
}

static CONTROL: Mutex<Option<Control>> = Mutex::new(None);
static ACTIONS: Mutex<Vec<UiAction>> = Mutex::new(Vec::new());
static IN_PIP: AtomicBool = AtomicBool::new(false);
static FOREGROUND: AtomicBool = AtomicBool::new(true);

pub fn bind(
    commands: mpsc::Sender<CallCommand>,
    net: NetSender,
    channel_id: String,
    muted: bool,
) {
    if let Ok(mut slot) = CONTROL.lock() {
        *slot = Some(Control {
            commands,
            net,
            channel_id,
            muted,
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
    clear();
}

pub fn update_service(muted: bool, camera: bool) {
    let state = format!(
        "muted={};camera={}",
        if muted { 1 } else { 0 },
        if camera { 1 } else { 0 }
    );
    let _ = super::jvm::call_activity(
        "updateCallService",
        "(Ljava/lang/String;)V",
        Some(&state),
    );
    if let Ok(mut slot) = CONTROL.lock()
        && let Some(control) = slot.as_mut()
    {
        control.muted = muted;
    }
}

pub fn set_presentation(active: bool, video: bool) {
    let state = if !active {
        "off"
    } else if video {
        "video"
    } else {
        "voice"
    };
    let _ = super::jvm::call_activity(
        "setCallPresentation",
        "(Ljava/lang/String;)V",
        Some(state),
    );
}

pub fn is_in_pip() -> bool {
    IN_PIP.load(Ordering::Relaxed)
}

pub fn is_foreground() -> bool {
    FOREGROUND.load(Ordering::Relaxed)
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

//! A call no Android: por enquanto, nenhuma.
//!
//! O motor inteiro é `webrtcbin` do GStreamer, que no Android vem de um SDK
//! próprio — é o PR de mídia. Recusar aqui, no mesmo ponto em que a área de
//! trabalho recusa quando falta GStreamer, faz `Call::start` devolver `None`
//! e a interface já sabe o que fazer com isso.

use std::sync::{Arc, mpsc};

use super::ice::IceConfig;
use super::{Command, Shared};

pub(super) fn spawn(
    _channel_id: String,
    _ice: IceConfig,
    shared: Arc<Shared>,
    _commands: mpsc::Receiver<Command>,
    _inbox: mpsc::Sender<Command>,
    _signals: mpsc::Sender<String>,
    _repaint: egui::Context,
) -> Option<std::thread::JoinHandle<()>> {
    shared.fail("voz e vídeo ainda não existem no Android");
    None
}

/// O `papo voice-check` é da linha de comando, que não existe no Android.
pub fn check() {
    log::warn!("voz e vídeo ainda não existem no Android");
}

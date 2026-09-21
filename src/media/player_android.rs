//! O player no Android: por enquanto, nenhum.
//!
//! Tudo aqui depende do GStreamer, que no Android vem de um SDK próprio e
//! entra no PR de mídia. Até lá o substituto responde "não dá" a todo mundo,
//! em vez de o aplicativo não compilar: quem chama já sabia lidar com a
//! ausência — na área de trabalho o GStreamer também pode faltar.

use std::path::{Path, PathBuf};

/// Sempre falso: sem GStreamer não há o que iniciar.
pub fn init() -> bool {
    false
}

/// Nunca é construído — `open` não devolve nenhum. Os métodos existem só
/// para o resto da interface continuar compilando igual nos dois lados.
pub struct Player {
    /// Posição em segundos enquanto o usuário arrasta o cursor.
    pub scrubbing: Option<f64>,
    pub muted: bool,
}

impl Player {
    pub fn open(path: &Path, _video: bool, _repaint: egui::Context) -> Option<Self> {
        log::warn!("tocar {} ainda não existe no Android", path.display());
        None
    }

    pub fn play(&mut self) {}

    pub fn pause(&mut self) {}

    pub fn toggle(&mut self) {}

    pub fn is_playing(&self) -> bool {
        false
    }

    pub fn position(&self) -> f64 {
        0.0
    }

    pub fn duration(&self) -> f64 {
        0.0
    }

    pub fn seek(&mut self, _seconds: f64) {}

    pub fn set_muted(&mut self, _muted: bool) {}

    pub fn aspect(&self) -> f32 {
        16.0 / 9.0
    }

    pub fn error(&self) -> Option<String> {
        None
    }

    pub fn update(&mut self) {}

    pub fn frame(&mut self, _ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        None
    }
}

/// A forma de onda sai de um pipeline do GStreamer; sem ele, nada a desenhar.
pub fn waveform(_path: &Path) -> Option<Vec<f32>> {
    None
}

/// Gravar áudio também é GStreamer — e ainda depende da permissão de
/// microfone, que é assunto do PR de plataforma.
pub struct Recorder {
    _private: (),
}

impl Recorder {
    pub fn start(_dir: &Path) -> Option<Self> {
        log::warn!("gravar áudio ainda não existe no Android");
        None
    }

    pub fn elapsed(&self) -> f64 {
        0.0
    }

    pub fn finish(self) -> Option<PathBuf> {
        None
    }

    pub fn cancel(self) {}
}

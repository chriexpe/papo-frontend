//! Confere se o GStreamer do Android subiu.
//!
//! Roda uma vez, na abertura, e escreve o resultado no logcat. Não é teste
//! de unidade: o que se quer saber é se o `.so` gerado pelo CMake carregou,
//! se o registro estático tem os elementos que foram ligados nele e se um
//! pipeline anda até o fim num aparelho de verdade.
//!
//! O conjunto de plugins é decidido na ligação (ver
//! `android/gstreamer/CMakeLists.txt`): no Android não há
//! `gst-plugin-scanner` nem plugin carregado em tempo de execução. Por isso
//! a lista abaixo é a do que foi pedido lá — se algum sumir, foi a ligação
//! que mudou, não o aparelho.

use gstreamer as gst;
use gstreamer::prelude::*;

/// Elementos que os plugins ligados devem oferecer.
const EXPECTED: &[(&str, &str)] = &[
    ("fakesink", "o ralo dos testes"),
    ("queue", "fila entre partes do pipeline"),
    ("appsrc", "a ponte de entrada do Papo"),
    ("appsink", "a ponte de saída, que vira textura"),
    ("decodebin", "quem decide como decodificar um anexo"),
    ("videoconvert", "conversão de formato de vídeo"),
    ("audioconvert", "conversão de formato de áudio"),
    ("videotestsrc", "vídeo de teste"),
    ("audiotestsrc", "áudio de teste"),
    // Os anexos que o Papo tem de verdade: .mp4 com H.264/AAC e recados
    // de voz em Ogg/Opus.
    ("qtdemux", "abre os .mp4"),
    ("oggdemux", "abre os recados de voz"),
    ("h264parse", "o vídeo antes do decodificador"),
    ("aacparse", "o áudio do .mp4 antes do decodificador"),
    ("opusdec", "os recados de voz"),
    ("openslessink", "a saída de áudio do Android"),
];

/// Sobe o GStreamer e conta o que encontrou.
pub fn run() {
    if let Err(error) = gst::init() {
        log::error!("gstreamer: não iniciou: {error}");
        return;
    }
    log::info!("gstreamer: {}", gst::version_string());

    let registry = gst::Registry::get();
    log::info!(
        "gstreamer: {} plugin(s) no registro estático",
        registry.plugins().len()
    );

    let mut missing = 0;
    for (name, what) in EXPECTED {
        match gst::ElementFactory::find(name) {
            Some(_) => log::info!("  [ok]   {name:<14} {what}"),
            None => {
                missing += 1;
                log::error!("  [FALTA] {name:<14} {what}");
            }
        }
    }
    if missing > 0 {
        log::error!("gstreamer: {missing} elemento(s) faltando — confira o CMakeLists");
    }

    // Os decodificadores do aparelho têm nome próprio de cada aparelho
    // (`amcviddec-omxqcomvideodecoderavc` e afins), então não dá para
    // procurá-los pelo nome: conta-se quantos apareceram.
    let hardware = registry
        .features(gst::ElementFactory::static_type())
        .iter()
        .filter(|feature| feature.name().starts_with("amc"))
        .count();
    if hardware > 0 {
        log::info!("gstreamer: {hardware} decodificador(es) do próprio aparelho");
    } else {
        log::warn!("gstreamer: nenhum decodificador do aparelho — o vídeo vai depender de software");
    }

    probe("videotestsrc num-buffers=5 ! fakesink");
    probe("audiotestsrc num-buffers=5 ! fakesink");
}

/// Roda um pipeline até o fim e diz o que aconteceu.
fn probe(description: &str) {
    let pipeline = match gst::parse::launch(description) {
        Ok(pipeline) => pipeline,
        Err(error) => {
            log::error!("gstreamer: `{description}` não montou: {error}");
            return;
        }
    };
    if pipeline.set_state(gst::State::Playing).is_err() {
        log::error!("gstreamer: `{description}` não começou");
        let _ = pipeline.set_state(gst::State::Null);
        return;
    }

    let mut outcome = "não terminou em 5s";
    if let Some(bus) = pipeline.bus() {
        for message in bus.iter_timed(gst::ClockTime::from_seconds(5)) {
            match message.view() {
                gst::MessageView::Eos(_) => {
                    outcome = "correu até o fim";
                    break;
                }
                gst::MessageView::Error(error) => {
                    log::error!("gstreamer: `{description}`: {}", error.error());
                    outcome = "erro";
                    break;
                }
                _ => {}
            }
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    log::info!("gstreamer: `{description}` — {outcome}");
}

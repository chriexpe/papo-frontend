use gstreamer as gst;
use gstreamer::prelude::*;

pub fn initialize_platform() {
    #[cfg(target_os = "android")]
    demote_broken_decoders();
}

pub fn audio_sink_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "android")]
    { &["openslessink", "autoaudiosink"] }

    #[cfg(target_os = "linux")]
    { &["autoaudiosink", "pipewiresink", "pulsesink", "alsasink"] }

    #[cfg(target_os = "windows")]
    { &["autoaudiosink", "wasapi2sink", "wasapisink"] }

    #[cfg(target_os = "macos")]
    { &["autoaudiosink", "osxaudiosink"] }

    #[cfg(not(any(
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        target_os = "macos"
    )))]
    { &["autoaudiosink"] }
}

#[cfg(target_os = "android")]
fn demote_broken_decoders() {
    let registry = gst::Registry::get();
    let mut demoted = 0;
    for feature in registry.features(gst::ElementFactory::static_type()).iter() {
        let name = feature.name();
        let broken = name.starts_with("amcviddec-omx")
            || (name.starts_with("amcauddec-") && name.contains("opus"));
        if broken && feature.rank() > gst::Rank::MARGINAL {
            feature.set_rank(gst::Rank::NONE);
            demoted += 1;
        }
    }
    if demoted > 0 {
        log::info!("{demoted} decodificador(es) do aparelho despriorizado(s)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_sink_lists_are_not_linux_fallbacks() {
        let linux = ["autoaudiosink", "pipewiresink", "pulsesink", "alsasink"];
        let windows = ["autoaudiosink", "wasapi2sink", "wasapisink"];
        let macos = ["autoaudiosink", "osxaudiosink"];
        let android = ["openslessink", "autoaudiosink"];

        assert!(linux.contains(&"pipewiresink"));
        assert!(windows.contains(&"wasapi2sink"));
        assert!(!windows.contains(&"pipewiresink"));
        assert!(macos.contains(&"osxaudiosink"));
        assert!(!macos.contains(&"pulsesink"));
        assert_eq!(android[0], "openslessink");
    }
}

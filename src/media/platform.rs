#[cfg(target_os = "android")]
use gstreamer as gst;
#[cfg(target_os = "android")]
use gstreamer::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GstPlatform {
    Android,
    Linux,
    Windows,
    Macos,
    Other,
}

fn current_platform() -> GstPlatform {
    #[cfg(target_os = "android")]
    { return GstPlatform::Android; }
    #[cfg(target_os = "linux")]
    { return GstPlatform::Linux; }
    #[cfg(target_os = "windows")]
    { return GstPlatform::Windows; }
    #[cfg(target_os = "macos")]
    { return GstPlatform::Macos; }
    #[allow(unreachable_code)]
    GstPlatform::Other
}

fn audio_sink_candidates_for(platform: GstPlatform) -> &'static [&'static str] {
    match platform {
        GstPlatform::Android => &["openslessink", "autoaudiosink"],
        GstPlatform::Linux => &["autoaudiosink", "pipewiresink", "pulsesink", "alsasink"],
        GstPlatform::Windows => &["autoaudiosink", "wasapi2sink", "wasapisink"],
        GstPlatform::Macos => &["autoaudiosink", "osxaudiosink"],
        GstPlatform::Other => &["autoaudiosink"],
    }
}

pub fn initialize_platform() {
    #[cfg(target_os = "android")]
    demote_broken_decoders();
}

pub fn audio_sink_candidates() -> &'static [&'static str] {
    audio_sink_candidates_for(current_platform())
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
    fn linux_has_native_candidates() {
        let sinks = audio_sink_candidates_for(GstPlatform::Linux);
        assert!(sinks.contains(&"pipewiresink"));
        assert!(sinks.contains(&"pulsesink"));
        assert!(sinks.contains(&"alsasink"));
    }

    #[test]
    fn windows_uses_wasapi_not_linux_sinks() {
        let sinks = audio_sink_candidates_for(GstPlatform::Windows);
        assert!(sinks.contains(&"wasapi2sink"));
        assert!(sinks.contains(&"wasapisink"));
        assert!(!sinks.contains(&"pipewiresink"));
        assert!(!sinks.contains(&"pulsesink"));
    }

    #[test]
    fn macos_uses_osx_audio_not_linux_sinks() {
        let sinks = audio_sink_candidates_for(GstPlatform::Macos);
        assert!(sinks.contains(&"osxaudiosink"));
        assert!(!sinks.contains(&"pipewiresink"));
        assert!(!sinks.contains(&"alsasink"));
    }

    #[test]
    fn android_preserves_tested_fallback() {
        assert_eq!(
            audio_sink_candidates_for(GstPlatform::Android),
            &["openslessink", "autoaudiosink"]
        );
    }
}

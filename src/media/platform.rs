#[cfg(target_os = "android")]
use gstreamer as gst;
#[cfg(target_os = "android")]
use gstreamer::prelude::*;

// All variants are intentionally represented on every target so host CI can
// test the complete policy matrix without cross-compiling each OS.
#[allow(dead_code)]
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


fn audio_source_candidates_for(platform: GstPlatform) -> &'static [&'static str] {
    match platform {
        GstPlatform::Android => &["openslessrc"],
        GstPlatform::Linux => &["pipewiresrc", "pulsesrc", "alsasrc"],
        GstPlatform::Windows => &["wasapi2src", "wasapisrc"],
        GstPlatform::Macos => &["osxaudiosrc"],
        GstPlatform::Other => &["autoaudiosrc"],
    }
}

fn camera_source_candidates_for(platform: GstPlatform) -> &'static [&'static str] {
    match platform {
        GstPlatform::Android => &["ahcsrc"],
        GstPlatform::Linux => &["v4l2src"],
        GstPlatform::Windows => &["mfvideosrc", "ksvideosrc"],
        GstPlatform::Macos => &["avfvideosrc"],
        GstPlatform::Other => &["autovideosrc"],
    }
}

pub fn prepare_environment() {
    #[cfg(target_os = "windows")]
    {
        let Ok(exe) = std::env::current_exe() else { return };
        let Some(root) = exe.parent() else { return };

        let plugins = root.join("gstreamer-1.0");
        if plugins.is_dir() {
            // SAFETY: called once before gst::init() starts GStreamer threads.
            unsafe {
                std::env::set_var("GST_PLUGIN_PATH_1_0", &plugins);
                std::env::set_var("GST_PLUGIN_SYSTEM_PATH_1_0", &plugins);
            }
        }

        let scanner = root.join("gst-plugin-scanner.exe");
        if scanner.is_file() {
            unsafe { std::env::set_var("GST_PLUGIN_SCANNER", &scanner) };
        }

        let gio = root.join("gio-modules");
        if gio.is_dir() {
            unsafe { std::env::set_var("GIO_EXTRA_MODULES", &gio) };
        }
    }
}

pub fn initialize_platform() {
    #[cfg(target_os = "android")]
    demote_broken_decoders();
}

pub fn audio_sink_candidates() -> &'static [&'static str] {
    audio_sink_candidates_for(current_platform())
}

pub fn audio_source_candidates() -> &'static [&'static str] {
    audio_source_candidates_for(current_platform())
}

pub fn camera_source_candidates() -> &'static [&'static str] {
    camera_source_candidates_for(current_platform())
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

        assert_eq!(
            audio_source_candidates_for(GstPlatform::Windows),
            &["wasapi2src", "wasapisrc"]
        );
        assert_eq!(
            camera_source_candidates_for(GstPlatform::Windows),
            &["mfvideosrc", "ksvideosrc"]
        );
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

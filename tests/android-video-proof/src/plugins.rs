//! Registers only the static plugins needed by the VP8 fixture and the native call engine.

#[cfg(target_os = "android")]
macro_rules! plugins {
    ($($register:ident),+ $(,)?) => {
        unsafe extern "C" { $(fn $register();)+ }

        pub fn initialize() -> bool {
            if gstreamer::init().is_err() {
                return false;
            }
            // SAFETY: The matching GStreamer 1.0 Android archives are linked into this library.
            unsafe { $($register();)+ }
            ["webrtcbin", "nicesrc", "appsrc", "appsink", "rtpvp8depay", "vp8dec", "videoconvert"]
                .iter()
                .all(|name| gstreamer::ElementFactory::find(name).is_some())
        }
    };
}

#[cfg(target_os = "android")]
plugins!(
    gst_plugin_app_register,
    gst_plugin_coreelements_register,
    gst_plugin_audioconvert_register,
    gst_plugin_audioresample_register,
    gst_plugin_audiomixer_register,
    gst_plugin_audiotestsrc_register,
    gst_plugin_level_register,
    gst_plugin_volume_register,
    gst_plugin_opus_register,
    gst_plugin_rtp_register,
    gst_plugin_rtpmanager_register,
    gst_plugin_webrtc_register,
    gst_plugin_nice_register,
    gst_plugin_dtls_register,
    gst_plugin_srtp_register,
    gst_plugin_vpx_register,
    gst_plugin_videoconvertscale_register,
    gst_plugin_typefindfunctions_register,
);

#[cfg(not(target_os = "android"))]
pub fn initialize() -> bool {
    gstreamer::init().is_ok()
}

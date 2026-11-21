//! Links the selected static plugins from the relocatable GStreamer Android SDK.
use std::{env, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }
    println!("cargo:rerun-if-env-changed=PAPO_GST_ABI_ROOT");
    let lib =
        PathBuf::from(env::var_os("PAPO_GST_ABI_ROOT").expect("set PAPO_GST_ABI_ROOT")).join("lib");
    // Android rejects unresolved native imports at dlopen, not at APK packaging time.
    println!("cargo:rustc-link-arg=-Wl,--no-undefined");
    println!("cargo:rustc-link-arg=-Wl,--start-group");
    for plugin in [
        "app",
        "coreelements",
        "audioconvert",
        "audioresample",
        "audiomixer",
        "audiotestsrc",
        "level",
        "volume",
        "opus",
        "rtp",
        "rtpmanager",
        "webrtc",
        "nice",
        "dtls",
        "srtp",
        "vpx",
        "videoconvertscale",
        "typefindfunctions",
    ] {
        println!(
            "cargo:rustc-link-arg={}",
            lib.join(format!("gstreamer-1.0/libgst{plugin}.a"))
                .display()
        );
    }
    for archive in [
        "gstapp-1.0",
        "gstwebrtcnice-1.0",
        "gstwebrtc-1.0",
        "gstsctp-1.0",
        "gstnet-1.0",
        "gstsdp-1.0",
        "gstrtp-1.0",
        "gstpbutils-1.0",
        "gstaudio-1.0",
        "gsttag-1.0",
        "gstvideo-1.0",
        "gstallocators-1.0",
        "gstbase-1.0",
        "gstreamer-1.0",
        "gio-2.0",
        "gobject-2.0",
        "gthread-2.0",
        "gmodule-2.0",
        "glib-2.0",
        "nice",
        "ssl",
        "crypto",
        "srtp2",
        "opus",
        "vpx",
        "orc-0.4",
        "intl",
        "ffi",
        "iconv",
        "pcre2-8",
    ] {
        println!(
            "cargo:rustc-link-arg={}",
            lib.join(format!("lib{archive}.a")).display()
        );
    }
    println!("cargo:rerun-if-env-changed=PAPO_ANDROID_BUILTINS");
    let builtins =
        PathBuf::from(env::var_os("PAPO_ANDROID_BUILTINS").expect("set PAPO_ANDROID_BUILTINS"));
    println!("cargo:rustc-link-arg={}", builtins.display());
    println!("cargo:rustc-link-arg=-Wl,--end-group");
    for system in ["android", "log", "dl", "z", "m", "atomic", "c++_shared"] {
        println!("cargo:rustc-link-arg=-l{system}");
    }
}

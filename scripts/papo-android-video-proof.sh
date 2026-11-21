#!/usr/bin/env bash
# Build the standalone JNI library for the debug-only synthetic fixture.
set -euo pipefail
cd "$(dirname "$0")/.."
output=${1:?pass an absolute JNI output directory}
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-${ANDROID_HOME:-$HOME/Android/Sdk}/ndk/30.0.16248370}"
sdk="${GSTREAMER_ROOT_ANDROID:-$HOME/Android/gst}"
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_ALL_STATIC=1 PKG_CONFIG_PATH=
compilers=("$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/clang)
clang="${compilers[0]}"
[[ -x "$clang" ]] || { echo 'Missing Android NDK clang' >&2; exit 1; }
for abi in x86_64 arm64-v8a; do
    gst_abi=$abi
    clang_target=x86_64-linux-android31
    if [[ $abi == arm64-v8a ]]; then
        gst_abi=arm64
        clang_target=aarch64-linux-android31
    fi
    # GStreamer is built with Clang; Rust's link excludes implicit compiler libraries.
    PAPO_ANDROID_BUILTINS=$("$clang" --target="$clang_target" --rtlib=compiler-rt --print-libgcc-file-name)
    export PAPO_ANDROID_BUILTINS
    [[ -f "$PAPO_ANDROID_BUILTINS" ]] || { echo 'Missing Android NDK compiler builtins' >&2; exit 1; }
    export PAPO_GST_ABI_ROOT="$sdk/$gst_abi"
    export PKG_CONFIG_LIBDIR="$PAPO_GST_ABI_ROOT/lib/pkgconfig"
    if [[ ! -f "$PKG_CONFIG_LIBDIR/gstreamer-webrtc-1.0.pc" || ! -f "$PAPO_GST_ABI_ROOT/lib/gstreamer-1.0/libgstvpx.a" ]]; then
        printf 'Missing GStreamer Android SDK pkg-config data for %s under %s\n' "$abi" "$sdk" >&2
        exit 1
    fi
    cargo ndk -t "$abi" --platform 31 --link-libcxx-shared -o "$output" build --locked \
        --manifest-path tests/android-video-proof/Cargo.toml
done

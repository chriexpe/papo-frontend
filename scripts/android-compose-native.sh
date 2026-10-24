#!/usr/bin/env bash
# Builds the UI-independent call runtime and the UniFFI adapter for each ABI.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
profile=$1
output=$2
abis=$3
export GSTREAMER_ROOT_ANDROID="${GSTREAMER_ROOT_ANDROID:-$HOME/Android/gst}"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/30.0.16248370}"
IFS=, read -ra targets <<< "$abis"
for abi in "${targets[@]}"; do
    case "$abi" in
        arm64-v8a) triplet=aarch64-linux-android ;;
        x86_64) triplet=x86_64-linux-android ;;
        *) echo "Unsupported call ABI: $abi" >&2; exit 2 ;;
    esac
    gst_build="$root/android/compose/build/call-gstreamer/$abi"
    cmake -S "$root/android/call-gstreamer" -B "$gst_build" \
        -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" \
        -DANDROID_ABI="$abi" -DANDROID_PLATFORM=android-31 \
        -DANDROID_STL=c++_shared -DCMAKE_BUILD_TYPE=Release
    cmake --build "$gst_build" --parallel 4
    mkdir -p "$output/$abi"
    cp "$gst_build/libgstreamer_android.so" "$output/$abi/"
    cp "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/$triplet/libc++_shared.so" "$output/$abi/"
    for dep in GSTREAMER_1_0 GSTREAMER_APP_1_0 GSTREAMER_VIDEO_1_0 \
               GSTREAMER_AUDIO_1_0 GSTREAMER_BASE_1_0 GSTREAMER_WEBRTC_1_0 \
               GSTREAMER_SDP_1_0 GSTREAMER_RTP_1_0 GLIB_2_0 GOBJECT_2_0 GIO_2_0; do
        export "SYSTEM_DEPS_${dep}_NO_PKG_CONFIG=1"
        export "SYSTEM_DEPS_${dep}_LIB=gstreamer_android"
        export "SYSTEM_DEPS_${dep}_SEARCH_NATIVE=$output/$abi"
    done
    args=()
    if [ "$profile" = release ]; then args+=(--release); fi
    cargo ndk -t "$abi" --platform 31 -o "$output" build --locked -p papo-android --features calls --lib "${args[@]}"
done

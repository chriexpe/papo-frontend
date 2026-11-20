#!/usr/bin/env bash
# Builds the UniFFI adapter for each ABI.
set -euo pipefail
profile=$1
output=$2
abis=$3
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/30.0.16248370}"
IFS=, read -ra targets <<< "$abis"
for abi in "${targets[@]}"; do
    mkdir -p "$output/$abi"
    rm -f "$output/$abi/libgstreamer_android.so" "$output/$abi/libc++_shared.so"
    args=()
    if [ "$profile" = release ]; then args+=(--release); fi
    cargo ndk -t "$abi" --platform 31 -o "$output" build --locked -p papo-android --lib "${args[@]}"
done

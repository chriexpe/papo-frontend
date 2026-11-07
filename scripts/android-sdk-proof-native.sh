#!/usr/bin/env bash
# CI-only UniFFI integration fixture. No Compose/GStreamer legacy build.
set -euo pipefail
cd "$(dirname "$0")/.."
output=$1
cargo ndk -t x86_64 --platform 31 -o "$output" build --locked \
    -p papo-ffi --features integration-proof --lib

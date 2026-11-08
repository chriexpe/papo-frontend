#!/usr/bin/env bash
# Build only the opt-in SDK fixture and generate bindings from the packaged library.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# != 2 ]]; then
    echo "usage: $0 <jni-directory> <kotlin-directory>" >&2
    exit 2
fi
jni_directory="$1"
kotlin_directory="$2"
python3 -B scripts/prepare-winit.py
cargo ndk -t x86_64 --platform 31 -o "$jni_directory" build --locked \
    -p papo-ffi --features integration-proof --lib
cargo run --locked -p papo-ffi --features bindgen --bin papo-sdk-bindgen -- \
    generate --library "$jni_directory/x86_64/libpapo_ffi.so" \
    --language kotlin --out-dir "$kotlin_directory"

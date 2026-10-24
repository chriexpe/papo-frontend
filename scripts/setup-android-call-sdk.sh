#!/usr/bin/env bash
set -euo pipefail
version=1.28.7
checksum=fa914bcf8695af1c6fa1f81db2e8da019a0306a89c2c3e48659438e1654023da
sdk="${GSTREAMER_ROOT_ANDROID:-$HOME/Android/gst}"
if [ -f "$sdk/arm64/lib/pkgconfig/gstreamer-1.0.pc" ] && [ -f "$sdk/x86_64/lib/pkgconfig/gstreamer-1.0.pc" ]; then exit 0; fi
archive=$(mktemp)
trap 'rm -f "$archive"' EXIT
curl --fail --location --retry 3 "https://gstreamer.freedesktop.org/data/pkg/android/$version/gstreamer-1.0-android-universal-$version.tar.xz" -o "$archive"
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check -
mkdir -p "$sdk"
tar -xf "$archive" -C "$sdk" arm64 x86_64

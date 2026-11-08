#!/usr/bin/env bash
set -euo pipefail
version=1.28.7
checksum=fa914bcf8695af1c6fa1f81db2e8da019a0306a89c2c3e48659438e1654023da
sdk="${GSTREAMER_ROOT_ANDROID:-$HOME/Android/gst}"
if [ -f "$sdk/arm64/lib/pkgconfig/gstreamer-1.0.pc" ] && [ -f "$sdk/x86_64/lib/pkgconfig/gstreamer-1.0.pc" ]; then exit 0; fi
archive=$(mktemp)
trap 'rm -f "$archive"' EXIT
url="https://gstreamer.freedesktop.org/data/pkg/android/$version/gstreamer-1.0-android-universal-$version.tar.xz"
attempts=20
# The host closes long transfers: HTTP/2 streams with PROTOCOL_ERROR, and HTTP/1.1
# ones every ~80 MB of this ~1 GB archive. Curl's --retry does not resume a
# partial body, so each attempt continues the file from where the last one stopped.
for attempt in $(seq "$attempts"); do
    printf 'GStreamer SDK download attempt %s/%s (%s bytes so far)\n' "$attempt" "$attempts" "$(wc -c < "$archive")"
    if curl --http1.1 --fail --location --silent --show-error \
        --connect-timeout 30 --speed-limit 10240 --speed-time 90 \
        --continue-at - "$url" -o "$archive"; then
        break
    else
        status=$?
    fi
    # 33: the server refused the range request. Appending a full response to a
    # partial file would corrupt it, so start over.
    if [ "$status" -eq 33 ]; then : > "$archive"; fi
    if [ "$attempt" -eq "$attempts" ]; then
        printf 'GStreamer SDK download failed after %s attempts\n' "$attempts" >&2
        exit 1
    fi
    sleep 3
done
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check -
mkdir -p "$sdk"
tar -xf "$archive" -C "$sdk" arm64 x86_64

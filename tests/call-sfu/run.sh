#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
video="${TMPDIR:-/tmp}/papo-call-fixture.ivf"
if [ ! -s "$video" ]; then
    ffmpeg -nostdin -loglevel error -f lavfi -i testsrc2=size=320x180:rate=10 -t 3 -c:v libvpx -b:v 150k -g 10 -f ivf "$video"
fi
go run . -video "$video" "$@"

#!/usr/bin/env bash
# Generate short, distinguishable VP8/Opus publisher streams and start the loopback SFU.
set -euo pipefail
cd "$(dirname "$0")"
work=$(mktemp -d)
trap 'rm -r "$work"' EXIT
for color in red green blue magenta; do
    ffmpeg -nostdin -v error -f lavfi -i "color=$color:size=160x90:rate=10" -t 2 \
        -vf "drawbox=x=0:y=70:w=24:h=20:color=white:t=fill:enable='mod(n,2)',drawbox=x=25:y=70:w=24:h=20:color=white:t=fill:enable='mod(floor(n/2),2)',drawbox=x=50:y=70:w=24:h=20:color=white:t=fill:enable='mod(floor(n/4),2)'" \
        -c:v libvpx -threads 1 -g 1 -f ivf "$work/$color.ivf"
done
for hz in 440 880; do
    ffmpeg -nostdin -v error -f lavfi -i "sine=frequency=$hz:sample_rate=48000" -t 2 \
        -c:a libopus -ac 2 -page_duration 20000 "$work/$hz.opus"
done
go build -o "$work/call-sfu" .
"$work/call-sfu" -media "$work" "$@" &
fixture=$!
trap 'kill "$fixture" 2>/dev/null || true; rm -r "$work"' EXIT
trap 'exit 130' INT TERM
wait "$fixture"

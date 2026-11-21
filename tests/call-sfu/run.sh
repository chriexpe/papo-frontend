#!/usr/bin/env bash
# Generate short, distinguishable VP8/Opus publisher streams and start the loopback SFU.
set -euo pipefail
cd "$(dirname "$0")"
profile=""
args=("$@")
for ((i=0; i<${#args[@]}; i++)); do
    case "${args[$i]}" in
        -profile)
            if (( i + 1 < ${#args[@]} )); then
                profile="${args[$((i+1))]}"
            fi
            ;;
        -profile=*)
            profile="${args[$i]#-profile=}"
            ;;
    esac
done

work=$(mktemp -d)
cleanup() {
    if [[ -n "${fixture:-}" ]]; then
        kill "$fixture" 2>/dev/null || true
        wait "$fixture" 2>/dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

media_dir="$work"
if [[ -z "$profile" ]]; then
    for color in red green blue magenta; do
        ffmpeg -nostdin -v error -f lavfi -i "color=$color:size=160x90:rate=10" -t 2 \
            -vf "drawbox=x=0:y=70:w=24:h=20:color=white:t=fill:enable='mod(n,2)',drawbox=x=25:y=70:w=24:h=20:color=white:t=fill:enable='mod(floor(n/2),2)',drawbox=x=50:y=70:w=24:h=20:color=white:t=fill:enable='mod(floor(n/4),2)'" \
            -c:v libvpx -threads 1 -g 1 -f ivf "$work/$color.ivf"
    done
    for hz in 440 880; do
        ffmpeg -nostdin -v error -f lavfi -i "sine=frequency=$hz:sample_rate=48000" -t 2 \
            -c:a libopus -ac 2 -page_duration 20000 "$work/$hz.opus"
    done
else
    cache_root="${PAPO_SFU_MEDIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/papo-call-sfu-media}"
    media_dir="$cache_root/$profile"
    mkdir -p "$media_dir"
    gen_vp8() {
        local name=$1 w=$2 h=$3 bitrate=$4 tint=$5
        if [[ -s "$media_dir/$name.ivf" ]]; then
            return 0
        fi
        local tmp="$media_dir/$name.ivf.tmp.$$"
        ffmpeg -nostdin -v error -y -f lavfi -i "testsrc2=size=${w}x${h}:rate=30" -t 2 \
            -vf "drawbox=x=0:y=0:w=80:h=40:color=${tint}:t=fill,format=yuv420p" \
            -c:v libvpx -b:v "$bitrate" -deadline realtime -cpu-used 4 -threads 1 -g 30 -keyint_min 30 -auto-alt-ref 0 -f ivf "$tmp"
        mv "$tmp" "$media_dir/$name.ivf"
    }
    case "$profile" in
        360p)
            gen_vp8 red 640 360 450k red
            ;;
        720p)
            gen_vp8 red 1280 720 1500k red
            ;;
        1080p)
            gen_vp8 red 1920 1080 3000k red
            ;;
        four-720p)
            gen_vp8 red 1280 720 1500k red
            gen_vp8 green 1280 720 1500k green
            gen_vp8 blue 1280 720 1500k blue
            gen_vp8 magenta 1280 720 1500k magenta
            for hz in 440 880; do
                if [[ ! -s "$media_dir/$hz.opus" ]]; then
                    opus_tmp="$media_dir/$hz.opus.tmp.$$"
                    ffmpeg -nostdin -v error -y -f lavfi -i "sine=frequency=$hz:sample_rate=48000" -t 2 \
                        -c:a libopus -ac 2 -page_duration 20000 -f ogg "$opus_tmp"
                    mv "$opus_tmp" "$media_dir/$hz.opus"
                fi
            done
            ;;
        *)
            printf 'Unsupported profile: %s\n' "$profile" >&2
            exit 1
            ;;
    esac
fi

go build -o "$work/call-sfu" .
"$work/call-sfu" -media "$media_dir" "$@" &
fixture=$!
wait "$fixture"

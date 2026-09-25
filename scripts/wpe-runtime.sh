#!/usr/bin/env bash
set -euo pipefail

# Stage the tiny WPEPlatform ABI bridge Papo consumes.  We intentionally use
# the distro's WPE WebKit runtime instead of WaterUI's self-contained runtime:
# WaterUI has the publishing workflow, but no wpe-runtime-v* artifact has been
# released yet.  This keeps local development practical and still avoids GTK.
#
# The bridge source is pinned so Papo's Rust ABI (v3) cannot silently drift.
waterui_commit="034057120bdade53c00ce2a63c1094172d9c01a2"
base="https://raw.githubusercontent.com/water-rs/waterui/${waterui_commit}/components/platform/browser-wpe/native"

profile="${1:-debug}"
destination="${2:-target/${profile}/waterui-browser/wpe}"
work="${XDG_CACHE_HOME:-$HOME/.cache}/papo/wpe-bridge/${waterui_commit}"

required_commands=(cc curl pkg-config sed)
for command in "${required_commands[@]}"; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "missing build tool: $command" >&2
        exit 2
    fi
done

packages=(
    wpe-webkit-2.0
    wpe-platform-2.0
    libsoup-3.0
    json-glib-1.0
    libdrm
)

missing=()
for package in "${packages[@]}"; do
    if ! pkg-config --exists "$package"; then
        missing+=("$package")
    fi
done

if (( ${#missing[@]} )); then
    echo "missing WPE development packages: ${missing[*]}" >&2
    if command -v pacman >/dev/null 2>&1; then
        echo "On Arch/CachyOS install the required packages first, e.g.:" >&2
        echo "  sudo pacman -S --needed wpewebkit libsoup3 json-glib libdrm pkgconf" >&2
    else
        echo "Install your distribution's WPE WebKit 2.x development packages." >&2
    fi
    exit 2
fi

webkit_version="$(pkg-config --modversion wpe-webkit-2.0)"
platform_version="$(pkg-config --modversion wpe-platform-2.0)"
echo "Using system WPE WebKit ${webkit_version} / WPEPlatform ${platform_version}"

mkdir -p "$work" "$destination/lib"

for file in waterui_wpe.c waterui_wpe.h; do
    target="$work/$file"
    if [[ ! -s "$target" ]]; then
        curl --fail --location --retry 3 --output "$target" "$base/$file"
    fi
done

source="$work/waterui_wpe-papo.c"
cp "$work/waterui_wpe.c" "$source"

# The upstream self-contained bundle rewrites WebKit/GStreamer helper paths to
# files adjacent to libwaterui_wpe.so.  Papo's development bridge deliberately
# uses the distro runtime, so those overrides would point at nonexistent files.
if ! grep -q '^[[:space:]]*water_wpe_configure_runtime_paths();[[:space:]]*
output="$destination/lib/libwaterui_wpe.so"
cc \
    -std=c11 \
    -O2 \
    -fPIC \
    -shared \
    -D_GNU_SOURCE \
    -I"$work" \
    "$source" \
    -o "$output" \
    $(pkg-config --cflags --libs "${packages[@]}") \
    -ldl

echo "WPE bridge staged at $output"
echo "Papo will load the distro WPE runtime through that bridge."
echo "If Papo is elsewhere: export PAPO_WPE_RUNTIME=$(realpath "$destination")"
 "$source"; then
    echo "pinned WaterUI bridge changed: runtime-path call not found" >&2
    exit 1
fi
sed -i \
    '0,/^[[:space:]]*water_wpe_configure_runtime_paths();[[:space:]]*$/s//    \/\* Papo uses the system WPE runtime; do not override its helper paths. *\// ' \
    "$source"

output="$destination/lib/libwaterui_wpe.so"
cc \
    -std=c11 \
    -O2 \
    -fPIC \
    -shared \
    -D_GNU_SOURCE \
    -I"$work" \
    "$source" \
    -o "$output" \
    $(pkg-config --cflags --libs "${packages[@]}") \
    -ldl

echo "WPE bridge staged at $output"
echo "Papo will load the distro WPE runtime through that bridge."
echo "If Papo is elsewhere: export PAPO_WPE_RUNTIME=$(realpath "$destination")"

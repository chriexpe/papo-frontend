#!/usr/bin/env bash
set -euo pipefail

# Build Papo's small WPEPlatform bridge against the distro WPE WebKit runtime.
# No GTK, no downloaded WaterUI runtime, and no generated source rewriting.

profile="${1:-debug}"
destination="${2:-target/${profile}/waterui-browser/wpe}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source="$repo_root/native/wpe/papo_wpe.c"
include="$repo_root/native/wpe"

required_commands=(cc pkg-config)
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
        echo "On Arch/CachyOS:" >&2
        echo "  sudo pacman -S --needed wpewebkit libsoup3 libdrm pkgconf" >&2
    else
        echo "Install your distribution's WPE WebKit 2.x development packages." >&2
    fi
    exit 2
fi

webkit_version="$(pkg-config --modversion wpe-webkit-2.0)"
platform_version="$(pkg-config --modversion wpe-platform-2.0)"
echo "Using system WPE WebKit ${webkit_version} / WPEPlatform ${platform_version}"

mkdir -p "$destination/lib"
output="$destination/lib/libwaterui_wpe.so"

cc \
    -std=c11 \
    -D_GNU_SOURCE \
    -O2 \
    -fPIC \
    -shared \
    -Wall \
    -Wextra \
    -Werror \
    -Wpedantic \
    -I"$include" \
    "$source" \
    -o "$output" \
    $(pkg-config --cflags --libs "${packages[@]}")

echo "Papo WPE bridge staged at $output"
echo "Papo will use the distro WPE runtime through this bridge."
echo "If Papo is elsewhere: export PAPO_WPE_RUNTIME=$(realpath "$destination")"

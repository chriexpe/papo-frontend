#!/usr/bin/env bash
set -euo pipefail

version="2.52.5"
release="https://github.com/water-rs/waterui/releases/download/wpe-runtime-v${version}"
manifest_url="${release}/browser-runtime-manifest.json"

case "$(uname -m)" in
  x86_64) arch="x86_64" ;;
  aarch64) arch="aarch64" ;;
  *) echo "WPE runtime supports x86_64/aarch64 only" >&2; exit 2 ;;
esac

profile="${1:-debug}"
destination="${2:-target/${profile}/waterui-browser/wpe}"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/papo/wpe/${version}/${arch}"
mkdir -p "$cache"

manifest="$cache/browser-runtime-manifest.json"
curl --fail --location --retry 3 -o "$manifest" "$manifest_url"

mapfile -t selected < <(python3 - "$manifest" "$version" "$arch" <<'PY'
import json, sys
path, version, arch = sys.argv[1:]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)
matches = [
    a for a in data["artifacts"]
    if a["engine"] == "wpe"
    and a["version"] == version
    and a["platform"] == "linux"
    and a["architecture"] == arch
]
if len(matches) != 1:
    raise SystemExit(f"expected one WPE artifact, got {len(matches)}")
a = matches[0]
print(a["url"])
print(a["sha256"])
PY
)

url="${selected[0]}"
sha="${selected[1]}"
archive="$cache/runtime.zip"

if [[ ! -f "$archive" ]] || ! printf '%s  %s\n' "$sha" "$archive" | sha256sum -c --status; then
  curl --fail --location --retry 3 -o "$archive" "$url"
fi
printf '%s  %s\n' "$sha" "$archive" | sha256sum -c

rm -rf "$destination"
mkdir -p "$destination"
python3 - "$archive" "$destination" <<'PY'
import sys, zipfile
archive, destination = sys.argv[1:]
with zipfile.ZipFile(archive) as z:
    z.extractall(destination)
PY

echo "WPE WebKit ${version} staged at ${destination}"
echo "If Papo is elsewhere: export PAPO_WPE_RUNTIME=$(realpath "$destination")"

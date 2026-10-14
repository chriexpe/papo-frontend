#!/usr/bin/env bash
# Build an experimental, ad-hoc-signed self-contained Apple Silicon app.
# Developer signing and notarization remain separate release work.
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <papo executable> <semver> <output directory>" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS app bundling requires a Mac (iconutil, sips, codesign)" >&2
  exit 2
fi

binary="$1"
version="$2"
output="$3"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "expected a three-component numeric version (e.g. 0.7.5)" >&2
  exit 2
fi
if [[ ! -f "$binary" ]]; then
  echo "missing executable: $binary" >&2
  exit 2
fi

app="$output/Papo.app"
contents="$app/Contents"
rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Resources"
install -m 755 "$binary" "$contents/MacOS/papo"

cat > "$contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Papo</string>
  <key>CFBundleExecutable</key><string>papo</string>
  <key>CFBundleIdentifier</key><string>io.github.chriexpe.Papo</string>
  <key>CFBundleIconFile</key><string>Papo.icns</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Papo</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Papo needs your microphone for voice calls and voice messages.</string>
  <key>NSCameraUsageDescription</key><string>Papo needs your camera for video calls.</string>
</dict>
</plist>
EOF

# Use the repository's icon source instead of a macOS-only derivative.
source_icon="assets/icon-source.png"
if [[ ! -f "$source_icon" ]]; then
  echo "missing icon source: $source_icon" >&2
  exit 2
fi
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
iconset="$tmp/Papo.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -s format png -z "$size" "$size" "$source_icon" \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
done
for size in 16 32 128 256 512; do
  retina=$((size * 2))
  sips -s format png -z "$retina" "$retina" "$source_icon" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns --output "$contents/Resources/Papo.icns" "$iconset"

plutil -lint "$contents/Info.plist"
python3 packaging/macos/bundle-gstreamer.py "$app"
# Apple Silicon requires valid code signatures. This local ad-hoc signature
# provides no developer identity or notarization and is NOT a release signature.
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
echo "Experimental macOS app with bundled GStreamer: $app"

#!/usr/bin/env bash
# Relocatable AppImage with the dlopen'ed GStreamer plugins needed for calls.
set -euo pipefail
version="${1:?version required}"
arch="${2:?architecture required}"
binary="${3:?binary required}"
rsrpc="${4:?rsrpc helper required}"
test -x "$binary" && test -x "$rsrpc"
app="io.github.chriexpe.Papo"
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
appdir="$workdir/AppDir"
install -Dm755 "$rsrpc" "$appdir/usr/libexec/papo/rsrpc"
install -Dm644 "assets/icons/256.png" "$workdir/$app.png"
install -Dm644 "packaging/$app.metainfo.xml" \
  "$appdir/usr/share/metainfo/$app.metainfo.xml"
# linuxdeploy bundles dynamically linked libraries, and the GStreamer plugin
# copies runtime-discovered plugins (including "bad" for webrtcbin/libnice).
curl -fL --retry 3 -o "$workdir/linuxdeploy.AppImage" \
  "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$arch.AppImage"
curl -fL --retry 3 -o "$workdir/linuxdeploy-plugin-gstreamer" \
  "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gstreamer/432af639725bc8b99df12351e86bb2e8a6b3585f/linuxdeploy-plugin-gstreamer.sh"
chmod +x "$workdir/linuxdeploy.AppImage" "$workdir/linuxdeploy-plugin-gstreamer"
export PATH="$workdir:$PATH"
export APPIMAGE_EXTRACT_AND_RUN=1
export GSTREAMER_INCLUDE_BAD_PLUGINS=1
export VERSION="$version"
export ARCH="$arch"
"$workdir/linuxdeploy.AppImage" --appdir "$appdir" \
  --executable "$binary" \
  --desktop-file "packaging/$app.desktop" \
  --icon-file "$workdir/$app.png" \
  --plugin gstreamer \
  --output appimage
found="$(find . -maxdepth 1 -name '*.AppImage' -type f -print -quit)"
test -n "$found"
out="papo-$version-$arch.AppImage"
mv "$found" "$out"
chmod +x "$out"
sha256sum "$out" > "$out.sha256"

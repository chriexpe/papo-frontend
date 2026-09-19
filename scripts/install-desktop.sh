#!/usr/bin/env bash
# Instala o .desktop, os ícones e os metadados no diretório do usuário, para
# que a área de trabalho associe janela, bandeja e notificações ao aplicativo.
# Não instala o binário — para isso, `cargo install --path .`.
set -eu
cd "$(dirname "$0")/.."

app=io.github.chriexpe.Papo
data="${XDG_DATA_HOME:-$HOME/.local/share}"
apps="$data/applications"
icons="$data/icons/hicolor"

install -Dm644 "packaging/$app.desktop" "$apps/$app.desktop"
install -Dm644 "packaging/$app.metainfo.xml" "$data/metainfo/$app.metainfo.xml"

for size in 16 22 24 32 48 64 128 256; do
    dir="$icons/${size}x${size}/apps"
    mkdir -p "$dir"
    magick assets/icon.png -resize "${size}x${size}" "$dir/$app.png"
done

command -v update-desktop-database >/dev/null && update-desktop-database "$apps" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$icons" 2>/dev/null || true

echo "$app instalado em $data"

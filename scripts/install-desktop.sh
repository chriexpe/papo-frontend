#!/usr/bin/env bash
# Instala o .desktop e o ícone no diretório do usuário, para que o KDE
# associe janela, bandeja e notificações ao aplicativo.
set -eu
cd "$(dirname "$0")/.."

apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"

mkdir -p "$apps"
install -Dm644 packaging/papo.desktop "$apps/papo.desktop"

for size in 16 22 24 32 48 64 128 256; do
    dir="$icons/${size}x${size}/apps"
    mkdir -p "$dir"
    magick assets/icon.png -resize "${size}x${size}" "$dir/papo.png"
done

command -v update-desktop-database >/dev/null && update-desktop-database "$apps" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$icons" 2>/dev/null || true

echo "papo.desktop e ícones instalados"

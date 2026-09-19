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

# Os tamanhos vêm prontos em assets/icons: pedir ImageMagick aqui quebrava
# no Debian e no Ubuntu, que trazem a versão 6 (só `convert`, sem `magick`).
for icon in assets/icons/*.png; do
    size="$(basename "$icon" .png)"
    install -Dm644 "$icon" "$icons/${size}x${size}/apps/$app.png"
done

command -v update-desktop-database >/dev/null && update-desktop-database "$apps" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$icons" 2>/dev/null || true

echo "$app instalado em $data"

#!/usr/bin/env sh
# Instala o que está neste tarball no perfil do usuário, sem sudo.
#
# É o caminho mais simples: descompactar e rodar. Para uma instalação de
# sistema, use o pacote da sua distribuição ou o Flatpak.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"

bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

install -Dm755 "$here/bin/papo" "$bin/papo"
cp -r "$here/share/." "$data/"

command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -f -t "$data/icons/hicolor" 2>/dev/null || true

echo "papo instalado em $bin/papo"
case ":$PATH:" in
    *":$bin:"*) ;;
    *) echo "acrescente $bin ao PATH para chamá-lo pelo nome" ;;
esac

#!/usr/bin/env sh
# Diz — ou instala — os pacotes de sistema que o Papo precisa.
#
#   ./scripts/deps.sh            mostra o comando da sua distribuição
#   ./scripts/deps.sh --install  roda esse comando
#   ./scripts/deps.sh --runtime  só o que o binário pronto precisa
#   ./scripts/deps.sh --command  só a linha de comando, para canalizar
#
# O binário liga o GStreamer dinamicamente, então mesmo quem baixa o tarball
# precisa dos pacotes de execução. Quem compila precisa também dos -dev.
set -eu

install=false
runtime_only=false
bare=false
for arg in "$@"; do
    case "$arg" in
        --install) install=true ;;
        --runtime) runtime_only=true ;;
        --command) bare=true ;;
        -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "opção desconhecida: $arg" >&2; exit 2 ;;
    esac
done

# Execução: o que o binário carrega. Compilação: os cabeçalhos por cima.
# O `gstreamer1.0-nice` (libnice) é o ICE do webrtcbin: sem ele a call nem
# começa a negociar. Ele não vem com o -bad, é pacote à parte nas três
# distribuições.
apt_runtime="libgstreamer1.0-0 libgstreamer-plugins-base1.0-0
    gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav
    gstreamer1.0-nice gstreamer1.0-pipewire libxkbcommon0 libwayland-client0 libx11-6
    libfontconfig1 fonts-noto-color-emoji xdg-desktop-portal"
apt_build="build-essential pkg-config libgstreamer1.0-dev
    libgstreamer-plugins-base1.0-dev libxkbcommon-dev libwayland-dev
    libx11-dev libxcursor-dev libxi-dev libxrandr-dev libfontconfig1-dev"

dnf_runtime="gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good
    gstreamer1-plugins-bad-free gstreamer1-plugin-libav libnice-gstreamer1
    libxkbcommon libwayland-client
    libX11 fontconfig google-noto-color-emoji-fonts xdg-desktop-portal"
dnf_build="gcc pkgconf-pkg-config gstreamer1-devel
    gstreamer1-plugins-base-devel libxkbcommon-devel wayland-devel
    libX11-devel libXcursor-devel libXi-devel libXrandr-devel
    fontconfig-devel"

pacman_runtime="gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad
    gst-libav libnice gst-plugin-pipewire libxkbcommon wayland libx11 fontconfig
    noto-fonts-emoji xdg-desktop-portal"
pacman_build="base-devel pkgconf"

if command -v apt-get >/dev/null 2>&1; then
    manager="sudo apt-get install -y"
    packages="$apt_runtime"
    $runtime_only || packages="$packages $apt_build"
elif command -v dnf >/dev/null 2>&1; then
    manager="sudo dnf install -y"
    packages="$dnf_runtime"
    $runtime_only || packages="$packages $dnf_build"
elif command -v pacman >/dev/null 2>&1; then
    manager="sudo pacman -S --needed"
    packages="$pacman_runtime"
    $runtime_only || packages="$packages $pacman_build"
else
    cat >&2 <<'MSG'
Não reconheci o gerenciador de pacotes desta distribuição.

Instale, com os nomes que ela usar: GStreamer 1.x com os plugins base, good,
bad e libav; libxkbcommon; wayland; libX11; fontconfig; uma fonte de emoji
colorido (Noto Color Emoji); e o xdg-desktop-portal. Para compilar, os
pacotes de desenvolvimento correspondentes.
MSG
    exit 1
fi

# Tira as quebras de linha da lista.
packages="$(echo "$packages" | tr -s ' \n' ' ')"

if $bare; then
    echo "$manager $packages"
elif $install; then
    echo "$manager $packages"
    # shellcheck disable=SC2086
    $manager $packages
else
    echo "Instale com:"
    echo
    echo "    $manager $packages"
    echo
    echo "Ou rode este script com --install."
fi

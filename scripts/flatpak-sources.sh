#!/usr/bin/env bash
# Gera packaging/cargo-sources.json a partir do Cargo.lock.
#
# A compilação do Flatpak roda sem rede, então cada crate precisa estar
# declarada como fonte. Rode este script sempre que o Cargo.lock mudar.
set -eu
cd "$(dirname "$0")/.."

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

generator="${FLATPAK_CARGO_GENERATOR:-}"
if [ -z "$generator" ]; then
    generator="$work/flatpak-cargo-generator.py"
    curl -fsSL -o "$generator" \
        https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi

# O gerador importa aiohttp e tomlkit, que não vêm no python do sistema.
# Um ambiente descartável evita mexer no python da máquina.
python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet --disable-pip-version-check aiohttp tomlkit

"$work/venv/bin/python" "$generator" Cargo.lock -o packaging/cargo-sources.json
echo "packaging/cargo-sources.json atualizado"

# O rsRPC é outro módulo do manifesto, com o Cargo.lock dele, na revisão
# fixada. O manifesto repete o commit (o flatpak-builder não lê arquivos);
# aqui se confere que os dois não se desencontraram.
revision="$(tr -d '[:space:]' < packaging/rsrpc/REVISION)"
if ! grep -q "commit: $revision" packaging/io.github.chriexpe.Papo.yml; then
    echo "o manifesto do Flatpak não usa o rsRPC $revision de packaging/rsrpc/REVISION" >&2
    exit 1
fi
curl -fsSL -o "$work/rsrpc-Cargo.lock" \
    "https://raw.githubusercontent.com/pog5/rsrpc/$revision/Cargo.lock"
"$work/venv/bin/python" "$generator" "$work/rsrpc-Cargo.lock" \
    -o packaging/rsrpc-cargo-sources.json
echo "packaging/rsrpc-cargo-sources.json atualizado"

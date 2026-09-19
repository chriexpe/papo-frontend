#!/usr/bin/env bash
# Gera packaging/cargo-sources.json a partir do Cargo.lock.
#
# A compilação do Flatpak roda sem rede, então cada crate precisa estar
# declarada como fonte. Rode este script sempre que o Cargo.lock mudar.
set -eu
cd "$(dirname "$0")/.."

generator="${FLATPAK_CARGO_GENERATOR:-}"
if [ -z "$generator" ]; then
    generator="$(mktemp -d)/flatpak-cargo-generator.py"
    curl -fsSL -o "$generator" \
        https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi

python3 "$generator" Cargo.lock -o packaging/cargo-sources.json
echo "packaging/cargo-sources.json atualizado"

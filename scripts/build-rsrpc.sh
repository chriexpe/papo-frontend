#!/usr/bin/env bash
# Compila o rsRPC na revisão fixada em packaging/rsrpc/REVISION.
#
#   scripts/build-rsrpc.sh <pasta-de-saída>
#
# Os pacotes levam um rsRPC compilado aqui, do fonte, em vez do binário da
# release `nightly` do upstream: aquela etiqueta é regravada a cada
# atualização da lista de jogos (a soma fixada quebrava) e os binários de
# cada plataforma saíam de commits diferentes. Compilando da revisão, o
# binário corresponde exatamente ao fonte citado em packaging/rsrpc/NOTICE,
# e a lista de jogos embutida é a daquela revisão em todas as plataformas.
set -euo pipefail
cd "$(dirname "$0")/.."

out="${1:?pasta de saída}"
revision="$(tr -d '[:space:]' < packaging/rsrpc/REVISION)"

cargo install --locked \
    --git https://github.com/pog5/rsrpc.git \
    --rev "$revision" \
    --root "$out" \
    rsrpc
echo "rsRPC $revision em $out/bin"

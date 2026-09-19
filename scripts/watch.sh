#!/usr/bin/env bash
# Recompila e reabre o Papo a cada mudança em src/, assets/ ou Cargo.toml.
# Sem dependências: compara o estado dos arquivos a cada segundo.
set -u
cd "$(dirname "$0")/.." || exit 1

fingerprint() {
    find src assets Cargo.toml Cargo.lock -type f -printf '%T@ %s %p\n' 2>/dev/null | sha1sum
}

restart() {
    pkill -x papo 2>/dev/null
    RUST_LOG="${RUST_LOG:-info,zbus=warn}" ./target/debug/papo &
}

echo "== papo: observando src/ e assets/ (ctrl+c para sair)"
last=""
while true; do
    current=$(fingerprint)
    if [ "$current" != "$last" ]; then
        last=$current
        echo "== compilando..."
        if cargo build 2>&1 | grep -E "^(error|warning: unused)" -A 4; then
            echo "== compilado com avisos"
        fi
        if [ -x target/debug/papo ] && cargo build --quiet 2>/dev/null; then
            echo "== reabrindo"
            restart
        else
            echo "== build falhou; janela anterior mantida"
        fi
    fi
    sleep 1
done

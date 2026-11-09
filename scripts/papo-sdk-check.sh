#!/usr/bin/env bash
# SDK-only quality gate; no UI/native client dependencies are required.
set -euo pipefail
cd "$(dirname "$0")/.."

packages=(-p papo-api -p papo-sdk -p papo-sdk-ui -p papo-ffi -p papo-test-server)
python_bin=python3
if ! command -v "$python_bin" >/dev/null 2>&1; then
    python_bin=python
fi
# Cargo loads workspace patches even when selected SDK packages do not use them.
"$python_bin" -B scripts/prepare-winit.py
"$python_bin" -B -m unittest discover -s tests -p 'test_sdk_boundaries.py'
"$python_bin" -B scripts/check-sdk-boundaries.py
if [[ -n "${PAPO_BACKEND:-}" ]]; then
    "$python_bin" -B scripts/papo-api-contract.py --backend "$PAPO_BACKEND"
    "$python_bin" -B scripts/papo-api-events.py --backend "$PAPO_BACKEND"
else
    echo "Backend contract and gateway drift checks not run: PAPO_BACKEND is unset."
fi
"$python_bin" -B -m unittest discover -s tests -p 'test_papo_api_contract.py'
cargo fmt "${packages[@]}" -- --check
cargo clippy --locked "${packages[@]}" --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings" cargo doc --locked "${packages[@]}" \
    --all-features --no-deps --document-private-items
cargo test --locked "${packages[@]}" --all-features
cargo build --locked "${packages[@]}"

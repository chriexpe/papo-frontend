#!/usr/bin/env bash
# CI-only setup: user-facing installation remains scripts/deps.sh.
set -euo pipefail

if [[ ${GITHUB_ACTIONS:-} != true || $# != 1 ]]; then
    echo "This helper requires GitHub Actions and a package-cache directory." >&2
    exit 2
fi

package_cache=$1
mkdir -p "$package_cache"

# The hosted runner's Azure mirror took 114 minutes for one dependency install.
# Keep its standard Ubuntu suites/components and use the canonical HTTPS mirrors.
if [[ -f /etc/apt/apt-mirrors.txt ]]; then
    printf '%s\n' 'https://archive.ubuntu.com/ubuntu/' | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
fi
if [[ -f /etc/apt/apt-mirrors-security.txt ]]; then
    printf '%s\n' 'https://security.ubuntu.com/ubuntu/' | sudo tee /etc/apt/apt-mirrors-security.txt >/dev/null
fi

apt_options=(
    -o "Dir::Cache::archives=$package_cache"
    -o APT::Keep-Downloaded-Packages=true
    -o Binary::apt-get::APT::Keep-Downloaded-Packages=true
    -o Acquire::Retries=2
    -o Acquire::http::Timeout=30
    -o Acquire::https::Timeout=30
)

# A connection timeout alone cannot bound an endlessly trickling download.
timeout --kill-after=15s 2m sudo apt-get "${apt_options[@]}" update

# Reuse the repository's actual package list without eval or another copy of it.
read -r -a install_args < <(./scripts/deps.sh --command)
if [[ ${install_args[0]:-} != sudo || ${install_args[1]:-} != apt-get ||
      ${install_args[2]:-} != install || ${install_args[3]:-} != -y ]]; then
    echo "Expected the Ubuntu apt-get install command from scripts/deps.sh." >&2
    exit 2
fi
timeout --kill-after=15s 10m sudo apt-get "${apt_options[@]}" --no-install-recommends "${install_args[@]:2}"

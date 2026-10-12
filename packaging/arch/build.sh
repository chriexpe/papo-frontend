#!/usr/bin/env bash
# Use actual Arch's makepkg to produce a valid package from our built tarball.
set -euo pipefail
version="${1:?version required}"
arch="${2:?architecture required}"
tarball="papo-$version-$arch-linux.tar.gz"
test -f "$tarball"
# Native x86_64/aarch64 Ubuntu GitHub runners both provide Docker.
docker run --rm \
  -e "PAPO_VERSION=$version" \
  -v "$PWD:/release" \
  --workdir /tmp \
  archlinux:base-devel /bin/bash -euc '
    mkdir -p /tmp/papo-arch
    cp /release/packaging/arch/PKGBUILD.release /tmp/papo-arch/PKGBUILD
    cp "/release/papo-$PAPO_VERSION-$(uname -m)-linux.tar.gz" /tmp/papo-arch/
    chown -R nobody:nobody /tmp/papo-arch
    cd /tmp/papo-arch
    runuser -u nobody -- env PAPO_VERSION="$PAPO_VERSION" makepkg \
      --force --noconfirm --nodeps
    cp ./*.pkg.tar.zst /release/
  '
for pkg in ./*.pkg.tar.zst; do
  sha256sum "$pkg" > "$pkg.sha256"
done

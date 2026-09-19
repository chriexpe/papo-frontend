#!/usr/bin/env bash
# Monta o .deb a partir de um binário já compilado.
#
#   packaging/deb/build.sh <versão> <arquitetura-debian> <binário>
#
# Feito com dpkg-deb direto para não depender do cargo-deb: são quatro
# arquivos e um control, e assim o workflow instala uma ferramenta a menos.
set -euo pipefail
version="${1:?versão}"
arch="${2:?arquitetura (amd64 ou arm64)}"
binary="${3:?caminho do binário}"

cd "$(dirname "$0")/../.."
app=io.github.chriexpe.Papo
root="$(mktemp -d)/papo_${version}_${arch}"

install -Dm755 "$binary" "$root/usr/bin/papo"
install -Dm644 "packaging/$app.desktop" "$root/usr/share/applications/$app.desktop"
install -Dm644 "packaging/$app.metainfo.xml" "$root/usr/share/metainfo/$app.metainfo.xml"
install -Dm644 LICENSE "$root/usr/share/doc/papo/copyright"
for icon in assets/icons/*.png; do
    size="$(basename "$icon" .png)"
    install -Dm644 "$icon" "$root/usr/share/icons/hicolor/${size}x${size}/apps/$app.png"
done

install -d "$root/DEBIAN"
sed -e "s/@VERSION@/$version/" -e "s/@ARCH@/$arch/" \
    packaging/deb/control.in > "$root/DEBIAN/control"

# O dpkg quer o tamanho instalado em kB.
size_kb=$(du -sk "$root" | cut -f1)
echo "Installed-Size: $size_kb" >> "$root/DEBIAN/control"

dpkg-deb --build --root-owner-group "$root" >/dev/null
mv "$root.deb" .
echo "$(basename "$root").deb"

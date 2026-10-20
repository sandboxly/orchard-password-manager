#!/usr/bin/env bash
# Assemble the portable app folder that every Windows/Linux package is made
# from (installer, zip, tar.gz, deb, rpm, AppImage):
#
#   packaging/stage.sh win-x64   target/release/orchard-password-manager.exe dist/orchard-password-manager
#   packaging/stage.sh linux-x64 target/release/orchard-password-manager     dist/orchard-password-manager
#   packaging/stage.sh licenses  -  "Orchard Password Manager.app/Contents/Resources/licenses"
#
# Layout: the executable and licenses/ (LICENSE.txt, THIRD-PARTY-RUST.txt).
# Everything is linked statically, so there are no libraries to ship. The `licenses` form only copies the license folder (for macOS).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -ne 3 ]]; then
  sed -n '2,11p' "$0" >&2
  exit 2
fi
platform="$1" binary="$2" dest="$3"

stage_licenses() {
  local out="$1"
  rm -rf "$out"
  mkdir -p "$out"
  cp "$ROOT/LICENSE" "$out/LICENSE.txt"
  cp "$ROOT/licenses/THIRD-PARTY-RUST.txt" "$out/"
}

case "$platform" in
  licenses)
    stage_licenses "$dest"
    echo "licenses: $dest"
    exit 0
    ;;
  win-*) exe=orchard-password-manager.exe ;;
  linux-*) exe=orchard-password-manager ;;
  *)
    echo "unsupported platform: $platform (win-x64, linux-x64, licenses)" >&2
    exit 2
    ;;
esac

if [[ ! -f "$binary" ]]; then
  echo "missing $binary (cargo build --release)" >&2
  exit 1
fi
rm -rf "$dest"
mkdir -p "$dest"
cp "$binary" "$dest/$exe"
chmod 755 "$dest/$exe"
stage_licenses "$dest/licenses"
echo "staged $platform: $dest"
find "$dest" -type f -exec ls -l {} + | awk '{print "  " $5 "\t" $NF}'

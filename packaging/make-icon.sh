#!/usr/bin/env bash
# Rebuilds Formust.icns (the app bundle's icon) from formust-icon-1024.png, the master artwork.
# Needs sips and iconutil (macOS).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/formust-icon.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
iconset="$tmp/Formust.iconset"
mkdir "$iconset"
# Each pair is the pixel size and the name iconutil expects: the @2x images are twice the size of the name.
for pair in 16:icon_16x16 32:icon_16x16@2x 32:icon_32x32 64:icon_32x32@2x 128:icon_128x128 \
    256:icon_128x128@2x 256:icon_256x256 512:icon_256x256@2x 512:icon_512x512 1024:icon_512x512@2x; do
    sips -z "${pair%%:*}" "${pair%%:*}" "$here/formust-icon-1024.png" --out "$iconset/${pair#*:}.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$here/Formust.icns"
echo "wrote $here/Formust.icns"

#!/usr/bin/env bash
# Regenerate the embedded tray icon pixmap (assets/tray-icon-128.raw)
# from assets/tray-icon.svg. Requires librsvg (rsvg-convert) and
# ImageMagick (magick). The result is consumed by gpui/src/tray.rs.
set -euo pipefail
cd "$(dirname "$0")/.."

rsvg-convert --keep-aspect-ratio -w 128 -h 128 assets/tray-icon.svg -o /tmp/tray_card.png
magick /tmp/tray_card.png -gravity center -background none -extent 128x128 rgba:assets/tray-icon-128.raw
echo "assets/tray-icon-128.raw regenerated (128x128 RGBA)"

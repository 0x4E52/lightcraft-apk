#!/usr/bin/env bash
# Regenerate the Android launcher icons in packaging/android/res/ from assets/app-icon/lightcraft.svg.
#
# Needs: resvg (cargo install resvg --locked). The outputs are committed, so building and packaging
# never need it. The two XML resources (ic_launcher.xml, ic_launcher_background.xml) are hand-written
# and not touched here.
#
#   packaging/android/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SVG="$ROOT/assets/app-icon/lightcraft.svg"
RES="$ROOT/packaging/android/res"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (cargo install resvg --locked)" >&2; exit 1; }

# The SVG is a 512 tile clipped to rx=112 with a full-bleed #f2a516 field and two paths: the paper
# (#efe9dc) and the ink (#0b0b0c) drawn on it. Adaptive icons take the layers apart: the background is
# a colour resource, the foreground is the drawing alone on a 108 dp canvas, and the monochrome layer
# is the same drawing with the paper in black (themed icons use only the alpha channel, so it is a
# solid silhouette without the ink detail).
#
# Scale: Android shows a 72 dp viewport of the 108 dp canvas, and launchers mask it (a circle of 66 dp
# diameter is the guaranteed-visible safe zone). The artwork is a bust whose right and bottom edges are
# straight cuts where the original tile clipped the chest and shoulder. They must stay at or beyond the
# viewport edge (90 dp) so the chest runs off the icon under every mask; with the card's viewBox, 768
# units across the 108 dp canvas, tile x/y 505 land at 89 dp. A bigger viewBox would pull the ear tufts
# toward the safe circle but leave the cuts visible inside the icon, so the card's value is kept and the
# tufts under circle masks are left to the device check (see docs/android-port.md, APK-14).
FG="$TMP/foreground.svg"
VIEWBOX="-128 -128 768 768"
sed -e 's/ clip-path="url(#t)"//' \
    -e 's|<rect width="512" height="512" fill="#f2a516"/>||' \
    -e "s/viewBox=\"0 0 512 512\"/viewBox=\"$VIEWBOX\"/" "$SVG" >"$FG"
grep -q "viewBox=\"$VIEWBOX\"" "$FG" || { echo "error: unexpected viewBox in $SVG" >&2; exit 1; }
if grep -q 'clip-path="url(#t)"' "$FG"; then echo "error: clip-path group not found in $SVG" >&2; exit 1; fi
if grep -q 'fill="#f2a516"' "$FG"; then echo "error: background rect not found in $SVG" >&2; exit 1; fi
grep -q 'fill="#efe9dc"' "$FG" || { echo "error: paper path fill not found in $SVG" >&2; exit 1; }
grep -q 'fill="#0b0b0c"' "$FG" || { echo "error: ink path fill not found in $SVG" >&2; exit 1; }

MONO="$TMP/monochrome.svg"
sed 's/fill="#efe9dc"/fill="#000000"/' "$FG" >"$MONO"
grep -q 'fill="#000000"' "$MONO" || { echo "error: could not make the monochrome layer" >&2; exit 1; }
if grep -q 'fill="#efe9dc"' "$MONO"; then echo "error: monochrome layer keeps the paper colour" >&2; exit 1; fi

render() { resvg -w "$2" -h "$2" "$1" "$3" </dev/null; }

# density, legacy icon px, adaptive layer px
for row in mdpi:48:108 hdpi:72:162 xhdpi:96:216 xxhdpi:144:324 xxxhdpi:192:432; do
  IFS=: read -r d legacy layer <<<"$row"
  mkdir -p "$RES/mipmap-$d"
  render "$SVG" "$legacy" "$RES/mipmap-$d/ic_launcher.png"
  render "$FG" "$layer" "$RES/mipmap-$d/ic_launcher_foreground.png"
  render "$MONO" "$layer" "$RES/mipmap-$d/ic_launcher_monochrome.png"
done
echo "icons written to $RES"

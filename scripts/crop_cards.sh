#!/usr/bin/env bash
# Cuts a rough picture of every Res Arcana card out of the owner's local photos.
#
# The photos (game-rules/, gitignored: copyrighted art) are never committed, and neither are the pictures this makes
# (games/res-arcana-gui/web/cards/, gitignored). The rectangles live in games/res-arcana-gui/cards.map. Replace a
# picture by dropping a file with the same name (<id>.jpg) into that folder.
#
# Needs ImageMagick (`magick` or `convert`). Usage: scripts/crop_cards.sh [width]   (default 320 px wide)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
map="$root/games/res-arcana-gui/cards.map"
photos="$root/game-rules/res-arcana"
out="$root/games/res-arcana-gui/web/cards"
width="${1:-320}"

if command -v magick >/dev/null 2>&1; then im=(magick); elif command -v convert >/dev/null 2>&1; then im=(convert); else
  echo "ImageMagick is needed (magick or convert)" >&2; exit 1
fi
[ -f "$map" ] || { echo "missing $map" >&2; exit 1; }
[ -d "$photos" ] || { echo "missing $photos (the local card photos)" >&2; exit 1; }

mkdir -p "$out"
count=0
while IFS=$'\t' read -r id name source x y w h; do
  case "$id" in ''|'#'*) continue ;; esac
  "${im[@]}" "$photos/$source" -crop "${w}x${h}+${x}+${y}" +repage -resize "${width}x" -quality 82 "$out/$id.jpg"
  count=$((count + 1))
done < "$map"
echo "cut $count card pictures into $out"

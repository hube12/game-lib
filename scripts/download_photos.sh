#!/usr/bin/env bash
# Downloads the two Res Arcana photos that scripts/crop_cards.sh cuts the card pictures from:
#   all_cards.jpg   the grid of all cards
#   components.jpg  the places of power and the magic items
# They go to game-rules/res-arcana/ (gitignored: copyrighted art, never committed).
#
# Note: games/res-arcana-gui/cards.map holds pixel rectangles for the photos it was written against
# (all_cards.jpg 3958x2677, components.jpg 2911x2572). If the downloaded files have other sizes the
# rectangles will not line up; the script prints the sizes it got so you can tell.
#
# Usage: scripts/download_photos.sh [--force]   (--force downloads again over existing files)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/game-rules/res-arcana"
force="${1:-}"

cards_url='https://cf.geekdo-images.com/gyOoerf8UfCxYh2oFo2Rbg__original/img/EMBtbVgJgwMYGgia0CgfN8SmQi8=/0x0/filters:format(jpeg)/pic8654301.jpg'
components_url='https://cf.geekdo-images.com/pJXHudmt4M5YZtRtgFVFxw__original/img/vR688-AAX_LNts61uaxLxN_071o=/0x0/filters:format(jpeg)/pic8226560.jpg'

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fL --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -O "$2" "$1"; }
else
  echo "curl or wget is needed" >&2; exit 1
fi

mkdir -p "$dir"

get() { # name url
  local file="$dir/$1" tmp="$dir/$1.part"
  if [ -s "$file" ] && [ "$force" != "--force" ]; then
    echo "$1 already there (use --force to download again)"
  else
    echo "downloading $1"
    if ! fetch "$2" "$tmp"; then rm -f "$tmp"; echo "download of $1 failed" >&2; exit 1; fi
    mv "$tmp" "$file"
  fi
  if command -v identify >/dev/null 2>&1; then
    echo "  $1: $(identify -format '%wx%h' "$file[0]")"
  fi
}

get all_cards.jpg "$cards_url"
get components.jpg "$components_url"
echo "photos are in $dir; now run scripts/crop_cards.sh"

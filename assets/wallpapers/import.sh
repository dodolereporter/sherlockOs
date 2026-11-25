#!/usr/bin/env bash
# Imports the author's wallpapers into this folder, one original per wallpaper, under its
# English id. A PNG source is stored as JPEG quality 95 without chroma subsampling (see
# "Wallpapers" in docs/compositor.md for the measurements); a JPEG source is copied as is,
# since encoding it again would only lose quality. Needs ImageMagick 7 (`magick`).
#
#   assets/wallpapers/import.sh <folder of a collection> [ring image]
#
# The folder is one of the author's collections, dark or light: the wallpapers found in it are
# imported, the others left as they are. Run it again when the author provides new versions
# (4K), then commit the new originals.
set -euo pipefail

source_dir=${1:?usage: import.sh <folder of a collection> [ring image]}
ring=${2:-}
here=$(dirname "$0")

# Source file of each id, relative to its collection: the author's names, kept as data.
sources=$here/sources.json

store() {
    local source=$1 id=$2
    case $source in
        *.jpg | *.jpeg) cp "$source" "$here/$id.jpg" ;;
        *) magick "$source" -strip -quality 95 -sampling-factor 4:4:4 "$here/$id.jpg" ;;
    esac
    echo "$id.jpg $(stat -c %s "$here/$id.jpg") bytes"
}

imported=0
while read -r name id; do
    found=$(compgen -G "$source_dir/$name.*" | head -n 1 || true)
    if [[ -n $found ]]; then
        store "$found" "$id"
        imported=$((imported + 1))
    fi
done < <(python3 -c 'import json, sys
for name, id in json.load(open(sys.argv[1])).items():
    print(name, id)' "$sources")
if ((imported == 0)); then
    echo "no known wallpaper in $source_dir" >&2
    exit 1
fi
if [[ -n $ring ]]; then
    store "$ring" ring
fi

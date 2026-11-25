#!/usr/bin/env bash
# Imports the author's wallpapers into this folder, one original per wallpaper, under its
# English id. A PNG source is stored as JPEG quality 95 without chroma subsampling (see
# "Wallpapers" in docs/compositor.md for the measurements); a JPEG source is copied as is,
# since encoding it again would only lose quality. Needs ImageMagick 7 (`magick`).
#
#   assets/wallpapers/import.sh <folder of the collection> [ring image]
#
# Run it again when the author provides new versions (4K), then commit the new originals.
set -euo pipefail

source_dir=${1:?usage: import.sh <folder of the collection> [ring image]}
ring=${2:-}
here=$(dirname "$0")

# Source file, relative to the collection, then id in the repository.
pairs=(
    "00-Par-defaut/Le-premier-indice first-clue"
    "01-Embleme/Relief-de-jade jade-relief"
    "01-Embleme/Aurore-polaire aurora"
    "01-Embleme/Eclipse-de-papier paper-eclipse"
    "02-Les-enquetes-de-Sherlock/La-piste-sous-la-pluie rain-trail"
    "02-Les-enquetes-de-Sherlock/La-page-disparue missing-page"
    "02-Les-enquetes-de-Sherlock/Le-dernier-train last-train"
)

store() {
    local source=$1 id=$2
    case $source in
        *.jpg | *.jpeg) cp "$source" "$here/$id.jpg" ;;
        *) magick "$source" -strip -quality 95 -sampling-factor 4:4:4 "$here/$id.jpg" ;;
    esac
    echo "$id.jpg $(stat -c %s "$here/$id.jpg") bytes"
}

for pair in "${pairs[@]}"; do
    read -r name id <<<"$pair"
    found=$(compgen -G "$source_dir/$name.*" | head -n 1 || true)
    if [[ -z $found ]]; then
        echo "missing: $source_dir/$name.*" >&2
        exit 1
    fi
    store "$found" "$id"
done
if [[ -n $ring ]]; then
    store "$ring" ring
fi

#!/bin/sh
set -eu

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root_dir"

"$root_dir/scripts/build-universal.sh"

asset=bmtop-macos-universal.tar.gz
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
cp dist/bmtop LICENSE THIRD-PARTY-NOTICES.md README.md "$staging/"
tar -czf "dist/$asset" -C "$staging" bmtop LICENSE THIRD-PARTY-NOTICES.md README.md
(cd dist && shasum -a 256 "$asset" > "$asset.sha256")
cat "dist/$asset.sha256"

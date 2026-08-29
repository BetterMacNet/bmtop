#!/bin/sh
set -eu

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root_dir"

: "${CARGO_HOME:=$root_dir/.cargo-cache}"
export CARGO_HOME
export PATH="$HOME/.cargo/bin:$PATH"

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "missing Rust target: $target" >&2
        echo "install it outside this script with: rustup target add $target" >&2
        exit 2
    fi
    cargo build --release --target "$target" -p bmtop
done

mkdir -p dist
lipo -create \
    target/aarch64-apple-darwin/release/bmtop \
    target/x86_64-apple-darwin/release/bmtop \
    -output dist/bmtop
chmod 755 dist/bmtop

if [ "${BMTOP_CODESIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$BMTOP_CODESIGN_IDENTITY" dist/bmtop
fi

shasum -a 256 dist/bmtop > dist/bmtop.sha256
file dist/bmtop
cat dist/bmtop.sha256

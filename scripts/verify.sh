#!/bin/sh
set -eu

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root_dir"
: "${CARGO_HOME:=$root_dir/.cargo-cache}"
export CARGO_HOME
export PATH="$HOME/.cargo/bin:$PATH"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p bmtop -- doctor --format json >/dev/null

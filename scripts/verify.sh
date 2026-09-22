#!/bin/sh
# --fix-format is explicit: the default never modifies source files.
set -eu
root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root_dir"
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1; then
    echo "ERROR: cargo/rustc unavailable; Rust compilation and tests were NOT executed." >&2
    exit 127
fi
case "${1:-}" in
    --fix-format) cargo fmt --all ;;
    "") ;;
    *) echo 'Usage: scripts/verify.sh [--fix-format]' >&2; exit 64 ;;
esac
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -q -p bmtop -- --no-config doctor --format json >/dev/null
if command -v python3 >/dev/null 2>&1; then
    python3 -m unittest discover -s scripts/tests -v
    if command -v clang >/dev/null 2>&1; then
        python3 scripts/test_native_contract.py
    fi
fi

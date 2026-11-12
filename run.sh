#!/usr/bin/env bash
set -euo pipefail
TIDE_APP_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [[ -x "$TIDE_APP_DIR/bin/tide-terminal" ]]; then
    exec "$TIDE_APP_DIR/bin/tide-terminal" "$@"
fi
exec cargo run --release --locked --manifest-path "$TIDE_APP_DIR/Cargo.toml" -- "$@"

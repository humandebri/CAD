#!/usr/bin/env bash
# scripts: run the MVP local verification flow from a clean checkout.
# It regenerates ignored review artifacts before testing the viewer.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p cad-cli -- --help >/dev/null
cargo run -p cad-cli -- format examples/house-small
cargo run -p cad-cli -- check examples/house-small --format json --out examples/house-small/build/check.json
cargo run -p cad-cli -- render examples/house-small --format svg --out examples/house-small/build/plan_1f.svg
cargo run -p cad-cli -- diff examples/house-small examples/house-small-modified --format json --out examples/house-small/build/diff.json
cargo run -p cad-cli -- diff examples/house-small examples/house-small-modified --format svg --out examples/house-small/build/plan_1f.diff.svg
cargo metadata --no-deps --format-version 1 >/dev/null

pnpm --dir apps/viewer install
pnpm --dir apps/viewer exec playwright install chromium
pnpm --dir apps/viewer build
pnpm --dir apps/viewer test
pnpm --dir apps/viewer test:e2e

find . \
  -path ./.git -prune -o \
  -path ./target -prune -o \
  -path ./.pnpm-store -prune -o \
  -path ./apps/viewer/node_modules -prune -o \
  -path ./apps/viewer/dist -prune -o \
  -path ./apps/viewer/test-results -prune -o \
  -path ./examples/house-small/build -prune -o \
  -maxdepth 4 -type f -print
git status --short --branch

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
IMPORT_PARENT="$(mktemp -d)"
cargo run -p cad-cli -- import-jww examples/jww-fixtures/Test1.jww --out "$IMPORT_PARENT/test1_imported"
cargo run -p cad-cli -- check "$IMPORT_PARENT/test1_imported" --format json --out "$IMPORT_PARENT/test1_imported/build/check.json"
cargo run -p cad-cli -- render "$IMPORT_PARENT/test1_imported" --format svg --out "$IMPORT_PARENT/test1_imported/build/test1.svg"
cargo run -p cad-cli -- export-jww "$IMPORT_PARENT/test1_imported" --drawing test1 --out "$IMPORT_PARENT/test1-exported.jww" --report "$IMPORT_PARENT/test1-export-report.json"
test -s "$IMPORT_PARENT/test1-exported.jww"
test -s "$IMPORT_PARENT/test1-export-report.json"
cargo run -p cad-cli -- import-jww "$IMPORT_PARENT/test1-exported.jww" --out "$IMPORT_PARENT/test1_reimported"
cargo run -p cad-cli -- check "$IMPORT_PARENT/test1_reimported" --format json --out "$IMPORT_PARENT/test1_reimported/build/check.json"
cargo metadata --no-deps --format-version 1 >/dev/null

pnpm --dir apps/viewer install
pnpm --dir apps/viewer exec playwright install chromium
pnpm --dir apps/viewer build
pnpm --dir apps/viewer test
pnpm --dir apps/viewer test:e2e
pnpm --dir apps/viewer desktop:build

find . \
  -path ./.git -prune -o \
  -path ./target -prune -o \
  -path ./.pnpm-store -prune -o \
  -path ./apps/viewer/node_modules -prune -o \
  -path ./apps/viewer/dist -prune -o \
  -path ./apps/viewer/test-results -prune -o \
  -path ./examples/house-small/build -prune -o \
  -name .DS_Store -prune -o \
  -maxdepth 4 -type f -print
git status --short --branch

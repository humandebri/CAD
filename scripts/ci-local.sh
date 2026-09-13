#!/usr/bin/env bash
# Run the local verification flow without installing dependencies or rewriting
# tracked fixtures. Run scripts/ci-bootstrap.sh once on a fresh checkout.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

command -v cargo >/dev/null || { echo "cargo is required; run the Rust toolchain setup" >&2; exit 1; }
command -v pnpm >/dev/null || { echo "pnpm is required; run scripts/ci-bootstrap.sh" >&2; exit 1; }
test -x apps/viewer/node_modules/.bin/playwright || {
  echo "viewer dependencies are missing; run scripts/ci-bootstrap.sh" >&2
  exit 1
}
STATUS_BEFORE="$(git status --short)"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check -p cad-desktop --no-default-features
cargo run -p cad-cli -- --help >/dev/null
SMOKE_ROOT="$(mktemp -d)"
trap 'rm -rf "$SMOKE_ROOT"' EXIT
cp -R examples/house-small "$SMOKE_ROOT/house-small"
cargo run -p cad-cli -- format "$SMOKE_ROOT/house-small"
cargo run -p cad-cli -- check "$SMOKE_ROOT/house-small" --format json --out "$SMOKE_ROOT/check.json"
cargo run -p cad-cli -- render "$SMOKE_ROOT/house-small" --format svg --out "$SMOKE_ROOT/plan_1f.svg"
cargo run -p cad-cli -- diff "$SMOKE_ROOT/house-small" examples/house-small-modified --format json --out "$SMOKE_ROOT/diff.json"
cargo run -p cad-cli -- diff "$SMOKE_ROOT/house-small" examples/house-small-modified --format svg --out "$SMOKE_ROOT/plan_1f.diff.svg"
IMPORT_PARENT="$SMOKE_ROOT/jww"
mkdir -p "$IMPORT_PARENT"
cargo run -p cad-cli -- import-jww examples/jww-fixtures/Test1.jww --out "$IMPORT_PARENT/test1_imported"
cargo run -p cad-cli -- check "$IMPORT_PARENT/test1_imported" --format json --out "$IMPORT_PARENT/test1_imported/build/check.json"
cargo run -p cad-cli -- render "$IMPORT_PARENT/test1_imported" --format svg --out "$IMPORT_PARENT/test1_imported/build/test1.svg"
cargo run -p cad-cli -- export-jww "$IMPORT_PARENT/test1_imported" --drawing test1 --out "$IMPORT_PARENT/test1-exported.jww" --report "$IMPORT_PARENT/test1-export-report.json" --allow-lossy
test -s "$IMPORT_PARENT/test1-exported.jww"
test -s "$IMPORT_PARENT/test1-export-report.json"
cargo run -p cad-cli -- import-jww "$IMPORT_PARENT/test1-exported.jww" --out "$IMPORT_PARENT/test1_reimported"
cargo run -p cad-cli -- check "$IMPORT_PARENT/test1_reimported" --format json --out "$IMPORT_PARENT/test1_reimported/build/check.json"
cargo metadata --no-deps --format-version 1 >/dev/null

pnpm --dir apps/viewer build
pnpm --dir apps/viewer test:unit
pnpm --dir apps/viewer test:e2e
if [[ "$(uname -s)" == "Darwin" ]]; then
  pnpm --dir apps/viewer desktop:e2e
fi
git diff --check
STATUS_AFTER="$(git status --short)"
if [[ "$STATUS_BEFORE" != "$STATUS_AFTER" ]]; then
  echo "local verification changed tracked or untracked files" >&2
  diff -u <(printf '%s\n' "$STATUS_BEFORE") <(printf '%s\n' "$STATUS_AFTER") || true
  exit 1
fi

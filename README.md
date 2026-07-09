# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

Phase 5 establishes strict loading, checking, SVG rendering, semantic diff, and
a local viewer for generated review artifacts.

## Layout

```text
crates/
  cad-model
  cad-check
  cad-render-svg
  cad-diff
  cad-cli
apps/
  viewer
examples/
  house-small
  house-small-modified
```

## Checks

```bash
cargo test --workspace
cargo run -p cad-cli -- --help
cargo run -p cad-cli -- format examples/house-small
cargo run -p cad-cli -- check examples/house-small --format json --out examples/house-small/build/check.json
cargo run -p cad-cli -- render examples/house-small --format svg --out examples/house-small/build/plan_1f.svg
cargo run -p cad-cli -- diff examples/house-small examples/house-small-modified --format json --out examples/house-small/build/diff.json
cargo run -p cad-cli -- diff examples/house-small examples/house-small-modified --format svg --out examples/house-small/build/plan_1f.diff.svg
cargo metadata --no-deps --format-version 1
pnpm --dir apps/viewer install
pnpm --dir apps/viewer exec playwright install chromium
pnpm --dir apps/viewer build
pnpm --dir apps/viewer test
pnpm --dir apps/viewer test:e2e
find . -maxdepth 4 -type f
git status --short --branch
```

## Out Of Scope For MVP

- DXF/PDF/JWW/DWG export
- QCAD integration
- `cadc serve`
- comment creation API
- desktop app packaging
- in-app LLM
- cloud sync

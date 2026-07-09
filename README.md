# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

Phase 1 establishes strict loading for the NDJSON/TOML CAD source model. Full
checking, rendering, and diff behavior start in later phases.

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
```

## Checks

```bash
cargo test --workspace
cargo run -p cad-cli -- --help
cargo run -p cad-cli -- format examples/house-small
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

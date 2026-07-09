# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

Phase 6 establishes the MVP flow: strict loading, checking, SVG rendering,
semantic diff, a local viewer, and one local CI command.

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
scripts/ci-local.sh
```

The script runs the same checks as the manual sequence below.

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

## MVP Flow

1. Edit `examples/house-small/drawings/plan_1f/entities.ndjson`.
2. Run `cargo run -p cad-cli -- format examples/house-small`.
3. Run `cargo run -p cad-cli -- check examples/house-small --format json --out examples/house-small/build/check.json`.
4. Run `cargo run -p cad-cli -- render examples/house-small --format svg --out examples/house-small/build/plan_1f.svg`.
5. Run `cargo run -p cad-cli -- diff examples/house-small examples/house-small-modified --format json --out examples/house-small/build/diff.json`.
6. Run `cargo run -p cad-cli -- diff examples/house-small examples/house-small-modified --format svg --out examples/house-small/build/plan_1f.diff.svg`.
7. Run `pnpm --dir apps/viewer dev` and open `http://127.0.0.1:5173/`.

## Fixtures And Snapshots

- Source fixtures live under `examples/house-small` and `examples/house-small-modified`.
- Generated review artifacts live under `examples/house-small/build` and are ignored.
- Inline Rust snapshots live in the crate tests through `insta::assert_snapshot!`.
- Browser review coverage lives in `apps/viewer/tests/e2e/viewer.spec.ts`.

## Check Error Codes

- `format.missing_file`
- `format.invalid_toml`
- `format.invalid_ndjson`
- `format.empty_ndjson_line`
- `format.unsupported_schema`
- `format.invalid_entity_id`
- `reference.undefined_layer`
- `reference.undefined_text_style`
- `reference.undefined_dimension_style`
- `reference.undefined_block`
- `reference.duplicate_id`
- `reference.undefined_color`
- `reference.undefined_line_type`
- `layer.invalid_line_width`
- `layer.non_print_annotation`
- `geometry.invalid_bbox`
- `geometry.zero_length`
- `geometry.invalid_arc`
- `geometry.invalid_circle`
- `geometry.tiny_gap`
- `geometry.open_closed_polyline`
- `geometry.self_intersection`

## Out Of Scope For MVP

- DXF/PDF/JWW/DWG export
- QCAD integration
- `cadc serve`
- comment creation API
- desktop app packaging
- in-app LLM
- cloud sync

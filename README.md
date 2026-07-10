# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

Phase 8 adds one-way JWW import on top of the macOS-first desktop app and MVP
flow: strict
loading, checking, SVG rendering, semantic diff, and a local viewer.

## Layout

```text
crates/
  cad-model
  cad-check
  cad-render-svg
  cad-diff
  cad-import-jww
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
cargo run -p cad-cli -- import-jww examples/jww-fixtures/Test1.jww --out /tmp/test1_imported
cargo metadata --no-deps --format-version 1
pnpm --dir apps/viewer install
pnpm --dir apps/viewer exec playwright install chromium
pnpm --dir apps/viewer build
pnpm --dir apps/viewer test
pnpm --dir apps/viewer test:e2e
pnpm --dir apps/viewer desktop:build
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

## Desktop App

The desktop app is macOS-first and uses Tauri `invoke` commands instead of an
HTTP API. It calls the Rust CAD crates directly and computes diff as
`HEAD vs working tree` for the opened project.

```bash
pnpm --dir apps/viewer desktop:dev
pnpm --dir apps/viewer desktop:build
```

In the app, choose `Open Project` and select a CAD project directory such as
`examples/house-small`. If the project is outside Git, has no commit, or the
project files do not exist in `HEAD`, check/render still runs and diff is shown
as unavailable.

Use `Import JWW` to choose a `.jww` file and a destination parent folder. The
app creates a new `<jww_stem>_imported` project directory, opens it, and shows
the generated review. Existing output directories are not overwritten.

Viewer zoom is based on the full-paper viewBox: `100%` means the initial paper
view, and deep zoom is available up to `4096x`. The mouse wheel zooms around the
cursor, while left drag pans. Jw_cad-style left+right button gestures use
right-down for area zoom, left-up for half-scale zoom out, right-up for content
fit, left-down for previous view, and a click for recentering. `Zoom Area` and
`Previous View` toolbar buttons provide the same operations for a macOS
trackpad. Entity stroke widths remain fixed in screen pixels while zooming;
the generated SVG retains its physical print widths.

## JWW Import

JWW import is one-way. The imported NDJSON/TOML project becomes the source of
truth.

```bash
cargo run -p cad-cli -- import-jww examples/jww-fixtures/Test1.jww --out /tmp/test1_imported
```

The importer currently handles JWW line, circle, arc, ellipse, elliptical arc,
text, dimension, block, layer, pen color, and line type records. JWW curve
angles are converted from radians to the degree fields used by the CAD source.
JWW blocks are flattened into ordinary NDJSON entities at import time, and a
malformed block definition fails the whole import instead of emitting partial
geometry. Expansion is bounded to 250,000 output entities and 1,000,000
expansion steps; exceeding either limit fails the import instead of truncating
the drawing. JWW text size is converted from `size_x`,
`size_y`, and `spacing` into generated CAD text styles. JWW paper size and
single layer-group scale are reflected in `sheet.toml`; mixed layer-group scales
fall back to `1/1` with a warning. JWW header color tables are not parsed yet,
so default pen color mapping is reported as a warning.
Known unsupported records and style conflicts are reported as warnings in
`build/import-jww-report.json`. Unknown JWW classes are fatal because their
record length cannot be skipped safely.
Geometry that would fail the strict checker, such as sub-0.001mm lines or
zero-span arcs, is skipped with a warning during import. Geometry and block
records containing non-finite numeric values are also skipped before ID or
style generation. If no valid entity remains, the import fails.
Project files are staged beside the destination and published only after every
file has been written successfully. Publication uses a no-replace rename, so a
concurrent import cannot overwrite an existing destination directory.

Reflected JWW block text is preserved in CAD source with `text.mirror_y`.
Dimension text uses `dimension.text_rotation_deg` and
`dimension.text_mirror_y`; all three fields default to their untransformed
values when omitted from existing schema `0.1` NDJSON.

## Fixtures And Snapshots

- Source fixtures live under `examples/house-small` and `examples/house-small-modified`.
- Generated review artifacts live under `examples/house-small/build` and are ignored.
- Desktop bundles live under `target/release/bundle` and are ignored.
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
- in-app LLM
- cloud sync
- JWW export or lossless JWW round-trip

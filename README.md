# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

Phase 9C adds direct drafting and entity editing to the macOS-first desktop app,
including drawing selection, atomic NDJSON edits, snapping, and live review.

## Layout

```text
crates/
  cad-model
  cad-check
  cad-render-svg
  cad-diff
  cad-jww-codec
  cad-import-jww
  cad-export-jww
  cad-edit
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
cargo run -p cad-cli -- export-jww /tmp/test1_imported --drawing Test1 --out /tmp/Test1.jww
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

Desktop review watches the opened project's CAD source files continuously.
Changes to project, rule, drawing, and comment TOML/NDJSON files are debounced
for 250ms and automatically re-run check/render/diff. The toolbar reports
`Live: watching`, `refreshing`, or `error`. A transient invalid file keeps the
last successful SVG visible and retries on the next source change. Zoom,
previous view, and a still-existing entity selection are preserved across
same-project refreshes. `Re-run Review` remains available as a manual retry.
Generated `build/` and Git files are excluded from watching.
Semantic diff JSON uses schema `0.2`; besides entity changes it reports typed
project, sheet, layer, pen, style, and drawing configuration changes. When Git
diff is unavailable the diff pane remains explicitly unavailable instead of
showing the sheet SVG as a substitute.

Use `Import JWW` to choose a `.jww` file and a destination parent folder. The
app creates a new `<jww_stem>_imported` project directory, opens it, and shows
the generated review. Existing output directories are not overwritten.

The left pane exposes a Jw_cad-style layer workspace. Layer groups can be
expanded, searched, filtered to used layers, hidden, locked, or isolated.
Visibility changes apply immediately and are persisted to `rules/layers.toml`.
Layer updates carry a revision of the exact TOML bytes, preserve file mode, and
fail with a reloadable conflict if another editor changed the file first.
`Restore Visibility` restores the previous isolate state. The experimental JWW
export command in the toolbar exports a selected drawing after reporting any
strict blockers or lossy substitutions.

Desktop editing is scoped to the selected drawing. The toolbar creates line,
polyline, circle, arc, text, dimension, and point entities. The detail panel
edits typed entity properties and supports numeric move/copy/delete operations;
Move and Copy also support pointer placement. Endpoint, midpoint, and nearby
line intersection snaps use a 10px screen tolerance, with Shift temporarily
disabling snap. Hidden layers do not snap, while locked layers remain available
as snap references but cannot be edited.

Each edit includes a BLAKE3 revision of the exact `entities.ndjson` bytes.
Application-internal file-backed writers are serialized, and the revision is checked again
immediately before the atomic rename. A stale operation fails with
`revision_conflict` instead of overwriting the newer source. A non-cooperating
external writer in the final check-to-rename syscall window cannot be excluded
by the portable filesystem API. Edited lines are published atomically and unrelated raw NDJSON
lines are preserved. The app does not add its own undo history; recovery remains
Git's responsibility.

Generated AI context is published as a matched JSON/Markdown generation under
`build/ai-context/`. `build/ai-context-current.json` atomically points to the
current pair, so readers never combine files from different selections.

Viewer zoom is based on the full-paper viewBox: `100%` means the initial paper
view, and deep zoom is available up to `4096x`. The mouse wheel zooms around the
cursor, while left drag pans. Jw_cad-style left+right button gestures use
right-down for area zoom, left-up for half-scale zoom out, right-up for content
fit, left-down for previous view, and a click for recentering. `Zoom Area` and
`Previous View` toolbar buttons provide the same operations for a macOS
trackpad. Entity stroke widths remain fixed in screen pixels while zooming;
the generated SVG retains its physical print widths.

## JWW Boundary Codec

JWW remains a boundary format. Imported NDJSON/TOML becomes the source of truth,
and re-importing an exported JWW does not preserve entity IDs.
`cad-jww-codec` owns binary header, class PID, entity-list, and block-list
decoding and version 600 encoding; the importer only maps decoded records into
the CAD source schema.

```bash
cargo run -p cad-cli -- import-jww examples/jww-fixtures/Test1.jww --out /tmp/test1_imported
cargo run -p cad-cli -- export-jww /tmp/test1_imported --drawing Test1 --out /tmp/Test1.jww
```

The importer currently handles JWW line, circle, arc, ellipse, elliptical arc,
text, dimension, point, polygon solid, curve solid, block, layer, pen color,
and line type records. Entity pen attributes are preserved independently from
layer defaults. JWW layer groups, ordering, scale, visibility, lock state, and
active layer are represented in `rules/layers.toml`. Empty JWW layers are kept
when they have a name or a non-default visibility, lock, or active state. JWW curve
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

Experimental export writes JWW version 600 with CP932 strings. It supports
line, polyline expansion, arc, circle, ellipse, text, dimension, point,
polygon solid, and curve solid entities. Strict mode does not create an output
when a drawing contains an unsupported construct. `--allow-lossy` records every
skip or substitution in the export report. Existing files require `--force` in
the CLI; failed generation leaves the existing file intact. The exporter stays
Experimental until its output is compared in Windows Jw_cad. Point records and
the public `Test1.jww` fixture are covered; solid compatibility still requires a
licensed public fixture containing polygon, circle, ellipse, arc, and ring
solids before it is considered complete.

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

- DXF/PDF/DWG export
- QCAD integration
- `cadc serve`
- comment creation API
- in-app LLM
- cloud sync
- lossless JWW round-trip and stable IDs across JWW re-import

# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

Phase 5A makes schema 0.2 source/history transactions crash-recoverable,
uses `layouts.toml` as the paper authority, shares PDF export between CLI and
desktop, and gates the macOS desktop path with a real-file GUI smoke test.

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
  cad-render-pdf
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

On a fresh checkout, run `scripts/ci-bootstrap.sh` once to install the frozen
viewer dependencies and Chromium. `scripts/ci-local.sh` only verifies the
existing environment and does not rewrite tracked fixtures or install packages.

`scripts/ci-bootstrap.sh` is the only setup step. The repeatable local gate is:

```bash
scripts/ci-local.sh
```

It runs the same core gates as CI without installing packages or rewriting
fixtures. The enforced checks include:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check -p cad-desktop --no-default-features
pnpm --dir apps/viewer typecheck
pnpm --dir apps/viewer build
pnpm --dir apps/viewer test:unit
pnpm --dir apps/viewer test:e2e
pnpm --dir apps/viewer desktop:smoke
pnpm --dir apps/viewer desktop:e2e # macOS only in the local script
git diff --check
```

CI runs `macos-desktop-e2e` on `macos-15` and uploads WDIO logs and the
temporary PDF only on failure. `scripts/ci-local.sh` also verifies that the
tracked/untracked file list is unchanged across the run.

`cadc format` normalizes numeric values in drawing and comment NDJSON to three
decimal places while preserving unknown fields, line endings, trailing newline,
and file permissions. TOML files are validated but not rewritten.

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
project, layout, layer, pen, style, and drawing configuration changes. When Git
diff is unavailable the diff pane remains explicitly unavailable instead of
showing the current drawing review as a substitute.

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
Application-internal file-backed writers are serialized and use an atomic
exchange. The bytes displaced at the exchange point are revision-checked, so a
stale operation never silently overwrites an external update. Ambiguous races
and crash states retain every observed candidate under `build/.cad-recovery/`
and fail closed. Edited lines preserve unrelated raw NDJSON lines. The desktop
app keeps entity, comment, and layer undo/redo manifest snapshots under
`build/.cad-history/`; these are generated files and are independent of Git
history. A single history entry can restore all affected files, including their
newline policy, permissions, and file-existence state.

Generated AI context is published as a matched JSON/Markdown generation under
`build/ai-context/`. `build/ai-context-current.json` atomically points to the
current pair, so readers never combine files from different selections.

PDF can also be exported without the desktop app:

```bash
cargo run -p cad-cli -- export-pdf examples/house-small \
  --drawing plan_1f --out build/plan_1f.pdf
```

The command refuses an existing output unless `--force` is supplied and uses
the same active-layout renderer and source-revision checks as the desktop app.

Desktop comments can be created for the selected entity and toggled between
`open` and `resolved`. Comment writes use the comments file revision, preserve
line endings and permissions, and publish atomically; stale writes are rejected
with `revision_conflict`.

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
JWW blocks are preserved as block definitions and references by default; use
the CLI's explicit `--flatten` option when a flattened drawing is needed. A
malformed block definition fails the whole import instead of emitting partial
geometry. Expansion is bounded to 250,000 output entities and 1,000,000
expansion steps; exceeding either limit fails the import instead of truncating
the drawing. JWW text size is converted from `size_x`,
`size_y`, and `spacing` into generated CAD text styles. JWW paper size and
layout scale are reflected in `drawings/<drawing>/layouts.toml`; mixed
layer-group scales fall back to `1/1` with a warning. JWW header color tables
are not parsed yet,
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
`dimension.text_mirror_y`; all three fields are explicit in schema `0.2`.

Experimental export writes JWW version 600 with CP932 strings. It supports
line, polyline expansion, arc, circle, ellipse, text, dimension, point,
polygon solid, curve solid, deterministic block references, and simple hatch
loops. Active layout paper and scale are written to the JWW header. Strict mode does not create an output
when a drawing contains an unsupported construct. `--allow-lossy` records every
skip or substitution in the export report. Existing files require `--force` in
the CLI; failed generation leaves the existing file intact. The exporter stays
Experimental until its output is compared in Windows Jw_cad. Point records and
the public `Test1.jww` fixture are covered; solid compatibility still requires a
licensed public fixture containing polygon, circle, ellipse, arc, and ring
solids before it is considered complete.

JWW block definitions and references are retained in schema `0.2`, and export
uses the retained block list. Unsupported block constructs remain a strict
export blocker rather than a lossy
reconstruction. Lossless block round-trip is outside the current source schema.
Windows/Jw_cad compatibility is tracked in
[`examples/jww-fixtures/COMPATIBILITY_CHECKLIST.md`](examples/jww-fixtures/COMPATIBILITY_CHECKLIST.md);
completed runs use the templates under
[`examples/jww-fixtures/validation/`](examples/jww-fixtures/validation/).

PDF export uses the active layout's paper, orientation, scale, origin, margins,
and plot area and publishes the result atomically. Only visible, printable
layers are rendered; block references are transformed and simple hatch loops
are filled.

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

## Out Of Scope

- DXF/DWG export
- QCAD integration
- `cadc serve`
- in-app LLM
- cloud sync
- lossless JWW round-trip and stable IDs across JWW re-import

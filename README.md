# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

The drafting workspace uses schema 0.3, with transactional editing, canonical layouts, native
desktop watching, cached Git review, real-file macOS GUI verification, and
styled PDF output suitable for Jw_cad migration review.

Schema 0.3 is the only editable project format; no legacy reader or migration
command is provided. Repository samples and JWW imports produce this format.

## Drafting workflow

`rectangle`, `endpoint`, `stretch`, `fillet`, `chamfer`, and `rectangular_array`
are available from the toolbar and command bar. Select two lines before a
fillet/chamfer and pick the retained side of each. Stretch takes two selection
rectangle corners, a base point, and a destination; it moves only enclosed
line/polyline vertices, or whole enclosed non-line entities. Hidden and locked
entities are excluded. Circle/arc trimming and concentric offsets use the same
geometry functions as curve snapping. Offset creates new entities.

The drafting parameter panel supplies distances, angles, text, hatch settings,
and block placement. A preview is not a save: Enter or Apply confirms it and
Esc cancels it. Backspace removes the last draft point; Enter or Space while
idle repeats the last successful drafting command, never deletion or export.
The command bar also accepts `x,y`, `@dx,dy`, and `@distance<angle`.

Dimensions can be fixed or reference geometry in the same drawing/block.
Aligned, horizontal, vertical, chained, baseline, angular, radius, and diameter
dimensions use the common model evaluator for checking and output. An edit
that invalidates a reference stops for an explicit detach-or-delete decision;
external dangling references are checker errors, never silently repaired.
Dimension-driven geometry and references into a placed block are not supported.

Block creation captures selected geometry at a chosen base point. Its contents
can be staged in the block editor, saved for all placements, or duplicated as
an independent definition. Placement offers rotation, positive uniform scale,
and reflection. Hatch vertices are completed with Enter, or use enclosed-region
pickup; gaps are not healed. Curved hatch boundaries become polygon loops within
the documented geometry tolerance and expansion limit, not associative boundaries.

See [the acceptance drawing and checklist](docs/cad-acceptance.md) for automatic
checks and the remaining Jw_cad display/print acceptance work.

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
3. Run `cargo run -p cad-cli -- check examples/house-small --target cad --format json --out examples/house-small/build/check.json`.
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

`New Project` creates a validated canonical project without overwriting an
existing folder. The template supports A3/A4, portrait/landscape, and scales
1:20, 1:50, 1:100, and 1:200; the default is A3 landscape at 1:100. The drawing
selector can add a blank drawing or duplicate a drawing with new entity IDs.
The local unsigned arm64 application is produced under
`target/release/bundle/macos/` by `desktop:build`.

Desktop review watches the opened project's CAD source files continuously.
Changes to project, rule, drawing, and comment TOML/NDJSON files are debounced
for 250ms and automatically re-run check/render/diff. The toolbar reports
`Live: watching`, `refreshing`, or `error`. A transient invalid file keeps the
last successful SVG visible and retries on the next source change. Zoom,
previous view, and a still-existing entity selection are preserved across
same-project refreshes. `Re-run Review` remains available as a manual retry.
Generated `build/` and Git files are excluded from watching.
Semantic diff JSON uses schema `0.3`; besides entity changes it reports typed
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
Move and Copy also support pointer placement. Endpoint, midpoint, nearby line
intersection, center, quadrant, nearest, and perpendicular snaps use a 10px
screen tolerance. F3 toggles snap, F8 toggles orthogonal input, and Shift
temporarily disables snap. The lower command bar accepts command names plus
`x,y`, `@dx,dy`, and `@distance<angle`. Shift-drag selects by window (left to
right) or crossing (right to left). Hidden layers do not snap, while locked
layers remain available as snap references but cannot be edited.

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
current pair, so readers never combine files from different selections. Identical
content reuses its existing generation; `generated_at` records when that generation
was first created. Earlier generations are retained, including generations created
before content reuse was introduced. Incomplete or mismatched generations are
rejected without replacing the current manifest.

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

JWW remains a boundary format. Imported NDJSON/TOML becomes the editable source
of truth, while the exact input bytes and their BLAKE3/SHA-256 provenance are
retained under `interop/jww/`. Editable v600 imports also write a hash-verified
`records.ndjson` mapping; imports that cannot establish a one-to-one mapping
are marked `exact_only`. Re-importing a generated JWW does not preserve
entity IDs.
`cad-jww-codec` owns binary header, class PID, entity-list, and block-list
decoding and version 600 encoding; the importer only maps decoded records into
the CAD source schema.

```bash
cargo run -p cad-cli -- import-jww examples/jww-fixtures/Test1.jww --out /tmp/test1_imported
cargo run -p cad-cli -- inspect-jww examples/jww-fixtures/Test1.jww
cargo run -p cad-cli -- check /tmp/test1_imported --drawing test1 --target jww-v600 --format json --out -
cargo run -p cad-cli -- export-jww /tmp/test1_imported --drawing test1 --out /tmp/Test1.jww --report /tmp/Test1-report.json
cargo run -p cad-cli -- extract-original-jww /tmp/test1_imported --out /tmp/Test1-original.jww
```

After import, the canonical TOML and NDJSON files are the editable source of
truth. AI agents edit those files directly, keep stable entity IDs, and run the
CAD and JWW-target checkers after each edit batch. `interop/`, provenance,
history, transaction, recovery, and generated files are not editable source.
Repository-specific agent rules are in [`AGENTS.md`](AGENTS.md).

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
layer-group scales fall back to `1/1` with a warning. JWW v600 display and
print color tables and print-width values are retained in style definitions.
`rgb` drives the SVG display, while optional `print_rgb` drives PDF and the JWW
print palette. Import/export tests preserve all ten basic palette entries even
when a color is not referenced by an entity.
Known unsupported records and style conflicts are reported as warnings in
`build/import-jww-report.json`. A v600 file containing an unknown class, or a
file with an unsupported version, is imported as a read-only preservation
project: its exact bytes can be extracted, but CAD source edits are rejected.
Malformed files still fail import.
Geometry that would fail the strict checker, such as sub-0.001mm lines or
zero-span arcs, is skipped with a warning during import. Geometry and block
records containing non-finite numeric values are also skipped before ID or
style generation. If no valid entity remains, the import fails.
Project files are staged beside the destination and published only after every
file has been written successfully. Publication uses a no-replace rename, so a
concurrent import cannot overwrite an existing destination directory.

Reflected JWW block text is preserved in CAD source with `text.mirror_y`.
Dimension text uses `dimension.text_rotation_deg` and
`dimension.text_mirror_y`; all three fields are explicit in schema `0.3`.
For a mapped existing JWW dimension, strict preservation accepts only a
displayed-value change and retains the original dimension line, text geometry,
SXF fields, auxiliary lines, and auxiliary points. Normal best-effort export
regenerates an edited dimension when geometry, offset, style, layer/pen,
rotation, or mirroring changes and records the approximation in its JSON
report.

Preserve export reads the original and record provenance as one hash-verified
snapshot and rechecks the canonical source manifest immediately before atomic
publication. It also rejects invalid projects, dangling or duplicate block
definition numbers, and preserved block base-point changes without replacing
an existing output.

Best-effort export writes JWW version 600 with CP932 strings. It supports
line, polyline expansion, arc, circle, ellipse, text, dimension, point,
polygon solid, curve solid, deterministic block references, and hatch loops.
Parallel and cross hatch families use model-space millimetres for pitch and are
expanded to clipped JWW lines with an explicit compatibility warning. Active
layout paper and scale are written to the JWW header. Strict mode does not create an output
when a drawing requires an approximation. Normal mode records every expansion,
substitution, or omission in the export report; `--strict` promotes those
conditions to blockers. Existing files require `--force` in the CLI, and failed
generation leaves the existing file intact. Point records and the public
`Test1.jww` fixture are covered; solid compatibility still requires a licensed
public fixture containing polygon, circle, ellipse, arc, and ring solids.
Desktop export writes the JSON compatibility report beside the selected JWW as
`<name>.jww.report.json`; disabling `Allow lossy export` also blocks all
approximations.
Paired exports reject aliased destinations and non-regular target files. On a
publication failure they restore the previous files; if recovery itself fails,
the error lists retained backup paths for manual recovery. This does not provide
two-file atomicity across power loss.

Breaking change for canonical v0.3 hatch data: `pattern` must be `solid`,
`parallel`, or `cross`, and `fill` must name a color in `rules/styles.toml`.
Existing arbitrary pattern names and omitted/null fills are rejected; there is
no automatic migration or fallback. Correct the entity and field identified by
the diagnostic before checking, rendering, or exporting again. Each patterned
hatch family is limited to 20,000 candidate lines; excessive density blocks
rendering and export, including best-effort JWW. Increase its model-space pitch
or reduce its extent.

JWW provenance is selected automatically. With no relevant source change it
publishes the original JWW byte-for-byte. For an edited compatible project it
retains the original v600 header and untouched records, while edited entities
may replace one original record with multiple generated records. If header,
layout, or palette changes cannot be merged, normal mode generates a complete
v600 file and records `preservation_fallback_generated`; `--strict` blocks the
fallback. This is measured file-format compatibility, not a claim of complete
Jw_cad compatibility.

JWW block definitions and references are retained in schema `0.3`, and export
uses the retained block list. Unsupported block constructs remain a strict
export blocker rather than a lossy
reconstruction. Lossless block round-trip is outside the current source schema.
File-format compatibility is gated by the hashed corpus in
[`examples/jww-fixtures/manifest.json`](examples/jww-fixtures/manifest.json),
byte-exact unchanged export, record inventory, and semantic re-import checks.
External application checks may be recorded as optional evidence, but Windows
or Jw_cad execution is not a release requirement.

PDF export uses the active layout's paper, orientation, scale, origin, margins,
and plot area and publishes the result atomically. Only visible, printable
layers are rendered; block references are transformed, solid hatches are
filled, and parallel/cross hatches are clipped to their even-odd loops. PDF
circles, arcs, and ellipses use cubic Bézier path segments instead of visible
polyline faceting. PDF stroke color, physical line width, dash pattern, fill color,
text size/width/spacing/alignment, dimension geometry, and curve solids use the
same canonical style data as SVG. Japanese text uses an OFL-licensed M+ 1p
TrueType subset embedded as a CID font with a ToUnicode map. PDF appearance and
text extraction therefore do not depend on fonts installed on the viewer.

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

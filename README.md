# CAD

Git-native CAD source tooling for Jw_cad-style 2D architectural drafting.

## Current Phase

The drafting workspace uses schema 0.3, with transactional editing, canonical layouts, native
desktop watching, cached Git review, real-file macOS GUI verification, and
styled PDF output suitable for Jw_cad migration review.

Schema 0.3 is the only editable project format; no legacy reader or migration
command is provided. Repository samples and JWW imports produce this format.

## Drafting workflow

`rectangle`, `endpoint`, `stretch`, `scale`, `fillet`, `chamfer`, and `rectangular_array`
are available from the toolbar and command bar. Select two lines before a
fillet/chamfer and pick the retained side of each. Stretch takes two selection
rectangle corners, a base point, and a destination; it moves only enclosed
line/polyline vertices, or whole enclosed non-line entities. Hidden and locked
entities are excluded. Circle/arc trimming and concentric offsets use the same
geometry functions as curve snapping. Offset creates new entities.

Scale transforms selected geometry uniformly about a picked base point with a
finite positive factor. Radii, dimension offsets, and per-entity block, hatch,
and point scales follow it; shared paper-unit text and line styles stay unchanged.
Referenced dimensions continue following their source geometry. Preview, layer
permissions, source revision checks, and Undo/Redo use the common edit transaction.

Use selected attributes acquires the selected entity's layer and pen plus its
applicable text/dimension style or fill for subsequent drawing. It copies no
geometry and writes no source. The acquired layer overrides the active layer;
choosing another active layer drops that override. Clear acquired attributes
restores defaults. Both actions cancel a current draft, and missing references
discard the acquired settings. New rectangles and block references support an
explicit pen as well as ordinary created primitives.

Desktop's Text search and batch edits panel searches literal, case-sensitive
annotation text in the active drawing or selected entities. It shows the match
count, replacement text and target style before preview/apply. Style-only changes
preserve the annotation content and geometry. Changed inputs or sources discard
the candidate; dimension labels and block contents are excluded.

CLI style-only changes use `generate PROJECT --drawing NAME --text-style
--request REQUEST.json --out EDIT.json` with `{"style":"STYLE","entity_ids":[]}`.
An empty ID list targets every annotation in that drawing. Use the resulting
revision-bound request with `edit` to preview and apply the transaction.

Building tools' `calculated_text` evaluates finite scalar arithmetic with `+`,
`-`, `*`, `/`, parentheses and scientific notation, then formats an annotation
with 0–12 decimal places and an explicit prefix/suffix. Division by zero,
overflow, underflow and excessive nesting are errors. It uses binary f64
arithmetic; units are supplied by the user. The resulting text does not retain
an expression dependency or recalculate automatically.

Building tools can create a circle of a specified radius tangent to two selected
straight lines, choosing the center nearest a supplied point. The warning and
preview show that tangency uses infinite line extensions. Parallel or nearly
parallel lines and coordinates with insufficient precision are rejected.

The drafting parameter panel supplies distances, angles, text, hatch settings,
and block placement. A preview is not a save: Enter or Apply confirms it and
Esc cancels it. Backspace removes the last draft point; Enter or Space while
idle repeats the last successful drafting command, never deletion or export.
The command bar also accepts `x,y`, `@dx,dy`, and `@distance<angle`.

Dimensions can be fixed or reference geometry in the same drawing/block.
Endpoints, polyline vertices and segment midpoints, curve midpoints, centers,
and quadrants can follow their source geometry; other picked points remain fixed.
Aligned, horizontal, vertical, chained, baseline, angular, radius, and diameter
dimensions use the common model evaluator for checking and output. An edit
that invalidates a reference stops for an explicit detach-or-delete decision;
external dangling references are checker errors, never silently repaired.
Dimension-driven geometry and references into a placed block are not supported.

Block creation captures selected geometry at a chosen base point. Its contents
can be staged in the block editor (including new lines, polylines, circles, arcs,
ellipses, text, points, and hatches), saved for all placements, or duplicated as
an independent definition. Placement offers rotation, positive uniform scale,
and reflection. Hatch vertices are completed with Enter, or use enclosed-region
pickup; gaps are not healed. Curved hatch boundaries become polygon loops within
the documented geometry tolerance and expansion limit, not associative boundaries.

See [the acceptance drawing and checklist](docs/cad-acceptance.md) for automatic
checks and the remaining Jw_cad display/print acceptance work.

See [the Jw_cad gap analysis and Git integration proposals](docs/jwcad-gap-analysis.md)
for implementation candidates and their acceptance conditions. For agent-driven
drawing work, use [cad-drafting](.agents/skills/cad-drafting/SKILL.md) to edit
canonical source and [cad-review](.agents/skills/cad-review/SKILL.md) to review
drawing revisions.

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
and file permissions. Recorded comment bindings retain their original numeric values
and hashes. TOML files are validated but not rewritten.

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

Print Preview renders the same checked PDF bytes as export, including printable
layers, print colors, clipping, margins, and embedded fonts. It creates no output
file and refreshes when the reviewed source changes. PDF.js draws the generated
page on canvas, including in the macOS WebKit desktop app.

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

New comments also retain the original entity and a normalized source hash. Creation
checks the displayed drawing revision; later status changes keep the binding unchanged.
Desktop shows entity changes, deletion, moves, broader canonical source changes and
unverified metadata. The source hash covers all drawings and definitions, so unrelated
source edits conservatively mark the context changed. Older comments remain unbound.
The recorded Git HEAD is the commit observed at creation; the hash includes working edits.

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

## Drawing tools, Git review, and exchange

Desktop provides doors, windows, double lines, centerlines, ellipses, Bezier approximation,
external-point circle tangents, division points, and measurements. Generated geometry uses
an explicit preview and an undoable Batch edit. Measurements report unsupported entities
and exclude them from totals. Nested block instances include their transforms. Choose additive
area totals or a per-drawing union that excludes overlaps; curved unions report their
polygon approximation tolerance and area error estimate. Length totals remain additive.

CLI examples (prefix each command with `cargo run -p cad-cli --`):

```text
entity-history PROJECT --entity ID --revision HEAD --limit 50 --out build/entity-history.json
entity-blame PROJECT --entity ID --revision HEAD --limit 50 --out build/entity-blame.json
review-bundle PROJECT --base HEAD --head worktree --out NEW_DIRECTORY
verify-review-bundle NEW_DIRECTORY
git-diff PROJECT --base HEAD --head index --format json --out build/staged.diff.json
git-stage PROJECT --drawing NAME --entity ID --out build/stage-plan.json
git-stage PROJECT --drawing NAME --entity ID --apply --expected-plan HASH --out build/stage-result.json
git-commit PROJECT --message-file MESSAGE.txt --out build/commit-plan.json
git-commit PROJECT --message-file MESSAGE.txt --apply --expected-plan HASH --without-hooks --out build/commit-result.json
git-merge-plan PROJECT --base BASE --ours worktree --theirs REF --out NEW_PROJECT
git-merge-apply PROJECT --drawing NAME --base BASE --theirs REF --out build/merge-apply-plan.json
git-merge-apply PROJECT --drawing NAME --base BASE --theirs REF --apply --expected-plan HASH --out build/merge-apply-result.json
copy-entities PROJECT --drawing NAME --entity ID --base-point 0,0 --out PART.cadpart.json
paste-entities PROJECT --drawing NAME --part PART.cadpart.json --at 10000,20000 --out build/paste-plan.json
paste-entities PROJECT --drawing NAME --part PART.cadpart.json --at 10000,20000 --apply --expected-plan HASH --out build/paste-result.json
export-set PROJECT --page DRAWING@LAYOUT --out NEW_DIRECTORY
verify-export-set NEW_DIRECTORY
measure PROJECT --drawing NAME --format csv --out build/measure.csv
measure PROJECT --drawing NAME --area-mode union --curve-tolerance-mm 0.1 --out build/union.json
generate PROJECT --drawing NAME --request build/input.json --out build/edit.json
edit PROJECT --request build/edit.json --out build/preview.json
edit PROJECT --request build/edit.json --apply --out build/applied.json
export-dxf PROJECT --drawing NAME --out OUTPUT.dxf --report REPORT.json
import-dxf INPUT.dxf --out NEW_PROJECT --report IMPORT_REPORT.json
import-jws INPUT.jws --coordinate-scale 100 --out NEW_PROJECT --report IMPORT_REPORT.json
copy-jws INPUT.jws --coordinate-scale 100 --out PART.cadpart.json --report JWS_REPORT.json
list-parts PART_FOLDER --offset 0 --limit 12 --coordinate-scale 100 --out LIBRARY_REPORT.json
```

`git-merge-apply` とDesktopのMerge CAD source changesは、共通base・作業中・incoming commitを固定し、全変更ファイル・項目競合・設定差分・CAD検査を提示する。確認したplan hashで既存正本へ適用し、プロジェクト単位のUndo/Redoで元のバイト列へ戻せる。Git branchのmergeやstage・commitは行わない。正本・index・HEAD/branch・来歴の変更、実行中のGit操作、競合・検査エラー、comments/interop変更、ファイル追加・削除は適用を阻止する。別の図面や共有設定も変更対象になり得るので、全ファイルを確認する。候補資料の出力には従来の `git-merge-plan` を使える。

DesktopのCAD clipboard and partsで選択図形をコピーし、別図面・別プロジェクトを開いて貼付できる。コピー基点と貼付点はモデルmm。参照寸法は参照先を一緒にコピーするか、選択外の参照を固定座標へ解除するか、拒否するかを選ぶ。貼付は新IDを割り当て、共有・入れ子blockを別の定義として取り込み、異なる同名layer/pen/style/color/groupを改名する。同じ設定は再利用し、既存の定義と原文の図形行を保持する。可視性・ロックも保持するため、警告と定義対応表を確認する。全正本のCAS・CAD検査・プレビューを経て一括Undo/Redoへ入る。

`copy-entities` の `--entity` は繰り返せる。`--dimensions include-references|detach-external|reject-external` は既定でinclude。保存形式は `cad-clipboard/1` のJSONで、JWW/JWK/JWSのバイナリ形式ではない。comments・JWW来歴・用紙変更は貼付しない。新規部品ファイルは既存ファイルへ上書きせず、Desktopで保存・読込できる。上限64 MiB、図面図形とblock内図形はそれぞれ10万件、block定義は1万件。貼付は `--rotation-deg` と `--scale`（正の均等倍率）を指定でき、基点を貼付点へ移して拡大・回転する。shared blockの内部座標と紙上単位の共有styleは保持する。水平/垂直の参照寸法は90度単位に限り、90度回転で測定軸を交換する。その他の回転は阻止し、長さを黙って変えない。Part folder paletteで部品フォルダーの一覧・サムネイル・ページ切替から選択できる。`.cadpart.json` と `.jws` を対象とし、JWS係数を明示する。選択時に一覧のfile hashを再検査し、変更があれば再読込する。読み取り専用・非再帰で、一ページ最大24件（Desktopは12件）。展開・SVGサイズの上限に達した部品はサムネイルを省略し、検査済み部品の選択は可能。未対応JWSも理由と全報告を表示する。非均等倍率とJWKは残る。

Git comparisons freeze commit, index, or worktree sources without a checkout. Selective
staging validates the complete CAD index candidate and dependencies, preserves unrelated
staged content, and requires the reviewed plan hash before replacing the locked index. Merge plans
create checked candidates and retain conflicts; they do not replace the original project.
Export sets contain ordered SVGs, a multi-page PDF, source identity, and output hashes.

Field attribution reports the last first-parent commit, author and value for normalized
entity fields, grouping coordinate arrays. Drawing moves preserve the earlier field origins;
unchanged-entity display dependencies are listed separately. Unresolved origins, decode gaps
and scan limits remain visible, and `reliable=false` marks incomplete history. Desktop reads
committed HEAD fields through **Load field origins** and can compare a field's parent/commit.
This does not include working edits, block-internal entity IDs, or all merge-parent ancestry.

Desktop **Commit staged changes** previews the complete repository index, including
unrelated staged files, the configured author/committer and the staged patch. It checks
the selected CAD project, then requires accepting an unsigned commit without Git hooks.
CLI uses the same plan hash and explicit `--without-hooks` policy. Working files and
index bytes are preserved. Index changes, branch switches and active Git operations
block publication; detached HEAD and incomplete/oversized patch previews are unsupported.
Initial commits are supported. Other CAD projects and non-CAD staged content require
separate review. Git hooks/signing and merge/rebase commits use the normal Git workflow.
Preview may create unreferenced tree objects, and a failed publication can retain a
commit object whose OID is reported for inspection. No push is performed.
Reference publication uses a prepared [Git reference transaction](https://git-scm.com/docs/git-update-ref)
to verify the checked-out branch while its reference locks are held.

Review bundles include every drawing and named layout from both pinned inputs, both PDFs,
per-layout SVGs, per-drawing semantic diff SVGs, configuration snapshots, unchanged raw
comment files, CAD checks, and JWW target-check warnings. They retain a blocked report
when sources fail parsing or validation. These checks do not certify a JWW export or a
receiving CAD application. New Desktop comments record their original entity and normalized
canonical source identity. Each pinned input evaluates these immutable bindings, reporting
changed, deleted, moved or unverified entities and broader source changes. Legacy comments
remain unbound. Full comment schema and anchor placement are not validated by the bundle.
Verification checks artifact integrity; it does not authenticate
a manifest. Both revisions must contain a CAD project, and its directory must exist in the
current checkout; whole-project creation/deletion comparisons are not yet supported.

The manual **CAD source review** GitHub Actions workflow runs this separately from software
CI and retains artifacts even on a blocked check. For a local/other CI run, set
`CAD_REVIEW_PROJECT`, `CAD_REVIEW_BASE`, `CAD_REVIEW_HEAD`, and an unused
`CAD_REVIEW_OUT`, then run `bash scripts/cad-review-ci.sh`. It performs no checkout or
Git publication. PR comments and pushes require a separate explicit action.

DXF exchange currently supports planar ASCII DXF and exports R2013 model space in mm.
Supported primitives include lines, straight polylines, circles, arcs, ellipses, text,
points, block inserts, and SOLID triangles. Dimensions and hatches expand to primitives;
curve fills use a reported polygon approximation. Unknown records, unsupported transforms,
paper-space entities, and 3D geometry block import instead of silently disappearing.
Unitless input requires `--unit-mm MM_PER_DXF_UNIT`. Import creates a new A3 landscape
project at 1:100 with new IDs and substituted fonts. Every exchange requires a JSON report;
`export-dxf --strict` blocks reported substitutions and semantic reductions, including the
loss of canonical page layouts. Receiving-CAD visual comparison remains necessary.

Desktop **Import DXF** creates and opens a checked new project, with an optional
millimeters-per-unit override. **Export DXF** selects a drawing and strict policy.
Both show conversion warnings and blockers and require a saved external report.
Existing outputs, canonical source, provenance, history and Git metadata are protected.
Import publishes the validated directory atomically after saving the report.
The report's `status=ready` describes prepared conversion, not proof of publication;
a later file-publication error retains the report and must be resolved before delivery.

JWS import supports the validated 351, 420 and 600 archive layouts. An explicit
`--coordinate-scale` selects model millimetres per stored coordinate unit; 100 is
appropriate when the intended interpretation is paper coordinates at 1:100.
The importer expands referenced blocks, checks the whole canonical candidate,
and atomically publishes a new project. Unknown classes, unexplained trailing bytes,
recursive blocks and dropped or invalid geometry block publication while preserving
the JSON report. JWS lacks the complete drawing environment, so palette, layer names,
print settings, fonts and sheet settings are substituted and reported. Original group
scales and placement origin remain in the report. Mixed scales are not reinterpreted
automatically. This is not a lossless JWS/JWW round trip. See
[the JWS validation record](docs/jws-compatibility.md) for corpus coverage and limitations.

`copy-jws` converts the same checked geometry into a new portable part and saves its full compatibility report first. Desktop can load JWS into the CAD clipboard with an explicit coordinate scale, display the full report, then preview and paste at the reported origin. The report remains visible across drawing/project changes. Blocked conversion preserves the existing clipboard. Input and part limits are 64 MiB; existing files and protected paths in other CAD projects are also rejected. Original JWS bytes remain external; no JWS export is provided.

The [drafting skill](.agents/skills/cad-drafting/SKILL.md) documents source edits and generated
requests; the [review skill](.agents/skills/cad-review/SKILL.md) documents revision comparison
and exchange checks. Remaining parity work is tracked in [the implementation plan](plans/011-jwcad-parity.md).

Prism-based 2.5D projection, flat-ground sun shadows and horizontal sky-view diagrams
are available in Building tools and `generate --analysis-out NEW_REPORT.json`.
`analyze-massing` also accepts explicit footprints, individual heights and bases without
editing a CAD project. Reports retain calculation inputs, results, source revisions and
the drawing generation request. Generated 2D geometry does not update automatically.
These calculations use explicit solar angles and finite azimuth sampling, and do not
assess statutory compliance. See [methods and limits](docs/massing-calculations.md).

Independently scaled views can share one SVG/PDF sheet. Desktop **Scaled views on
one sheet** and `sheet-viewports` preview and apply clipped source-drawing views
without changing model geometry. Placement and size use paper millimetres; each view
has its own model origin, scale and optional layer filter. Layout updates participate
in Undo/Redo and checked source revisions. Git comparisons report source-view changes;
the entity diff overlay still uses model coordinates. JWW export blocks these layouts
because its clipping/placement mapping remains unvalidated. See
[sheet settings and review limitations](docs/sheet-viewports.md).

Annotations support explicit upright vertical columns in SVG/PDF and semantic review.
Desktop text creation and batch edits select the writing direction; CLI uses
`generate --text-edit` with `set_writing_mode`. IDs, text and styles remain intact,
and reviewed edits support Undo/Redo. JWW/DXF expand the columns to positioned
single-character records with compatibility reports; strict export blocks the expansion.
Vertical font shaping and native JWW vertical-glyph mapping remain unvalidated.
See [annotation placement and limits](docs/annotation-writing.md).

## Out Of Scope

- DWG export and complete DXF/SXF compatibility
- QCAD integration
- `cadc serve`
- in-app LLM
- cloud sync
- lossless JWW round-trip and stable IDs across JWW re-import

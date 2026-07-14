# Plan 002: Make history, project switching, and export revisions coherent

> **Executor instructions**: Follow the sequence below and preserve the public
> Tauri/TypeScript wire shapes unless a step explicitly says otherwise. Stop
> on drift or a failed invariant; do not work around it with a timeout.
>
> **Drift check (run first)**: `git diff --stat cc8694e..HEAD -- crates/cad-edit/src/lib.rs apps/viewer/src-tauri/src/lib.rs apps/viewer/src/main.tsx apps/viewer/src/desktop-editor.ts apps/viewer/src/desktop-live-review.ts apps/viewer/src/artifacts.ts`.

## Status

- **Implementation**: COMPLETED (2026-07-13)

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/001-safe-atomic-project-writes.md`
- **Category**: bug, tech-debt
- **Planned at**: commit `cc8694e`, 2026-07-13

## Why this matters

History, live review, project switching, and export all operate asynchronously.
The current implementation can expose a project-wide layer edit only in the
drawing context that created it, let a startup load overwrite a later manual
selection, and generate a PDF from bytes that changed during export. These are
state-integrity failures even when each individual command succeeds.

## Current state

- `stage_history_transaction` currently stores all entries under the shared
  `build/.cad-history/` root in `crates/cad-edit/src/lib.rs:448-455`, with the
  drawing and scope recorded only in metadata.
- Layer mutation passes the current drawing while declaring scope `project` in
  `apps/viewer/src-tauri/src/lib.rs:1331-1347`. The shared index makes the
  entry visible across contexts, but `clear_drawing_history` can remove shared
  project entries when invoked for one drawing; the scope semantics need an
  explicit test and implementation guard.
- `main.tsx:327-340` loads the last project without a generation check, while
  `chooseProject` starts an independent asynchronous open.
- PDF transport (`apps/viewer/src-tauri/src/lib.rs:532-546`) has no expected
  source revision; `pdfSequenceRef` only suppresses stale UI responses.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Typecheck/build | `pnpm --dir apps/viewer build` | exit 0 |
| Unit tests | `pnpm --dir apps/viewer test` | all runnable tests pass |
| E2E | `pnpm --dir apps/viewer test:e2e` | all pass |
| Rust tests | `cargo test --workspace --all-features` | all pass |

## Scope

**In scope**

- `crates/cad-edit/src/lib.rs` history root/index/state logic
- `apps/viewer/src-tauri/src/lib.rs` history/export command coordination
- `apps/viewer/src/main.tsx` project-load and action generations
- `apps/viewer/src/desktop-editor.ts`, `desktop-live-review.ts`, and
  `artifacts.ts` only where the existing contract requires a field or guard
- Corresponding Rust/TypeScript tests

**Out of scope**

- New geometry commands
- Undo/Redo snapshot format redesign beyond project-scope placement
- PDF rendering style fidelity
- Rewriting the entire viewer component tree

## Steps

### Step 1: Preserve project-scope entries while clearing drawing history

Keep the shared `build/.cad-history/` index, but update clear/list/undo/redo
semantics so `scope == "project"` entries remain visible from every drawing
context and are not accidentally deleted by a drawing-only clear. A clear from
any context may explicitly clear project history only through a separate, named
project-history operation; do not silently broaden the existing drawing clear.
Add a migration-free test fixture; malformed entries must block history rather
than being silently moved.

**Verify**: `cargo test -p cad-edit --all-features` passes tests that create a
layer/project entry in drawing A, list and undo it from drawing B, clear drawing
A without deleting the project entry, and reject a stale manifest without
changing any file.

### Step 2: Add a generation guard to project opening

Introduce a monotonically increasing open generation in `main.tsx`. Increment
it when startup loading begins and when the user chooses a project. After every
`await` in `openDesktopProject` and its watcher/review setup, apply state only
if the generation and project path still match. Invalidate the previous
watcher, review queue, history sequence, command, selection, and preview when a
new generation starts.

**Verify**: Add a unit test with deferred promises where the startup project
resolves after a manually selected project; the manual selection remains
visible. `pnpm --dir apps/viewer test` passes (or reports only the known
Chromium sandbox launch limitation).

### Step 3: Bind PDF export to an immutable source revision

Extend the internal request path, without changing the user-facing button, to
capture the current drawing/layout file revisions before export. The Rust
command must validate those revisions immediately before reading/rendering and
again before publish. On mismatch, return `revision_conflict` and keep the
existing PDF. The UI must treat this as a normal stale request: refresh review
and do not show the result as current.

**Verify**: Add a Tauri/Rust test that changes the entities or layouts file
during export and asserts the old PDF remains byte-identical. Add a TypeScript
test that ignores a stale export response after a drawing switch.

### Step 4: Re-synchronize all dependent state after history actions

After successful Undo/Redo, reload the full review snapshot, history state,
comments, layers, selection, and editor revision as one guarded sequence. Do
not patch only the editor revision and then enqueue an unguarded review. On
conflict or corrupt history, disable controls until the guarded reload finishes.

**Verify**: Playwright/unit coverage for edit → undo → redo, drawing switch
during undo, project-wide layer history, and conflict recovery.

## Done criteria

- [x] Project-scope layer history is available from every drawing context.
- [x] A late startup/open/review/export result cannot overwrite newer project
      state.
- [x] Export rejects source changes and preserves the previous output.
- [x] Undo/Redo refreshes all dependent UI state under one sequence guard.
- [x] Rust and viewer verification commands pass.

## STOP conditions

- The existing history index format cannot represent separate project and
  drawing stacks without changing persisted metadata; stop and report the
  required format decision.
- A transport change would break a documented external consumer; stop and
  report instead of adding a silent compatibility shim.
- A stale-result test passes only by adding arbitrary sleeps.

## Maintenance notes

Every new asynchronous command must capture the relevant project/drawing/layout
generation and check it after each await. Every new history scope must define
its storage root, context visibility, and expected file manifest explicitly.

# Plan 001: Make every project write safe and atomic

> **Executor instructions**: Follow this plan in order. The repository is a
> CAD editor that writes user-owned source files and generated artifacts. Do
> not widen the public wire contracts. If a STOP condition occurs, stop and
> report instead of improvising.
>
> **Drift check (run first)**: `git diff --stat cc8694e..HEAD -- apps/viewer/src-tauri/src/lib.rs crates/cad-edit/src/lib.rs crates/cad-render-pdf/src/lib.rs crates/cad-cli/src/main.rs`.
> Also inspect the uncommitted diff because the planned baseline is dirty.

## Status

- **Implementation**: COMPLETED (2026-07-14)

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: none
- **Category**: security, bug, tech-debt
- **Planned at**: commit `cc8694e`, 2026-07-13

## Why this matters

The application currently has several independent write paths. PDF export uses
an existence check followed by an unconditional rename, comments re-check a
revision and then rename without a compare-and-swap guarantee, and CLI reports
write directly with `fs::write`. AI-context generation also follows a project
`build` symlink. These paths can lose concurrent edits, overwrite an output that
was created after the check, or write outside the opened project. This plan
creates one safe publish primitive and applies it to every in-scope writer.

## Current state

- `crates/cad-edit/src/lib.rs` already contains private atomic replacement and
  permission helpers, but comments/layers in `apps/viewer/src-tauri/src/lib.rs`,
  PDF in `crates/cad-render-pdf/src/lib.rs`, and CLI output in
  `crates/cad-cli/src/main.rs` do not share them.
- PDF `publish` checks `output.exists()` at `crates/cad-render-pdf/src/lib.rs:543`
  and then calls `staging.persist(output)` at `:560`, allowing a TOCTOU replace.
- Comment publish rechecks at `apps/viewer/src-tauri/src/lib.rs:703-710` and
  unconditionally persists at `:785-787`; rollback uses direct `fs::write`.
- Layer rollback in `apps/viewer/src-tauri/src/lib.rs:1390-1403` also uses
  direct writes after a rename.
- CLI `write_json_report` and `write_text_file` at
  `crates/cad-cli/src/main.rs:259-282` truncate the destination before writing.
- `write_ai_context_for_path` creates and persists under `project_path/build`
  at `apps/viewer/src-tauri/src/lib.rs:1421-1563` without rejecting a symlinked
  parent. The selected-entity-empty path invokes this automatically.

Match the existing `Result`/`thiserror` style and preserve raw bytes, newline
style, permissions, and no-partial-output behavior already tested in
`crates/cad-edit/src/lib.rs`.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| Rust tests | `cargo test --workspace --all-features` | all pass |
| Diff hygiene | `git diff --check` | no output |

## Scope

**In scope**

- `crates/cad-edit/src/lib.rs` and its tests
- `crates/cad-render-pdf/src/lib.rs` and its tests
- `crates/cad-cli/src/main.rs` and its tests
- `apps/viewer/src-tauri/src/lib.rs` and its tests
- Cargo manifests only when needed to reuse the shared writer

**Out of scope**

- New CAD entities or commands
- PDF visual fidelity (pen, dash, text metrics)
- Changing user-selected import/export path semantics
- Git commits, pushes, or deleting existing user changes

## Steps

### Step 1: Define a shared safe publish primitive

Extract or expose a small filesystem API from the existing `cad-edit` atomic
helpers. It must support: expected-byte revision checking immediately before
publish, replace-existing mode, no-replace mode, permission application to the
staging file, same-directory staging, and rollback metadata. Return typed errors
that callers can map without string matching. Do not use a check-then-rename
implementation for no-replace mode; use a platform-appropriate no-replace
operation or fail closed when it cannot be guaranteed.

Add a project-internal destination guard that canonicalizes the project root,
rejects symlinked `build`/generation parents, and verifies generated paths stay
under that root. The project root itself may be a user-selected symlink, but an
interior `build` or `ai-context` symlink must be rejected.

**Verify**: `cargo test -p cad-edit --all-features` passes, including tests for
replace, no-replace race simulation, permissions, CRLF bytes, and symlinked
`build` rejection.

### Step 2: Route comments and layers through the primitive

Replace the duplicated staging/persist/rollback code in
`mutate_comments_locked` and `update_layer_rules_for_path_locked` with the
shared primitive. Preserve each file's exact before bytes, line endings, final
newline policy, permissions, and file-existence state. A failed history commit
must restore through the same atomic path and must not silently fall back to
`fs::write`.

Add a regression test that edits the file between the preflight revision read
and publish; the operation must return `revision_conflict` and leave the
external bytes untouched.

**Verify**: `cargo test -p cad-desktop --all-features` passes, including comment
and layer conflict/rollback tests.

### Step 3: Fix PDF and CLI publication

Make PDF export use the no-replace/replace mode of the shared primitive while
keeping `overwrite` behavior unchanged. Add a test proving an output created
after the initial check is not replaced when `overwrite=false`.

Change CLI JSON/SVG/report writers to stage in the destination directory,
flush/write fully, and atomically rename. Existing output must remain valid if
serialization, write, or rename fails.

**Verify**: `cargo test -p cad-render-pdf -p cad-cli --all-features` passes and
existing-output failure tests assert byte-for-byte preservation.

### Step 4: Harden AI-context generated paths

Call the destination guard before every `build`, `build/ai-context`, and
manifest publish. Add tests for a normal project, a missing build directory,
and a `build` symlink pointing outside the project. The symlink case must fail
without creating or replacing the external target.

**Verify**: `cargo test -p cad-desktop --all-features` passes and no test writes
outside its temporary project directory.

## Test plan

- Model the tests after existing tempfile-based atomic edit tests in
  `crates/cad-edit/src/lib.rs` and Tauri unit tests in
  `apps/viewer/src-tauri/src/lib.rs`.
- Cover concurrent revision change, no-replace output race, permissions,
  CRLF/trailing newline preservation, history commit failure rollback, and
  symlink containment.
- Run the full commands in the Commands table after all steps.

## Done criteria

- [x] All project-internal writers use one tested atomic publish primitive.
- [x] No `staging.persist` or direct `fs::write` path can overwrite a source or
      generated destination without the requested revision/overwrite policy.
- [x] Symlinked project `build` paths are rejected before any write.
- [x] `cargo fmt`, clippy, workspace tests, and `git diff --check` pass.
- [x] No files outside Scope are modified by the verification flow; the
      broader implementation changes are tracked in Plans 002–004.

## STOP conditions

- The existing atomic helper cannot provide no-replace semantics on a target
  platform without adding an unapproved dependency; stop and report the API
  limitation.
- A test requires modifying a real project outside a temporary directory.
- Preserving the existing public `overwrite` behavior would require changing a
  Tauri or CLI wire field.

## Maintenance notes

Every new file-writing feature must call the shared primitive and add a
same-directory failure test. Reviewers should inspect both the preflight
revision check and the final publish operation; a separate `exists()` check is
not sufficient for no-replace semantics.

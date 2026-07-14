# Plan 003: Add a real desktop verification gate and reproducible CI

> **Executor instructions**: This plan adds verification and tooling only. Do
> not weaken tests to accommodate the current sandbox Chromium failure; mark
> that environment limitation separately if it remains.
>
> **Drift check (run first)**: `git diff --stat cc8694e..HEAD -- apps/viewer/tests apps/viewer/playwright.config.ts apps/viewer/package.json scripts/ci-local.sh .github rust-toolchain.toml`.

## Status

- **Implementation**: COMPLETED (2026-07-14)
- **Priority**: P1
- **Effort**: M-L
- **Risk**: MED
- **Depends on**: Plans 001 and 002
- **Category**: tests, dx
- **Planned at**: commit `cc8694e`, 2026-07-13

## Why this matters

The browser E2E suite exercises the static viewer, while desktop editing,
history, watcher sequencing, atomic publish, and PDF export need a real-file
smoke. A green static test run alone does not prove the product's main desktop
workflows.

## Current state

- `apps/viewer/tests/e2e/viewer.spec.ts` covers static SVG display, zoom, and
  layer interactions; it does not start Tauri.
- `apps/viewer/tests/unit/desktop-layers.spec.ts` checks PDF invoke arguments,
  not a real PDF file or revision conflict.
- `scripts/ci-local.sh` runs `pnpm install` and Playwright installation,
  `cargo test --workspace` without `--all-features`, and no PDF command path.
- `desktop:smoke` now runs a temporary-project Rust/Tauri backend smoke that
  asserts real file bytes and conflict behavior.
- `.github/workflows/ci.yml` contains Rust, viewer, and desktop-smoke jobs.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Web build | `pnpm --dir apps/viewer build` | exit 0 |
| Unit | `pnpm --dir apps/viewer test` | all runnable tests pass |
| Static E2E | `pnpm --dir apps/viewer test:e2e` | all pass |
| Rust gate | `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features` | all pass |

## Scope

**In scope**

- `apps/viewer/tests/e2e/` and test harness/configuration
- `apps/viewer/package.json` scripts
- `scripts/ci-local.sh`
- `.github/workflows/ci.yml` (create)
- `rust-toolchain.toml` and documented Node/pnpm/browser preflight
- Minimal transport seams required to inject a real temporary-project backend

**Out of scope**

- Production feature behavior unrelated to testability
- Cloud CI credentials or deployment
- Windows automation pretending to be Jw_cad compatibility

## Steps

### Step 1: Make local gates deterministic

Split the local script into fast Rust, viewer unit, static E2E, and desktop
smoke targets. Use frozen dependency installation, preflight checks for the
required toolchain/browser, and `--all-features`. Do not regenerate tracked
fixtures; write all outputs under temporary or ignored build directories.

**Verify**: each script can run independently from a clean checkout and exits
0 without modifying tracked files (`git status --short`).

### Step 2: Add a desktop smoke harness

Use the Tauri backend test profile backed by a temporary CAD project. The
`desktop_file_smoke_covers_edit_comment_layer_history_and_pdf_conflicts` test
covers open project, edit, comment, layer update, review, Undo, Redo, PDF
export, external revision conflict, and output overwrite conflict. It asserts
actual file bytes and state, not only invoke args. Drawing switching during an
async request remains covered by the guarded TypeScript unit tests; a full
macOS GUI launch remains a release-gate smoke.

**Verify**: `pnpm --dir apps/viewer desktop:smoke` runs the backend smoke and
leaves no files outside the test temp directory.

### Step 3: Add CI workflow and toolchain pinning

Create a workflow with cached Rust/pnpm dependencies and separate jobs for
Rust, viewer unit/build, static E2E, and desktop smoke. Pin Rust through
`rust-toolchain.toml`; document minimum Node/pnpm and browser versions. Keep
the macOS desktop smoke as a required release job if the hosted environment
cannot run Tauri reliably on Linux.

**Verify**: CI invokes exactly the commands in this plan and fails when a
  command or fixture is missing; local dry-run scripts remain green.

## Done criteria

- [x] Desktop smoke exercises real file mutation and conflict paths.
- [x] CI exists and covers all-features Rust plus viewer build/unit/E2E.
- [x] Local verification does not install or mutate dependencies on every run.
- [x] Toolchain and browser prerequisites are explicit and pinned.
- [x] `git status --short` is unchanged after verification except ignored output.

## STOP conditions

- The full Tauri GUI runtime cannot be launched in CI without a platform-specific
  runner; keep the macOS GUI smoke as a release gate instead of weakening the
  backend smoke.
- A proposed test requires weakening production revision or atomicity checks.
- The existing Chromium sandbox failure cannot be isolated from the test
  harness; report it as an environment prerequisite instead.

## Maintenance notes

Every new Tauri command needs one real-file smoke case and one stale/conflict
case where applicable. Keep static viewer tests separate from desktop tests so
their failures identify the correct layer.

# Plan 004: Establish one canonical layout and source contract

> **Executor instructions**: This is a contract cleanup, not a migration
> project. The product is still in development and schema `0.2` is the only
> supported schema. Do not add automatic legacy migration or dual-runtime
> fallback.
>
> **Drift check (run first)**: `git diff --stat cc8694e..HEAD -- ADR.md README.md plan.md crates/cad-model/src/lib.rs crates/cad-check/src/lib.rs crates/cad-diff/src/lib.rs crates/cad-render-svg/src/lib.rs crates/cad-render-pdf/src/lib.rs crates/cad-export-jww/src/lib.rs`.

## Status

- **Implementation**: COMPLETED (2026-07-14)
- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: Plan 001
- **Category**: bug, docs, contract
- **Planned at**: commit `cc8694e`, 2026-07-13

## Why this matters

The model previously loaded both `sheet.toml` and `layouts.toml`, which allowed
paper calculations to diverge between check/diff and output renderers. The
source contract now has one layout authority and the design documents describe
the current schema `0.2` behavior.

## Current state

- `cad-model` loads only `layouts.toml`; checker, diff, SVG, PDF, and JWW all
  consume the active layout.
- Schema `0.2` fixtures and import output no longer create a second paper file.
- `ADR.md`, `README.md`, and `plan.md` state the current contract.

## Scope

**In scope**

- `cad-model`, `cad-check`, `cad-diff`, SVG/PDF/JWW render/export code
- schema 0.2 fixtures and tests
- `ADR.md`, `README.md`, and `plan.md`

**Out of scope**

- Supporting schema 0.1
- `cadc migrate` or automatic sheet-to-layout conversion
- PDF style fidelity and JWW file-level compatibility validation

## Steps

### Step 1: Make active layout the only paper calculation

Use the active layout for checker, diff, SVG, PDF, and JWW render/export code.
`sheet.toml` is not part of schema `0.2` and is neither read nor generated.

**Verify**: use a fixture with multiple layouts and a non-default active layout;
check, diff, SVG, PDF, and JWW must all report/use that active layout result.

### Step 2: Update schema 0.2 fixtures and source allowlist

Remove obsolete sheet assumptions from generated examples and tests. Ensure
watcher/source manifests include every canonical block, hatch, layout, comment,
layer, and entity file exactly once, while `.cad-history` and build outputs are
excluded.

**Verify**: `cargo test --workspace --all-features` and watcher path contract
tests pass; no test requires schema 0.1.

### Step 3: Synchronize design documents

Update ADR, README, and `plan.md` to state: schema 0.2 only, layouts.toml is
canonical, blocks are preserved by default, PDF/JWW are implemented but JWW
remains limited to the recorded fixture corpus, and the next hardening plans
are the release gates in `plans/README.md`. Remove contradictory MVP statements
instead of adding more superseding paragraphs.

**Verify**: `rg -n "sheet.toml|schema 0.1|cadc migrate" ADR.md README.md plan.md`
returns no obsolete runtime contract, and the documented CLI commands match
`cadc --help`.

## Done criteria

- [x] One active-layout calculation drives all paper/outside-page decisions.
- [x] Schema 0.2 fixtures no longer require a second paper source.
- [x] Source filtering has one canonical contract and excludes generated history.
- [x] ADR/README/plan agree on current behavior and release gates.
- [x] No legacy migration or compatibility code is added.

## STOP conditions

- A new external consumer requires a paper source other than active layouts;
  stop and record the contract change instead of adding fallback loading.
- A renderer/exporter requires materially different paper semantics; stop and
  document the discrepancy before changing the shared calculation.

## Maintenance notes

Any new output format must consume the shared layout calculation and add a
fixture where active layout differs from stale metadata. Documentation changes
must update the canonical phase/status table rather than appending another
historical phase description.

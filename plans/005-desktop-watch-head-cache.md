# Plan 005: Reduce desktop watcher and Git HEAD review overhead

> **Executor instructions**: Preserve the existing invoke/event contracts and
> source-safety invariants. This plan changes backend selection and snapshot
> reuse only; it must not add frontend resynchronization logic or broaden the
> canonical source set.

## Status

- **Implementation**: COMPLETED (2026-07-15)
- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: Plans 001–004
- **Category**: performance, desktop, maintainability

## Why this matters

The desktop previously scanned the complete project every 100 ms with a
content-comparing `PollWatcher`. Review also expanded every tracked HEAD file
for each run. Both costs were paid while idle or while repeatedly reviewing an
unchanged commit.

## Current state

- The desktop recursively registers raw `notify::RecommendedWatcher` events.
  A 250 ms worker batch preserves pathless rescan signals and source-directory
  rename/delete events, then derives exact paths from the canonical manifest.
- If native watcher creation or initial registration fails, a dedicated loop
  compares `cad_model::source_manifest` every 1,000 ms. Runtime native errors
  remain visible error events and do not trigger backend switching.
- The fallback keeps its last valid manifest after an error, reports one error
  per continuous failure, and resumes change reporting after recovery.
- Native and fallback monitoring share the same last-good manifest state.
  Native runtime errors stay on the native backend and the next healthy source
  event performs a catch-up review.
- A managed one-entry HEAD cache is keyed by canonical repository root,
  canonical project root, and resolved commit OID. The cached `ProjectSource`
  retains its temporary extraction directory.
- HEAD expansion asks Git only for canonical source files. An unchanged OID
  reuses the parsed snapshot even when the working tree changes.
- Watcher and HEAD snapshot implementation live in `project_watch.rs` and
  `head_snapshot.rs`; `lib.rs` retains command and event wiring.

## Scope

**In scope**

- Native watcher selection and canonical manifest polling fallback
- Canonical source event filtering and fallback recovery
- One-project/one-OID HEAD snapshot cache
- Deterministic backend-selection and counting Git runner tests
- Tauri backend module separation

**Out of scope**

- Native runtime-error failover
- Frontend event/sequence contract changes
- Checker spatial indexing and general Rust/UI decomposition
- PDF style fidelity and JWW file-level compatibility validation

## Done criteria

- [x] Native recursive watching is the primary backend.
- [x] Rescan, pathless, and canonical source-directory events cannot be dropped.
- [x] Native creation/registration failure selects the 1,000 ms manifest fallback.
- [x] Fallback reports canonical additions, updates, and deletions and ignores generated files.
- [x] Manifest errors preserve the last valid snapshot and monitoring resumes after recovery.
- [x] Native runtime errors notify the frontend and recover without backend failover.
- [x] Repeated review of one OID avoids repeated `ls-tree`, `show`, and project parsing.
- [x] Working-tree changes retain a cache hit; OID or project changes replace the cache entry.
- [x] Failed HEAD snapshots are not cached.
- [x] Existing Tauri invoke and `cad-project-watch` event shapes are unchanged.

## Verification

Run the workspace Rust gates, viewer type/build/unit/static E2E gates, backend
desktop smoke, and macOS embedded-WebDriver desktop E2E. Tests assert backend
selection and Git command counts rather than using elapsed-time thresholds.
Raw native events and manifest batches are exercised through injected,
sleep-free state transitions rather than platform event timing.

## Maintenance notes

New canonical source kinds must be added only to `cad-model`; both watcher
backends and HEAD extraction inherit that classification. Expanding the cache
beyond one project/OID requires an explicit lifetime and memory policy because
each entry owns a parsed project and temporary filesystem tree.

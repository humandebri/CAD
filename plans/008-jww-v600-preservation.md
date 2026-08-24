# Plan 008: Preserve JWW v600 originals and fail closed on incompatible edits

## Status

- **Implementation**: IN PROGRESS (file-level fixture corpus expansion pending)
- **Priority**: P1
- **Risk**: HIGH
- **Depends on**: Plans 001, 004, and 007
- **Category**: JWW compatibility, data preservation, desktop workflow

## Contract

- JWW is an interchange format, not the canonical project source.
- Only version 600 files decoded by the shared Rust codec are editable.
- Every import stores the exact input at `interop/jww/original.jww` and records
  BLAKE3, SHA-256, compatibility state, drawing name, and source revisions in
  `interop/jww/preservation.toml`.
- Unknown classes and unsupported versions are retained as read-only projects;
  their bytes can always be extracted, but source mutation is rejected.
- An unchanged editable import exports the original bytes exactly.
- An edited preserve export retains the original v600 header and replaces only
  the decoded archive when every affected record is representable without an
  approximation. Text, dimension, hatch, layout, and palette edits currently
  fail closed because the schema does not retain every original record field.
- Generated and hybrid JWW export are measured against a hashed v600 fixture
  corpus. This project does not claim compatibility outside recorded classes.

## Done criteria

- [x] Inspect compatibility before import and expose it through CLI JSON.
- [x] Preserve the exact original and cryptographic provenance in the project.
- [x] Open unknown/unsupported input as a read-only preservation project.
- [x] Reject source edits when provenance is read-only or invalid.
- [x] Extract the exact original through CLI and Desktop.
- [x] Export an unchanged import byte-for-byte through CLI and Desktop.
- [x] Preserve the original v600 header for the restricted edited path.
- [x] Reject edited records whose unmodeled fields could be lost.
- [x] Display compatibility state and safe actions in the macOS Desktop UI.
- [x] Record fixture SHA-256 and decoded record inventory in a machine-readable manifest.
- [ ] Add licensed dimension, solid, block, hatch, and palette fixtures.
- [ ] Promote editable record types only after byte-exact and semantic re-import checks pass.

## Remaining compatibility work

- Per-record raw provenance for editable text, dimension, hatch, and block
  metadata.
- Jw_cad controls and semantics not present in schema 0.2, including settings,
  attributes, image/OLE/plugin records, and version-specific extensions.
- Licensed solid/block fixtures and independent file-level expected records.

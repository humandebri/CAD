# Plan 009: Preserve mapped JWW v600 record fields across edits

Status: IN PROGRESS — best-effort hybrid export implemented; fixture coverage pending.

## Contract

- The imported JWW remains available byte-for-byte at
  `interop/jww/original.jww`.
- Preservation manifest schema `0.2` records whether an import is
  `mapped_v600` or `exact_only` and verifies `records.ndjson` with BLAKE3 and
  SHA-256. Schema `0.1`, missing provenance, and modified provenance fail
  closed to `exact_only`.
- `records.ndjson` binds a CAD entity ID to its canonical imported JSON and
  complete writable JWW record. It retains text end/type, dimension SXF and
  auxiliary records, and block-definition base/reference/reserved metadata.
- Unchanged records are reused. Compatible one-record edits patch modeled
  fields while preserving record-specific metadata. Existing mapped dimensions
  may change only their displayed value; their line, text geometry, SXF data,
  auxiliary lines, and auxiliary points remain byte-for-byte equivalent.
- Original and record-provenance bytes are read through one verified snapshot.
  The canonical source manifest is compared again immediately before publish,
  and a revision conflict leaves the destination unchanged.
- Preserve export runs the canonical checker and validates every block reference
  against the final archive. Missing or duplicate definitions, block-number
  exhaustion, and preserved block base-point changes fail closed.
- Normal best-effort export allows one CAD entity to expand into multiple JWW
  records and falls back to generated v600 with an explicit warning when header
  edits cannot be merged. `--strict` retains the fail-closed behavior.

## Completion gate

- Rust codec/import/model/export and Desktop tests pass on macOS.
- Record provenance tampering is rejected.
- Text edits and dimension value-only edits retain their v600-only fields after
  export and re-import. Existing dimension geometry edits remain blockers until
  a complete endpoint/auxiliary-record mapping is implemented.
- Original/provenance tampering, source races, dangling block references, and
  unsupported preserved block changes are rejected without replacing output.
- The hashed corpus records class counts and validates unchanged bytes plus
  edited semantic re-import for line/arc/text/dimension/point/solid/block before
  the covered set is considered complete.

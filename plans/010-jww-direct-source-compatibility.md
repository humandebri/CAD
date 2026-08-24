# Plan 010: Direct AI source editing and JWW v600 file compatibility

Status: IN PROGRESS

## Contract

- AI agents edit canonical schema 0.2 TOML and NDJSON directly. Generated,
  history, transaction, recovery, and JWW provenance files are not source.
- `cadc check --target cad` validates direct edits. `--target jww-v600` adds
  deterministic approximation and substitution diagnostics.
- Normal JWW export is best-effort. `--strict` promotes approximations to
  blockers. Every best-effort difference is present in the JSON report.
- Imported provenance is selected automatically. Unchanged files remain
  byte-exact; edited entities can replace one record with multiple records.
- Compatibility claims are limited to the hashed fixture manifest and semantic
  re-import assertions. Windows is not a release gate.

## Completion gate

- [x] Document direct-source agent rules and source examples.
- [x] Add CAD/JWW target checker selection and stdout JSON.
- [x] Add automatic best-effort hybrid export and explicit strict mode.
- [x] Support one-to-many edited record replacement and report it.
- [x] Publish a hashed fixture manifest with decoded record inventory.
- [ ] Add licensed solid, dimension, block, hatch, and palette fixtures.
- [ ] Pass the complete workspace and Desktop verification gates.

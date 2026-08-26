# AGENTS.md

## CAD source of truth

- JWW is an import/export boundary. After import, edit the canonical TOML and NDJSON project files directly.
- Editable source paths are limited to `cad.project.toml`, `rules/*.toml`, `drawings/*/layouts.toml`, `drawings/*/entities.ndjson`, `blocks/*/definition.toml`, and `blocks/*/entities.ndjson`.
- `comments/*.ndjson` is app-managed canonical project data, but is not part of the AI direct-edit surface.
- Do not edit `interop/`, JWW provenance, `build/`, `.cad-history`, transaction journals, recovery data, or generated PDF/SVG/JWW files as source.
- Keep an existing entity `id` when updating it. Assign a new unique valid entity ID only when creating an entity. Delete an entity by deleting its complete NDJSON line.
- Keep NDJSON as exactly one JSON entity per non-empty line. Preserve valid layer, pen, style, layout, block, and fill references.
- Direct source edits are external edits and do not enter Desktop Undo/Redo history. Use Git diff/history to review and recover them.

## Required validation

- After every direct source edit batch, run:

  ```bash
  cargo run -p cad-cli -- check <PROJECT> --target cad --format json --out -
  ```

- Before JWW export, run:

  ```bash
  cargo run -p cad-cli -- check <PROJECT> --drawing <NAME> --target jww-v600 --format json --out -
  ```

- Treat checker errors as blockers. Review every JWW warning because it describes geometry expansion, style approximation, or text substitution.
- Export JWW with a JSON report. Do not claim exact or lossless compatibility when the report contains warnings or when the fixture manifest does not cover the emitted record classes.
- Never hide, delete, or rewrite a compatibility report merely to make a validation pass.

## JWW export policy

- Normal `cadc export-jww` is best-effort and preserves visual results through deterministic v600 approximations where possible.
- Use `--strict` when approximations and substitutions must block publication.
- An unchanged compatible import must export the original bytes exactly. Edited imports should retain the original header and untouched records whenever provenance permits.
- Unknown record classes whose binary boundaries cannot be proven remain exact-original-only; never guess their length or silently omit them.

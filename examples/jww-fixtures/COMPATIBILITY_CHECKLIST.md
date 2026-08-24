# JWW Compatibility Checklist

Use `manifest.json` as the canonical fixture inventory. Optional external
application evidence may use `validation/record-template.md` and be stored
under `validation/<yyyy-mm-dd>-<fixture>/`.

The automated Rust tests cover codec parsing/writing, the public `Test1.jww`
fixture, synthetic solids, strict blockers, best-effort reports, and
import/export/re-import validation. Required file-level checks are:

- [ ] Record fixture SHA-256, JWW version, record class counts, and block count.
- [ ] Confirm codec inspection matches `manifest.json`.
- [ ] Verify line, arc, circle, ellipse, text, dimension, point, solid, and block semantics after re-import.
- [ ] Verify layer groups, palette, print widths, paper, and scale after re-import.
- [ ] Confirm an unchanged automatic provenance export has the same SHA-256 as the input.
- [ ] Directly edit canonical source, run both checker targets, export, and re-import.
- [ ] Confirm every expansion, approximation, substitution, or omission appears in the JSON report.
- [ ] Confirm strict blockers and revision conflicts leave an existing output unchanged.

For every failure, record the smallest reproducible fixture, entity type,
layer, coordinates, JWW record/class, and the failing stage (codec, import,
export, or re-import). Convert confirmed regressions into Rust tests before
expanding the claimed compatibility set.

Compatibility claims must name the covered fixture and record inventory until
every required class has licensed fixture coverage.
